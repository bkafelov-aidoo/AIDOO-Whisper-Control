use super::*;

fn settings() -> AppSettings {
    AppSettings::default()
}

#[test]
fn default_shortcut_is_safe() {
    validate_settings(&settings()).unwrap();
    assert_eq!(
        settings().dictation_shortcut,
        ShortcutBinding::key("alt_gr", &[])
    );
}

#[test]
fn rejects_bare_letters() {
    assert!(validate_binding(&ShortcutBinding::key("key_a", &[])).is_err());
}

#[test]
fn key_matching_requires_the_exact_modifier_set() {
    let binding = ShortcutBinding::key("space", &["control", "meta"]);
    let exact = BTreeSet::from(["control", "meta"]);
    let extra = BTreeSet::from(["control", "meta", "shift"]);
    assert!(binding_matches_key(&binding, "space", &exact));
    assert!(!binding_matches_key(&binding, "space", &extra));
    assert!(!binding_matches_key(&binding, "key_v", &exact));
}

#[test]
fn captured_shortcut_is_suppressed_until_the_user_releases_the_chord() {
    let mut runtime = ShortcutRuntime::default();

    runtime.track_physical_key("alt_gr", true);
    runtime.suppress_shortcuts_until_current_keys_are_released();

    // A repeated/duplicate key-down from the same physical press must not start dictation.
    runtime.track_physical_key("alt_gr", true);
    assert!(runtime.consume_capture_release_guard());

    // The release that ends shortcut capture is consumed as part of the same gesture.
    runtime.track_physical_key("alt_gr", false);
    assert!(runtime.consume_capture_release_guard());

    // A later, deliberate press is the first event that may start dictation.
    runtime.track_physical_key("alt_gr", true);
    assert!(!runtime.consume_capture_release_guard());
}

#[test]
fn right_option_matches_as_a_dedicated_physical_key() {
    let binding = ShortcutBinding::key("alt_gr", &[]);
    let platform_side_effect = BTreeSet::from(["control"]);
    assert!(binding_matches_key(
        &binding,
        "alt_gr",
        &platform_side_effect
    ));
}

#[test]
#[cfg(target_os = "macos")]
fn macos_listener_uses_accessibility_event_tap() {
    // Listen-only taps request the separate Input Monitoring TCC service and can silently
    // stop working after an identity migration. The default tap is covered by the existing
    // Accessibility permission that is required for Auto Paste as well.
    assert_eq!(MACOS_EVENT_TAP_OPTIONS, 0);
}

#[test]
#[cfg(target_os = "macos")]
fn right_option_release_is_detected_by_transition_or_missing_flag() {
    assert!(macos_modifier_transition_is_pressed(
        61,
        MACOS_OPTION_FLAG_MASK,
        false
    ));
    assert!(!macos_modifier_transition_is_pressed(
        61,
        MACOS_OPTION_FLAG_MASK,
        true
    ));
    assert!(macos_right_option_missing_from_flags(true, 0));
    assert!(!macos_right_option_missing_from_flags(
        true,
        MACOS_OPTION_FLAG_MASK
    ));
    assert!(!macos_right_option_missing_from_flags(false, 0));
}

#[test]
#[cfg(target_os = "macos")]
fn listener_reset_releases_stale_push_to_talk_state() {
    let mut runtime = ShortcutRuntime {
        modifiers: ModifierState {
            alt: true,
            ..ModifierState::default()
        },
        dictation_trigger: Some(InputTrigger::Key("alt_gr".into())),
        ..ShortcutRuntime::default()
    };

    assert!(runtime.reset_stale_inputs());
    assert!(runtime.dictation_trigger.is_none());
    assert!(runtime.modifiers.names_for("key_a").is_empty());
    assert!(!runtime.reset_stale_inputs());
}
