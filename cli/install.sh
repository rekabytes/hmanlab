#!/bin/sh
# hmanlab installer.
#
# Detects your OS/arch, downloads the latest matching binary from
# GitHub Releases, drops it in ~/.local/bin/hmanlab, and tells you
# whether that directory is on your PATH.
#
# Usage:
#   curl -fsSL https://github.com/hmanlab/hmanlab/releases/latest/download/install.sh | sh
#
# Override install location with HMANLAB_INSTALL_DIR=/usr/local/bin sh.

set -eu

REPO="hmanlab/hmanlab"
INSTALL_DIR="${HMANLAB_INSTALL_DIR:-$HOME/.local/bin}"

# OS detection — match what release.yml ships.
case "$(uname -s)" in
  Linux)  OS="linux" ;;
  Darwin) OS="darwin" ;;
  *)
    echo "✗ hmanlab: unsupported OS $(uname -s)." >&2
    echo "  Windows is supported via npm: npm i -g hmanlab" >&2
    exit 1
    ;;
esac

# Arch detection — also matches release.yml's naming.
case "$(uname -m)" in
  x86_64|amd64)   ARCH="x64"   ;;
  aarch64|arm64)  ARCH="arm64" ;;
  *)
    echo "✗ hmanlab: unsupported architecture $(uname -m)." >&2
    echo "  Supported: x86_64/amd64, aarch64/arm64" >&2
    exit 1
    ;;
esac

PLAT="${OS}-${ARCH}"
ASSET="hmanlab-${PLAT}"
URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"

echo "→ Downloading hmanlab for ${PLAT}…"
mkdir -p "${INSTALL_DIR}"

# curl preferred; fall back to wget if not available.
if command -v curl >/dev/null 2>&1; then
  curl -fSL --progress-bar "${URL}" -o "${INSTALL_DIR}/hmanlab"
elif command -v wget >/dev/null 2>&1; then
  wget --show-progress -qO "${INSTALL_DIR}/hmanlab" "${URL}"
else
  echo "✗ hmanlab: need curl or wget to download. Install one and re-run." >&2
  exit 1
fi

chmod +x "${INSTALL_DIR}/hmanlab"

echo
echo "✓ Installed: ${INSTALL_DIR}/hmanlab"

# PATH sanity check.
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*)
    echo "  Run: hmanlab"
    ;;
  *)
    echo
    echo "⚠ ${INSTALL_DIR} is not in your PATH."
    echo "  Add this to your shell rc (~/.bashrc, ~/.zshrc, etc.):"
    echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
    echo "  Or run directly: ${INSTALL_DIR}/hmanlab"
    ;;
esac
