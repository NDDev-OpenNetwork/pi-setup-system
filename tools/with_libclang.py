"""Make the declared libclang build prerequisite available to Cargo.

The CI maintainers own this build-only pin. The wheel is maintained at
https://github.com/sighingnow/libclang; it bundles LLVM under Apache-2.0 WITH
LLVM-exception. Its ELF SONAME and the existing C compiler's standard headers
are required for bindgen, not for any shipped provider. Remove this wrapper
when the Linux runner images supply the declared development prerequisite.
macOS uses its selected Xcode SDK and explicitly exposes its library to Cargo's
build-script loader. Windows uses the LLVM/MSVC SDK already on the runner.
"""

from __future__ import annotations

import hashlib
import io
import os
import platform
import shlex
import subprocess
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path

VERSION = "18.1.1"
WHEELS = {
    "x86_64": (
        (
            "1d/fc/716c1e62e512ef1c160e7984a73a5fc7df45166f2ff3f254e71c58076f7c/"
            f"libclang-{VERSION}-py2.py3-none-manylinux2010_x86_64.whl"
        ),
        "c533091d8a3bbf7460a00cb6c1a71da93bffe148f172c7d03b1c31fbf8aa2a0b",
        24_515_943,
        63_703_544,
    ),
    "aarch64": (
        (
            "3c/3d/f0ac1150280d8d20d059608cf2d5ff61b7c3b7f7bcf9c0f425ab92df769a/"
            f"libclang-{VERSION}-py2.py3-none-manylinux2014_aarch64.whl"
        ),
        "54dda940a4a0491a9d1532bf071ea3ef26e6dbaf03b5000ed94dd7174e8f9592",
        23_784_972,
        60_393_984,
    ),
}


def prepend(env: dict[str, str], name: str, value: str) -> None:
    env[name] = os.pathsep.join(item for item in (value, env.get(name, "")) if item)


def clang_arguments(env: dict[str, str], arguments: list[str]) -> None:
    env["BINDGEN_EXTRA_CLANG_ARGS"] = " ".join(
        item
        for item in (shlex.join(arguments), env.get("BINDGEN_EXTRA_CLANG_ARGS", ""))
        if item
    )


def macos(arguments: list[str]) -> int:
    compiler = Path(
        subprocess.check_output(["xcrun", "--find", "clang"], text=True).strip()
    )
    library = compiler.parent.parent / "lib"
    sdk = subprocess.check_output(["xcrun", "--show-sdk-path"], text=True).strip()
    resources = Path(
        subprocess.check_output(
            [str(compiler), "-print-resource-dir"], text=True
        ).strip()
    )
    if (
        not (library / "libclang.dylib").is_file()
        or not (resources / "include/stdarg.h").is_file()
        or not Path(sdk).is_dir()
    ):
        raise SystemExit("the selected Xcode SDK lacks libclang or standard C headers")
    env = os.environ.copy()
    env["LIBCLANG_PATH"] = str(library)
    prepend(env, "DYLD_FALLBACK_LIBRARY_PATH", str(library))
    clang_arguments(env, ["-isysroot", sdk, "-isystem", str(resources / "include")])
    return subprocess.call(arguments, env=env)


def main(arguments: list[str]) -> int:
    if not arguments:
        raise SystemExit("with_libclang requires a command")
    if sys.platform == "win32":
        return subprocess.call(arguments)
    if sys.platform == "darwin":
        return macos(arguments)
    if sys.platform != "linux" or platform.machine() not in WHEELS:
        raise SystemExit("with_libclang supports Linux x86_64/arm64, macOS and Windows")
    suffix, digest, wheel_bytes, library_bytes = WHEELS[platform.machine()]
    # The same C compiler is already a prerequisite of bundled SQLite. Its
    # standard headers supply stdarg.h; the library-only wheel contains none.
    include = Path(
        subprocess.check_output(["cc", "-print-file-name=include"], text=True).strip()
    )
    if not include.is_absolute() or not (include / "stdarg.h").is_file():
        raise SystemExit("the C compiler's standard headers are unavailable")
    with urllib.request.urlopen(
        "https://files.pythonhosted.org/packages/" + suffix, timeout=30
    ) as response:
        wheel = response.read(wheel_bytes + 1)
    if len(wheel) != wheel_bytes or hashlib.sha256(wheel).hexdigest() != digest:
        raise SystemExit("the pinned libclang wheel size or SHA-256 does not match")

    # Extract exactly one known member, never a wheel's paths or Python code.
    with zipfile.ZipFile(io.BytesIO(wheel)) as archive:
        member = archive.getinfo(
            f"libclang-{VERSION}.data/platlib/clang/native/libclang.so"
        )
        if member.file_size != library_bytes:
            raise SystemExit("the pinned libclang member size does not match")
        library = archive.read(member)
    with tempfile.TemporaryDirectory(
        prefix="ai-stp-libclang-", dir=os.environ.get("RUNNER_TEMP")
    ) as temporary:
        directory = Path(temporary)
        (directory / "libclang.so").write_bytes(library)
        (directory / "libclang.so.18.1").symlink_to("libclang.so")
        env = os.environ.copy()
        env["LIBCLANG_PATH"] = temporary
        # clang-sys caches its emitted -L path without tracking LIBCLANG_PATH.
        # A restored Cargo cache may name a deleted temporary directory. Supply
        # the current link-time search path as well as the runtime loader path.
        prepend(env, "LIBRARY_PATH", temporary)
        prepend(env, "LD_LIBRARY_PATH", temporary)
        clang_arguments(env, ["-isystem", str(include)])
        return subprocess.call(arguments, env=env)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
