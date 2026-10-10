"""Build seven self-contained crates.io source packages from the shared tree.

The public repositories are workspaces because that is the clearest form for
reading and contributing. crates.io publishes one package at a time and rejects
unpublished path dependencies. This projection therefore nests the three
shared crates as private modules inside each harness package. The source remains
single-authority here; no generated package is committed.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HARNESSES = (
    "antigravity",
    "claude",
    "codex",
    "cursor",
    "grok",
    "opencode",
    "pi",
)
MODULES = {
    "setup_core": "setup-core",
    "provider_v3": "provider-v3",
    "harness_runtime": "harness-runtime",
}
PRODUCTS = {
    "antigravity": "Antigravity CLI",
    "claude": "Claude Code",
    "codex": "Codex CLI",
    "cursor": "Cursor CLI",
    "grok": "Grok Build",
    "opencode": "OpenCode",
    "pi": "Pi Coding Agent",
}


def version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    return re.search(r'(?m)^version = "([^"]+)"$', text).group(1)  # type: ignore[union-attr]


def external_paths(text: str) -> str:
    for name in MODULES:
        text = re.sub(rf"(?<![:\w]){name}::", f"crate::{name}::", text)
    return text


def nested_source(text: str, module: str) -> str:
    text = text.replace("crate::", f"crate::{module}::")
    return external_paths(text).replace("../../../provider-kit/", "../../provider-kit/")


def dependency_tables(package: str) -> str:
    # The standalone package embeds the workspace crates, so it needs their
    # actual declarations, including platform conditions. A Unix-only kernel
    # primitive must not become an unconditional dependency of a Windows crate.
    source = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    workspace = tomllib.loads(source)["workspace"]["dependencies"]
    block = source.split("[workspace.dependencies]", 1)[1].split("\n[", 1)[0]
    declarations = {
        line.split("=", 1)[0].strip(): line
        for line in block.splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    }
    groups: dict[str, set[str]] = {"dependencies": set()}
    for crate in (*MODULES.values(), package):
        manifest = tomllib.loads((ROOT / "crates" / crate / "Cargo.toml").read_text())
        tables = {"dependencies": manifest.get("dependencies", {})}
        tables.update({
            f"target.'{target}'.dependencies": table.get("dependencies", {})
            for target, table in manifest.get("target", {}).items()
        })
        for table, names in tables.items():
            for name, inherited in names.items():
                if inherited != {"workspace": True}:
                    raise SystemExit(f"unsupported dependency override in {crate}: {name}")
                value = workspace[name]
                if isinstance(value, dict) and "path" in value:
                    continue
                if tomllib.loads(declarations[name]).get(name) != value:
                    raise SystemExit(f"dependency declaration must fit one line: {name}")
                groups.setdefault(table, set()).add(name)
    return "\n\n".join(
        f"[{table}]\n" + "\n".join(declarations[name] for name in sorted(names))
        for table, names in sorted(groups.items()) if names
    )


def cargo_toml(harness: str, release: str) -> str:
    package = f"{harness}-setup-system"
    product = PRODUCTS[harness]
    return f'''[package]
name = "{package}"
version = "{release}"
edition = "2024"
rust-version = "1.89"
license = "AGPL-3.0-or-later"
description = "Managed ai-stp installation component for {product}, maintained by NDDev."
repository = "https://github.com/NDDev-OpenNetwork/{package}"
homepage = "https://github.com/ai-engineers-guild/ai-stp"
readme = "README.md"
keywords = ["ai", "agent", "setup", "harness", "cli"]
categories = ["command-line-utilities", "development-tools"]
publish = ["crates-io"]

{dependency_tables(package)}

[profile.release]
lto = true
codegen-units = 1
strip = "symbols"
panic = "abort"

[workspace]
'''


def readme(harness: str) -> str:
    package = f"{harness}-setup-system"
    return f"""# {package}

An ai-stp installation component for {PRODUCTS[harness]}, maintained by NDDev.
The ai-stp CLI owns user workflows; this component owns final-state writes and
recovery through provider protocol v3 and adaptation-bound `ai-stp-bundle/2`
packages. Its repository and release identity remain independent.

