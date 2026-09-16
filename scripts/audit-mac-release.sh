#!/usr/bin/env bash
set -euo pipefail

project_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$project_root"

version="$(python3 - "$project_root/package.json" <<'PY'
from pathlib import Path
import json, sys
print(json.loads(Path(sys.argv[1]).read_text())["version"])
PY
)"
dmg="${1:-$project_root/release/$version/AIDOO Whisper Lite_${version}_aarch64.dmg}"
checksum="$dmg.sha256"
test -f "$dmg"
test -f "$checksum"
test "$(basename -- "$dmg")" = "AIDOO Whisper Lite_${version}_aarch64.dmg"

python3 - "$project_root" <<'PY'
from pathlib import Path
import json, re, sys

root = Path(sys.argv[1])
package_version = json.loads((root / "package.json").read_text())["version"]
tauri_version = json.loads((root / "src-tauri/tauri.conf.json").read_text())["version"]
cargo = (root / "src-tauri/Cargo.toml").read_text()
match = re.search(r'^version\s*=\s*"([^"]+)"', cargo, re.MULTILINE)
if not match:
    raise SystemExit("Cargo package version is missing")
if len({package_version, tauri_version, match.group(1)}) != 1:
    raise SystemExit("package.json, tauri.conf.json and Cargo.toml versions differ")

required = [
    root / "website/privacy.html",
    root / "website/support.html",
    root / "website/release-notes.html",
    root / "resources/THIRD_PARTY_NOTICES.txt",
    root / "src-tauri/infoplist/bg.lproj/InfoPlist.strings",
    root / "src-tauri/infoplist/en.lproj/InfoPlist.strings",
]
missing = [str(path) for path in required if not path.is_file() or path.stat().st_size == 0]
if missing:
    raise SystemExit("Missing release files: " + ", ".join(missing))

sources = "\n".join(
    path.read_text(errors="replace")
    for path in [
        root / "package.json",
        root / "src-tauri/Cargo.toml",
        root / "src-tauri/tauri.conf.json",
        root / "src-tauri/capabilities/main.json",
        root / "src-tauri/capabilities/overlay.json",
    ]
)
for forbidden in ("@tauri-apps/plugin-updater", "tauri-plugin-updater", "createUpdaterArtifacts\": true"):
    if forbidden in sources:
        raise SystemExit(f"Updater artifact remains configured: {forbidden}")

if (root / "public/app-icon.png").read_bytes() != (root / "website/app-icon.png").read_bytes():
    raise SystemExit("Website and product icons differ")
PY

expected_checksum="$(shasum -a 256 "$dmg" | awk '{print $1}')  $(basename -- "$dmg")"
test "$(cat "$checksum")" = "$expected_checksum"
checksum_dir="$(dirname -- "$checksum")"
(cd "$checksum_dir" && shasum -a 256 -c "$(basename -- "$checksum")")

mount_dir="$(mktemp -d -t aidoo-whisper-lite-audit)"
mounted=false
cleanup() {
  if [[ "$mounted" == true ]]; then
    hdiutil detach "$mount_dir" -quiet || true
  fi
  python3 - "$mount_dir" <<'PY'
from pathlib import Path
import shutil, sys
path = Path(sys.argv[1])
if path.exists():
    shutil.rmtree(path)
PY
}
trap cleanup EXIT

hdiutil attach "$dmg" -readonly -nobrowse -mountpoint "$mount_dir" -quiet
mounted=true
app="$mount_dir/AIDOO Whisper Lite.app"
test -d "$app"

codesign --verify --deep --strict --verbose=2 "$app"
xcrun stapler validate "$app"
xcrun stapler validate "$dmg"
spctl --assess --verbose=2 --type execute "$app"
spctl --assess --verbose=2 --type open --context context:primary-signature "$dmg"
codesign --verify --strict --verbose=2 "$dmg"
file "$app/Contents/MacOS/aidoo-whisper-lite" | grep -q 'arm64'
test "$(plutil -extract CFBundleIdentifier raw "$app/Contents/Info.plist")" = 'app.aidoo.whisper-lite'
test "$(plutil -extract CFBundleShortVersionString raw "$app/Contents/Info.plist")" = "$version"
test "$(plutil -extract LSMinimumSystemVersion raw "$app/Contents/Info.plist")" = '13.0'
test "$(plutil -extract NSMicrophoneUsageDescription raw "$app/Contents/Info.plist")" = 'AIDOO Whisper Lite uses your selected microphone for dictation and testing. If you enable Hey, AIDOO, wake-phrase detection stays local on this Mac.'
test "$(plutil -extract NSMicrophoneUsageDescription raw "$app/Contents/Resources/en.lproj/InfoPlist.strings")" = 'AIDOO Whisper Lite uses your selected microphone for dictation and testing. If you enable Hey, AIDOO, wake-phrase detection stays local on this Mac.'
test "$(plutil -extract NSMicrophoneUsageDescription raw "$app/Contents/Resources/bg.lproj/InfoPlist.strings")" = 'AIDOO Whisper Lite използва избрания микрофон за диктовка и тест. Ако включите Hey, AIDOO, разпознаването на фразата остава локално на този Mac.'
test "$(plutil -extract NSAppleEventsUsageDescription raw "$app/Contents/Info.plist")" = 'AIDOO Whisper Lite uses browser automation to show and refresh the relevant AIDOO Kontrol patient screen after confirmed changes.'
test "$(plutil -extract NSAppleEventsUsageDescription raw "$app/Contents/Resources/en.lproj/InfoPlist.strings")" = 'AIDOO Whisper Lite uses browser automation to show and refresh the relevant AIDOO Kontrol patient screen after confirmed changes.'
test "$(plutil -extract NSAppleEventsUsageDescription raw "$app/Contents/Resources/bg.lproj/InfoPlist.strings")" = 'AIDOO Whisper Lite използва автоматизация на браузъра, за да показва и обновява правилния пациентски екран в AIDOO Kontrol след потвърдени промени.'
test -f "$app/Contents/Resources/THIRD_PARTY_NOTICES.txt"
cmp -s "$app/Contents/Resources/icon.icns" "$project_root/src-tauri/icons/icon.icns"

signature="$(codesign -d --verbose=4 "$app" 2>&1)"
printf '%s' "$signature" | grep -Fq 'Authority=Developer ID Application: Aidoo Ltd. OOD (4KKVT2TUUA)'
printf '%s' "$signature" | grep -Fq 'TeamIdentifier=4KKVT2TUUA'
printf '%s' "$signature" | grep -Eq '^CodeDirectory .*flags=.*runtime'

dmg_signature="$(codesign -d --verbose=4 "$dmg" 2>&1)"
printf '%s' "$dmg_signature" | grep -Fq 'Authority=Developer ID Application: Aidoo Ltd. OOD (4KKVT2TUUA)'
printf '%s' "$dmg_signature" | grep -Fq 'TeamIdentifier=4KKVT2TUUA'

entitlements="$(codesign -d --entitlements :- "$app" 2>/dev/null)"
printf '%s' "$entitlements" | grep -q 'com.apple.security.device.audio-input'
printf '%s' "$entitlements" | grep -q 'com.apple.security.network.client'
printf '%s' "$entitlements" | grep -q 'com.apple.security.automation.apple-events'

printf 'AIDOO Whisper Lite macOS release audit passed: %s\n' "$dmg"
