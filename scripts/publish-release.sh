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
[ "$(cat "$OUT/built-from" 2>/dev/null)" = "$SHA" ] && [ -z "$(git status --porcelain --untracked-files=no)" ] \
  || { echo "the macOS build is not from $SHA (or the tree has changes): run scripts/macos-release.sh again" >&2; exit 1; }

# Windows installers from the newest completed CI run of this commit (the Windows job uploads them
# when it succeeds, even if another OS's job failed)
RUN=$(gh run list -R $REPO --commit "$SHA" --json databaseId,status --jq '[.[] | select(.status == "completed")][0].databaseId')
[ -n "$RUN" ] && [ "$RUN" != null ] || { echo "no completed CI run for $SHA (yet)" >&2; exit 1; }
WIN=$(mktemp -d)
gh run download "$RUN" -R $REPO -n nomospice-windows-latest -D "$WIN" || { echo "run $RUN has no Windows installers" >&2; exit 1; }
EXE=$(find "$WIN" -name "nomospice_${VERSION}_x64-setup.exe" | head -1)
MSI=$(find "$WIN" -name "nomospice_${VERSION}_x64_*.msi" | head -1)
[ -n "$EXE" ] && [ -n "$MSI" ] || { echo "the CI run has no $VERSION installers" >&2; exit 1; }
cp "$EXE" "$MSI" "$OUT/"
EXE=$OUT/$(basename "$EXE") MSI=$OUT/$(basename "$MSI")
scripts/sign-update.sh "$EXE"
scripts/sign-update.sh "$MSI"

# latest.json: what the updater installs on each platform
URL=https://github.com/$REPO/releases/download/$TAG \
MAC_NAME=$(basename "$MAC") MAC_SIG=$(cat "$MAC.sig") EXE_NAME=$(basename "$EXE") EXE_SIG=$(cat "$EXE.sig") MSI_NAME=$(basename "$MSI") MSI_SIG=$(cat "$MSI.sig") \
VERSION=$VERSION NOTES=$(cat "$NOTES_FILE") node -e '
  const e = process.env;
  console.log(JSON.stringify({ version: e.VERSION, notes: e.NOTES, pub_date: new Date().toISOString(), platforms: {
    "darwin-aarch64": { signature: e.MAC_SIG, url: `${e.URL}/${e.MAC_NAME}` },
    // per installer, so an MSI install is not updated by the NSIS setup (and vice versa)
    "windows-x86_64-nsis": { signature: e.EXE_SIG, url: `${e.URL}/${e.EXE_NAME}` },
    "windows-x86_64-msi": { signature: e.MSI_SIG, url: `${e.URL}/${e.MSI_NAME}` },
    "windows-x86_64": { signature: e.EXE_SIG, url: `${e.URL}/${e.EXE_NAME}` },
  } }, null, 2));' > "$OUT/latest.json"

gh release create "$TAG" -R $REPO --target "$SHA" --title "nomospice $VERSION" --notes-file "$NOTES_FILE" \
  "$OUT/nomospice_${VERSION}_aarch64.dmg" "$MAC" "$MAC.sig" "$EXE" "$EXE.sig" "$MSI" "$MSI.sig" "$OUT/latest.json"
echo "published https://github.com/$REPO/releases/tag/$TAG"