For user workflows and the native CLI's implemented capabilities, see
[ai-stp](https://github.com/ai-engineers-guild/ai-stp). Direct component commands
remain compatibility and maintenance interfaces:

```console
cargo install {package}
{package} list
{package} provider-info
```

Source, security policy and release provenance:
<https://github.com/NDDev-OpenNetwork/{package}>.

Configuration backup/restore remains a compatibility feature. Software
installation, update and removal use transaction metadata and create no
configuration backups.
"""


def build(harness: str, out_root: Path, release: str) -> Path:
    package = f"{harness}-setup-system"
    out = out_root / package
    if out.exists():
        shutil.rmtree(out)
    (out / "src").mkdir(parents=True)

    for module, crate in MODULES.items():
        destination = out / "src" / module
        destination.mkdir()
        source_directory = ROOT / "crates" / crate / "src"
        for source in sorted(source_directory.rglob("*.rs")):
            relative = source.relative_to(source_directory)
            name = Path("mod.rs") if relative == Path("lib.rs") else relative
            target = destination / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(
                nested_source(source.read_text(encoding="utf-8"), module),
                encoding="utf-8",
            )

    source_root = ROOT / "crates" / package
    main = source_root.joinpath("src/main.rs").read_text(encoding="utf-8")
    main = main.replace("mod software;", "")
    main = external_paths(main).replace("../../../provider-kit/", "../provider-kit/")
    split = main.index("\nuse std::process::ExitCode;")
    docs, body = main[:split], main[split:]
    modules = "\n".join(f"mod {name};" for name in (*MODULES, "software"))
    projection_lints = """#![allow(
    dead_code,
    unused_imports,
    reason = "the standalone crate nests the complete shared implementation; public workspace APIs unused by this harness remain intentionally present"
)]"""
    (out / "src/main.rs").write_text(
        f"{docs}\n\n{projection_lints}\n\n{modules}\n{body}", encoding="utf-8"
    )
    software = source_root.joinpath("src/software.rs").read_text(encoding="utf-8")
    (out / "src/software.rs").write_text(external_paths(software), encoding="utf-8")

    build_rs = source_root.joinpath("build.rs").read_text(encoding="utf-8")
    build_rs = build_rs.replace(
        'let root = manifest.join("..").join("..").join("setups");',
        'let root = manifest.join("setups");',
    )
    (out / "build.rs").write_text(build_rs, encoding="utf-8")
    shutil.copytree(ROOT / "provider-kit", out / "provider-kit")
    scoped_catalog = ROOT / "setups" / harness
    shutil.copytree(scoped_catalog if scoped_catalog.is_dir() else ROOT / "setups", out / "setups")
    (out / "Cargo.toml").write_text(cargo_toml(harness, release), encoding="utf-8")
    (out / "README.md").write_text(readme(harness), encoding="utf-8")
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--version", default=version())
    parser.add_argument("--harness", action="append", choices=HARNESSES)
    parser.add_argument("--self-check", action="store_true")
    args = parser.parse_args()
    if args.self_check:
        with tempfile.TemporaryDirectory(prefix="nddev-crates-io-") as temporary:
            root = Path(temporary)
            # This workspace carries all seven crates; a rendered public tree
            # carries exactly one. The check walks whichever exist rather than
            # asserting the shared layout in a tree that never had it.
            harnesses = [
                harness
                for harness in HARNESSES
                if (ROOT / "crates" / f"{harness}-setup-system").is_dir()
            ]
            if not harnesses:
                raise SystemExit("no harness crate under crates/ -- nothing to self-check")
            for harness in harnesses:
                package = build(harness, root, args.version)
                document = package.joinpath("Cargo.toml").read_text(encoding="utf-8")
                expected = f'name = "{harness}-setup-system"'
                if expected not in document or "nddev-" + harness in document:
                    raise SystemExit(f"{harness}: generated package name is not {expected}")
                subprocess.run(
                    ["cargo", "package", "--manifest-path", str(package / "Cargo.toml"), "--no-verify"],
                    check=True,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            first = harnesses[0]
            built = root / f"{first}-setup-system"
            subprocess.run(
                ["cargo", "build", "--quiet", "--manifest-path", str(built / "Cargo.toml")],
                check=True,
            )
            answer = subprocess.run(
                [built / "target/debug" / f"{first}-setup-system", "provider-info"],
                check=True,
                capture_output=True,
                text=True,
            )
            info = json.loads(answer.stdout)
            if info["provider_id"] != f"{first}-setup-system" or info["projection_profile"][
                "bundle_formats"
            ] != ["ai-stp-bundle/2"]:
                raise SystemExit(
                    f"the installed-shape provider-info is not the v2-only {first} provider"
                )
        print(
            f"crates.io: {len(harnesses)} same-name package(s); all package, "
            "and a standalone provider runs"
        )
        return 0
    if args.out is None:
        parser.error("--out is required unless --self-check is used")
    for harness in args.harness or HARNESSES:
        print(build(harness, args.out, args.version))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
