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
- If OpenAI has already returned text, recovery finishes clipboard and local files without sending or charging the audio again.
- New versions are distributed as notarized DMG downloads from the AIDOO website.
- No analytics. Diagnostics are created locally only when the user requests them.

## Requirements

- Apple Silicon Mac
- macOS 13 or newer
- Node.js 22, Python 3.11+, the pinned Rust 1.93.1 toolchain and Xcode Command Line Tools
- An OpenAI Platform account with API billing enabled

## Development

```sh
npm ci
python3 scripts/generate-third-party-notices.py
npm run check
cargo check --locked --release --target aarch64-apple-darwin --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --release --target aarch64-apple-darwin --manifest-path src-tauri/Cargo.toml -- -D warnings
CARGO_TARGET_DIR=/tmp/aidoo-whisper-lite-target npm run tauri dev
```

`rustup` reads the exact compiler, target, Clippy and rustfmt versions from `rust-toolchain.toml`. Native unit tests are run separately with the same release target in CI.

The app bundle identifier is `app.aidoo.whisper-lite`. User-facing recordings default to `~/Documents/AIDOO Whisper Lite/Transcriptions`. Private settings, history and recovery data live under `~/Library/Application Support/AIDOO Whisper Lite`.

## Release

The local release path uses the installed Developer ID Application certificate and the `AIDOO_VIEWER_NOTARY` notarytool Keychain profile. Run:

```sh
npm run release:mac
```

Configure the six GitHub release secrets without placing their values in the repository:

```sh
./scripts/configure-github-release-secrets.sh
```

The wizard verifies the exact Developer ID Application identity before writing encrypted repository secrets. It does not create a tag or start a release. The release script tests the app, builds the Apple Silicon app/DMG, notarizes and staples both the application and DMG, verifies Gatekeeper, then creates and independently audits one website package under `release/<version>/`. That folder contains the DMG, checksum, public pages and a SHA-256 manifest. See [docs/RELEASE.md](docs/RELEASE.md) for website release steps.

## Source layout

- `src/App.tsx` — application shell and native event coordination.
- `src/pages/` — complete product screens; `src/components/` contains reusable interface sections, `src/hooks/` owns React lifecycles, and `src/lib/` contains framework-independent helpers.
- `src-tauri/src/lib.rs` — Tauri composition root and process lifecycle.
- `src-tauri/src/app_ui.rs`, `dictation.rs`, `recovery.rs` and `commands.rs` — native presentation, recording orchestration, fail-safe completion and the command interface.
- `src-tauri/src/audio.rs`, `shortcuts.rs`, `storage.rs` and `transcription.rs` — focused adapters for devices, operating-system input, persistence and OpenAI.
- `website/` — ready-to-publish privacy, support and release pages.
- `docs/` — architecture, model source, release and manual acceptance checklist.

`npm run check:structure` prevents source files from growing beyond 1,000 lines. Split at a cohesive module seam before extending a file past that limit.

## Support

Use Settings → Diagnostics to create a local ZIP that excludes the API key, transcript content and audio. Review it before sharing, then email `support@aidoo.bg` and attach the ZIP only if you choose to. The app never uploads diagnostics automatically.

The Windows implementation will be created as a separate sibling project after the Mac behavior is accepted, so its native shortcut, paste, storage, signing and installer code can follow Windows conventions.
