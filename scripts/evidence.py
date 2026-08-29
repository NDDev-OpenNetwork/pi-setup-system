#!/usr/bin/env python3
"""Drive this build through the whole software lifecycle, against real bytes.

Everything else in this repository proves the program is internally consistent:
the unit tests, the lifecycle probe, the surfaces check, the boundary job. None
of them ever meets a vendor. This does.

    plan software_install   -> the exact bytes, named offline
    fetch                   -> whoever holds the network, which here is CI
    apply                   -> verified against the digest, with the network gone
    software                -> which version is installed, and what runs it
    launch                  -> the real product starts, pointed at --target

and where a product is known to write into a surface this provider owns, two
more steps that are the whole point:

    the product writes      -> a home this repository did not create
    install / restore       -> byte-exact against what the product left

That last pair is the gap every Windows defect this project shipped lived in.
The three-OS matrix proved the code compiled and its tests passed; the probe
proved a target survives a round trip. Neither ever ran against a directory a
real product wrote.

This is not a gate and must not become one. It reaches a vendor's registry, and
a repository gate that depends on an external environment reports someone
else's outage as your failure. It runs on a schedule and on demand, and a red
result here is information about the world rather than about this commit.

Usage:

    evidence.py --binary <path> --harness <id> [--writes "<argv>"]

`--writes` is one shell-quoted argument list handed to `launch` to make the
product write its own configuration -- measured per product, absent where no
credential-free command does it.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

# A plan carries an expiry in exactly this shape, and refuses any other. The
# value only has to be in the future: nothing here is racing a deadline.
EXPIRES_AT = "2099-01-01T00:00:00.000Z"
OPERATION_ID = "operation_" + "0" * 23 + "1"


def contained(room: Path) -> dict[str, str]:
    """The environment a launched product gets, with nowhere to write but here.

    A product started through `launch` is pointed at `--target` by the variable
    its own documentation names. That governs where it *reads* its
    configuration; it does not govern everything it writes. Measured: opencode
    reads from the target and writes the global
    `~/.config/opencode/opencode.json` whatever the variable says, and creates a
    `.gitignore` in that directory on start.

    In CI the home is an ephemeral runner and none of that matters. Run on the
    machine of whoever is developing this, it reaches into their real
    configuration -- which it did, repeatedly, before this existed. A tool that
    verifies a provider must not be the thing that edits your home.

    So the product gets a `HOME` of its own, and the two variables the XDG
    convention uses, all inside the scratch directory this run already deletes.
    """
    home = room / "home"
    (home / ".config").mkdir(parents=True, exist_ok=True)
    (home / ".local" / "share").mkdir(parents=True, exist_ok=True)
    environment = dict(os.environ)
    environment.update(
        {
            "HOME": str(home),
            "USERPROFILE": str(home),
            "XDG_CONFIG_HOME": str(home / ".config"),
            "XDG_DATA_HOME": str(home / ".local" / "share"),
        }
    )
    return environment


class Failed(Exception):
    """One step did not do what it said. The message is the report."""


class NothingToProve(Exception):
    """The provider declined for a reason that is true, so there is nothing here.

    Distinct from `Failed` on purpose: a vendor that publishes no build for a
    platform is not a defect, and a job that cannot tell the two apart teaches
    people to ignore its reds.
    """


def run_text(argv: list[str]) -> str:
    """Run one provider command that answers a person, and return what it said."""
    done = subprocess.run(argv, capture_output=True, text=True)
    if done.returncode != 0:
        raise Failed(f"{argv[1]} exited {done.returncode}: {done.stderr.strip()}")
    return done.stdout


def run_json(argv: list[str]) -> dict:
    """Run one provider command that answers a machine, and parse its envelope."""
    done = subprocess.run(argv, capture_output=True, text=True)
    try:
        answer = json.loads(done.stdout)
    except json.JSONDecodeError:
        raise Failed(
            f"{argv[1]} printed no JSON (exit {done.returncode}):\n"
            f"  stdout: {done.stdout[:400]!r}\n  stderr: {done.stderr[:400]!r}"
        ) from None
    if not isinstance(answer, dict):
        raise Failed(f"{argv[1]} printed {type(answer).__name__}, not an envelope")
    return answer


def digest_of(path: Path) -> str:
    reader = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            reader.update(block)
    return "sha256:" + reader.hexdigest()


def tree_digests(root: Path, skip: str) -> dict[str, str]:
    """Every file under `root`, by relative path, excluding this provider's own."""
    found = {}
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        relative = path.relative_to(root).as_posix()
        if skip in relative:
            continue
        found[relative] = digest_of(path)
    return found


