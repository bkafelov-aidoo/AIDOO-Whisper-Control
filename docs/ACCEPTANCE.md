# Mac acceptance checklist

## First run

- Launch on a clean macOS 13+ Apple Silicon account.
- Confirm Bulgarian follows a Bulgarian system language and English follows other system languages.
- Confirm onboarding can be closed and resumed without enabling dictation prematurely.
- Open the OpenAI key page from the app, verify an invalid key shows a clear error, then save a valid key.
- Confirm the key exists in Keychain and is absent from settings, diagnostics and application files.
- Test the selected microphone and grant Accessibility.
- Capture Right Option and one modified shortcut; confirm mouse buttons are ignored and never become the shortcut.

## Dictation

- Hold the shortcut and verify the overlay appears above the Dock on the active display.
- Confirm long status/error text wraps and remains readable.
- Release, verify FLAC upload progress, clipboard content and automatic paste.
- Deny Accessibility temporarily and confirm the exact clipboard fallback toast appears.
- Test Bulgarian, English, automatic detection, Economy and Maximum accuracy.
- With automatic fallback enabled, disconnect the selected microphone and confirm the system or another available microphone is used with a clear warning in the main window and overlay.
- Disable automatic fallback, disconnect the selected microphone and confirm recording is refused with a clear error.
- While recording or transcribing, confirm API key, microphone, shortcut and settings changes are disabled and rejected by the native layer.

## Local data

- Test all combinations of FLAC, TXT and history toggles.
- Change the output folder and confirm new files use it.
- Open FLAC/TXT from history, copy text and retranscribe saved audio.
- Delete a history entry while keeping files, then delete another entry with its linked files.
- Trigger an API/balance/network failure, restart the app, retry the retained audio, then test explicit deletion.
- After restart with retained audio, confirm the main window and menu bar report that action is required; after retry or deletion, confirm they return to ready.

## App lifecycle and startup

- Close the main window and confirm the menu bar icon and shortcut remain active; reopen the window from both the Dock icon and menu bar.
- Confirm menu bar state, stop action, error details, settings and Quit.
- Enable and disable Launch at Login.
- Confirm the application contains no in-app or background update checker; new versions are installed from the AIDOO website.
- Create a diagnostics package, confirm Finder reveals it, and inspect that it has no API key, transcript text or audio; confirm the separate support button opens the AIDOO Whisper GitHub Issues page without uploading anything automatically.

## Distribution

- Verify `codesign --verify --deep --strict --verbose=2` for the app.
- Verify `xcrun stapler validate` and Gatekeeper acceptance for the DMG.
- Install from the DMG on a second clean Mac and repeat the critical dictation path.
- Verify the published SHA-256 checksum for the website DMG.
