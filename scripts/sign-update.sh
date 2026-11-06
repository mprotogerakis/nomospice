#!/bin/sh
# sign-update.sh <file>: writes <file>.sig, bound to the version in tauri.conf.json (requireSignedVersion),, the signature the in-app updater checks against the public
# key in tauri.conf.json. Key: ~/.tauri/nomospice-updater.key (or $UPDATER_KEY); its password is in the
# login keychain (service "nomospice-updater").
set -eu
cd "$(dirname "$0")/.."
KEY=${UPDATER_KEY:-$HOME/.tauri/nomospice-updater.key}
# the password goes through the environment, not the command line (visible to other processes)
TAURI_SIGNING_PRIVATE_KEY_PASSWORD=$(security find-generic-password -s nomospice-updater -w) \
  node_modules/.bin/tauri signer sign -f "$KEY" --app-version "$(node -p "require('./src-tauri/tauri.conf.json').version")" "$1" >/dev/null
test -s "$1.sig"