def plan(
    binary: str,
    target: Path,
    prefix: Path,
    operation: str,
    nonce: int,
    version: str = "",
) -> dict:
    info = run_json([binary, "provider-info"])
    named = ["--software-version", version] if version else []
    answer = run_json(
        [
            binary,
            "plan-operation",
            "--target", str(target),
            "--prefix", str(prefix),
            "--json",
            "--operation", operation,
            "--provider-release-digest", info["provider_build_digest"],
            "--operation-id", "operation_" + f"{nonce:024x}",
            "--expires-at", EXPIRES_AT,
            *named,
        ]
    )
    if answer.get("reason") == "unsupported_platform":
        # An honest answer, not a failure. Cursor publishes no Windows build,
        # and the provider says so by name rather than planning something it
        # could not apply. Treating that as a red would make this job report a
        # vendor's product range as a defect of ours.
        raise NothingToProve(str(answer.get("detail", "")))
    if answer.get("state") != "planned":
        raise Failed(
            f"plan-operation refused: {answer.get('reason')} {answer.get('detail')}"
        )
    return answer


def fetch(url: str, into: Path, expect_bytes: int, expect_digest: str) -> None:
    """Download the exact artifact the plan named, and check it before using it.

    The provider verifies this again during apply. Checking here as well is not
    redundant: a mismatch at this point says the registry moved, and a mismatch
    inside apply says the same thing while looking like a provider defect.
    """
    # A User-Agent that names this job, because the default one is refused.
    #
    # `urllib` sends `Python-urllib/3.x`, and `downloads.cursor.com` answers
    # that with **403 Forbidden** while serving the identical URL to `curl`.
    # Measured: default agent 403, `curl` 200, `urllib` with a real agent 200.
    #
    # This reached CI rather than a local run for a reason worth naming: every
    # time I fetched a pinned artifact by hand I used `curl`, and the script
    # uses `urllib`. The tool I measured with and the tool that ships were
    # different tools, so the thing that ships had never been tried against six
    # of the seven vendors. Naming the job is also the more polite thing to
    # send a stranger's CDN.
    request = urllib.request.Request(
        url,
        headers={
            "User-Agent": (
                "nddev-setup-system-evidence/1 "
                "(+https://github.com/NDDev-OpenNetwork)"
            )
        },
    )
    with urllib.request.urlopen(request) as response, into.open("wb") as handle:
        shutil.copyfileobj(response, handle)
    size = into.stat().st_size
    if size != expect_bytes:
        raise Failed(
            f"the registry served {size} bytes and the pin says {expect_bytes}; "
            "the artifact behind this URL has moved"
        )
    got = digest_of(into)
    if got != expect_digest:
        raise Failed(
            f"the registry served {got} and the pin says {expect_digest}; "
            "the bytes behind this URL are not the pinned ones"
        )


