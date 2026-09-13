#!/usr/bin/env bash
set -euo pipefail

project_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$project_root"

key_path="${TAURI_UPDATER_KEY_PATH:-$HOME/Library/Application Support/AIDOO Release Keys/Whisper Lite/updater.key}"
if [[ ! -f "$key_path" ]]; then
  echo "Липсва updater signing key: $key_path" >&2
  exit 1
fi
export TAURI_SIGNING_PRIVATE_KEY="$key_path"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-$(security find-generic-password -a updater-key-password -s app.aidoo.whisper-lite.release -w)}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/aidoo-whisper-lite-release-target}"

npm run check
cargo test --release --manifest-path src-tauri/Cargo.toml
cargo clippy --release --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npx tauri build --target aarch64-apple-darwin --bundles app,dmg

dmg="$(find "$CARGO_TARGET_DIR/aarch64-apple-darwin/release/bundle/dmg" -maxdepth 1 -name '*.dmg' -print -quit)"
app="$(find "$CARGO_TARGET_DIR/aarch64-apple-darwin/release/bundle/macos" -maxdepth 1 -name '*.app' -print -quit)"
archive="$(find "$CARGO_TARGET_DIR/aarch64-apple-darwin/release/bundle/macos" -maxdepth 1 -name '*.app.tar.gz' -print -quit)"
signature="$archive.sig"
test -f "$dmg" && test -d "$app" && test -f "$archive" && test -f "$signature"
"$project_root/scripts/notarize-app-mac.sh" "$app"
tar -czf "$archive" -C "$(dirname "$app")" "$(basename "$app")"
env -u TAURI_SIGNING_PRIVATE_KEY npx tauri signer sign --private-key-path "$key_path" --password "$TAURI_SIGNING_PRIVATE_KEY_PASSWORD" "$archive"
test -f "$signature"
"$project_root/scripts/notarize-mac.sh" "$dmg"

version="$(node -p "JSON.parse(require('fs').readFileSync('src-tauri/tauri.conf.json','utf8')).version")"
repo="${GITHUB_REPOSITORY:-bkafelov-aidoo/Aidoo-Whisper}"
asset_name="$(basename "$archive")"
pub_date="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
VERSION="$version" REPO="$repo" ASSET_NAME="$asset_name" PUB_DATE="$pub_date" SIGNATURE_PATH="$signature" node <<'NODE'
const fs = require('fs');
const metadata = {
  version: process.env.VERSION,
  notes: `AIDOO Whisper Lite ${process.env.VERSION}`,
  pub_date: process.env.PUB_DATE,
  platforms: {
    'darwin-aarch64': {
      signature: fs.readFileSync(process.env.SIGNATURE_PATH, 'utf8').trim(),
      url: `https://github.com/${process.env.REPO}/releases/download/lite-v${process.env.VERSION}/${encodeURIComponent(process.env.ASSET_NAME)}`
    }
  }
};
fs.writeFileSync('latest-lite.json', JSON.stringify(metadata, null, 2) + '\n');
NODE

printf 'Release assets:\n%s\n%s\n%s\n%s\n' "$dmg" "$dmg.sha256" "$archive" "$project_root/latest-lite.json"
