use crate::{
    models::{AppSettings, ShortcutBinding},
    AppState,
};
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::mpsc::{self, Receiver};
#[cfg(target_os = "macos")]
use std::sync::{atomic::AtomicBool, mpsc::Sender, Arc};
use tauri::{AppHandle, Emitter, Manager};

const TARGET_DICTATION: &str = "dictation";
#[cfg(target_os = "macos")]
const MACOS_EVENT_TAP_OPTIONS: u32 = 0; // kCGEventTapOptionDefault (Accessibility)
#[cfg(target_os = "macos")]
const MACOS_OPTION_FLAG_MASK: u64 = 1_u64 << 19;

#[cfg(target_os = "macos")]
fn macos_modifier_transition_is_pressed(code: i64, flags: u64, was_pressed: bool) -> bool {
    let mask = match code {
        54 | 55 => 1_u64 << 20, // Command
        56 | 60 => 1_u64 << 17, // Shift
        58 | 61 => MACOS_OPTION_FLAG_MASK,
        59 | 62 => 1_u64 << 18, // Control
        63 => 1_u64 << 23,      // Fn
        57 => 1_u64 << 16,      // Caps Lock
        _ => 0,
    };
    // FlagsChanged identifies a physical modifier transition. If the same physical key was
    // already down, its next transition is a release even when another Option key keeps the
    // aggregate family bit set.
    mask != 0 && flags & mask != 0 && !was_pressed
}

