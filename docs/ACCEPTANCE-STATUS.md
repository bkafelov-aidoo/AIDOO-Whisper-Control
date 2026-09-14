# macOS 1.0.0 acceptance status

Status date: 14 September 2026

## Verified on the development Apple Silicon Mac

- The complete TypeScript production build passes.
- All 70 representative backend error cases render without Bulgarian text in the English interface.
- All 19 native unit tests pass and native linting reports no warnings.
- The application and DMG are signed with `Developer ID Application: Aidoo Ltd. OOD (4KKVT2TUUA)`.
- Apple notarization tickets are stapled to both the application and DMG.
- Gatekeeper accepts both artifacts.
- The distribution is arm64, uses bundle ID `app.aidoo.whisper-lite`, and requires macOS 13 or newer.
- Required microphone and network entitlements are present.
- Product and website icons match; no updater is configured.
- `npm audit --omit=dev` reports no production dependency vulnerabilities.
- RustSec reports no known vulnerabilities in the locked Rust dependency graph. Its macOS-target dependency tree excludes the separately reported Linux-only `glib` advisory.
- The GitHub macOS release workflow repeats both dependency audits and runs the same signed-DMG audit used locally before storing a website artifact.
- The release audit passes against `release/1.0.0/AIDOO Whisper Lite_1.0.0_aarch64.dmg`.
- SHA-256: `aa41a6e53b175f294b633af9db7e740e7303b315d398996cff9b7c8386272b5e`.

Earlier interactive checks on this Mac covered onboarding, settings, shortcut capture, the overlay state flow, FLAC/TXT/history persistence, recovery after a failed transcription, clipboard copy, and the close/reopen lifecycle. They were not repeated after the final privacy-only change because microphone tests were paused at the user's request.

## Required before public launch

- Complete every behavioral item in [ACCEPTANCE.md](ACCEPTANCE.md) on the signed build while a tester is present.
- Complete the critical onboarding and dictation path on a second Mac or clean macOS account.
- Upload the exact audited DMG and matching checksum to the AIDOO website with the privacy, support, and release-notes pages.
