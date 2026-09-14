#!/usr/bin/env bash
set -euo pipefail

project_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$project_root"

notary_profile="${NOTARY_PROFILE:-AIDOO_VIEWER_NOTARY}"
target_root="${CARGO_TARGET_DIR:-src-tauri/target}"
dmg_path="${1:-}"
if [[ -z "$dmg_path" ]]; then
  dmg_path="$(find "$target_root/aarch64-apple-darwin/release/bundle/dmg" -maxdepth 1 -name '*.dmg' -print -quit 2>/dev/null || true)"
fi
if [[ -z "$dmg_path" || ! -f "$dmg_path" ]]; then
  echo "DMG файлът не е намерен. Първо изпълнете npm run bundle:mac." >&2
  exit 1
fi

result="$(xcrun notarytool submit "$dmg_path" --keychain-profile "$notary_profile" --wait --timeout 60m --output-format json)"
status="$(printf '%s' "$result" | plutil -extract status raw -o - - 2>/dev/null || true)"
submission_id="$(printf '%s' "$result" | plutil -extract id raw -o - - 2>/dev/null || true)"
if [[ "$status" != "Accepted" ]]; then
  printf 'Apple notarization failed (%s). Submission: %s\n' "${status:-unknown}" "${submission_id:-unknown}" >&2
  exit 1
fi

xcrun stapler staple "$dmg_path"
xcrun stapler validate "$dmg_path"
spctl -a -vv --type open --context context:primary-signature "$dmg_path"
dmg_name="$(basename "$dmg_path")"
dmg_hash="$(shasum -a 256 "$dmg_path" | awk '{print $1}')"
printf '%s  %s\n' "$dmg_hash" "$dmg_name" | tee "$dmg_path.sha256"
printf 'Notarized DMG: %s\n' "$dmg_path"