#[cfg(target_os = "macos")]
fn macos_right_option_missing_from_flags(was_pressed: bool, flags: u64) -> bool {
    was_pressed && flags & MACOS_OPTION_FLAG_MASK == 0
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShortcutCapturedPayload {
    target: String,
    binding: ShortcutBinding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InputTrigger {
    Key(String),
}

#[derive(Debug, Clone, Copy)]
enum InputEvent {
    Key {
        code: &'static str,
        pressed: bool,
    },
    #[cfg(target_os = "macos")]
    Reset,
}

#[derive(Debug, Default)]
struct ModifierState {
    function: bool,
    control: bool,
    meta: bool,
    shift: bool,
    alt: bool,
}

impl ModifierState {
    fn update(&mut self, code: &str, pressed: bool) {
        match code {
            "function" => self.function = pressed,
            "control_left" | "control_right" => self.control = pressed,
            "meta_left" | "meta_right" => self.meta = pressed,
            "shift_left" | "shift_right" => self.shift = pressed,
            "alt" | "alt_gr" => self.alt = pressed,
            _ => {}
        }
    }

    fn names_for(&self, pressed_code: &str) -> BTreeSet<&'static str> {
        let mut names = BTreeSet::new();
        if self.function && pressed_code != "function" {
            names.insert("fn");
        }
        if self.control && !matches!(pressed_code, "control_left" | "control_right") {
            names.insert("control");
        }
        if self.meta && !matches!(pressed_code, "meta_left" | "meta_right") {
            names.insert("meta");
        }
        if self.shift && !matches!(pressed_code, "shift_left" | "shift_right") {
            names.insert("shift");
        }
        if self.alt && !matches!(pressed_code, "alt" | "alt_gr") {
            names.insert("alt");
        }
        names
    }
}

#[derive(Debug, Default)]
struct ShortcutRuntime {
    modifiers: ModifierState,
    dictation_trigger: Option<InputTrigger>,
    pressed_keys: BTreeSet<String>,
    suppress_shortcuts_until_release: bool,
}

impl ShortcutRuntime {
    fn track_physical_key(&mut self, code: &str, pressed: bool) {
        if pressed {
            self.pressed_keys.insert(code.into());
        } else {
            self.pressed_keys.remove(code);
        }
    }

    fn suppress_shortcuts_until_current_keys_are_released(&mut self) {
        self.suppress_shortcuts_until_release = !self.pressed_keys.is_empty();
    }

    fn consume_capture_release_guard(&mut self) -> bool {
        if !self.suppress_shortcuts_until_release {
            return false;
        }
        if self.pressed_keys.is_empty() {
            self.suppress_shortcuts_until_release = false;
        }
        true
    }

    #[cfg(target_os = "macos")]
    fn reset_stale_inputs(&mut self) -> bool {
        let dictation_was_active = self.dictation_trigger.take().is_some();
        self.modifiers = ModifierState::default();
        self.pressed_keys.clear();
        self.suppress_shortcuts_until_release = false;
        dictation_was_active
    }
}

pub fn validate_settings(settings: &AppSettings) -> Result<(), String> {
    validate_binding(&settings.dictation_shortcut)
}

fn validate_binding(binding: &ShortcutBinding) -> Result<(), String> {
    let ShortcutBinding::Key { code, modifiers } = binding;
    if !is_supported_key_code(code) {
        return Err("Този клавиш не се поддържа като глобален shortcut.".into());
    }
    let modifier_set = normalized_modifiers(modifiers)?;
    if code == "escape" {
        return Err("Escape е запазен и не може да бъде shortcut за диктовка.".into());
    }
    if code == "alt_gr" {
        if !modifier_set.is_empty() {
            return Err(
                "Десният Option/Alt се използва самостоятелно, без допълнителни модификатори."
                    .into(),
            );
        }
        return Ok(());
    }
    if is_modifier_code(code) {
        return Err("Самостоятелно може да се използва само десният Option/Alt. За останалите изберете комбинация.".into());
    }
    if modifier_set.is_empty() && !is_function_key(code) {
        return Err("Добавете модификатор или използвайте F1–F12, за да не се изписва случаен знак в активното поле.".into());
    }
    Ok(())
}

fn normalized_modifiers(modifiers: &[String]) -> Result<BTreeSet<&str>, String> {
    let mut normalized = BTreeSet::new();
    for modifier in modifiers {
        if !matches!(
            modifier.as_str(),
            "fn" | "control" | "meta" | "shift" | "alt"
        ) {
            return Err("Shortcut-ът съдържа непознат модификатор.".into());
        }
        if !normalized.insert(modifier.as_str()) {
            return Err("Shortcut-ът съдържа повторен модификатор.".into());
        }
    }
    Ok(normalized)
}

pub fn begin_capture(target: String, state: &AppState) -> Result<(), String> {
    if target != TARGET_DICTATION {
        return Err("Непозната shortcut настройка.".into());
    }
    *state
        .shortcut_capture
        .lock()
        .map_err(|_| "Shortcut recorder-ът е заключен.")? = Some(target);
    Ok(())
}

pub fn cancel_capture(state: &AppState) -> Result<bool, String> {
    let removed = state
        .shortcut_capture
        .lock()
        .map_err(|_| "Shortcut recorder-ът е заключен.")?
        .take()
        .is_some();
    Ok(removed)
}

pub fn install(app: AppHandle) {
    let (sender, receiver) = mpsc::channel();
    let processor_app = app.clone();
    std::thread::spawn(move || process_events(processor_app, receiver));

    #[cfg(not(target_os = "macos"))]
    let keyboard_sender = sender.clone();
    #[cfg(not(target_os = "macos"))]
    std::thread::spawn(move || {
        let result = rdev::listen(move |event| {
            let translated = match event.event_type {
                rdev::EventType::KeyPress(key) => key_code(key).map(|code| InputEvent::Key {
                    code,
                    pressed: true,
                }),
                rdev::EventType::KeyRelease(key) => key_code(key).map(|code| InputEvent::Key {
                    code,
                    pressed: false,
                }),
                _ => None,
            };
            if let Some(event) = translated {
                let _ = keyboard_sender.send(event);
            }
        });
        if let Err(error) = result {
            eprintln!("global shortcut listener error: {error:?}");
        }
    });

    #[cfg(target_os = "macos")]
    install_macos_input_listener(sender);
}

fn process_events(app: AppHandle, receiver: Receiver<InputEvent>) {
    let mut runtime = ShortcutRuntime::default();
    while let Ok(event) = receiver.recv() {
        process_event(&app, &mut runtime, event);
    }
}

fn process_event(app: &AppHandle, runtime: &mut ShortcutRuntime, event: InputEvent) {
    #[cfg(target_os = "macos")]
    if matches!(event, InputEvent::Reset) {
        crate::storage::append_shortcut_diagnostic("event-tap reset");
        if runtime.reset_stale_inputs() {
            crate::request_dictation_stop(app);
        }
        return;
    }
    let InputEvent::Key { code, pressed } = event else {
        return;
    };
    if code == "alt_gr" {
        crate::storage::append_shortcut_diagnostic(if pressed {
            "right-option down received"
        } else {
            "right-option up received"
        });
    }
    runtime.track_physical_key(code, pressed);
    if pressed {
        runtime.modifiers.update(code, true);
    }
    if capture_is_active(app) {
        process_capture_key(app, &runtime.modifiers, code, pressed);
        if !capture_is_active(app) {
            runtime.suppress_shortcuts_until_current_keys_are_released();
        }
        if !pressed {
            runtime.modifiers.update(code, false);
        }
        return;
    }
    if runtime.consume_capture_release_guard() {
        if !pressed {
            runtime.modifiers.update(code, false);
        }
        return;
    }
    process_shortcut_key(app, runtime, code, pressed);
    if !pressed {
        runtime.modifiers.update(code, false);
    }
}

fn capture_is_active(app: &AppHandle) -> bool {
    app.state::<AppState>()
        .shortcut_capture
        .lock()
        .map(|capture| capture.is_some())
        .unwrap_or(false)
}

fn process_capture_key(app: &AppHandle, modifiers: &ModifierState, code: &str, pressed: bool) {
    if !pressed {
        return;
    }
    if code == "escape" {
        let target = app
            .state::<AppState>()
            .shortcut_capture
            .lock()
            .ok()
            .and_then(|mut capture| capture.take());
        if let Some(target) = target {
            crate::release_shortcut_capture_operation(app);
            let _ = app.emit("shortcut:capture-cancelled", target);
        }
        return;
    }
    if is_modifier_code(code) && code != "alt_gr" {
        return;
    }
    let modifier_names = modifiers
        .names_for(code)
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    try_finish_capture(
        app,
        ShortcutBinding::Key {
            code: code.into(),
            modifiers: modifier_names,
        },
    );
}

fn try_finish_capture(app: &AppHandle, binding: ShortcutBinding) {
    if let Err(error) = validate_binding(&binding) {
        let _ = app.emit("shortcut:capture-error", error);
        return;
    }
    let settings = app
        .state::<AppState>()
        .settings
        .lock()
        .ok()
        .map(|value| value.clone());
    let Some(mut settings) = settings else {
        let _ = app.emit("shortcut:capture-error", "Настройките са заключени.");
        return;
    };
    settings.dictation_shortcut = binding.clone();
    if let Err(error) = validate_settings(&settings) {
        let _ = app.emit("shortcut:capture-error", error);
        return;
    }
    let target = app
        .state::<AppState>()
        .shortcut_capture
        .lock()
        .ok()
        .and_then(|mut capture| capture.take());
    let Some(target) = target else { return };
    crate::release_shortcut_capture_operation(app);
    let _ = app.emit(
        "shortcut:captured",
        ShortcutCapturedPayload { target, binding },
    );
}

fn process_shortcut_key(app: &AppHandle, runtime: &mut ShortcutRuntime, code: &str, pressed: bool) {
    let trigger = InputTrigger::Key(code.into());
    if !pressed {
        release_trigger(app, runtime, &trigger);
        return;
    }
    let settings = app
        .state::<AppState>()
        .settings
        .lock()
        .ok()
        .map(|value| value.clone());
    let Some(settings) = settings else { return };
    let active_modifiers = runtime.modifiers.names_for(code);
    if runtime.dictation_trigger.is_none()
        && binding_matches_key(&settings.dictation_shortcut, code, &active_modifiers)
    {
        crate::storage::append_shortcut_diagnostic("dictation trigger matched; native start");
        if crate::request_dictation_start(app) {
            runtime.dictation_trigger = Some(trigger);
        }
    }
}

fn release_trigger(app: &AppHandle, runtime: &mut ShortcutRuntime, trigger: &InputTrigger) {
    if runtime.dictation_trigger.as_ref() == Some(trigger) {
        runtime.dictation_trigger = None;
        crate::storage::append_shortcut_diagnostic("dictation trigger released; native stop");
        crate::request_dictation_stop(app);
    }
}

fn binding_matches_key(
    binding: &ShortcutBinding,
    code: &str,
    active_modifiers: &BTreeSet<&str>,
) -> bool {
    let ShortcutBinding::Key {
        code: expected,
        modifiers,
    } = binding;
    if expected != code {
        return false;
    }
    if code == "alt_gr" && modifiers.is_empty() {
        return true;
    }
    normalized_modifiers(modifiers)
        .map(|expected| expected == *active_modifiers)
        .unwrap_or(false)
}

fn is_modifier_code(code: &str) -> bool {
    matches!(
        code,
        "function"
            | "control_left"
            | "control_right"
            | "meta_left"
            | "meta_right"
            | "shift_left"
            | "shift_right"
            | "alt"
            | "alt_gr"
    )
}

fn is_function_key(code: &str) -> bool {
    matches!(
        code,
        "f1" | "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10" | "f11" | "f12"
    )
}

fn is_supported_key_code(code: &str) -> bool {
    key_code_values().contains(&code)
}

fn key_code_values() -> &'static [&'static str] {
    &[
        "alt",
        "alt_gr",
        "backspace",
        "caps_lock",
        "control_left",
        "control_right",
        "delete",
        "down",
        "end",
        "escape",
        "f1",
        "f2",
        "f3",
        "f4",
        "f5",
        "f6",
        "f7",
        "f8",
        "f9",
        "f10",
        "f11",
        "f12",
        "home",
        "left",
        "meta_left",
        "meta_right",
        "page_down",
        "page_up",
        "return",
        "right",
        "shift_left",
        "shift_right",
        "space",
        "tab",
        "up",
        "print_screen",
        "scroll_lock",
        "pause",
        "num_lock",
        "back_quote",
        "num_1",
        "num_2",
        "num_3",
        "num_4",
        "num_5",
        "num_6",
        "num_7",
        "num_8",
        "num_9",
        "num_0",
        "minus",
        "equal",
        "key_q",
        "key_w",
        "key_e",
        "key_r",
        "key_t",
        "key_y",
        "key_u",
        "key_i",
        "key_o",
        "key_p",
        "left_bracket",
        "right_bracket",
        "key_a",
        "key_s",
        "key_d",
        "key_f",
        "key_g",
        "key_h",
        "key_j",
        "key_k",
        "key_l",
        "semicolon",
        "quote",
        "backslash",
        "intl_backslash",
        "key_z",
        "key_x",
        "key_c",
        "key_v",
        "key_b",
        "key_n",
        "key_m",
        "comma",
        "dot",
        "slash",
        "insert",
        "kp_return",
        "kp_minus",
        "kp_plus",
        "kp_multiply",
        "kp_divide",
        "kp_0",
        "kp_1",
        "kp_2",
        "kp_3",
        "kp_4",
        "kp_5",
        "kp_6",
        "kp_7",
        "kp_8",
        "kp_9",
        "kp_delete",
        "function",
    ]
}

