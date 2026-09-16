use super::*;

#[cfg(target_os = "macos")]
pub(super) fn install_macos_power_observers(app: AppHandle) {
    use block2::RcBlock;
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillSleepNotification,
    };
    use objc2_foundation::NSNotification;
    use std::ptr::NonNull;

    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let sleep_app = app.clone();
    let sleep_block = RcBlock::new(move |_: NonNull<NSNotification>| {
        storage::append_diagnostic("system will sleep; wake word listener stopped");
        stop_wake_word_listener(&sleep_app.state::<AppState>());
    });
    let wake_block = RcBlock::new(move |_: NonNull<NSNotification>| {
        storage::append_diagnostic("system woke; scheduling wake word listener restart");
        schedule_wake_word_reconcile(&app, std::time::Duration::from_secs(1));
    });
    // SAFETY: NSWorkspace owns its notification center for the application lifetime. Both blocks
    // capture only owned AppHandle values, accept the documented NSNotification argument, and the
    // center retains the returned observer tokens until process exit.
    unsafe {
        let _ = center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceWillSleepNotification),
            None,
            None,
            &sleep_block,
        );
        let _ = center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidWakeNotification),
            None,
            None,
            &wake_block,
        );
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn install_macos_power_observers(_app: AppHandle) {}

pub(super) fn install_wake_word_events(app: AppHandle) {
    let Some(events) = app.state::<AppState>().wake_word.take_events() else {
        return;
    };
    std::thread::Builder::new()
        .name("aidoo-wakeword-events".into())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                match event {
                    wake_word::WakeWordEvent::Detected { confidence } => {
                        let state = app.state::<AppState>();
                        if !state.wake_word_listening.load(Ordering::Acquire)
                            || !wake_word_should_listen(&state)
                        {
                            continue;
                        }
                        if state.wake_word_calibrating.load(Ordering::Acquire) {
                            let _ = app.emit("wake-word:calibration-detected", confidence);
                            continue;
                        }
                        stop_wake_word_listener(&state);
                        if let Ok(mut error) = state.wake_word_error.lock() {
                            *error = None;
                        }
                        storage::append_diagnostic(&format!(
                            "wake word detected; confidence={confidence:.3}"
                        ));
                        state.assistant_start_request.request();
                        show_main_window(&app, false);
                        let _ = app.emit("assistant:requested", ());
                    }
                    wake_word::WakeWordEvent::Scores {
                        rms,
                        primary,
                        confirmation,
                    } => {
                        if app
                            .state::<AppState>()
                            .wake_word_calibrating
                            .load(Ordering::Acquire)
                        {
                            storage::append_diagnostic(&format!(
                                "wake calibration score; rms={rms:.5} primary={primary:.5} confirmation={confirmation:.5}"
                            ));
                            let _ = app.emit(
                                "wake-word:calibration-score",
                                serde_json::json!({
                                    "rms": rms,
                                    "primary": primary,
                                    "confirmation": confirmation,
                                }),
                            );
                        }
                    }
                    wake_word::WakeWordEvent::Level { rms } => {
                        if app
                            .state::<AppState>()
                            .wake_word_calibrating
                            .load(Ordering::Acquire)
                        {
                            let _ = app.emit("wake-word:calibration-level", rms);
                        }
                    }
                    wake_word::WakeWordEvent::Failed(error) => {
                        let state = app.state::<AppState>();
                        let calibrating = state.wake_word_calibrating.swap(false, Ordering::AcqRel);
                        stop_wake_word_listener(&state);
                        if let Ok(mut current) = state.wake_word_error.lock() {
                            *current = Some(error.clone());
                        }
                        storage::append_diagnostic(&format!("wake word stream failed: {error}"));
                        let _ = app.emit("wake-word:status", "error");
                        if calibrating {
                            let _ = app.emit("wake-word:calibration-error", &error);
                        } else {
                            let _ = app.emit("toast", &error);
                        }
                        refresh_tray_menu(&app);
                        schedule_wake_word_reconcile(&app, std::time::Duration::from_secs(5));
                    }
                }
            }
        })
        .expect("wake word event thread must start");
}
