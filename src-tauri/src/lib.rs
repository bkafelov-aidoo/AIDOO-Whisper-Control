#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

mod aidoo;
mod app_ui;
mod audio;
mod commands;
mod dictation;
mod feedback_sound;
mod live;
mod recovery;
mod wake_runtime;
mod wake_word;

use aidoo::commands::*;
use aidoo::protocol::*;
use aidoo::schedule::*;
use app_ui::*;
use commands::*;
use dictation::*;
use recovery::*;
use usage::*;
use wake_runtime::*;

mod models;
mod shortcuts;
mod storage;
#[cfg(test)]
mod tests;
mod text_insertion;
mod transcription;
mod usage;

use chrono::{Local, Utc};
use models::{
    AppSettings, BootstrapState, FailedRecording, OverlayBootstrapState, RecordingProgress,
    RecordingSnapshot, TranscriptEntry, TranscriptionCompleted, UsageLedger,
};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::menu::{
    AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu, HELP_SUBMENU_ID, WINDOW_SUBMENU_ID,
};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State, WindowEvent};
use zeroize::Zeroizing;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

const KEYRING_SERVICE: &str = "app.aidoo.whisper-lite";
const KEYRING_USER: &str = "openai-api-key";
const AIDOO_KEYRING_USER: &str = "aidoo-password";
const TRAY_ID: &str = "aidoo-whisper-lite";
const APP_MENU_ID: &str = "aidoo-app-menu";
const APP_QUIT_MENU_ID: &str = "aidoo-app-quit";
const MAX_RECORDING_DURATION: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const IN_FLIGHT_RECOVERY_ERROR: &str = "Възстановен е запис след прекъсване. Не може да бъде изпратен повторно автоматично, за да се избегне повторно API таксуване.";
const CHARGED_RECOVERY_ERROR: &str =
    "Този recovery запис вече е транскрибиран. Изберете „Изтрий“, за да не бъде таксуван повторно.";

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecoveryPlan {
    FinishLocally(String),
    Transcribe,
}

#[derive(Default)]
struct AssistantStartRequest(AtomicBool);

impl AssistantStartRequest {
    fn request(&self) {
        self.0.store(true, Ordering::Release);
    }

    fn take(&self) -> bool {
        self.0.swap(false, Ordering::AcqRel)
    }
}

fn recovery_plan(failed: &FailedRecording) -> Result<RecoveryPlan, String> {
    if let Some(text) = failed.completed_text.as_ref() {
        return Ok(RecoveryPlan::FinishLocally(text.clone()));
    }
    if failed.retryable {
        return Ok(RecoveryPlan::Transcribe);
    }
    Err(CHARGED_RECOVERY_ERROR.into())
}

struct AppState {
    settings: Mutex<AppSettings>,
    history: Mutex<Vec<TranscriptEntry>>,
    usage: Mutex<UsageLedger>,
    failed_recording: Mutex<Option<FailedRecording>>,
    recorder: audio::RecorderService,
    wake_word: wake_word::WakeWordService,
    shortcut_capture: Mutex<Option<String>>,
    recording_status: Mutex<String>,
    recording_progress: Mutex<RecordingProgress>,
    recording_started_at: Mutex<Option<std::time::Instant>>,
    recording_trigger: Mutex<Option<String>>,
    recording_active: AtomicBool,
    operation_active: AtomicBool,
    stop_requested: AtomicBool,
    status_generation: AtomicU64,
    last_recording_error: Mutex<Option<String>>,
    wake_word_error: Mutex<Option<String>>,
    wake_word_listening: AtomicBool,
    wake_word_calibrating: AtomicBool,
    live_session_active: AtomicBool,
    live_session_generation: AtomicU64,
    live_usage_timing: Mutex<Option<usage::LiveUsageTiming>>,
    live_backend_response_ids: Mutex<HashSet<String>>,
    live_phase: Mutex<String>,
    assistant_start_request: AssistantStartRequest,
    aidoo: aidoo::runtime::AidooRuntime,
    aidoo_connection_error: Mutex<Option<String>>,
    api_key: Mutex<Option<Zeroizing<String>>>,
}