def cross_two_releases(
    binary: str, target: Path, prefix: Path, room: Path, info: dict
) -> None:
    """Move a real product between two real releases, and back.

    The two operations this file could not reach. Both are declared by every
    build that installs a program, and until a second version was pinned
    neither had ever crossed two trees: an update needs a version to come from,
    a rollback a tree to return to.

    The pair is deliberately two *consecutive vendor releases* rather than a
    fabricated one. They differ in whatever the vendor actually changed, so
    what runs here is the transition a person really performs -- and a
    fabricated pair would prove the plumbing against a case nobody runs.

    Skipped, with the reason printed, for a build that has not been bumped
    since it was pinned. That is an absence of a second release rather than a
    failure, and saying which is the whole difference.
    """
    earlier = previous_version(binary, target, prefix)
    if not earlier:
        print(
            "update -> this build names one version, so there is no transition "
            "to cross yet; it appears here on the next bump"
        )
        return

    # Start from the earlier release, because an update of nothing is refused
    # and rightly: installing instead would be doing something else.
    print(f"back  -> starting from {earlier}, the release before the pin")
    place(binary, target, prefix, room, info, "software_install", 3, earlier)

    print("update", end="", flush=True)
    updated = place(binary, target, prefix, room, info, "software_update", 4)
    print(f"-> {earlier} to {updated['version']}, both trees kept")

    said = run_text([binary, "software", "--prefix", str(prefix)])
    for version in (earlier, updated["version"]):
        if version not in said:
            raise Failed(
                f"after the update the prefix should hold {earlier} and "
                f"{updated['version']}; `software` said:\n{said}"
            )

    print("roll  ", end="", flush=True)
    back = run_text(
        [binary, "rollback", "--to", earlier, "--prefix", str(prefix)]
    )
    if earlier not in back:
        raise Failed(f"rollback did not say it moved to {earlier}:\n{back}")
    now = run_text([binary, "software", "--prefix", str(prefix)])
    if f"runs {earlier}" not in now:
        raise Failed(
            f"rollback answered but the prefix still runs something else:\n{now}"
        )
    print(f"-> back on {earlier}, and {updated['version']} is still there")

    # Forward again: a move that only goes one way is half an operation.
    run_text(
        [binary, "rollback", "--to", updated["version"], "--prefix", str(prefix)]
    )
    forward = run_text([binary, "software", "--prefix", str(prefix)])
    if f"runs {updated['version']}" not in forward:
        raise Failed(f"the command did not move forward again:\n{forward}")
    print(f"      -> and forward to {updated['version']} again")


def previous_version(binary: str, target: Path, prefix: Path) -> str:
    """The release before the pinned one, asked of the build rather than a file.

    Read from the provider's own refusal, which names every version it can
    install. That keeps this script from carrying a second copy of a fact the
    binary already states -- the copy that eventually disagrees.
    """
    answer = run_json(
        [
            binary,
            "plan-operation",
            "--target", str(target),
            "--prefix", str(prefix),
            "--json",
            "--operation", "software_install",
            "--provider-release-digest",
            run_json([binary, "provider-info"])["provider_build_digest"],
            "--operation-id", "operation_" + f"{9:024x}",
            "--expires-at", EXPIRES_AT,
            "--software-version", "0.0.0-not-a-release",
        ]
    )
    named = re.search(r"names \S+ ([^;]+);", str(answer.get("detail", "")))
    if not named:
        return ""
    versions = [part.strip() for part in named.group(1).split(" and ")]
    return versions[1] if len(versions) > 1 else ""


def place(
    binary: str,
    target: Path,
    prefix: Path,
    room: Path,
    info: dict,
    operation: str,
    nonce: int,
    version: str = "",
) -> dict:
    """Plan, fetch and apply one software operation, and answer what it did."""
    planned = plan(binary, target, prefix, operation, nonce, version)
    artifact = planned["plan"]["software_artifacts"][0]
    blob = room / f"artifact-{nonce}"
    fetch(artifact["url"], blob, artifact["byte_length"], artifact["sha256"])
    body = room / f"plan-{nonce}.json"
    body.write_text(
        json.dumps(planned["plan"], separators=(",", ":"), sort_keys=True),
        encoding="utf-8",
    )
    applied = run_json(
        [
            binary,
            "apply-operation",
            "--target", str(target),
            "--prefix", str(prefix),
            "--json",
            "--plan", str(body),
            "--plan-digest", planned["plan_digest"],
            "--provider-release-digest", info["provider_build_digest"],
            "--software-artifact", str(blob),
        ]
    )
    if applied.get("state") != "verified":
        raise Failed(
            f"{operation} answered {applied.get('state')}: "
            f"{applied.get('reason')} {applied.get('detail')}"
        )
    blob.unlink(missing_ok=True)
    return applied


