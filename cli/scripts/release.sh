#!/usr/bin/env bash
# Bump every version-bearing file to the same number, commit, tag, push.
# Usage: scripts/release.sh 0.3.0
#
# Touched files:
#   - Cargo.toml                      (package.version)
#   - npm/hmanlab/package.json        (version + every optionalDependencies entry)
#   - npm/@hmanlab/*/package.json     (version)
#
# After this script lands the tag, the `release.yml` workflow on GitHub
# Actions builds binaries for every target and publishes to npm.

set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <new-version>" >&2
  echo "example: $0 0.3.0" >&2
  exit 2
fi

NEW="$1"

if ! [[ "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "error: '$NEW' is not a valid semver" >&2
  exit 2
fi

cd "$(dirname "$0")/.."

# Sanity: no uncommitted changes — the script is going to commit, so we
# want the working tree clean before we start.
if [[ -n "$(git status --porcelain)" ]]; then
  echo "error: working tree not clean. commit or stash first." >&2
  exit 1
fi

CUR="$(grep '^version = ' Cargo.toml | head -1 | sed -E 's/version = "(.*)"/\1/')"
echo "Bumping $CUR → $NEW"

# Cargo.toml — only touch the package version line, not anything in
# [dependencies] (some crate versions look the same).
sed -i -E "0,/^version = \"$CUR\"$/s//version = \"$NEW\"/" Cargo.toml

# Every package.json under npm/ — `"version": "X"` and `"@hmanlab/...": "X"`.
find npm -name 'package.json' -print0 | while IFS= read -r -d '' f; do
  sed -i -E "s/(\"version\": \")$CUR(\")/\1$NEW\2/" "$f"
  sed -i -E "s/(\"@hmanlab\/[a-z0-9-]+\": \")$CUR(\")/\1$NEW\2/g" "$f"
done

# Verify the bump landed everywhere — refuse to commit if anything still
# references the old version.
if grep -RIn "\"$CUR\"" npm/ Cargo.toml 2>/dev/null | grep -v node_modules; then
  echo "error: some files still reference $CUR — aborting." >&2
  exit 1
fi

# Cargo.lock follows the version automatically on next build, but we
# refresh it now so the commit captures it.
cargo update --workspace --offline 2>/dev/null || cargo update --workspace

git add Cargo.toml Cargo.lock npm/
git commit -m "release: v$NEW"
git tag "v$NEW"

echo
echo "Done. Push with:"
echo "  git push && git push --tags"
echo
echo "Then watch https://github.com/hmanlab/hmanlab/actions"
