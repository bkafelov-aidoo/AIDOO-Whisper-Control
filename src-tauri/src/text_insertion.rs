#[cfg(target_os = "macos")]
use core_graphics::{
    event::{CGEvent, CGEventFlags, CGEventTapLocation, KeyCode},
    event_source::{CGEventSource, CGEventSourceStateID},
};
#[cfg(not(target_os = "macos"))]
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::{thread, time::Duration};

pub fn copy(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text).map_err(|e| e.to_string())
}

pub fn paste() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if !crate::accessibility_granted() {
        return Err("Липсва Accessibility разрешение за автоматично поставяне. Натиснете „Разреши Accessibility“ в Aidoo; разпознатият текст е запазен в Историята и clipboard.".into());
    }
    thread::sleep(Duration::from_millis(140));

    #[cfg(target_os = "macos")]
    {
        // Enigo resolves Unicode keys through the current macOS input source.
        // On recent macOS versions HIToolbox requires that lookup to happen on
        // the main queue and aborts the process when transcription finishes on
        // a Tokio worker. The physical V key is layout-independent for the
        // standard Paste shortcut, so post the chord directly through Quartz.
        post_macos_command_key(0x09)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
        let modifier = Key::Control;
        enigo
            .key(modifier, Direction::Press)
            .map_err(|e| e.to_string())?;
        enigo
            .key(Key::Unicode('v'), Direction::Click)
            .map_err(|e| e.to_string())?;
        enigo
            .key(modifier, Direction::Release)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn macos_event_source() -> Result<CGEventSource, String> {
    CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| "macOS не успя да създаде източник за клавиатурно събитие".to_string())
}

#[cfg(target_os = "macos")]
fn macos_key_event(source: CGEventSource, keycode: u16, down: bool) -> Result<CGEvent, String> {
    CGEvent::new_keyboard_event(source, keycode, down)
        .map_err(|_| "macOS не успя да създаде клавиатурно събитие".to_string())
}

#[cfg(target_os = "macos")]
fn post_macos_command_key(keycode: u16) -> Result<(), String> {
    let source = macos_event_source()?;

    // Build every event before posting any of them. That way a construction
    // error can never leave Command logically pressed in another application.
    let command_down = macos_key_event(source.clone(), KeyCode::COMMAND, true)?;
    let key_down = macos_key_event(source.clone(), keycode, true)?;
    let key_up = macos_key_event(source.clone(), keycode, false)?;
    let command_up = macos_key_event(source, KeyCode::COMMAND, false)?;

    command_down.set_flags(CGEventFlags::CGEventFlagCommand);
    key_down.set_flags(CGEventFlags::CGEventFlagCommand);
    key_up.set_flags(CGEventFlags::CGEventFlagCommand);
    command_up.set_flags(CGEventFlags::CGEventFlagNull);

    command_down.post(CGEventTapLocation::Session);
    thread::sleep(Duration::from_millis(8));
    key_down.post(CGEventTapLocation::Session);
    thread::sleep(Duration::from_millis(8));
    key_up.post(CGEventTapLocation::Session);
    thread::sleep(Duration::from_millis(8));
    command_up.post(CGEventTapLocation::Session);
    Ok(())
}
