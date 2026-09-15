# macOS 1.0.0 acceptance status

Status date: 15 September 2026

> The artifact recorded below is the last fully audited candidate. It is superseded by newer source changes and must not be published. Produce and audit a fresh signed build before launch, then replace this note and checksum with the final artifact evidence.

## Last audited candidate on the development Apple Silicon Mac

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
- RustSec reports no status-failing vulnerabilities in the locked dependency graph. It reports the informational Linux-only `glib` advisory `RUSTSEC-2024-0429`; an independent release check proves that the affected package is absent from the Apple Silicon macOS graph.
- The GitHub macOS release workflow repeats both dependency audits and runs the same app-first stapling, DMG rebuild, notarization and signed-DMG audit used locally before storing a website artifact.
- The release audit passes against `release/1.0.0/AIDOO Whisper Lite_1.0.0_aarch64.dmg`.
- The website release package contains the audited DMG, matching checksum, exact privacy/support/release pages and a verified SHA-256 manifest for every staged file.
- The public website validation passes for all three pages, local references, required content, version consistency and absence of active embedded elements.
- SHA-256: `b6a2a376e413d23a4743d9cba2b10130b5e9fa4c56d62014b49b1db8925260bb`.

Earlier interactive checks on this Mac covered onboarding, settings, shortcut capture, the overlay state flow, FLAC/TXT/history persistence, recovery after a failed transcription, clipboard copy, and the close/reopen lifecycle. They were not repeated after the final privacy and packaging changes because microphone tests were paused at the user's request.

## Current source after the superseded candidate

The current source contains additional safeguards that are not present in the artifact above:

- application commands and Tauri capabilities are separated by window, and the overlay receives only recording state;
- release workflows are pinned, secrets are limited to the steps that need them, and releases require clean source at the exact version tag;
- OpenAI uploads and response bodies have explicit size limits, API messages are bounded, and all network operations have timeouts;
- settings, history and recovery metadata have read limits; malformed private JSON is preserved in a private quarantine file before safe recovery;
- failed audio is recoverable even when its metadata is missing, and its filename records whether retry is safe without risking another API charge;
- temporary recording files and native audio-worker waits are bounded, including cleanup of results abandoned after a timeout;
- long recovery errors wrap in the main window, and a failed automatic paste remains available in the menu bar after the toast disappears.

The current source was inspected without launching the application or using the microphone. Short compilation and static checks completed after the relevant changes, but the full automated suite and every behavioral item below remain required on a fresh signed candidate.

## Required before public launch

- Build, sign, notarize, staple and audit a fresh candidate from the current source, then record its checksum here.
- Complete every behavioral item in [ACCEPTANCE.md](ACCEPTANCE.md) on the signed build while a tester is present.
- Complete the critical onboarding and dictation path on a second Mac or clean macOS account.
- Inject a failed `failed-recording.json` write, restart the signed app, and confirm that the newest Recovery audio reappears with its encoded retry status; a legacy file must reappear without a Retry action.
- Upload the exact audited DMG and matching checksum to the AIDOO website with the privacy, support, and release-notes pages.
