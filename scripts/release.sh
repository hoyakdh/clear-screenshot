#!/usr/bin/env bash
# Publish a release: bump version → build DMG → commit + tag → push → GitHub release with the DMG.
#
#   scripts/release.sh 0.2.0
#
# Before running, describe the changes under "## [Unreleased]" in CHANGELOG.md; this script turns
# that section into "## [0.2.0] - <date>" and uses it as the release notes.
set -euo pipefail

VERSION="${1:-}"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: $0 <major.minor.patch>   e.g. $0 0.2.0" >&2
  exit 1
fi
TAG="v$VERSION"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ -n "$(git status --porcelain)" ]]; then
  echo "working tree is not clean — commit or stash first" >&2
  exit 1
fi
if git rev-parse "$TAG" >/dev/null 2>&1; then
  echo "tag $TAG already exists" >&2
  exit 1
fi

# 1. version in Cargo.toml (package) and tauri.conf.json
perl -0pi -e "s/^version = \"[^\"]+\"/version = \"$VERSION\"/m" src-tauri/Cargo.toml
perl -pi -e "s/\"version\": \"[^\"]+\"/\"version\": \"$VERSION\"/" src-tauri/tauri.conf.json

# 2. changelog: [Unreleased] → [VERSION] - date (skipped if the section already exists)
TODAY="$(date +%Y-%m-%d)"
if ! grep -q "^## \[$VERSION\]" CHANGELOG.md; then
  perl -0pi -e "s/## \[Unreleased\]\n/## [Unreleased]\n\n## [$VERSION] - $TODAY\n/" CHANGELOG.md
fi
NOTES="$(awk -v v="$VERSION" '
  $0 ~ "^## \\[" v "\\]" { on = 1; next }
  on && /^## \[/ { exit }
  on && /^\[/ { exit }
  on { print }' CHANGELOG.md)"

# 3. build the app + DMG (also refreshes Cargo.lock)
( cd src-tauri && cargo tauri build --bundles app,dmg )
DMG="$(ls -t src-tauri/target/release/bundle/dmg/*"$VERSION"*.dmg | head -1)"
[[ -f "$DMG" ]] || { echo "DMG not found" >&2; exit 1; }

# 4. commit, tag, push
git add -A
git commit -m "Release $TAG" || true
git tag -a "$TAG" -m "Clear Screenshot $VERSION"
git push origin HEAD --follow-tags

# 5. GitHub release with the DMG attached
gh release create "$TAG" "$DMG" --title "Clear Screenshot $VERSION" --notes "$NOTES"
echo "released $TAG → $(gh release view "$TAG" --json url --jq .url)"
