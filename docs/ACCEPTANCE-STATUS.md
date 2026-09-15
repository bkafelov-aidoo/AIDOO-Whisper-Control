# macOS 1.0.0 acceptance status

Status date: 15 September 2026

> The artifact recorded below is the last fully audited candidate. It is superseded by newer source changes, contains `rustls 0.23.44` affected by `RUSTSEC-2026-0285`, and must not be published. Produce and audit a fresh signed build before launch, then replace this note and checksum with the final artifact evidence.

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
- At the time this candidate was audited, RustSec reported no status-failing vulnerabilities and only the informational Linux-only `glib` advisory `RUSTSEC-2024-0429`. The later `RUSTSEC-2026-0285` advisory now disqualifies this candidate because it contains `rustls 0.23.44`.
- The GitHub macOS release workflow repeats both dependency audits and runs the same app-first stapling, DMG rebuild, notarization and signed-DMG audit used locally before storing a website artifact.
- The release audit passed against the candidate now quarantined at `release-private/superseded-20260915-rustls-0.23.44/1.0.0/AIDOO Whisper Lite_1.0.0_aarch64.dmg`; the standard `release/1.0.0` publication path has been cleared.
- The website release package contains the audited DMG, matching checksum, exact privacy/support/release pages and a verified SHA-256 manifest for every staged file.
- The public website validation passes for all three pages, local references, required content, version consistency and absence of active embedded elements.
- SHA-256: `b6a2a376e413d23a4743d9cba2b10130b5e9fa4c56d62014b49b1db8925260bb`.

Earlier interactive checks on this Mac covered onboarding, settings, shortcut capture, the overlay state flow, FLAC/TXT/history persistence, recovery after a failed transcription, clipboard copy, and the close/reopen lifecycle. They were not repeated after the final privacy and packaging changes because microphone tests were paused at the user's request.

## Current source after the superseded candidate

The current source contains additional safeguards that are not present in the artifact above:

- application commands and Tauri capabilities are separated by window, and the overlay receives only recording state;
- API keys remain outside persisted app data and all owned in-memory key copies are zeroized when dropped;
- release and pull-request workflows use an active Apple Silicon macOS runner and pinned actions; release secrets are limited to the steps that need them, temporary certificate files are private, duplicate per-tag jobs are serialized, and releases require clean source at the exact version tag;
- OpenAI uploads and response bodies have explicit size limits, API messages are bounded, and all network operations have timeouts;
- settings, history and recovery metadata have read limits; malformed private JSON is preserved in a private quarantine file before safe recovery;
- failed audio is recoverable even when its metadata is missing, and its filename records whether retry is safe without risking another API charge;
- completed OpenAI text survives a local finalization failure, and its recovery action retries only local work while the audio filename remains fail-safe;
- ambiguous API outcomes are fail-safe and non-retryable, including timeouts after request start, HTTP 5xx and unreadable HTTP 2xx responses; history retranscription preserves the original FLAC and creates a blocking non-retryable Recovery copy;
- every new, recovery and history audio request is now persisted under a non-retryable Recovery filename before OpenAI can receive it, so a process crash cannot resurrect potentially charged audio as retryable; only a proven safe failure atomically restores the retryable marker;
- history-linked files are accepted for opening, retranscription or deletion only when their names match the exact generated timestamp and current twelve-character or legacy six-character identifier format; newly saved FLAC/TXT files use current-user-only permissions;
- diagnostic ZIPs use an exact generated filename allowlist, and at most one megabyte is read from each included Apple crash report;
- temporary recording files and native audio-worker waits are bounded, including cleanup of results abandoned after a timeout;
- long recovery errors wrap in the main window, and a failed automatic paste remains available in the menu bar after the toast disappears.
- the Settings view re-reads the real macOS Login Item state when the app regains focus while preserving an unsaved local toggle, and Save reconciles the requested state against macOS even when persisted settings had drifted.
- the native operation guard now rebuilds the menu bar at both acquisition and release, so Quit is disabled during API-key validation, microphone testing, shortcut capture, diagnostics, settings writes and every transcription path, then re-enabled automatically.
- the release guard rejects updater dependencies and any native HTTPS destination outside OpenAI key validation and transcription, keeping the no-analytics/no-updater privacy boundary enforceable.
- both OpenAI clients are HTTPS-only and reject redirects, so API credentials remain on the two fixed OpenAI request paths.
- the WebView CSP now explicitly blocks objects, frames, forms and base-URL changes while retaining only packaged UI assets and Tauri IPC.
- GitHub-owned workflow actions now use their current Node 24-based v7 releases, pinned to verified upstream commits; the release guard rejects any action outside the exact audited allowlist.
- the Rust crate denies undocumented unsafe blocks and implicit unsafe operations inside unsafe functions; the macOS Accessibility and event-tap FFI boundaries now state and compile-check their pointer and lifetime assumptions.
- the release guard also preserves the macOS HIG typography floor, system font stack, visible keyboard focus and Reduce Motion, Reduce Transparency and Increase Contrast adaptations without changing the pending overlay-shadow decision.
- support now opens a new email to the public `support@aidoo.bg` address instead of inaccessible Issues in the private source repository; the diagnostic ZIP remains local and is never attached or uploaded automatically.
- recovery actions model local finalization and API transcription as distinct native variants, so the transcription path cannot panic while assuming an optional API key is present.
- the OpenAI HTTPS dependency now uses `rustls 0.23.45`, which fixes `RUSTSEC-2026-0285`; a fresh RustSec scan reports no vulnerabilities, every pull request and direct push to the dedicated repository's `main` branch now runs the same pinned RustSec audit action as the release workflow, and the offline macOS dependency check rejects the affected version explicitly.

The current source was inspected without launching the application or using the microphone. On 15 September 2026, the production frontend build, all 85 representative English error-localization cases, release configuration validation, all three website page checks and Rust Clippy for the Apple Silicon release target with warnings denied passed. The native release target and all test targets also compile. A current RustSec database scan reports no vulnerabilities. It reports five unmaintained `unic-*` dependencies inherited from Tauri 2.11.5 through its locked `urlpattern 0.3` dependency; `proc-macro-error` and the advisory-affected `glib` package are absent from the Apple Silicon graph. These are tracked warnings rather than known exploitable findings, and Tauri currently offers no compatible `urlpattern` update. The dedicated repository's `glib` Dependabot alert is dismissed as `not_used` with this target-specific evidence, and the offline guard will fail if it ever enters the macOS graph. The unit tests were compiled but not executed because recording tests remain paused at the user's request. Every behavioral item below remains required on a fresh signed candidate.

## Required before public launch

- Build, sign, notarize, staple and audit a fresh candidate from the current source, then record its checksum here.
- Complete every behavioral item in [ACCEPTANCE.md](ACCEPTANCE.md) on the signed build while a tester is present.
- Complete the critical onboarding and dictation path on a second Mac or clean macOS account.
- Inject a failed `failed-recording.json` write, restart the signed app, and confirm that the newest Recovery audio reappears with its encoded retry status; a legacy file must reappear without a Retry action.
- Force-quit during each OpenAI request path and confirm the pre-request Recovery marker survives restart as Delete-only without a second request; repeat just before request start and confirm no conflicting marker copies remain.
- Upload the exact audited DMG and matching checksum to the AIDOO website with the privacy, support, and release-notes pages.
