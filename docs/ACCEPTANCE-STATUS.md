# macOS 1.0.0 acceptance status

Status date: 15 September 2026

## Current audited release candidate

- Source: parent commit `5d5d72527c1b8a79032bbdd79b51d5f6bcafe441` (`Prevent duplicate recovery transcription charges`), exported to the dedicated repository as `1eaeee7bafd285d01b70c71bc2da6f531d9d41e4`.
- Package: `release/1.0.0/AIDOO Whisper Lite_1.0.0_aarch64.dmg`.
- SHA-256: `6c268e0338e609c36c7cb434e71146823e35b317cae8bc626415173aada23f3d`.
- Architecture: Apple Silicon (`arm64`); minimum macOS version: 13; bundle ID: `app.aidoo.whisper-lite`.
- Signing identity: `Developer ID Application: Aidoo Ltd. OOD (4KKVT2TUUA)`.
- Apple notarization tickets are stapled to the application and DMG. Gatekeeper accepts both as `Notarized Developer ID`.
- The audited application was installed from this exact DMG and launched from `/Applications/AIDOO Whisper Lite.app`.

## Automated evidence

- `npm run release:mac` completed successfully from a clean tree at the exact local `lite-v1.0.0` source tag.
- TypeScript checking, all 89 representative English error-localization cases, release configuration, Apple Silicon dependency boundary and all three website-page checks pass.
- The production frontend build passes.
- All 45 native unit tests pass and Rust Clippy passes for all Apple Silicon release targets with warnings denied.
- `npm audit --omit=dev` and the current RustSec database scan report no production vulnerabilities.
- GitHub Actions run [34973311920](https://github.com/bkafelov-aidoo/AIDOO-Whisper-Lite/actions/runs/34973311920) passes checkout, pinned tool setup, both dependency audits, frontend checks/build, all Rust tests and strict Clippy for dedicated-repository commit `1eaeee7bafd285d01b70c71bc2da6f531d9d41e4`.
- Recovery safety tests prove that audio is marked non-retryable before an ambiguous OpenAI request, completed text can only take the local-finalization path, cleanup failure leaves a Delete-only item, missing/corrupt recovery metadata fails safe, and no already charged audio can re-enter the network path.
- History file deletion is journaled and crash-safe; the automated suite covers rollback before the history commit, cleanup after the commit and the narrow interruption between the JSON commit and journal phase update.
- The 25 MB upload boundary is tested on both sides, every FLAC/TXT save combination is tested with private file permissions, and a stale five-minute watchdog is proven unable to stop a later recording.
- The release audit verifies the checksum, architecture, minimum OS, bundle ID, entitlements, exact AIDOO Developer ID and Team ID, notarization tickets, Gatekeeper acceptance, absence of an updater, required website documents and matching product/website icons.
- The signed bundle includes English and Bulgarian macOS Microphone permission explanations and current third-party notices.

## Behavioral evidence on the development Apple Silicon Mac

- Existing settings and the Keychain API key survive replacement of an older build and installation from the final DMG. The API key is displayed only as saved and is absent from persisted settings and diagnostics.
- Onboarding was completed in English, including the explicit microphone test, Accessibility state, default Right Option shortcut, model descriptions/prices and independent FLAC/TXT/history storage explanations.
- A real Bulgarian microphone dictation completed through the Economy model. The result reached history and the clipboard, and its FLAC/TXT files were saved with a twelve-character identifier and `0600` permissions.
- The overlay appears while listening, shows elapsed recording time and remains above the main interface without an opaque window background.
- After recording stops, the overlay remains visible until the app can accept a new recording. It names the current operation and shows progress for preparation, compression, upload, OpenAI transcription and local finalization.
- The main window and menu bar use the same busy/error/ready state model. During recording the main controls are disabled and the Stop and transcribe action remains available.
- The exact notarized app installed from the final DMG successfully retranscribed existing 47-second FLAC audio with the Economy model. The overlay visibly showed `OpenAI is transcribing` with progress, disappeared only after completion, and the app then returned to `Ready for dictation` with a new history item.
- The same installed app successfully retranscribed a three-second FLAC with the Maximum accuracy model (`gpt-transcribe`). The result is labelled `Maximum accuracy` in history; the user's Economy preference was restored afterward.
- Newly generated release-test FLAC/TXT files use `0600`; legacy six-character files remain readable without being rewritten.
- A diagnostics package created by the installed build uses `0600`, contains only `diagnostics.log`, redacted `settings.json` and `system.txt`, and contains no OpenAI key, known transcription text or user home path.
- Earlier interactive checks also covered settings, shortcut capture cancellation, local persistence, recovery after a failed transcription, clipboard copy and the close/reopen lifecycle.

## Required before public launch

- Complete the first-run and critical dictation path from [ACCEPTANCE.md](ACCEPTANCE.md) on a second Apple Silicon Mac or a clean macOS account, including Bulgarian/English system-language behavior and fresh Microphone/Accessibility permission prompts.
- Complete the remaining physical-input cases: a modified shortcut, physical Escape during shortcut capture, five-minute automatic stop, sleep/wake synchronization and microphone disconnect with fallback enabled and disabled.
- Exercise every FLAC/TXT/history toggle combination through the signed UI, output-folder change, history deletion with and without linked files and Launch at Login. Maximum accuracy has been verified on the signed build; all FLAC/TXT combinations and crash-safe history deletion are covered by native tests.
- Run the failure-injection cases in [ACCEPTANCE.md](ACCEPTANCE.md), especially local finalization failure, over-25-MB audio, ambiguous OpenAI outcomes, forced termination around each OpenAI request path and recovery metadata write/corruption.
- Verify representative Bulgarian and English microphone/OpenAI errors, Accessibility-denied clipboard fallback, diagnostic ZIP failure cleanup and VoiceOver announcements.
- Configure the six Apple signing/notarization secrets in the private GitHub repository before relying on its release workflow. Local Keychain notarization is already validated.
- Upload this exact audited DMG and matching checksum to the AIDOO website together with the privacy, support and release-notes pages.
