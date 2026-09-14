# macOS release runbook

## Certificate and notarization

Direct website distribution uses **Developer ID Application: Aidoo Ltd. OOD (4KKVT2TUUA)** with Hardened Runtime. It does not use Apple Distribution, Mac Installer Distribution or a development certificate. The existing `AIDOO_VIEWER_NOTARY` notarytool Keychain profile authenticates Apple notarization.

The bundle identifier is `app.aidoo.whisper-lite`, the minimum system version is macOS 13, and the first release targets Apple Silicon only. The app requests microphone access and Accessibility; it is not App Sandbox constrained because it is distributed outside the Mac App Store.

## Version and local release

Keep the same semantic version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`. Update `website/release-notes.html`, regenerate `resources/THIRD_PARTY_NOTICES.txt`, commit, and create a `lite-vX.Y.Z` tag.

`npm run check:website` validates the three static public pages, their local links, required privacy/support/release content, matching product icon and release version. It also rejects active web elements and non-HTTPS external references.

Run `npm run release:mac`. It writes the final website assets to `release/<version>/`:

- notarized and stapled DMG;
- SHA-256 checksum for the DMG;
- privacy, support and release-notes pages with their shared style and product icon;
- `release-manifest.json` with the size and SHA-256 of every staged file.

Upload the DMG and checksum to the AIDOO website. Users install a new version by downloading the newer notarized DMG from the website; the application does not perform background or in-app update checks.

The release command automatically runs `npm run audit:mac` on the exact copied DMG. This independent check mounts the distribution image and revalidates its checksum, architecture, deployment target, identity, entitlements, notarization and Gatekeeper status without launching the application. Run `npm run audit:mac` again after transferring the files to another location or before upload.

Publish the DMG and checksum links on the product website. Publish the contents of `release/<version>/website/` alongside them. `npm run verify:website` confirms that every staged page matches `website/`, the checksum matches the DMG and the manifest matches all staged files.

## GitHub secrets for CI

The included workflow expects `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID`. Export the Developer ID Application identity with its private key as a password-protected `.p12`; store only its base64 value and password as repository secrets. Use an Apple app-specific password for notarization. The workflow audits the locked Rust and production JavaScript dependencies, then runs the same build, app-first stapling, DMG rebuild, notarization and `audit:mac` sequence used for a local release.

The local notarized release remains the reference path until those secrets are configured and a CI artifact passes the acceptance checklist.

## Mac App Store later

An App Store build requires a separate Apple Distribution signing path, App Sandbox entitlements and replacement of the private macOS overlay APIs. Treat it as a separate distribution target rather than reusing this direct-download bundle unchanged.
