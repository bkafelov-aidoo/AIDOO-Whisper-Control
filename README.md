# AIDOO Whisper Lite

A focused macOS voice typing app. Hold a keyboard shortcut, speak, and release it to send the recording to the selected OpenAI transcription model. The result stays in the clipboard and can be pasted automatically into the active application.

## Product behavior

- Six-step Bulgarian/English onboarding for API key, permissions, shortcut, model, language and local storage.
- OpenAI API key is verified and stored in macOS Keychain. The app never stores it in settings, logs or diagnostics.
- Economy model: `gpt-4o-mini-transcribe` (about $0.003/minute).
- Maximum accuracy: `gpt-transcribe` (about $0.0045/minute).
- Automatic language detection or an explicit language to reduce latency.
- Optional FLAC, TXT and 10-item local history, with independent controls.
- Automatic paste with a clear clipboard fallback message.
- A static AIDOO menu bar icon and a bottom-center recording/status overlay.
- Failed audio survives restarts until the user retries or deletes it.
- Signed updates are checked at most once per day and installed only after the user chooses to update.
- No analytics. Diagnostics are created locally only when the user requests them.

## Requirements

- Apple Silicon Mac
- macOS 13 or newer
- Node.js 22, Rust stable and Xcode Command Line Tools
- An OpenAI Platform account with API billing enabled

## Development

```sh
npm ci
python3 scripts/generate-third-party-notices.py
npm run check
CARGO_TARGET_DIR=/tmp/aidoo-whisper-lite-target npm run tauri dev
```

The app bundle identifier is `app.aidoo.whisper-lite`. User-facing recordings default to `~/Documents/AIDOO Whisper Lite/Transcriptions`. Private settings, history and recovery data live under `~/Library/Application Support/AIDOO Whisper Lite`.

## Release

The local release path uses the installed Developer ID Application certificate, the `AIDOO_VIEWER_NOTARY` notarytool Keychain profile and the updater key stored outside this repository. Run:

```sh
npm run release:mac
```

The script tests the app, builds the Apple Silicon app/DMG, notarizes and staples both the application and DMG, rebuilds and signs the updater archive from the stapled app, verifies Gatekeeper and writes `latest-lite.json`. See [docs/RELEASE.md](docs/RELEASE.md) for website and GitHub release steps.

## Source layout

- `src/` — React interface, onboarding, settings, history and overlay.
- `src-tauri/src/` — small native modules for audio, shortcuts, Keychain, local files and OpenAI requests.
- `website/` — ready-to-publish privacy, support and release pages.
- `docs/` — architecture, model source, release and manual acceptance checklist.

The Windows implementation will be created as a separate sibling project after the Mac behavior is accepted, so its native shortcut, paste, storage, signing and installer code can follow Windows conventions.