def remove_the_program(binary: str, target: Path, prefix: Path, info: dict) -> None:
    """Take the product back off, and prove the prefix says so.

    `software_remove` is declared by every build here and had never met a real
    vendor's bytes on any platform: the job installed and started a product and
    then left it there. An operation that is declared and never exercised is a
    promise nobody has read back.

    `software_update` and `rollback` now run here too, in
    [`cross_two_releases`], for harnesses that name a second version. Until one
    did, the reason they were absent was a measurement rather than an omission:
    a build pinning one version has nothing for an update to move *from* and
    nothing for a rollback to return *to*.
    """
    print("remove", end="", flush=True)
    planned = plan(binary, target, prefix, "software_remove", 2)
    body = prefix.parent / "remove.json"
    body.write_text(
        json.dumps(planned["plan"], separators=(",", ":"), sort_keys=True),
        encoding="utf-8",
    )
    applied = run_json(
        [
            binary,
            "apply-operation",
            "--target", str(target),
            "--prefix", str(prefix),
            "--json",
            "--plan", str(body),
            "--plan-digest", planned["plan_digest"],
            "--provider-release-digest", info["provider_build_digest"],
        ]
    )
    if applied.get("state") != "verified" or not applied.get("removed"):
        raise Failed(
            f"software_remove answered {applied.get('state')} removed="
            f"{applied.get('removed')}: {applied.get('detail')}"
        )
    said = run_text([binary, "software", "--prefix", str(prefix)])
    # What `software_remove` promises is precise, and the second pin is what
    # made the difference visible: it takes *the version this build pins* and
    # the exposed command, and deliberately leaves any other tree alone --
    # "this build pins X and does not decide about versions it does not pin",
    # in the plan's own words. Until a second version existed the prefix always
    # ended up empty, so an assertion that it was empty passed for a reason
    # that was about the fixture rather than about the operation.
    if applied["version"] in said:
        raise Failed(
            f"{applied['version']} was removed and `software` still lists it:\n{said}"
        )
    if "Nothing is exposed" not in said and "No version" not in said:
        raise Failed(
            f"the program was removed and a command is still exposed:\n{said}"
        )
    print(f"-> {applied['version']} taken off, and the prefix says so")


def run_the_probe(binary: str, target: Path, prefix: Path, probe: dict) -> None:
    """Run the probe, or say why there is none -- measured, not implied."""
    if not probe["argv"]:
        print(f"reads -> not asked: {probe['absent']}")
        return
    the_product_reads_our_setup(
        binary,
        target,
        prefix,
        probe["argv"],
        probe["kind"],
        probe["baseline"],
        probe["full_auto"],
    )


