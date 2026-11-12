#!/bin/bash
# Build DeepSider.app + DeepSider-<version>.dmg without Tauri's bundle_dmg.sh
# (its hdiutil staging volume cannot be unmounted reliably in this environment).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="$(python3 -c 'import json;print(json.load(open("src-tauri/tauri.conf.json"))["version"])')"
APP="src-tauri/target/release/bundle/macos/DeepSider.app"
OUT="dist"
NAME="DeepSider-${VERSION}"

echo "==> tauri build (app bundle only)"
./node_modules/.bin/tauri build --bundles app

echo "==> staging dmg"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"

mkdir -p "$OUT"
rm -f "$OUT/${NAME}.dmg"
diskutil image create from "$STAGE" --volumeName "$NAME" --format UDZO "$OUT/${NAME}.dmg"

echo "==> zipping app"
rm -f "$OUT/${NAME}.zip"
ditto -c -k --keepParent "$APP" "$OUT/${NAME}.zip"

echo "==> done"
ls -lh "$OUT"