#[cfg(not(target_os = "macos"))]
fn key_code(key: rdev::Key) -> Option<&'static str> {
    use rdev::Key::*;
    Some(match key {
        Alt => "alt",
        AltGr => "alt_gr",
        Backspace => "backspace",
        CapsLock => "caps_lock",
        ControlLeft => "control_left",
        ControlRight => "control_right",
        Delete => "delete",
        DownArrow => "down",
        End => "end",
        Escape => "escape",
        F1 => "f1",
        F2 => "f2",
        F3 => "f3",
        F4 => "f4",
        F5 => "f5",
        F6 => "f6",
        F7 => "f7",
        F8 => "f8",
        F9 => "f9",
        F10 => "f10",
        F11 => "f11",
        F12 => "f12",
        Home => "home",
        LeftArrow => "left",
        MetaLeft => "meta_left",
        MetaRight => "meta_right",
        PageDown => "page_down",
        PageUp => "page_up",
        Return => "return",
        RightArrow => "right",
        ShiftLeft => "shift_left",
        ShiftRight => "shift_right",
        Space => "space",
        Tab => "tab",
        UpArrow => "up",
        PrintScreen => "print_screen",
        ScrollLock => "scroll_lock",
        Pause => "pause",
        NumLock => "num_lock",
        BackQuote => "back_quote",
        Num1 => "num_1",
        Num2 => "num_2",
        Num3 => "num_3",
        Num4 => "num_4",
        Num5 => "num_5",
        Num6 => "num_6",
        Num7 => "num_7",
        Num8 => "num_8",
        Num9 => "num_9",
        Num0 => "num_0",
        Minus => "minus",
        Equal => "equal",
        KeyQ => "key_q",
        KeyW => "key_w",
        KeyE => "key_e",
        KeyR => "key_r",
        KeyT => "key_t",
        KeyY => "key_y",
        KeyU => "key_u",
        KeyI => "key_i",
        KeyO => "key_o",
        KeyP => "key_p",
        LeftBracket => "left_bracket",
        RightBracket => "right_bracket",
        KeyA => "key_a",
        KeyS => "key_s",
        KeyD => "key_d",
        KeyF => "key_f",
        KeyG => "key_g",
        KeyH => "key_h",
        KeyJ => "key_j",
        KeyK => "key_k",
        KeyL => "key_l",
        SemiColon => "semicolon",
        Quote => "quote",
        BackSlash => "backslash",
        IntlBackslash => "intl_backslash",
        KeyZ => "key_z",
        KeyX => "key_x",
        KeyC => "key_c",
        KeyV => "key_v",
        KeyB => "key_b",
        KeyN => "key_n",
        KeyM => "key_m",
        Comma => "comma",
        Dot => "dot",
        Slash => "slash",
        Insert => "insert",
        KpReturn => "kp_return",
        KpMinus => "kp_minus",
        KpPlus => "kp_plus",
        KpMultiply => "kp_multiply",
        KpDivide => "kp_divide",
        Kp0 => "kp_0",
        Kp1 => "kp_1",
        Kp2 => "kp_2",
        Kp3 => "kp_3",
        Kp4 => "kp_4",
        Kp5 => "kp_5",
        Kp6 => "kp_6",
        Kp7 => "kp_7",
        Kp8 => "kp_8",
        Kp9 => "kp_9",
        KpDelete => "kp_delete",
        Function => "function",
        Unknown(_) => return None,
    })
}