impl AppState {
    fn load() -> Self {
        let _ = storage::ensure_directories();
        audio::cleanup_stale_temporary_audio();
        let mut history = storage::load_history();
        recover_pending_history_deletion(&mut history);
        let usage = storage::load_usage(&history);
        let api_key = keyring_entry()
            .ok()
            .and_then(|entry| entry.get_password().ok())
            .map(Zeroizing::new);
        let failed_recording = storage::load_failed_recording();
        let recovery_error = failed_recording
            .as_ref()
            .map(|recording| recording.error.clone());
        Self {
            settings: Mutex::new(storage::load_settings()),
            history: Mutex::new(history),
            usage: Mutex::new(usage),
            failed_recording: Mutex::new(failed_recording),
            recorder: audio::RecorderService::new(),
            wake_word: wake_word::WakeWordService::new(),
            shortcut_capture: Mutex::new(None),
            recording_status: Mutex::new(if recovery_error.is_some() {
                "error".into()
            } else {
                "idle".into()
            }),
            recording_progress: Mutex::new(RecordingProgress::default()),
            recording_started_at: Mutex::new(None),
            recording_trigger: Mutex::new(None),
            recording_active: AtomicBool::new(false),
            operation_active: AtomicBool::new(false),
            stop_requested: AtomicBool::new(false),
            status_generation: AtomicU64::new(0),
            last_recording_error: Mutex::new(recovery_error),
            wake_word_error: Mutex::new(None),
            wake_word_listening: AtomicBool::new(false),
            wake_word_calibrating: AtomicBool::new(false),
            live_session_active: AtomicBool::new(false),
            live_session_generation: AtomicU64::new(0),
            live_usage_timing: Mutex::new(None),
            live_backend_response_ids: Mutex::new(HashSet::new()),
            live_phase: Mutex::new("idle".into()),
            assistant_start_request: AssistantStartRequest::default(),
            aidoo: aidoo::runtime::AidooRuntime::new(),
            aidoo_connection_error: Mutex::new(None),
            api_key: Mutex::new(api_key),
        }
    }
}

fn keyring_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|error| error.to_string())
}

fn aidoo_keyring_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, AIDOO_KEYRING_USER).map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

pub(crate) fn accessibility_granted() -> bool {
    #[cfg(target_os = "macos")]
    // SAFETY: AXIsProcessTrusted takes no pointers or caller-owned buffers and only returns the
    // current process trust state from the macOS ApplicationServices framework.
    unsafe {
        AXIsProcessTrusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

fn show_main_window(app: &AppHandle, settings_page: bool) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        if settings_page {
            let _ = app.emit("navigate", "settings");
        }
    }
}

