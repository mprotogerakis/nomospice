#!/bin/sh
# macOS release: Apple Silicon app with its own ngspice, signed (Developer ID, hardened runtime),
# notarized and stapled. Needs a "Developer ID Application" certificate in the keychain and an
# App Store Connect API key:
#   NOTARY_KEY=~/.appstoreconnect/private_keys/ApiKey_XXXX.p8 NOTARY_KEY_ID=XXXX [NOTARY_ISSUER=…] scripts/macos-release.sh
set -eu
cd "$(dirname "$0")/.."
: "${NOTARY_KEY:?}" "${NOTARY_KEY_ID:?}"
ID=$(security find-identity -v -p codesigning | grep -o '"Developer ID Application: [^"]*"' | head -1 | tr -d '"')
[ -n "$ID" ] || { echo "no Developer ID Application certificate" >&2; exit 1; }
VERSION=$(node -p "require('./src-tauri/tauri.conf.json').version")
NGV=47
NG=$PWD/src-tauri/target/ngspice-macos
NGLIB=$NG/out/opt/nomospice-ngspice/lib
notarize() { xcrun notarytool submit "$1" --key "$NOTARY_KEY" --key-id "$NOTARY_KEY_ID" ${NOTARY_ISSUER:+--issuer "$NOTARY_ISSUER"} --wait; }

# 1. ngspice for macOS 11+ (Homebrew's only runs on the macOS it was built for). The prefix does not
#    exist on users' machines, so ngspice finds no foreign spinit/code models; the app loads its own.
if [ ! -f "$NGLIB/libngspice.0.dylib" ]; then
  mkdir -p "$NG/src"
  curl -fsSL -A Wget -o "$NG/src/ngspice-$NGV.tar.gz" "https://downloads.sourceforge.net/project/ngspice/ng-spice-rework/$NGV/ngspice-$NGV.tar.gz"
  tar -xzf "$NG/src/ngspice-$NGV.tar.gz" -C "$NG/src"
  (cd "$NG/src/ngspice-$NGV" && make distclean >/dev/null 2>&1 || true
   MACOSX_DEPLOYMENT_TARGET=11.0 ./configure --with-ngshared --enable-cider --disable-openmp --disable-debug --prefix=/opt/nomospice-ngspice
   make -j8 && make install DESTDIR="$NG/out")
fi

# 2. the app; ngspice goes into Contents/Frameworks (searched first), code models into Frameworks/ngspice
pnpm tauri build --bundles app
APP=src-tauri/target/release/bundle/macos/nomospice.app
mkdir -p "$APP/Contents/Frameworks/ngspice"
cp "$NGLIB/libngspice.0.dylib" "$APP/Contents/Frameworks/libngspice.dylib"
cp "$NGLIB"/ngspice/*.cm "$APP/Contents/Frameworks/ngspice/"

# 3. sign inside-out, notarize the app and staple it (works offline once copied out of the image)
for f in "$APP"/Contents/Frameworks/ngspice/*.cm "$APP/Contents/Frameworks/libngspice.dylib" "$APP"; do
  codesign --force --timestamp --options runtime --sign "$ID" "$f"
done
OUT=src-tauri/target/release/bundle/release && rm -rf "$OUT" && mkdir -p "$OUT"
ditto -c -k --keepParent "$APP" "$OUT/app.zip" && notarize "$OUT/app.zip" && rm "$OUT/app.zip"
xcrun stapler staple "$APP"

# 4. disk image with an Applications link, signed, notarized, stapled
STAGE=$(mktemp -d) && cp -R "$APP" "$STAGE/" && ln -s /Applications "$STAGE/Applications"
DMG=$OUT/nomospice_${VERSION}_aarch64.dmg
hdiutil create -quiet -volname nomospice -srcfolder "$STAGE" -ov -format UDZO "$DMG" && rm -rf "$STAGE"
codesign --force --timestamp --sign "$ID" "$DMG"
notarize "$DMG"
xcrun stapler staple "$DMG"
spctl -a -t open --context context:primary-signature -v "$DMG"

# 5. the in-app updater's archive of the stapled app, signed (scripts/publish-release.sh uploads it)
COPYFILE_DISABLE=1 tar -czf "$OUT/nomospice_${VERSION}_aarch64.app.tar.gz" -C "$(dirname "$APP")" nomospice.app
scripts/sign-update.sh "$OUT/nomospice_${VERSION}_aarch64.app.tar.gz"
# which commit this is (publish-release.sh checks it); built with local changes: not a commit
echo "$(git rev-parse HEAD)$([ -n "$(git status --porcelain --untracked-files=no)" ] && echo -dirty)" > "$OUT/built-from"
echo "$DMG"
