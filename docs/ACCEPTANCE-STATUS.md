# macOS 1.0.0 acceptance status

Status date: 14 September 2026

## Verified on the development Apple Silicon Mac

- The complete TypeScript production build passes.
- All 70 representative backend error cases render without Bulgarian text in the English interface.
- Current OpenAI documentation confirms the selected model aliases, displayed price estimates, FLAC transcription input and explicit ISO-639-1 language hint.
- All 19 native unit tests pass and native linting reports no warnings.
- The application and DMG are signed with `Developer ID Application: Aidoo Ltd. OOD (4KKVT2TUUA)`.
- Apple notarization tickets are stapled to both the application and DMG.
- Gatekeeper accepts both artifacts.
- The distribution is arm64, uses bundle ID `app.aidoo.whisper-lite`, and requires macOS 13 or newer.
- Required microphone and network entitlements are present.
- The signed bundle includes verified English and Bulgarian macOS Microphone permission explanations, plus the third-party notices at the application resource root.
- Product and website icons match; no updater is configured.
- `npm audit --omit=dev` reports no production dependency vulnerabilities.
- RustSec reports no known vulnerabilities in the locked Rust dependency graph. Its macOS-target dependency tree excludes the separately reported Linux-only `glib` advisory.
- The GitHub macOS release workflow repeats both dependency audits and runs the same app-first stapling, DMG rebuild, notarization and signed-DMG audit used locally before storing a website artifact.
- The release audit passes against `release/1.0.0/AIDOO Whisper Lite_1.0.0_aarch64.dmg`.
- The website release package contains the audited DMG, matching checksum, exact privacy/support/release pages and a verified SHA-256 manifest for every staged file.
- The public website validation passes for all three pages, local references, required content, version consistency and absence of active embedded elements.
- SHA-256: `b6a2a376e413d23a4743d9cba2b10130b5e9fa4c56d62014b49b1db8925260bb`.

Earlier interactive checks on this Mac covered onboarding, settings, shortcut capture, the overlay state flow, FLAC/TXT/history persistence, recovery after a failed transcription, clipboard copy, and the close/reopen lifecycle. They were not repeated after the final privacy and packaging changes because microphone tests were paused at the user's request.

## Required before public launch

- Complete every behavioral item in [ACCEPTANCE.md](ACCEPTANCE.md) on the signed build while a tester is present.
- Complete the critical onboarding and dictation path on a second Mac or clean macOS account.
- Upload the exact audited DMG and matching checksum to the AIDOO website with the privacy, support, and release-notes pages.
