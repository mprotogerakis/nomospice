#!/bin/sh
# Publishes release v<version> (tauri.conf.json) for the current commit, which must be pushed and built
# by CI: the notarized macOS app (run scripts/macos-release.sh first), the Windows installers from the CI
# run of this commit, the updater files (.app.tar.gz, setup.exe, their .sig) and latest.json, which the
# app's updater reads from the latest release.
#   NOTES_FILE=notes.md scripts/publish-release.sh
set -eu
cd "$(dirname "$0")/.."
: "${NOTES_FILE:?release notes in markdown}"
REPO=mprotogerakis/nomospice
VERSION=$(node -p "require('./src-tauri/tauri.conf.json').version")
TAG=v$VERSION SHA=$(git rev-parse HEAD)
OUT=src-tauri/target/release/bundle/release
MAC=$OUT/nomospice_${VERSION}_aarch64.app.tar.gz
[ -f "$OUT/nomospice_${VERSION}_aarch64.dmg" ] && [ -f "$MAC.sig" ] || { echo "run scripts/macos-release.sh for $VERSION first" >&2; exit 1; }
[ "$(git rev-parse "@{u}")" = "$SHA" ] || { echo "push $SHA first" >&2; exit 1; }

# Windows installers from the successful CI run of this commit
RUN=$(gh run list -R $REPO --commit "$SHA" --json databaseId,conclusion --jq '[.[] | select(.conclusion == "success")][0].databaseId')
[ -n "$RUN" ] && [ "$RUN" != null ] || { echo "no successful CI run for $SHA (yet)" >&2; exit 1; }
WIN=$(mktemp -d)
gh run download "$RUN" -R $REPO -n nomospice-windows-latest -D "$WIN"
EXE=$(find "$WIN" -name "nomospice_${VERSION}_x64-setup.exe" | head -1)
MSI=$(find "$WIN" -name "nomospice_${VERSION}_x64_*.msi" | head -1)
[ -n "$EXE" ] && [ -n "$MSI" ] || { echo "the CI run has no $VERSION installers" >&2; exit 1; }
cp "$EXE" "$MSI" "$OUT/"
EXE=$OUT/$(basename "$EXE") MSI=$OUT/$(basename "$MSI")
scripts/sign-update.sh "$EXE"

# latest.json: what the updater installs on each platform
URL=https://github.com/$REPO/releases/download/$TAG \
MAC_NAME=$(basename "$MAC") MAC_SIG=$(cat "$MAC.sig") EXE_NAME=$(basename "$EXE") EXE_SIG=$(cat "$EXE.sig") \
VERSION=$VERSION NOTES=$(cat "$NOTES_FILE") node -e '
  const e = process.env;
  console.log(JSON.stringify({ version: e.VERSION, notes: e.NOTES, pub_date: new Date().toISOString(), platforms: {
    "darwin-aarch64": { signature: e.MAC_SIG, url: `${e.URL}/${e.MAC_NAME}` },
    "windows-x86_64": { signature: e.EXE_SIG, url: `${e.URL}/${e.EXE_NAME}` },
  } }, null, 2));' > "$OUT/latest.json"

gh release create "$TAG" -R $REPO --target "$SHA" --title "nomospice $VERSION" --notes-file "$NOTES_FILE" \
  "$OUT/nomospice_${VERSION}_aarch64.dmg" "$MAC" "$MAC.sig" "$EXE" "$EXE.sig" "$MSI" "$OUT/latest.json"
echo "published https://github.com/$REPO/releases/tag/$TAG"
