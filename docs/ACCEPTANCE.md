# Mac acceptance checklist

## Automated release audit

Run `npm run audit:mac` against the website DMG before publication. It verifies the published checksum, Apple Silicon architecture, macOS 13 minimum, bundle ID, entitlements, Developer ID signature, notarization tickets, Gatekeeper acceptance, removal of updater configuration, required website documents and matching product/website icons. The audit does not launch the application or access the microphone.

The remaining checks below are behavioral and must be completed on the signed build. Microphone checks require a person at the Mac; the final clean-install path requires a second or clean macOS account.

## First run

- Launch on a clean macOS 13+ Apple Silicon account.
- Confirm Bulgarian follows a Bulgarian system language and English follows other system languages.
- Confirm the macOS Microphone permission explanation follows the system language in Bulgarian and English.
- Confirm onboarding can be closed and resumed without enabling dictation prematurely.
- Navigate onboarding and deletion dialogs with the keyboard; confirm focus stays inside, Escape closes safely, and shortcut capture handles Escape without closing onboarding.
- With VoiceOver, confirm API-key and microphone fields have useful names and status/error messages are announced without reading upload percentage changes continuously.
- Open the OpenAI key page from the app, verify an invalid key shows a clear error, then save a valid key.
- Confirm the key exists in Keychain and is absent from settings, diagnostics and application files.
- Test the selected microphone and grant Accessibility.
- Capture Right Option and one modified shortcut; confirm mouse buttons are ignored and never become the shortcut.
- Start shortcut capture without pressing a key and confirm it cancels after one minute without leaving settings locked.

## Dictation

- Hold the shortcut and verify the overlay appears above the Dock on the active display.
- After hiding the main window and after waking the Mac from sleep, confirm the overlay immediately catches up with the native recording state instead of remaining on an earlier status.
- Confirm long status/error text wraps and remains readable.
- Release, verify FLAC upload progress, clipboard content and automatic paste.
- Use Stop from the menu bar while the overlay still says the microphone is starting; confirm the app proceeds to transcription or a clear short-recording error and never returns to the recording state.
- Deny Accessibility temporarily and confirm the exact clipboard fallback toast appears.
- Test Bulgarian, English, automatic detection, Economy and Maximum accuracy.
- With automatic fallback enabled, disconnect the selected microphone and confirm the system or another available microphone is used with a clear warning in the main window and overlay.
- Disable automatic fallback, disconnect the selected microphone and confirm recording is refused with a clear error.
- With Settings or the reopened onboarding visible, start dictation and confirm API key, microphone, shortcut and every settings control becomes disabled while recording or transcribing; confirm the native layer also rejects direct mutations.

## Local data

- Test all combinations of FLAC, TXT and history toggles.
- Change the output folder and confirm new files use it.
- Open FLAC/TXT from history, copy text and retranscribe saved audio.
- Delete a history entry while keeping files, then delete another entry with its linked files.
- Trigger an API/balance/network failure, restart the app, retry the retained audio, then test explicit deletion.
- After a successful retry, make recovery-audio cleanup fail and confirm the completed text remains available, the old item is not silently presented as cleared, the Retry button disappears, and direct retry is rejected to prevent another API charge.
- After restart with retained audio, confirm the main window and menu bar report that action is required; after retry or deletion, confirm they return to ready.

## App lifecycle and startup

- Close the main window and confirm the menu bar icon and shortcut remain active; reopen the window from both the Dock icon and menu bar.
- Confirm menu bar state, stop action, error details, settings and Quit.
- In both interface languages, trigger a representative OpenAI and microphone error; confirm the full menu-bar error and copied error text use the selected language.
- While recording, transcribing, validating a key or capturing a shortcut, confirm Quit cannot discard the active operation and becomes available again after it finishes.
- Enable and disable Launch at Login.
- Change the Login Item or Accessibility permission in macOS System Settings, return to the app and confirm the displayed state refreshes; remove the API key and confirm both the main window and menu bar stop reporting ready.
- Confirm the application contains no in-app or background update checker; new versions are installed from the AIDOO website.
- Create a diagnostics package, confirm Finder reveals it, and inspect that it has no API key, transcript text or audio; confirm the separate support button opens the AIDOO Whisper GitHub Issues page without uploading anything automatically.

## Distribution

- Verify `codesign --verify --deep --strict --verbose=2` for the app.
- Verify `xcrun stapler validate` and Gatekeeper acceptance for the DMG.
- Install from the DMG on a second clean Mac and repeat the critical dictation path.
- Verify the published SHA-256 checksum for the website DMG.