#[cfg(target_os = "macos")]
fn install_macos_input_listener(sender: Sender<InputEvent>) {
    // SAFETY: The callback context is heap-allocated before CGEventTapCreate, remains owned for
    // the complete CFRunLoopRun lifetime, and is freed only after the source and tap are released.
    // All Core Graphics event pointers are used synchronously during Apple's callback.
    std::thread::spawn(move || unsafe {
        use core_graphics::event::{CGEventTapLocation, CGEventType};
        use std::ffi::c_void;
        use std::sync::atomic::{AtomicPtr, Ordering};

        type CFMachPortRef = *const c_void;
        type CFRunLoopRef = *const c_void;
        type CFRunLoopSourceRef = *const c_void;
        type CFRunLoopMode = *const c_void;
        type CGEventTapProxy = *mut c_void;
        type CGEventRef = *mut c_void;
        type QCallback = unsafe extern "C" fn(
            CGEventTapProxy,
            CGEventType,
            CGEventRef,
            *mut c_void,
        ) -> CGEventRef;

        struct ListenerContext {
            sender: Sender<InputEvent>,
            tap: AtomicPtr<c_void>,
            right_option_pressed: Arc<AtomicBool>,
        }

        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn CGEventTapCreate(
                tap: CGEventTapLocation,
                place: u32,
                options: u32,
                events_of_interest: u64,
                callback: QCallback,
                user_info: *mut c_void,
            ) -> CFMachPortRef;
            fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
            fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
            fn CGEventGetFlags(event: CGEventRef) -> u64;
        }
        #[link(name = "CoreFoundation", kind = "framework")]
        extern "C" {
            fn CFMachPortCreateRunLoopSource(
                allocator: *const c_void,
                tap: CFMachPortRef,
                order: u64,
            ) -> CFRunLoopSourceRef;
            fn CFRunLoopGetCurrent() -> CFRunLoopRef;
            fn CFRunLoopAddSource(
                run_loop: CFRunLoopRef,
                source: CFRunLoopSourceRef,
                mode: CFRunLoopMode,
            );
            fn CFRunLoopRun();
            fn CFRelease(value: *const c_void);
            static kCFRunLoopCommonModes: CFRunLoopMode;
        }

        unsafe extern "C" fn callback(
            _proxy: CGEventTapProxy,
            event_type: CGEventType,
            event: CGEventRef,
            user_info: *mut c_void,
        ) -> CGEventRef {
            // SAFETY: CGEventTapCreate receives this exact non-null Box pointer below, and the
            // listener frees it only after CFRunLoopRun returns and the tap is released.
            let context = unsafe { &*(user_info as *const ListenerContext) };
            if matches!(
                event_type,
                CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
            ) {
                context.right_option_pressed.store(false, Ordering::Release);
                let _ = context.sender.send(InputEvent::Reset);
                let tap = context.tap.load(Ordering::Relaxed);
                if !tap.is_null() {
                    // SAFETY: callbacks run while the owning event tap is alive; the atomic is
                    // populated from that tap before the run loop starts.
                    unsafe { CGEventTapEnable(tap.cast_const(), true) };
                }
                return event;
            }
            // SAFETY: Core Graphics owns this event and guarantees that it remains valid for the
            // duration of the callback.
            let event_flags = unsafe { CGEventGetFlags(event) };
            // macOS occasionally reports a Right Option release with a different keycode after
            // an input-source transition. The aggregate flags on the same event are still
            // authoritative: once Option disappears, release the dedicated Right Option trigger
            // exactly once before processing the event-specific keycode.
            if macos_right_option_missing_from_flags(
                context.right_option_pressed.load(Ordering::Acquire),
                event_flags,
            ) && context.right_option_pressed.swap(false, Ordering::AcqRel)
            {
                let _ = context.sender.send(InputEvent::Key {
                    code: "alt_gr",
                    pressed: false,
                });
            }
            match event_type {
                CGEventType::KeyDown | CGEventType::KeyUp | CGEventType::FlagsChanged => {
                    // kCGKeyboardEventKeycode is field 9 in the stable CoreGraphics ABI.
                    // SAFETY: Core Graphics owns the callback event for this invocation and field
                    // 9 is kCGKeyboardEventKeycode for keyboard/flags events.
                    let raw_code = unsafe { CGEventGetIntegerValueField(event, 9) };
                    if let Some(code) = macos_key_code(raw_code) {
                        let pressed = match event_type {
                            CGEventType::KeyDown => true,
                            CGEventType::KeyUp => false,
                            CGEventType::FlagsChanged => macos_modifier_transition_is_pressed(
                                raw_code,
                                event_flags,
                                raw_code == 61
                                    && context.right_option_pressed.load(Ordering::Acquire),
                            ),
                            _ => false,
                        };
                        if raw_code == 61 {
                            let previous =
                                context.right_option_pressed.swap(pressed, Ordering::AcqRel);
                            if previous == pressed {
                                return event;
                            }
                        }
                        let _ = context.sender.send(InputEvent::Key { code, pressed });
                    }
                }
                _ => {}
            }
            event
        }

        fn macos_key_code(code: i64) -> Option<&'static str> {
            Some(match code {
                58 => "alt",
                61 => "alt_gr",
                51 => "backspace",
                57 => "caps_lock",
                59 => "control_left",
                62 => "control_right",
                117 => "delete",
                125 => "down",
                119 => "end",
                53 => "escape",
                122 => "f1",
                120 => "f2",
                99 => "f3",
                118 => "f4",
                96 => "f5",
                97 => "f6",
                98 => "f7",
                100 => "f8",
                101 => "f9",
                109 => "f10",
                103 => "f11",
                111 => "f12",
                115 => "home",
                123 => "left",
                55 => "meta_left",
                54 => "meta_right",
                121 => "page_down",
                116 => "page_up",
                36 => "return",
                124 => "right",
                56 => "shift_left",
                60 => "shift_right",
                49 => "space",
                48 => "tab",
                126 => "up",
                50 => "back_quote",
                18 => "num_1",
                19 => "num_2",
                20 => "num_3",
                21 => "num_4",
                23 => "num_5",
                22 => "num_6",
                26 => "num_7",
                28 => "num_8",
                25 => "num_9",
                29 => "num_0",
                27 => "minus",
                24 => "equal",
                12 => "key_q",
                13 => "key_w",
                14 => "key_e",
                15 => "key_r",
                17 => "key_t",
                16 => "key_y",
                32 => "key_u",
                34 => "key_i",
                31 => "key_o",
                35 => "key_p",
                33 => "left_bracket",
                30 => "right_bracket",
                0 => "key_a",
                1 => "key_s",
                2 => "key_d",
                3 => "key_f",
                5 => "key_g",
                4 => "key_h",
                38 => "key_j",
                40 => "key_k",
                37 => "key_l",
                41 => "semicolon",
                39 => "quote",
                42 => "backslash",
                10 => "intl_backslash",
                6 => "key_z",
                7 => "key_x",
                8 => "key_c",
                9 => "key_v",
                11 => "key_b",
                45 => "key_n",
                46 => "key_m",
                43 => "comma",
                47 => "dot",
                44 => "slash",
                76 => "kp_return",
                78 => "kp_minus",
                69 => "kp_plus",
                67 => "kp_multiply",
                75 => "kp_divide",
                82 => "kp_0",
                83 => "kp_1",
                84 => "kp_2",
                85 => "kp_3",
                86 => "kp_4",
                87 => "kp_5",
                88 => "kp_6",
                89 => "kp_7",
                91 => "kp_8",
                92 => "kp_9",
                65 => "kp_delete",
                63 => "function",
                _ => return None,
            })
        }

        let mask = (1_u64 << CGEventType::KeyDown as u64)
            | (1_u64 << CGEventType::KeyUp as u64)
            | (1_u64 << CGEventType::FlagsChanged as u64);
        let right_option_pressed = Arc::new(AtomicBool::new(false));
        loop {
            let context = Box::new(ListenerContext {
                sender: sender.clone(),
                tap: AtomicPtr::new(std::ptr::null_mut()),
                right_option_pressed: Arc::clone(&right_option_pressed),
            });
            let context_ptr = Box::into_raw(context);
            let tap = CGEventTapCreate(
                CGEventTapLocation::HID,
                0,
                MACOS_EVENT_TAP_OPTIONS,
                mask,
                callback,
                context_ptr.cast(),
            );
            if tap.is_null() {
                drop(Box::from_raw(context_ptr));
                std::thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }
            (*context_ptr).tap.store(tap.cast_mut(), Ordering::Relaxed);
            let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
            if source.is_null() {
                CFRelease(tap);
                drop(Box::from_raw(context_ptr));
                std::thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }
            let _ = sender.send(InputEvent::Reset);
            let run_loop = CFRunLoopGetCurrent();
            CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
            CGEventTapEnable(tap, true);
            CFRunLoopRun();
            let _ = sender.send(InputEvent::Reset);
            CFRelease(source);
            CFRelease(tap);
            drop(Box::from_raw(context_ptr));
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });
}

#[cfg(test)]
mod tests;
