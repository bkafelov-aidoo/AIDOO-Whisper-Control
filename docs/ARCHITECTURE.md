# Architecture

The interface and product flow live in React/TypeScript. Rust is a thin local native layer inside the Tauri process; there is no AIDOO server and no separate backend to operate.

`AppSettings` is the persisted product contract. It is normalized whenever it is loaded or saved. The API key is deliberately excluded and lives in macOS Keychain under service `app.aidoo.whisper-lite`. `storage.rs` owns atomic JSON writes, a bounded 10-item history, diagnostics redaction and failed-recording recovery.

The recording state machine is:

`idle → starting → recording → transcribing → done/error → idle`

The keyboard-only native shortcut listener begins recording on key-down and requests transcription on key-up. Recording produces a temporary mono WAV. The app converts it to FLAC before upload. A transcription failure moves the available WAV or FLAC audio to Recovery before reporting the error. A retry converts a retained WAV to a temporary FLAC while preserving the original until the whole operation succeeds. A successful result is copied to the clipboard first, then optionally written as FLAC/TXT and committed to history as one transaction, and finally optionally pasted. Files created by an operation that cannot be finalized are removed before the retained recovery audio is reported to the user.

The overlay receives state snapshots, progress events and user notices. It never owns recording state. It is centered in the active display's macOS work area, just above the Dock, and grows vertically so long status and error text can wrap. Closing the main window hides it while the tray and global shortcut remain active. A macOS Reopen event restores the main window from the Dock; Quit remains explicit in the menu bar.

Microphone routing follows the selected device, then the current system default, then the remaining available inputs when automatic fallback is enabled. With fallback disabled, only the selected device is attempted. API-key validation, microphone tests and history/recovery deletion share the native operation lock with recording and transcription. Settings and shortcut-capture mutations are rejected while that lock is active.

The WebView CSP permits only local IPC; OpenAI communication is implemented in native code and targets `api.openai.com`. API validation uses `GET /v1/models`; transcription uses `POST /v1/audio/transcriptions`. Audio is streamed from disk into the request and network calls have bounded connection and total timeouts. No analytics or background telemetry exists. A user-requested diagnostic ZIP contains sanitized app logs, settings without the API key, basic system/version data and up to three relevant Apple crash reports. OpenAI-style keys are redacted from included crash reports. The package remains local until the user chooses to share it.

Releases contain a notarized and stapled DMG plus its SHA-256 checksum. New versions are published on the AIDOO website and installed manually; the application has no updater endpoint or background version check.