def the_product_reads_our_setup(
    binary: str,
    target: Path,
    prefix: Path,
    probe: list[str],
    kind: str,
    baseline: str,
    full_auto: str,
) -> None:
    """Ask the product what configuration it resolved, and check our setup is in it.

    Everything before this proves the product *starts*, pointed at our target.
    That is not the same as the product *reading what we installed*, and the gap
    is where every silently-wrong setup this estate has shipped lived: a
    permission key of the wrong shape at a correctly-owned path, a plugin one
    directory above where the product looks. Each installed, verified and
    restored cleanly, and changed nothing about the product.

    `postures` is the stronger form: the probe must say something *different*
    with `full-auto` installed than with `baseline`, which no constant can do.
    `reads` only asks the product to name our file at our target.
    """
    def ask(setup: str) -> str:
        run_text([binary, "select", setup, "--target", str(target)])
        started = subprocess.run(
            [binary, "launch", "--target", str(target), "--prefix", str(prefix),
             "--json", "--", *probe],
            capture_output=True,
            text=True,
            timeout=180,
            env=contained(target.parent),
        )
        return started.stdout + started.stderr

    print("reads ", end="", flush=True)
    said = ask("baseline")
    if kind != "postures":
        # `reads` names a file, and a file name is not a string comparison on
        # every filesystem this runs on.
        #
        # Grok accepts three spellings of its instruction document -- AGENTS.md,
        # Agents.md and AGENT.md -- which `grok-baseline.json` has recorded from
        # the vendor all along. On Linux the product must match the name on disk
        # and prints ours. On macOS and Windows the filesystem folds case, so the
        # product opens our AGENTS.md by searching for *its own* preferred
        # spelling and then prints the spelling it searched for. It read our
        # bytes on all three; only the rendering differed, and this assertion was
        # reading the rendering.
        #
        # So fold case here -- and, because the docstring above has always
        # claimed this checks the file is *at our target* while nothing checked
        # it, check that too. Not the whole path: Windows printed
        # `C:\Users\RUNNER~1\...`, the 8.3 short form, where the target here is
        # `C:\Users\runneradmin\...`. An assertion on the full path would fail
        # on one system for a reason that has nothing to do with the subject.
        # The last two components survive that, and are what the claim needs.
        folded = said.casefold()
        wanted = baseline.casefold()
        inside = f"{target.name}/{wanted}".casefold()
        if inside not in folded.replace("\\", "/"):
            raise Failed(
                f"with `baseline` installed the product did not report "
                f"{baseline!r} inside {target.name!r}; it said:\n{said[:600]}"
            )
        print(f"-> the product named {baseline!r} inside our target")
        return
    if baseline not in said:
        raise Failed(
            f"with `baseline` installed the product did not report "
            f"{baseline!r}; it said:\n{said[:600]}"
        )
    other = ask("full-auto")
    if full_auto not in other:
        raise Failed(
            f"with `full-auto` installed the product did not report "
            f"{full_auto!r}; it said:\n{other[:600]}"
        )
    if baseline in other:
        raise Failed(
            f"the product reported {baseline!r} under both postures, so this "
            "probe cannot tell which setup is installed"
        )
    print(f"-> {baseline!r} under baseline, {full_auto!r} under full-auto")



def owned_surfaces_named_by_the_product(binary: str, prefix: Path, info: dict) -> None:
    """Report which owned namespaces the installed product names in its own bytes.

    Every row in a baseline's `native_surfaces` carries an `evidence` value
    saying what exercised it -- `ran`, `bytes`, or `page` alone. A `page` row is
    not suspicious; it is **unfalsifiable from inside the repository**, which is
    a worse property, and this estate has shipped exactly one wrong fact of that
    kind: a manifest filename that had a citation, sat beside correct rows, and
    passed every check here.

    Thirty-one rows were moved off `page` by a person opening each artifact by
    hand, once per harness. This is that pass, mechanised, so it reruns on a
    schedule against the bytes actually installed instead of being redone.

    **It reports and never promotes.** Writing an `evidence` value still takes
    somebody recording what they measured, and `tools/derive_evidence.py --check`
    refuses a value stronger than a row's own prose supports. Two instruments
    with one opinion between them would be one instrument.

    **Both inputs are asked of their owner rather than copied.** The namespaces
    come from `provider-info` and the configuration home from the binary's own
    first lines -- so a declaration that moves cannot leave this measuring the
    old one. There is no baseline read here at all.

    **The control is not optional.** A namespace no product could own is
    searched for alongside the real ones. If it is found, the search is matching
    something other than what it means to and every hit in the run is worthless;
    if the real ones are all absent *and* the control is absent, that is a
    reading rather than a broken instrument. Without it, "everything was found"
    and "the search matches everything" look identical.
    """
    profile = info.get("projection_profile") or {}
    namespaces = list(profile.get("native_namespaces") or [])
    if not namespaces:
        print("surfaces -> this build declares none, so there is nothing to look for")
        return

    said = subprocess.run(
        [binary], capture_output=True, text=True, timeout=60, check=False
    ).stdout
    leaf = ""
    for line in said.splitlines():
        if "configuration home" in line.lower():
            # `Documented configuration home: ~/.claude (CLAUDE_CONFIG_DIR)`
            home = line.split(":", 1)[1].strip().split(" ")[0]
            leaf = home.rstrip("/").rsplit("/", 1)[-1]
            break

    # A name nothing can own, shaped like the others so it is searched the same
    # way. If this is ever found the run says so and reports nothing else.
    control = "nddev-no-product-owns-this"
    wanted: dict[str, list[str]] = {}
    for namespace in namespaces + [control]:
        forms = [f"{leaf}/{namespace}"] if leaf else []
        forms.append(namespace)
        wanted[namespace] = forms

    counts = {namespace: {form: 0 for form in forms} for namespace, forms in wanted.items()}
    needles = [
        (namespace, form, form.encode("utf-8"))
        for namespace, forms in wanted.items()
        for form in forms
    ]
    scanned = 0
    for path in sorted(prefix.rglob("*")):
        if not path.is_file() or path.is_symlink():
            continue
        try:
            blob = path.read_bytes()
        except OSError:
            continue
        scanned += 1
        for namespace, form, needle in needles:
            found = blob.count(needle)
            if found:
                counts[namespace][form] += found

    if any(counts[control].values()):
        print(
            f"surfaces -> REFUSED: the control {control!r} was found, so this "
            "search is matching something other than a path and every count in "
            "it is worthless"
        )
        return

    print(
        f"surfaces -> {scanned} files scanned under {leaf or 'no documented home'}, "
        "control absent as it must be"
    )
    anchored_found = 0
    for namespace in namespaces:
        anchor = f"{leaf}/{namespace}" if leaf else ""
        if anchor and counts[namespace].get(anchor):
            anchored_found += 1
            print(f"   {namespace:34} {counts[namespace][anchor]:6} x {anchor}")
            continue
        bare = counts[namespace].get(namespace, 0)
        if bare:
            # Deliberately not a count. `agents` appears 1649 times in one
            # product's bundle as identifiers, keys and prose, and printing that
            # number beside an anchored 89 invites reading the larger as the
            # stronger. A bare name is **not evidence of a route**: it is a
            # common word that happens to match, and the honest report of it is
            # that the anchored form was not found.
            print(
                f"   {namespace:34} only the bare name, which proves nothing: "
                "the product may join this to a directory at runtime, in which "
                "case no literal exists to find"
            )
        else:
            print(f"   {namespace:34} named nowhere in the installed bytes")
    print(
        f"   {anchored_found} of {len(namespaces)} anchored. Recording one as "
        "`bytes` still takes writing down what was measured; this says only what "
        "is there, and an absence argues nothing in either direction."
    )


