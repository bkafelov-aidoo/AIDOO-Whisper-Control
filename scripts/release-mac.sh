#!/usr/bin/env bash
set -euo pipefail

project_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$project_root"

# Keep notarization credentials out of npm, Cargo and package build processes.
# They are exported only to the two short-lived notarization commands below.
notary_apple_id="${APPLE_ID:-}"
notary_apple_password="${APPLE_PASSWORD:-}"
notary_team_id="${APPLE_TEAM_ID:-}"
notary_api_key_path="${APPLE_API_KEY_PATH:-}"
notary_api_key="${APPLE_API_KEY:-}"
notary_api_issuer="${APPLE_API_ISSUER:-}"
unset APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID
unset APPLE_API_KEY_PATH APPLE_API_KEY APPLE_API_ISSUER

run_notarization() {
  APPLE_ID="$notary_apple_id" \
    APPLE_PASSWORD="$notary_apple_password" \
    APPLE_TEAM_ID="$notary_team_id" \
    APPLE_API_KEY_PATH="$notary_api_key_path" \
    APPLE_API_KEY="$notary_api_key" \
    APPLE_API_ISSUER="$notary_api_issuer" \
    "$@"
}

working_tree_state="$(git status --porcelain=v1 --untracked-files=normal -- .)"
if [[ -n "$working_tree_state" ]]; then
  printf 'Refusing to release from a working tree with uncommitted Control changes:\n%s\n' \
    "$working_tree_state" >&2
  exit 1
fi

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/aidoo-whisper-control-release-target}"
expected_signing_identity="Developer ID Application: Aidoo Ltd. OOD (4KKVT2TUUA)"
signing_identity="${APPLE_SIGNING_IDENTITY:-$expected_signing_identity}"
if [[ "$signing_identity" != "$expected_signing_identity" ]]; then
  printf 'Unexpected signing identity: %s\nExpected: %s\n' \
    "$signing_identity" "$expected_signing_identity" >&2
  exit 1
fi
version="$(python3 - "$project_root/package.json" <<'PY'
from pathlib import Path
import json, sys
print(json.loads(Path(sys.argv[1]).read_text())["version"])
PY
)"
expected_tag="control-v${version}"
if ! git tag --points-at HEAD --list "$expected_tag" | grep -Fxq "$expected_tag"; then
  printf 'Refusing to release untagged source. HEAD must have tag %s.\n' "$expected_tag" >&2
  exit 1
fi

python3 scripts/generate-third-party-notices.py
git diff --exit-code -- resources/THIRD_PARTY_NOTICES.txt
npm run check
cargo test --locked --release --target aarch64-apple-darwin --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --release --target aarch64-apple-darwin --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
# The release scripts notarize the stapled app first, then rebuild and notarize
# the DMG. Keep Tauri's automatic notarization disabled so CI and local releases
# use this exact sequence once.
npx tauri build --target aarch64-apple-darwin --bundles app,dmg --ci -- --locked

dmg="$CARGO_TARGET_DIR/aarch64-apple-darwin/release/bundle/dmg/AIDOO Whisper Control_${version}_aarch64.dmg"
app="$CARGO_TARGET_DIR/aarch64-apple-darwin/release/bundle/macos/AIDOO Whisper Control.app"
test -f "$dmg" && test -d "$app"
codesign --verify --deep --strict --verbose=2 "$app"
file "$app/Contents/MacOS/aidoo-whisper-control" | grep -q 'arm64'
test "$(plutil -extract CFBundleIdentifier raw "$app/Contents/Info.plist")" = 'app.aidoo.whisper-control'
test "$(plutil -extract CFBundleShortVersionString raw "$app/Contents/Info.plist")" = "$version"
test "$(plutil -extract LSMinimumSystemVersion raw "$app/Contents/Info.plist")" = '13.0'
signature="$(codesign -d --verbose=4 "$app" 2>&1)"
printf '%s' "$signature" | grep -Fq 'Authority=Developer ID Application: Aidoo Ltd. OOD (4KKVT2TUUA)'
printf '%s' "$signature" | grep -Fq 'TeamIdentifier=4KKVT2TUUA'
printf '%s' "$signature" | grep -Eq '^CodeDirectory .*flags=.*runtime'
run_notarization "$project_root/scripts/notarize-app-mac.sh" "$app"

# Rebuild the disk image from the stapled application so offline Gatekeeper validation
# succeeds for the exact copy that users install from the AIDOO website.
dmg_staging="$(mktemp -d -t aidoo-whisper-control-dmg)"
verify_mount="$(mktemp -d -t aidoo-whisper-control-mount)"
mounted=false
cleanup() {
  if [[ "$mounted" == true ]]; then
    hdiutil detach "$verify_mount" -quiet || true
  fi
  python3 - "$dmg_staging" "$verify_mount" <<'PY'
from pathlib import Path
import shutil, sys
for value in sys.argv[1:]:
    path = Path(value)
    if path.exists():
        shutil.rmtree(path)
PY
}
trap cleanup EXIT
ditto "$app" "$dmg_staging/$(basename "$app")"
ln -s /Applications "$dmg_staging/Applications"
python3 - "$dmg" <<'PY'
from pathlib import Path
import sys
Path(sys.argv[1]).unlink(missing_ok=True)
PY
hdiutil create -volname "AIDOO Whisper Control" -srcfolder "$dmg_staging" -ov -format UDZO "$dmg"
codesign --force --sign "$signing_identity" --timestamp "$dmg"
codesign --verify --strict --verbose=2 "$dmg"
dmg_signature="$(codesign -d --verbose=4 "$dmg" 2>&1)"
printf '%s' "$dmg_signature" | grep -Fq "Authority=$expected_signing_identity"
printf '%s' "$dmg_signature" | grep -Fq 'TeamIdentifier=4KKVT2TUUA'

run_notarization "$project_root/scripts/notarize-mac.sh" "$dmg"
notary_apple_id=""
notary_apple_password=""
notary_team_id=""
notary_api_key_path=""
notary_api_key=""
notary_api_issuer=""

hdiutil attach "$dmg" -readonly -nobrowse -mountpoint "$verify_mount" -quiet
mounted=true
installed_app="$verify_mount/$(basename "$app")"
codesign --verify --deep --strict --verbose=2 "$installed_app"
xcrun stapler validate "$installed_app"
spctl --assess --verbose=2 --type execute "$installed_app"
hdiutil detach "$verify_mount" -quiet
mounted=false

release_dir="$project_root/release/$version"
release_dmg="$release_dir/$(basename "$dmg")"
python3 "$project_root/scripts/package-website-release.py" --dmg "$dmg" --output "$release_dir"
"$project_root/scripts/audit-mac-release.sh" "$release_dmg"
python3 "$project_root/scripts/package-website-release.py" --verify --output "$release_dir"

printf 'Release package:\n%s\n' "$release_dir"
