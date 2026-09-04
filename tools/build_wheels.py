#!/usr/bin/env python3
"""Pack each released provider binary into a platform wheel, and nothing else.

The seven are Rust programs and their trusted path is a GitHub release whose
build attestation the consumer verifies before running the bytes. This is a
*second* path, decided by the owner on 2026-09-02 and shaped with the consumer
in their `ADR-0141`: the same binary, installable with `pip`, named after the
repository it comes from.

**A wheel here carries the executable and nothing else.** The first form we
agreed put the consumer's `release.json` in beside it; reading their code
killed that idea before it was built. Their manifest is a closed eighteen-field
document whose last three fields are `signing_key`, `signature_subject` and
`signature`, minted by an offline Ed25519 key that is *theirs* -- and for these
seven providers the consumer already writes the manifest itself, out of what an
attestation proved (`attested_bind`: `signing_key="attested"`, `signature=""`).
So shipping one here would have been a second source of truth, and would have
dragged a question about keys into a question about packaging. The wheel is the
payload; provenance is the index's to state and the consumer's to check.

**Every platform tag is measured, not assumed.** The linux binaries are
dynamically linked and their highest required symbol version is read out of the
ELF itself -- `GLIBC_2.34` on both architectures, not the 2.39 of the runner
that built them, which is what an assumption would have written. The macOS
minimums come from the Mach-O load commands: `10.12` on x86_64
(`LC_VERSION_MIN_MACOSX`), `11.0` on arm64 (`LC_BUILD_VERSION`). Windows needs
no floor. A tag claiming less than the truth installs on a machine the program
cannot run on, and the failure lands far from here.

**The executable mode is part of the archive member's type.** A ZIP entry with
permission bits `0755` but no POSIX regular-file bit looks executable when the
archive is inspected directly, yet pip 25.1.1 installs it as `0664`. Each member
therefore records a complete `st_mode`, and the self-check installs and runs a
probe wheel without repairing it first.

Usage:
    python3 tools/build_wheels.py --harness claude --version 0.0.58 \
        --assets <dir of release assets> --out <dir>
    python3 tools/build_wheels.py --self-check
"""

from __future__ import annotations

import argparse
import base64
import csv
import hashlib
import io
import json
import os
import stat
import struct
import subprocess
import sys
import tempfile
import zipfile
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

#: `harness id -> (distribution name, provider id)`. The distribution is named
#: after the repository, which is the owner's instruction and also removes a
#: translation: the consumer's own map from harness to repository has this
#: shape already.
HARNESSES: dict[str, str] = {
    "claude": "claude-setup-system",
    "codex": "codex-setup-system",
    "grok": "grok-setup-system",
    "pi": "pi-setup-system",
    "cursor": "cursor-setup-system",
    "opencode": "opencode-setup-system",
    "antigravity": "antigravity-setup-system",
}

#: `release asset suffix -> wheel platform tag`. Each floor is read from the
#: bytes; see the module docstring. `manylinux_2_34` is what the ELF requires,
#: and a wheel may always claim an *older* floor than the machine that built it
#: only when the symbols agree -- which is why this is measured per release
#: rather than written once.
PLATFORMS: dict[str, str] = {
    "x86_64-unknown-linux-gnu": "manylinux_2_34_x86_64",
    "aarch64-unknown-linux-gnu": "manylinux_2_34_aarch64",
    "x86_64-apple-darwin": "macosx_10_12_x86_64",
    "aarch64-apple-darwin": "macosx_11_0_arm64",
    "x86_64-pc-windows-msvc.exe": "win_amd64",
    "aarch64-pc-windows-msvc.exe": "win_arm64",
}

SUMMARY = "Install, select, back up and restore complete {product} setups."


@dataclass(frozen=True)
class Built:
    """One wheel, with the facts a caller checks it by."""

    path: Path
    tag: str
    member: str
    digest: str


