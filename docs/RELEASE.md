# macOS release runbook

## Certificate and notarization

Direct website distribution uses **Developer ID Application: Aidoo Ltd. OOD (4KKVT2TUUA)** with Hardened Runtime. It does not use Apple Distribution, Mac Installer Distribution or a development certificate. The existing `AIDOO_VIEWER_NOTARY` notarytool Keychain profile authenticates Apple notarization.

The bundle identifier is `app.aidoo.whisper-lite`, the minimum system version is macOS 13, and the first release targets Apple Silicon only. The app requests microphone access and Accessibility; it is not App Sandbox constrained because it is distributed outside the Mac App Store.

## Version and local release

Keep the same semantic version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`. Update `website/release-notes.html`, regenerate `resources/THIRD_PARTY_NOTICES.txt`, commit, and create a `lite-vX.Y.Z` tag.

Run `npm run release:mac`. The script retrieves the updater-key password from Keychain without putting it in the repository. It produces:

- notarized and stapled application and DMG;
- SHA-256 checksum;
- signed `.app.tar.gz` updater artifact containing the stapled application, plus its `.sig`;
- `latest-lite.json` pointing at the GitHub release tag.

Create the GitHub release with tag `lite-vX.Y.Z` and upload the DMG, checksum, updater archive and signature. Upload `latest-lite.json` last to the rolling `lite-stable` release, after the versioned artifacts are available. The app checks `releases/download/lite-stable/latest-lite.json`, so full AIDOO Whisper releases cannot accidentally replace the Lite update channel.

Publish the DMG and checksum links on the product website. Publish `website/privacy.html`, `website/support.html` and `website/release-notes.html` alongside it.

## GitHub secrets for CI

The included workflow expects `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`, `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Export the Developer ID Application identity with its private key as a password-protected `.p12`; store only its base64 value and password as repository secrets. Use an Apple app-specific password for notarization.

The local notarized release remains the reference path until those secrets are configured and a CI artifact passes the acceptance checklist.

## Mac App Store later

An App Store build requires a separate Apple Distribution signing path, App Sandbox entitlements and replacement of the private macOS overlay APIs. Treat it as a separate distribution target rather than reusing this direct-download bundle unchanged.