fn install_tray(app: &tauri::App) -> tauri::Result<()> {
    let has_recovery = app
        .state::<AppState>()
        .failed_recording
        .lock()
        .map(|recording| recording.is_some())
        .unwrap_or(true);
    let status = resolved_tray_state(
        "idle",
        accessibility_granted(),
        tray_setup_ready(&app.state::<AppState>()),
        has_recovery,
    );
    let menu = build_tray_menu(app.handle(), status)?;
    let initial_tooltip = match (uses_english_ui(app.handle()), status) {
        (true, "recovery") => "AIDOO Whisper Lite — action required",
        (false, "recovery") => "AIDOO Whisper Lite — нужно е действие",
        (true, "setup") => "AIDOO Whisper Lite — finish setup",
        (false, "setup") => "AIDOO Whisper Lite — довършете настройката",
        (true, "permission") => "AIDOO Whisper Lite — permission required",
        (false, "permission") => "AIDOO Whisper Lite — нужно е разрешение",
        (true, _) => "AIDOO Whisper Lite — ready",
        (false, _) => "AIDOO Whisper Lite — готов",
    };
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(initial_tooltip)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app, false),
            "settings" => show_main_window(app, true),
            "stop" => {
                if app
                    .state::<AppState>()
                    .live_session_active
                    .load(Ordering::Acquire)
                {
                    if release_live_session(app, None) {
                        let _ = app.emit("live:force-close", "tray-stop");
                    }
                } else {
                    request_dictation_stop(app);
                }
            }
            "copy-error" => {
                let state = app.state::<AppState>();
                let error = state
                    .last_recording_error
                    .lock()
                    .ok()
                    .and_then(|value| value.clone())
                    .or_else(|| {
                        state
                            .wake_word_error
                            .lock()
                            .ok()
                            .and_then(|value| value.clone())
                    });
                if let Some(error) = error {
                    let localized = localized_native_error(&error, uses_english_ui(app));
                    let _ = text_insertion::copy(&localized);
                }
            }
            "quit" => request_app_quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } = event
            {
                show_main_window(tray.app_handle(), false);
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .manage(AppState::load())
        .menu(build_application_menu)
        .on_menu_event(|app, event| {
            if event.id.as_ref() == APP_QUIT_MENU_ID {
                request_app_quit(app);
            }
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                } else if window.label() == "overlay" {
                    api.prevent_close();
                }
            }
        })
        .setup(|app| {
            install_tray(app)?;
            if let Some(overlay) = app.get_webview_window("overlay") {
                let _ = overlay.set_ignore_cursor_events(true);
                let _ = overlay.set_shadow(false);
                let _ = overlay.set_always_on_top(true);
                let _ = overlay.set_visible_on_all_workspaces(true);
                let _ = overlay.set_focusable(false);
            }
            shortcuts::install(app.handle().clone());
            install_wake_word_events(app.handle().clone());
            install_macos_power_observers(app.handle().clone());
            schedule_wake_word_reconcile(app.handle(), std::time::Duration::from_millis(500));
            schedule_aidoo_auto_reconnect(app.handle().clone(), false);
            storage::append_diagnostic("application started");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            overlay_bootstrap,
            update_settings,
            save_api_key,
            delete_api_key,
            connect_aidoo,
            reconnect_aidoo,
            disconnect_aidoo,
            aidoo_search_patients,
            aidoo_select_patient,
            aidoo_next_patient,
            aidoo_begin_status,
            aidoo_start_status_visit,
            aidoo_apply_status,
            aidoo_finish_status,
            aidoo_add_procedure,
            aidoo_write_diagnosis,
            aidoo_write_official_note,
            aidoo_find_schedule_slot,
            aidoo_book_schedule_slot,
            aidoo_status_catalog,
            aidoo_diagnosis_catalog,
            aidoo_procedure_catalog,
            aidoo_active_treatments,
            aidoo_create_status_visit,
            aidoo_prepare_status_draft,
            aidoo_confirm_status_draft,
            aidoo_cancel_status_draft,
            aidoo_prepare_treatment_draft,
            aidoo_confirm_treatment_draft,
            aidoo_cancel_treatment_draft,
            begin_shortcut_capture,
            cancel_shortcut_capture,
            test_microphone,
            start_wake_word_calibration,
            stop_wake_word_calibration,
            prepare_live_session,
            create_live_session,
            end_live_session,
            record_live_backend_usage,
            set_live_phase,
            request_live_stop,
            take_assistant_request,
            start_voice_dictation,
            start_recording,
            stop_and_transcribe,
            retry_failed_transcription,
            retranscribe_history_item,
            delete_failed_recording,
            current_recording_snapshot,
            copy_text,
            delete_history_item,
            open_accessibility_settings,
            refresh_accessibility_status,
            reposition_overlay,
            open_local_path,
            create_diagnostic_bundle
        ])
        .build(tauri::generate_context!())
        .expect("error while building AIDOO Whisper Lite");
    app.run(|app, event| match event {
        tauri::RunEvent::ExitRequested { api, .. }
            if app
                .state::<AppState>()
                .operation_active
                .load(Ordering::Acquire) =>
        {
            api.prevent_exit();
            let message = if uses_english_ui(app) {
                "Wait for the current operation to finish before quitting."
            } else {
                "Изчакайте текущата операция да приключи, преди да затворите приложението."
            };
            let _ = app.emit("toast", message);
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => show_main_window(app, false),
        tauri::RunEvent::Resumed => {
            stop_wake_word_listener(&app.state::<AppState>());
            schedule_wake_word_reconcile(app, std::time::Duration::from_secs(1));
            schedule_aidoo_auto_reconnect(app.clone(), true);
        }
        _ => {}
    });
}
