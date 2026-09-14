#!/usr/bin/env bash
set -euo pipefail

project_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$project_root"

app_path="${1:-}"
if [[ -z "$app_path" || ! -d "$app_path" ]]; then
  echo "Подписаното .app приложение не е намерено." >&2
  exit 1
fi

submission_zip="$(mktemp -t aidoo-whisper-lite-notary).zip"
trap 'python3 - "$submission_zip" <<'PY'
import os, sys
try:
    os.unlink(sys.argv[1])
except FileNotFoundError:
    pass
PY' EXIT

ditto -c -k --keepParent "$app_path" "$submission_zip"
# shellcheck source=notarytool-auth-mac.sh
source "$project_root/scripts/notarytool-auth-mac.sh"
configure_notary_auth
result="$(xcrun notarytool submit "$submission_zip" "${notary_auth_args[@]}" --wait --timeout 60m --output-format json)"
status="$(printf '%s' "$result" | plutil -extract status raw -o - - 2>/dev/null || true)"
submission_id="$(printf '%s' "$result" | plutil -extract id raw -o - - 2>/dev/null || true)"
if [[ "$status" != "Accepted" ]]; then
  printf 'Apple app notarization failed (%s). Submission: %s\n' "${status:-unknown}" "${submission_id:-unknown}" >&2
  exit 1
fi

xcrun stapler staple "$app_path"
xcrun stapler validate "$app_path"
codesign --verify --deep --strict --verbose=2 "$app_path"
spctl -a -vv --type execute "$app_path"
printf 'Notarized app: %s\n' "$app_path"
