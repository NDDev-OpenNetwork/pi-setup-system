#!/bin/sh
# Install pi-setup-system for the current user on Linux or macOS.
#
# Downloads the release artifact for this platform, checks it against the
# release's own SHA256SUMS, and places it in ~/.local/bin.
#
# Nothing here needs privilege and nothing is registered anywhere: ai-stp is
# handed a provider path by its caller, so the only thing an installer owes you
# is a path you can predict.
#
#   sh install.sh              # the version this checkout describes
#   sh install.sh 0.1.0        # a specific release
set -eu

REPO="NDDev-OpenNetwork/pi-setup-system"
BINARY="pi-setup-system"
VERSION="${1:-0.0.10}"
PREFIX="${PI_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s)" in
  Linux)  os=unknown-linux-gnu ;;
  Darwin) os=apple-darwin ;;
  *) echo "unsupported operating system: $(uname -s)" >&2
     echo "this installer covers Linux and macOS; on Windows use install.ps1" >&2
     exit 1 ;;
esac
case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=aarch64 ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

asset="${BINARY}-${arch}-${os}"
base="https://github.com/${REPO}/releases/download/${VERSION}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

fetch() {
  if command -v curl >/dev/null 2>&1; then curl -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then wget -qO "$2" "$1"
  else echo "neither curl nor wget is available" >&2; exit 1
  fi
}

echo "fetching ${asset} ${VERSION}"
fetch "${base}/${asset}" "${work}/${asset}"
fetch "${base}/SHA256SUMS" "${work}/SHA256SUMS"

# The digest is checked before anything is placed, and the check is the
# release's own list rather than a value written into this script -- a script
# that carried the digest would be a second place for it to be wrong.
( cd "$work" && grep " ${asset}\$" SHA256SUMS > expected.txt   && if command -v sha256sum >/dev/null 2>&1; then sha256sum -c expected.txt
     elif command -v shasum >/dev/null 2>&1; then shasum -a 256 -c expected.txt
     else echo "no sha256 tool available; refusing to install unverified bytes" >&2; exit 1
     fi )

mkdir -p "$PREFIX"
chmod +x "${work}/${asset}"
mv "${work}/${asset}" "${PREFIX}/${BINARY}"

echo "installed ${PREFIX}/${BINARY}"
echo
echo "Point ai-stp at it with the full path:"
echo "  ai-stp provider conformance --harness pi \\"
echo "    --executable ${PREFIX}/${BINARY} --target <dir> --protocol-version 3 --json"
case ":$PATH:" in
  *":$PREFIX:"*) ;;
  *) echo
     echo "note: ${PREFIX} is not on your PATH." ;;
esac
