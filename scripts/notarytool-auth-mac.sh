#!/usr/bin/env bash

# Populates notary_auth_args for xcrun notarytool. Local releases use the
# Keychain profile; CI can use either Apple ID or App Store Connect API keys.
configure_notary_auth() {
  notary_auth_args=()

  if [[ -n "${APPLE_API_KEY_PATH:-}" || -n "${APPLE_API_KEY:-}" || -n "${APPLE_API_ISSUER:-}" ]]; then
    if [[ -z "${APPLE_API_KEY_PATH:-}" || -z "${APPLE_API_KEY:-}" ]]; then
      echo "APPLE_API_KEY_PATH and APPLE_API_KEY must both be set for notarization." >&2
      return 1
    fi
    notary_auth_args=(--key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY")
    if [[ -n "${APPLE_API_ISSUER:-}" ]]; then
      notary_auth_args+=(--issuer "$APPLE_API_ISSUER")
    fi
    return 0
  fi

  if [[ -n "${APPLE_ID:-}" || -n "${APPLE_PASSWORD:-}" || -n "${APPLE_TEAM_ID:-}" ]]; then
    if [[ -z "${APPLE_ID:-}" || -z "${APPLE_PASSWORD:-}" || -z "${APPLE_TEAM_ID:-}" ]]; then
      echo "APPLE_ID, APPLE_PASSWORD and APPLE_TEAM_ID must all be set for notarization." >&2
      return 1
    fi
    notary_auth_args=(--apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID")
    return 0
  fi

  notary_auth_args=(--keychain-profile "${NOTARY_PROFILE:-AIDOO_VIEWER_NOTARY}")
  if [[ -n "${NOTARY_KEYCHAIN:-}" ]]; then
    notary_auth_args+=(--keychain "$NOTARY_KEYCHAIN")
  fi
}
