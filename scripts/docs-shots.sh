#!/bin/sh
# Documentation screenshots (macOS): plays docs/scenes/<name>.js in a dev build of the app and
# writes its pictures to docs/img/<name>/. The app runs under its own identifier, so the
# installed app's settings and recovery files are not touched; the window may stay behind others.
#
#   scripts/docs-shots.sh                  every scene
#   scripts/docs-shots.sh first-circuit    one scene
#   scripts/docs-shots.sh ../site/docs/scenes/x.js   a scene of another site: its pictures in
#                                          that site's docs/img/x/ (its scenes folder served to the app)
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
[ $# -gt 0 ] || set -- $(for f in docs/scenes/*.js; do basename "$f" .js; done)
work=$(mktemp -d)
k=0
app= dog=
# (interrupted: the scene's app and its watchdog end too, each a process group of its own)
trap 'for g in $app $dog; do kill -- -$g 2>/dev/null || true; done; rm -rf "$work"' EXIT
trap 'exit 130' INT TERM
data="$HOME/Library/Application Support/de.nomospice.docs"
for n in "$@"; do
  k=$((k + 1))
  case $n in
    */*) # (a scene file: absolute, without .js, for the app; the pictures beside its scenes folder)
      case $n in /*) ;; *) n=$start/$n ;; esac
      file=$(cd "$(dirname "$n")" && pwd)/$(basename "$n" .js)
      n=$(basename "$file")
      # (in a scenes folder, else its pictures folder would be anywhere)
      [ "$(basename "$(dirname "$file")")" = scenes ] && [ -f "$file.js" ] || { echo "not a scene in a scenes folder: $file.js" >&2; exit 1; }
      img=$(dirname "$(dirname "$file")")/img ;;
    *) file=$n; img=$PWD/docs/img ;;
  esac
  rm -rf "$img/$n"
  rm -f "$data"/recovery-* # (a scene ends with its schematic unsaved: no restore question next time)
  echo "scene $n"
  # (a process group of its own: pnpm, cargo and the app end together)
  set -m
  log=$work/$k-$n.log # (numbered: two sites' scenes may share a name)
  NOMOSPICE_SCENES=$file NOMOSPICE_SHOTS=$img NOMOSPICE_SCENE_WORK=$work/$k-$n \
    pnpm tauri dev --config '{"identifier":"de.nomospice.docs"}' >"$log" 2>&1 &
  app=$!
  # (a scene stuck, say on a native dialog it cannot answer: ended after 10 minutes; a group of
  # its own too, so its sleep ends with it)
  ( sleep 600; echo "scene $n: timed out" >>"$log"; kill -- -$app ) 2>/dev/null &
  dog=$!
  set +m
  ok=0; wait $app || ok=$?
  kill -- -$dog 2>/dev/null || true # (gone already when it fired)
  wait $dog 2>/dev/null || true # (reaped quietly; killed, it returns 143)
  app= dog= # (gone: their ids may be another process's by the end)
  if [ $ok -ne 0 ]; then tail -20 "$log"; exit 1; fi
  if [ -f "$img/$n/ERROR.png" ]; then echo "scene $n failed: $img/$n/ERROR.png"; grep -A3 "^scene:" "$log" | head -8; exit 1; fi
  # (a palette per picture: a quarter of the size, no visible difference; pngquant if there is one)
  if command -v pngquant >/dev/null; then pngquant --quality=80-95 --skip-if-larger --force --ext .png "$img/$n"/*.png || true; fi
  ls "$img/$n"
done
