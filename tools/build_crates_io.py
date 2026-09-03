#!/usr/bin/env python3
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


def cargo_toml(harness: str, release: str) -> str:
    package = f"{harness}-setup-system"
    product = PRODUCTS[harness]
    return f'''[package]
name = "{package}"
version = "{release}"
edition = "2024"
rust-version = "1.89"
license = "AGPL-3.0-or-later"
description = "Install, update, back up, restore and remove complete {product} configurations. Built by NDDev."
repository = "https://github.com/NDDev-OpenNetwork/{package}"
homepage = "https://nddev.it.com"
readme = "README.md"
keywords = ["ai", "agent", "setup", "backup", "cli"]
categories = ["command-line-utilities", "development-tools"]
publish = ["crates-io"]

[dependencies]
serde = {{ version = "1", features = ["derive"] }}
serde_json = {{ version = "1", features = ["preserve_order"] }}
sha2 = "0.11"
miniz_oxide = "0.9"

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

The NDDev setup system for {PRODUCTS[harness]}. It installs complete native
configurations through an explicit target, captures a backup before every
mutation, and restores exact bytes. It implements the `ai-stp` provider
protocol v3 and accepts adaptation-bound `ai-stp-bundle/2` packages.

```console
cargo install {package}
{package} list
{package} provider-info
```

Source, security policy and release provenance:
<https://github.com/NDDev-OpenNetwork/{package}>.
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
        for source in sorted((ROOT / "crates" / crate / "src").glob("*.rs")):
            name = "mod.rs" if source.name == "lib.rs" else source.name
            destination.joinpath(name).write_text(
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
            for harness in HARNESSES:
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
            codex = root / "codex-setup-system"
            subprocess.run(
                ["cargo", "build", "--quiet", "--manifest-path", str(codex / "Cargo.toml")],
                check=True,
            )
            answer = subprocess.run(
                [codex / "target/debug/codex-setup-system", "provider-info"],
                check=True,
                capture_output=True,
                text=True,
            )
            info = json.loads(answer.stdout)
            if info["provider_id"] != "codex-setup-system" or info["projection_profile"][
                "bundle_formats"
            ] != ["ai-stp-bundle/2"]:
                raise SystemExit("the installed-shape provider-info is not the v2-only Codex provider")
        print("crates.io: seven same-name packages; all package, and a standalone provider runs")
        return 0
    if args.out is None:
        parser.error("--out is required unless --self-check is used")
    for harness in args.harness or HARNESSES:
        print(build(harness, args.out, args.version))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