def _urlsafe(digest: bytes) -> str:
    return base64.urlsafe_b64encode(digest).rstrip(b"=").decode("ascii")


def glibc_floor(binary: bytes) -> str | None:
    """The highest `GLIBC_x.y` an ELF asks for, or None when it is not an ELF."""
    if binary[:4] != b"\x7fELF":
        return None
    found: list[tuple[int, int]] = []
    marker = b"GLIBC_"
    start = 0
    while (at := binary.find(marker, start)) != -1:
        end = at + len(marker)
        while end < len(binary) and binary[end : end + 1] in b"0123456789.":
            end += 1
        text = binary[at + len(marker) : end].decode("ascii", "replace")
        parts = text.split(".")
        if len(parts) >= 2 and all(part.isdigit() for part in parts[:2]):
            found.append((int(parts[0]), int(parts[1])))
        start = end
    if not found:
        return None
    major, minor = max(found)
    return f"{major}.{minor}"


def macos_minimum(binary: bytes) -> str | None:
    """The minimum macOS version a Mach-O declares, or None when it is not one."""
    if len(binary) < 32:
        return None
    magic = struct.unpack("<I", binary[:4])[0]
    if magic != 0xFEEDFACF:
        return None
    ncmds = struct.unpack("<I", binary[16:20])[0]
    offset = 32
    for _ in range(ncmds):
        if offset + 8 > len(binary):
            return None
        command, size = struct.unpack("<II", binary[offset : offset + 8])
        if command == 0x32 and offset + 20 <= len(binary):  # LC_BUILD_VERSION
            minimum = struct.unpack("<I", binary[offset + 12 : offset + 16])[0]
            return f"{minimum >> 16}.{(minimum >> 8) & 0xFF}"
        if command == 0x24 and offset + 12 <= len(binary):  # LC_VERSION_MIN_MACOSX
            minimum = struct.unpack("<I", binary[offset + 8 : offset + 12])[0]
            return f"{minimum >> 16}.{(minimum >> 8) & 0xFF}"
        offset += size
    return None


def measured_tag(suffix: str, binary: bytes) -> str:
    """The declared tag, refused when the bytes disagree with the table.

    The table is a claim; these two readings are the evidence. A release whose
    binaries move to a newer glibc or a later macOS minimum stops the build
    here rather than shipping a wheel that installs where it cannot run.
    """
    declared = PLATFORMS[suffix]
    if "manylinux" in declared:
        floor = glibc_floor(binary)
        if floor is None:
            raise SystemExit(f"{suffix}: declared {declared} and the file is not an ELF")
        if f"manylinux_{floor.replace('.', '_')}_" not in declared:
            raise SystemExit(
                f"{suffix}: the binary requires GLIBC_{floor} and the tag says "
                f"{declared}; measure the release and update PLATFORMS"
            )
    if "macosx" in declared:
        minimum = macos_minimum(binary)
        if minimum is None:
            raise SystemExit(f"{suffix}: declared {declared} and the file is not a Mach-O")
        if f"macosx_{minimum.replace('.', '_')}_" not in declared:
            raise SystemExit(
                f"{suffix}: the binary's minimum is macOS {minimum} and the tag says "
                f"{declared}; measure the release and update PLATFORMS"
            )
    return declared


