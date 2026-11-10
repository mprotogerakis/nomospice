#!/bin/sh
# Documentation screenshots (macOS): plays docs/scenes/<name>.js in a dev build of the app and
# writes its pictures to docs/img/<name>/. The app runs under its own identifier, so the
# installed app's settings and recovery files are not touched; the window may stay behind others.
#
#   scripts/docs-shots.sh                  every scene
#   scripts/docs-shots.sh first-circuit    one scene
#   scripts/docs-shots.sh ../site/docs/scenes/x.js   a scene of another site: its pictures in
#                                          that site's docs/img/x/ (its docs folder served to the app)
#
# Needs what `pnpm tauri dev` needs, and the ngspice library (NGSPICE_LIB, else the one
# scripts/macos-release.sh builds); pngquant, if there, makes the pictures smaller:
#   nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#pnpm nixpkgs#pngquant -c scripts/docs-shots.sh
set -eu
start=$PWD
cd "$(dirname "$0")/.."
: "${NGSPICE_LIB:=$PWD/src-tauri/target/ngspice-macos/out/opt/nomospice-ngspice/lib/libngspice.0.dylib}"
export NGSPICE_LIB
# (commits a scene makes: not the author's own name in the pictures)
export GIT_AUTHOR_NAME="nomospice docs" GIT_AUTHOR_EMAIL="docs@nomospice.invalid" GIT_COMMITTER_NAME="nomospice docs" GIT_COMMITTER_EMAIL="docs@nomospice.invalid"
names=${*:-$(for f in docs/scenes/*.js; do basename "$f" .js; done)}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
data="$HOME/Library/Application Support/de.nomospice.docs"
for n in $names; do
  case $n in
    */*) # (a scene file: absolute, without .js, for the app; the pictures beside its scenes folder)
      case $n in /*) ;; *) n=$start/$n ;; esac
      file=$(cd "$(dirname "$n")" && pwd)/$(basename "$n" .js)
      n=$(basename "$file")
      img=$(dirname "$(dirname "$file")")/img ;;
    *) file=$n; img=$PWD/docs/img ;;
  esac
  rm -rf "$img/$n"
  rm -f "$data"/recovery-* # (a scene ends with its schematic unsaved: no restore question next time)
  echo "scene $n"
  # (a process group of its own: pnpm, cargo and the app end together)
  set -m
  NOMOSPICE_SCENES=$file NOMOSPICE_SHOTS=$img NOMOSPICE_SCENE_WORK=$work/$n \
    pnpm tauri dev --config '{"identifier":"de.nomospice.docs"}' >"$work/$n.log" 2>&1 &
  app=$!
  set +m
  # (a scene stuck, say on a native dialog it cannot answer: ended after 10 minutes)
  ( sleep 600; echo "scene $n: timed out" >>"$work/$n.log"; kill -- -$app ) 2>/dev/null &
  dog=$!
  ok=0; wait $app || ok=$?
  kill $dog 2>/dev/null; wait $dog 2>/dev/null || true # (reaped quietly; killed, it returns 143)
  if [ $ok -ne 0 ]; then tail -20 "$work/$n.log"; exit 1; fi
  if [ -f "$img/$n/ERROR.png" ]; then echo "scene $n failed: $img/$n/ERROR.png"; grep -A3 "^scene:" "$work/$n.log" | head -8; exit 1; fi
  # (a palette per picture: a quarter of the size, no visible difference; pngquant if there is one)
  if command -v pngquant >/dev/null; then pngquant --quality=80-95 --skip-if-larger --force --ext .png "$img/$n"/*.png || true; fi
  ls "$img/$n"
done
