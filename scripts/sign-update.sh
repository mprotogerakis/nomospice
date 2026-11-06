#!/bin/sh
# sign-update.sh <file>: writes <file>.sig, the signature the in-app updater checks against the public
# key in tauri.conf.json. Key: ~/.tauri/nomospice-updater.key (or $UPDATER_KEY); its password is in the
# login keychain (service "nomospice-updater").
set -eu
cd "$(dirname "$0")/.."
KEY=${UPDATER_KEY:-$HOME/.tauri/nomospice-updater.key}
PW=$(security find-generic-password -s nomospice-updater -w)
node_modules/.bin/tauri signer sign -f "$KEY" -p "$PW" "$1" >/dev/null
test -s "$1.sig"
