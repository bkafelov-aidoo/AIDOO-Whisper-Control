# Mac acceptance checklist

## Automated release audit

Run `npm run check` before building and `npm run audit:mac` against the website DMG before publication. The first command also proves that known advisory-affected Linux packages have not entered the Apple Silicon macOS dependency graph. The artifact audit verifies the published checksum, Apple Silicon architecture, macOS 13 minimum, bundle ID, entitlements, the exact AIDOO Developer ID and Team ID on both the application and DMG, notarization tickets, Gatekeeper acceptance, removal of updater configuration, required website documents and matching product/website icons. Neither check launches the application or accesses the microphone.

The final `npm run release:mac` path additionally requires a clean Lite working tree, current generated third-party notices and the exact `lite-vX.Y.Z` tag on HEAD. GitHub Actions accepts the same exact tag and rejects manual branch releases.

The remaining checks below are behavioral and must be completed on the signed build. Microphone checks require a person at the Mac; the final clean-install path requires a second or clean macOS account.

## First run

- Launch on a clean macOS 13+ Apple Silicon account.
- Confirm Bulgarian follows a Bulgarian system language and English follows other system languages.
- Confirm the macOS Microphone permission explanation follows the system language in Bulgarian and English.
- Confirm onboarding can be closed and resumed without enabling dictation prematurely.
- Navigate onboarding and deletion dialogs with the keyboard; confirm focus stays inside, Escape closes safely, and shortcut capture handles Escape without closing onboarding.
- With VoiceOver, confirm API-key and microphone fields have useful names and status/error messages are announced without reading upload percentage changes continuously.
- Open the OpenAI key page from the app, verify an invalid key shows a clear error, then save a valid key. Confirm a valid endpoint-restricted key is not rejected only because it cannot list models.
- Confirm the key exists in Keychain and is absent from settings, diagnostics and application files.
- Test the selected microphone and grant Accessibility.
- Capture Right Option and one modified shortcut; confirm mouse buttons are ignored and never become the shortcut.
- Start shortcut capture without pressing a key and confirm it cancels after one minute without leaving settings locked.

## Dictation

- Hold the shortcut and verify the overlay appears above the Dock on the active display.
- After hiding the main window and after waking the Mac from sleep, confirm the overlay immediately catches up with the native recording state instead of remaining on an earlier status.
- Confirm long status/error text wraps and remains readable.
- Release, verify FLAC upload progress, clipboard content and automatic paste.
- Keep a recording active for five minutes and confirm it stops and transcribes automatically; start another recording before an older watchdog expires and confirm the old watchdog cannot stop it.
- Use Stop from the menu bar while the overlay still says the microphone is starting; confirm the app proceeds to transcription or a clear short-recording error and never returns to the recording state.
- Deny Accessibility temporarily and confirm the exact clipboard fallback toast appears.
- Test Bulgarian, English, automatic detection, Economy and Maximum accuracy.
- With automatic fallback enabled, disconnect the selected microphone and confirm the system or another available microphone is used with a clear warning in the main window and overlay.
- Disable automatic fallback, disconnect the selected microphone and confirm recording is refused with a clear error.
- With Settings or the reopened onboarding visible, start dictation and confirm API key, microphone, shortcut and every settings control becomes disabled while recording or transcribing; confirm the native layer also rejects direct mutations.

## Local data

- Test all combinations of FLAC, TXT and history toggles.
- Confirm newly saved FLAC and TXT files have current-user-only permissions and use the current twelve-character filename identifier; confirm a legacy six-character history filename can still be opened.
- Change the output folder and confirm new files use it.
- Open FLAC/TXT from history, copy text and retranscribe saved audio.
- During retranscription from history, inject a local finalization failure after the OpenAI response. Confirm the original FLAC remains in place, “Finish locally” appears, no second OpenAI request occurs, and another history retranscription is refused until the Recovery item is resolved.
- Delete a history entry while keeping files, then delete another entry with its linked files.
- Trigger an API/balance/network failure, restart the app, retry the retained audio, then test explicit deletion.
- Confirm an audio file over 25 MB is rejected before upload, retained locally and shown without Retry.
- Confirm every non-retryable recovery card explains that the unchanged file cannot be sent again and offers only Delete.
- After a successful retry, make recovery-audio cleanup fail and confirm the completed text remains available, the old item is not silently presented as cleared, the Retry button disappears, and direct retry is rejected to prevent another API charge.
- Make clipboard, TXT or history persistence fail after a successful OpenAI response; confirm the completed text is retained privately, every file already created remains in the output folder, and the card offers “Finish locally”. Complete it after fixing the local failure and confirm no second OpenAI request occurs.
- Restart with a pending “Finish locally” item and confirm it still finishes without an API key or network access. Remove or corrupt its metadata and confirm the fail-safe audio filename reappears without a Retry action.
- Inject a timeout after the OpenAI request starts, an HTTP 5xx response and an unreadable HTTP 2xx response. Confirm each retained recording is Delete-only. Repeat from a history FLAC and confirm a non-retryable Recovery copy blocks another transcription while the original history file remains intact.
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
- While a diagnostics package is being created, confirm Quit is unavailable and no temporary or partial ZIP remains after an injected write failure.

## Distribution

- Verify `codesign --verify --deep --strict --verbose=2` for the app.
- Verify `xcrun stapler validate` and Gatekeeper acceptance for the DMG.
- Install from the DMG on a second clean Mac and repeat the critical dictation path.
- Verify the published SHA-256 checksum for the website DMG.