def wheel_bytes(
    *, distribution: str, version: str, tag: str, member: str, payload: bytes, summary: str
) -> bytes:
    """One wheel, deterministic: fixed order, fixed timestamps, no extras."""
    package = distribution.replace("-", "_")
    dist_info = f"{package}-{version}.dist-info"
    metadata = "\n".join(
        [
            "Metadata-Version: 2.4",
            f"Name: {distribution}",
            f"Version: {version}",
            f"Summary: {summary}",
            "License-Expression: AGPL-3.0-or-later",
            "Requires-Python: >=3.9",
            f"Project-URL: Repository, https://github.com/NDDev-OpenNetwork/{distribution}",
            "Classifier: Operating System :: MacOS",
            "Classifier: Operating System :: Microsoft :: Windows",
            "Classifier: Operating System :: POSIX :: Linux",
            "",
            f"The {distribution} provider binary, the same bytes its GitHub release",
            "carries at this version. The wheel is a delivery path, not a different",
            "program: the trusted path remains the release, whose build attestation",
            "an `ai-stp` consumer verifies before running anything.",
            "",
        ]
    )
    wheel = "\n".join(
        [
            "Wheel-Version: 1.0",
            f"Generator: {distribution} {version}",
            "Root-Is-Purelib: false",
            f"Tag: py3-none-{tag}",
            "",
        ]
    )
    entries: list[tuple[str, bytes, int]] = [
        (f"{package}/__init__.py", b"", 0o644),
        (f"{package}/bin/{member}", payload, 0o755),
        (f"{dist_info}/METADATA", metadata.encode(), 0o644),
        (f"{dist_info}/WHEEL", wheel.encode(), 0o644),
    ]
    record = io.StringIO()
    writer = csv.writer(record, lineterminator="\n")
    for name, data, _ in entries:
        writer.writerow([name, f"sha256={_urlsafe(hashlib.sha256(data).digest())}", len(data)])
    writer.writerow([f"{dist_info}/RECORD", "", ""])
    entries.append((f"{dist_info}/RECORD", record.getvalue().encode(), 0o644))

    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, data, mode in entries:
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            # A POSIX mode without the file type is not a complete `st_mode`.
            # `zipfile`'s extractor happens to read the permission bits, while
            # pip treats such a member as an ordinary non-executable file.  A
            # regular-file marker is therefore part of the wheel's contract,
            # not archive decoration.
            info.external_attr = (stat.S_IFREG | mode) << 16
            info.create_system = 3
            archive.writestr(info, data)
    return buffer.getvalue()


def check_pip_preserves_executable() -> None:
    """Install a wheel without repair and prove its payload remains runnable."""
    if os.name == "nt":
        return
    payload = b"#!/bin/sh\nprintf 'wheel-mode-ok\\n'\n"
    wheel_name = "mode_probe-0.0.0-py3-none-any.whl"
    with tempfile.TemporaryDirectory(prefix="setup-wheel-mode-") as temporary:
        root = Path(temporary)
        wheel = root / wheel_name
        wheel.write_bytes(
            wheel_bytes(
                distribution="mode-probe",
                version="0.0.0",
                tag="any",
                member="mode-probe",
                payload=payload,
                summary="Wheel executable-mode probe.",
            )
        )
        target = root / "installed"
        subprocess.run(
            [
                sys.executable,
                "-m",
                "pip",
                "install",
                "--quiet",
                "--no-deps",
                "--target",
                str(target),
                str(wheel),
            ],
            check=True,
        )
        installed = target / "mode_probe" / "bin" / "mode-probe"
        if installed.read_bytes() != payload:
            raise SystemExit("pip changed the provider payload bytes")
        mode = installed.stat().st_mode
        if not mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH):
            raise SystemExit(f"pip installed the provider payload as {mode:o}, not executable")
        completed = subprocess.run(
            [str(installed)], check=True, capture_output=True, text=True
        )
        if completed.stdout != "wheel-mode-ok\n":
            raise SystemExit(f"the installed wheel payload answered {completed.stdout!r}")


