#!/usr/bin/env bash
# Build hmanlab from source and stage it into the matching npm subpackage
# so you can `npm pack` / `npm i -g <tarball>` end-to-end without touching
# CI. Useful for iterating on the JS shim or testing the install UX before
# pushing a release tag.
#
# Usage:
#   scripts/npm-local.sh build      # cargo build → copy into linux-x64/bin
#   scripts/npm-local.sh pack       # build + npm pack umbrella + linux-x64 → dist/
#   scripts/npm-local.sh install    # pack + npm i -g the umbrella tarball
#   scripts/npm-local.sh clean      # rm dist/ and staged binaries
#
# Always targets the host's `linux-x64` subpackage when run from a Linux
# x64 box — for cross-arch testing you'd use the CI workflow.

set -euo pipefail
cd "$(dirname "$0")/.."

PLAT=""
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) PLAT=linux-x64 ;;
  Linux-aarch64) PLAT=linux-arm64 ;;
  Darwin-x86_64) PLAT=darwin-x64 ;;
  Darwin-arm64) PLAT=darwin-arm64 ;;
  *) echo "host $(uname -sm) not in the supported matrix; can only stage CI binaries." >&2; exit 2 ;;
esac
BIN_NAME="hmanlab"
SUBPKG="npm/@hmanlab/$PLAT"

cmd_build() {
  cargo build --release
  mkdir -p "$SUBPKG/bin"
  cp "target/release/$BIN_NAME" "$SUBPKG/bin/$BIN_NAME"
  chmod +x "$SUBPKG/bin/$BIN_NAME"
  echo "Staged $SUBPKG/bin/$BIN_NAME ($(du -h "$SUBPKG/bin/$BIN_NAME" | cut -f1))"
}

cmd_pack() {
  cmd_build
  rm -rf dist && mkdir -p dist
  (cd "$SUBPKG" && npm pack --pack-destination "../../../dist")
  (cd npm/hmanlab && npm pack --pack-destination "../../dist")
  ls -lh dist/
}

cmd_install() {
  cmd_pack
  # The umbrella's optionalDependencies reference @hmanlab/<plat>-<arch> by
  # version. For a local-only install we point npm at both tarballs.
  UMBRELLA="$(ls dist/hmanlab-*.tgz | head -1)"
  SUB="$(ls dist/hmanlab-$PLAT-*.tgz 2>/dev/null | head -1)"
  if [[ -z "$SUB" ]]; then
    # `npm pack` of a scoped package writes the file as `hmanlab-<plat>-<ver>.tgz`
    SUB="$(ls dist/hmanlab-*-*.tgz | grep -v "^dist/hmanlab-[0-9]" | head -1)"
  fi
  echo "Installing umbrella=$UMBRELLA + sub=$SUB"
  # We have to register the sub in the npm cache by installing it first
  # (or use --install-strategy=hoisted; the tarball-direct approach works
  # on all npm 8+).
  npm i -g "$SUB" "$UMBRELLA"
  echo "Try: hmanlab --help"
}

cmd_clean() {
  rm -rf dist
  for p in linux-x64 linux-arm64 darwin-x64 darwin-arm64 win32-x64; do
    rm -f "npm/@hmanlab/$p/bin/hmanlab" "npm/@hmanlab/$p/bin/hmanlab.exe"
  done
  echo "Cleaned dist/ and staged binaries (kept .gitkeep)."
}

case "${1:-build}" in
  build) cmd_build ;;
  pack) cmd_pack ;;
  install) cmd_install ;;
  clean) cmd_clean ;;
  *) echo "usage: $0 {build|pack|install|clean}" >&2; exit 2 ;;
esac