def software_lifecycle(
    binary: str, harness: str, writes: list[str] | None, absent: str, probe: dict
) -> None:
    control = Path(binary).name
    # Made and removed by hand rather than with `TemporaryDirectory`, for one
    # measured reason. Grok holds `active_sessions.lock` open in its home, and
    # on Windows a locked file makes a removal raise -- which would turn a run
    # where every step passed into a failure during teardown, reported as though
    # the lifecycle had broken. `ignore_cleanup_errors` would say it too and
    # needs Python 3.10, which is a second thing to be right about on three
    # runner images. `ignore_errors` on the removal has always been there.
    scratch = tempfile.mkdtemp(prefix="evidence-")
    try:
        room = Path(scratch)
        target, prefix = room / "target", room / "prefix"
        target.mkdir()
        prefix.mkdir()

        print("plan  ", end="", flush=True)
        planned = plan(binary, target, prefix, "software_install", 1)
        artifacts = planned["plan"]["software_artifacts"]
        if len(artifacts) != 1:
            raise Failed(
                f"the plan names {len(artifacts)} artifacts; it filters to the "
                "running platform and should name exactly one"
            )
        artifact = artifacts[0]
        print(f"-> {artifact['platform']}, {artifact['byte_length']} bytes")

        print("fetch ", end="", flush=True)
        blob = room / "artifact"
        fetch(artifact["url"], blob, artifact["byte_length"], artifact["sha256"])
        print(f"-> {artifact['sha256'][:19]} matches the pin")

        print("apply ", end="", flush=True)
        body = room / "plan.json"
        body.write_text(
            json.dumps(planned["plan"], separators=(",", ":"), sort_keys=True),
            encoding="utf-8",
        )
        info = run_json([binary, "provider-info"])
        applied = run_json(
            [
                binary,
                "apply-operation",
                "--target", str(target),
                "--prefix", str(prefix),
                "--json",
                "--plan", str(body),
                "--plan-digest", planned["plan_digest"],
                "--provider-release-digest", info["provider_build_digest"],
                "--software-artifact", str(blob),
            ]
        )
        if applied.get("state") != "verified":
            raise Failed(
                f"apply-operation answered {applied.get('state')}: "
                f"{applied.get('reason')} {applied.get('detail')}"
            )
        print(f"-> verified, {applied['version']}, {applied['files']} files")

        owned_surfaces_named_by_the_product(binary, prefix, info)

        print("read  ", end="", flush=True)
        said = run_text([binary, "software", "--prefix", str(prefix)])
        if applied["version"] not in said:
            raise Failed(
                f"software does not report {applied['version']}; it said:\n{said}"
            )
        print(f"-> {applied['version']}")

        launches = "launch" in info["supported_commands"]
        if not launches:
            # Antigravity, and the refusal is the declaration keeping its word.
            print("launch -> not declared, so this build does not start a product")
            cross_two_releases(binary, target, prefix, room, info)
            remove_the_program(binary, target, prefix, info)
            return

        print("launch", end="", flush=True)
        started = subprocess.run(
            [
                binary, "launch",
                "--target", str(target),
                "--prefix", str(prefix),
                "--json", "--", "--version",
            ],
            capture_output=True,
            text=True,
            timeout=180,
            env=contained(room),
        )
        if started.returncode != 0:
            raise Failed(
                f"launch exited {started.returncode}: {started.stderr.strip()[:400]}"
            )
        first = (started.stdout.strip().splitlines() or [""])[-1]
        print(f"-> the product answered {first[:60]!r}")

        if not writes:
            # The reason comes from the caller, measured per product, rather
            # than from the absence of an argument. An empty `--writes` is a
            # condition about this invocation; it says nothing about the
            # product, and printing a claim about the product here would
            # exonerate one that had simply been left out.
            print(f"write -> not exercised: {absent}")
            run_the_probe(binary, target, prefix, probe)
            cross_two_releases(binary, target, prefix, room, info)
            remove_the_program(binary, target, prefix, info)
            return

        print("write ", end="", flush=True)
        wrote = subprocess.run(
            [
                binary, "launch",
                "--target", str(target),
                "--prefix", str(prefix),
                "--json", "--", *writes,
            ],
            capture_output=True,
            text=True,
            timeout=180,
            env=contained(room),
        )
        if wrote.returncode != 0:
            raise Failed(
                f"the product refused to write: exit {wrote.returncode}: "
                f"{wrote.stderr.strip()[:400]}"
            )
        everything = tree_digests(target, control)
        if not everything:
            raise Failed(
                f"{' '.join(writes)} left nothing in the target, so the step "
                "that follows would prove nothing"
            )
        # Only what the product wrote *inside a surface this provider owns*.
        #
        # Two reasons, and the second was measured rather than foreseen. The
        # claim under test is about owned surfaces: those are what a backup
        # captures and a restore returns, so an unowned file is outside it.
        # And Grok writes `active_sessions.lock` and `logs/unified.jsonl` into
        # its home -- files that change while the product runs. Comparing those
        # byte for byte would be a check that fails on a timestamp and reports
        # it as a broken restore.
        owned = set(info["projection_profile"]["native_namespaces"])
        theirs = {
            path: digest
            for path, digest in everything.items()
            if any(path == name or path.startswith(name + "/") for name in owned)
        }
        untouched = sorted(set(everything) - set(theirs))
        if not theirs:
            raise Failed(
                f"{' '.join(writes)} wrote {len(everything)} files and none of "
                f"them inside {sorted(owned)}, so a round trip here would prove "
                "nothing about this provider"
            )
        print(f"-> the product wrote {', '.join(sorted(theirs))} inside what this build owns")
        if untouched:
            head = ", ".join(untouched[:3])
            more = f" and {len(untouched) - 3} more" if len(untouched) > 3 else ""
            print(f"       and {head}{more} outside it, which this build never copies")

        print("round ", end="", flush=True)
        catalog = run_text([binary, "list"])
        setup = next(
            (
                line.strip()
                for line in catalog.splitlines()
                if line.startswith("  ") and line.strip() and " " not in line.strip()
            ),
            None,
        )
        if setup is None:
            raise Failed(f"no setup id could be read from list:\n{catalog}")
        run_text([binary, "install", setup, "--target", str(target)])
        # A second operation, so there is more than one slot. With a single slot
        # the oldest and the newest are the same reference, and this check
        # cannot tell whether the right one was chosen -- a mutation swapping
        # `[0]` for `[-1]` passed against one slot, which is how that was found.
        run_text([binary, "reinstall", "--target", str(target)])
        slots = run_text([binary, "backups", "--target", str(target)])
        refs = sorted({word for word in slots.split() if word.startswith("slot-")})
        if len(refs) < 2:
            raise Failed(
                f"expected at least two slots after install and reinstall, "
                f"found {len(refs)}:\n{slots}"
            )
        oldest = refs[0]
        run_text([binary, "restore", "--backup", oldest, "--target", str(target)])
        after = tree_digests(target, control)
        # Compare exactly what the product wrote, and nothing else. The state
        # file is this provider's own bookkeeping and is *supposed* to be there
        # after an operation -- an earlier version of this script compared the
        # whole tree and reported a byte-exact restore as a failure because the
        # provider had recorded that it ran.
        missing = sorted(set(theirs) - set(after))
        if missing:
            raise Failed(
                "restoring the oldest slot did not bring back what the product "
                f"wrote: {', '.join(missing)} is gone"
            )
        moved = {
            path: (theirs[path], after[path])
            for path in theirs
            if theirs[path] != after[path]
        }
        if moved:
            raise Failed(
                "restoring the oldest slot returned different bytes than the "
                f"product wrote: {moved}"
            )
        print(f"-> install {setup}, reinstall, restore {oldest} of {len(refs)}, byte-exact")

        run_the_probe(binary, target, prefix, probe)
        cross_two_releases(binary, target, prefix, room, info)
        remove_the_program(binary, target, prefix, info)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--harness", required=True)
    parser.add_argument("--probe", default="", help="argv that makes the product report its resolved configuration")
    parser.add_argument("--probe-kind", default="", choices=["", "postures", "reads"])
    parser.add_argument("--probe-baseline", default="", help="what the probe must say with `baseline` installed")
    parser.add_argument("--probe-full-auto", default="", help="what it must say with `full-auto` installed, when the kind is `postures`")
    parser.add_argument("--probe-absent", default="", help="the measured reason this product reports nothing without credentials")
    parser.add_argument(
        "--writes-absent",
        default="",
        help=(
            "The measured reason this product writes no configuration without "
            "credentials. Required when --writes is empty, so the absence "
            "carries a measurement rather than an assumption."
        ),
    )
    parser.add_argument(
        "--writes",
        default="",
        help=(
            "One shell-quoted argument list handed to `launch` so the product "
            "writes its own configuration. Taken as a string rather than as "
            "trailing arguments because these commands carry their own `--`, "
            "which argparse would swallow."
        ),
    )
    args = parser.parse_args()

    if not args.writes and not args.writes_absent:
        print(
            "one of --writes or --writes-absent is required: an unexercised "
            "round trip has to say why, measured rather than assumed",
            file=sys.stderr,
        )
        return 1

    binary = os.path.abspath(args.binary)
    if not os.path.isfile(binary):
        print(f"no binary at {binary}", file=sys.stderr)
        return 1

    print(f"== {args.harness} on {sys.platform} ==")
    try:
        software_lifecycle(
            binary,
            args.harness,
            shlex.split(args.writes),
            args.writes_absent,
            {
                "argv": shlex.split(args.probe),
                "kind": args.probe_kind,
                "baseline": args.probe_baseline,
                "full_auto": args.probe_full_auto,
                "absent": args.probe_absent,
            },
        )
    except NothingToProve as why:
        print(f"nothing to prove here: {why}")
        return 0
    except Failed as why:
        print(f"\nFAILED: {why}", file=sys.stderr)
        return 1
    except subprocess.TimeoutExpired as why:
        print(f"\nFAILED: {why.cmd[1]} did not finish", file=sys.stderr)
        return 1
    print("ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