def build(harness: str, version: str, assets: Path, out: Path) -> list[Built]:
    """Every platform wheel for one harness, from its release assets."""
    distribution = HARNESSES[harness]
    product = distribution.removesuffix("-setup-system")
    out.mkdir(parents=True, exist_ok=True)
    built: list[Built] = []
    for suffix, _ in PLATFORMS.items():
        asset = assets / f"{distribution}-{suffix}"
        if not asset.is_file():
            raise SystemExit(f"{asset.name} is not in {assets}; the release is incomplete")
        payload = asset.read_bytes()
        tag = measured_tag(suffix, payload)
        member = distribution + (".exe" if suffix.endswith(".exe") else "")
        data = wheel_bytes(
            distribution=distribution,
            version=version,
            tag=tag,
            member=member,
            payload=payload,
            summary=SUMMARY.format(product=product),
        )
        package = distribution.replace("-", "_")
        path = out / f"{package}-{version}-py3-none-{tag}.whl"
        path.write_bytes(data)
        built.append(
            Built(path, tag, member, "sha256:" + hashlib.sha256(payload).hexdigest())
        )
        print(f"  {path.name}  {member}  {len(payload)} byte(s)")
    return built


def self_check() -> int:
    """Prove the reader discriminates before it is trusted with a release.

    Three planted defects and three truths, in one run: an ELF asking for a
    newer glibc than the tag claims must be refused, a Mach-O with a later
    minimum must be refused, the honest readings must come back exactly, and
    pip must preserve and run an executable archived with its complete mode.
    """
    elf = b"\x7fELF" + b"\x00" * 60 + b"GLIBC_2.34\x00GLIBC_2.17\x00"
    assert glibc_floor(elf) == "2.34", glibc_floor(elf)
    newer = elf.replace(b"GLIBC_2.34", b"GLIBC_2.41")
    assert glibc_floor(newer) == "2.41"
    try:
        measured_tag("x86_64-unknown-linux-gnu", newer)
    except SystemExit as refusal:
        assert "requires GLIBC_2.41" in str(refusal), refusal
    else:
        raise SystemExit("a newer glibc was not refused")
    assert glibc_floor(b"not an elf") is None
    # The other half, and the sweep's planted defect found it missing: a check
    # that only refuses proves nothing about a table that has been loosened.
    # A tag claiming an older floor than the bytes require passes every
    # refusal test and installs on a machine the program cannot run on.
    honest = measured_tag("x86_64-unknown-linux-gnu", elf)
    assert honest == PLATFORMS["x86_64-unknown-linux-gnu"], honest
    assert "manylinux_2_34_" in honest, honest
    header = struct.pack("<IiiIIIII", 0xFEEDFACF, 0x100000C, 0, 2, 1, 24, 0, 0)
    command = struct.pack("<IIIII", 0x32, 24, 1, 11 << 16, 0)
    macho = header + command
    assert macos_minimum(macho) == "11.0", macos_minimum(macho)
    assert macos_minimum(b"\x7fELF" + b"\x00" * 40) is None
    later = header + struct.pack("<IIIII", 0x32, 24, 1, 15 << 16, 0)
    try:
        measured_tag("aarch64-apple-darwin", later)
    except SystemExit as refusal:
        assert "minimum is macOS 15.0" in str(refusal), refusal
    else:
        raise SystemExit("a later macOS minimum was not refused")
    accepted = measured_tag("aarch64-apple-darwin", macho)
    assert accepted == PLATFORMS["aarch64-apple-darwin"], accepted
    assert "macosx_11_0_" in accepted, accepted
    check_pip_preserves_executable()
    print(
        "self-check: the tag reader refuses both planted defects, reads both "
        "truths, accepts each honest binary under the declared tag, and pip "
        "installs an unchanged executable payload without a chmod repair"
    )
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-check", action="store_true")
    parser.add_argument("--harness", choices=sorted(HARNESSES))
    parser.add_argument("--version")
    parser.add_argument("--assets", type=Path)
    parser.add_argument("--out", type=Path, default=ROOT / "target" / "wheels")
    parsed = parser.parse_args(argv)
    if parsed.self_check:
        return self_check()
    if not (parsed.harness and parsed.version and parsed.assets):
        parser.error("--harness, --version and --assets are required")
    print(f"{HARNESSES[parsed.harness]} {parsed.version}")
    built = build(parsed.harness, parsed.version, parsed.assets, parsed.out)
    print(f"RESULT wheels={len(built)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
