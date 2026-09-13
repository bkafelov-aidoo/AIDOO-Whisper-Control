mod audio;
mod models;
mod shortcuts;
mod storage;
mod text_insertion;
mod transcription;

use chrono::{Local, Utc};
use models::{
    AppSettings, BootstrapState, FailedRecording, RecordingProgress, RecordingSnapshot,
    TranscriptEntry, TranscriptionCompleted,
};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State, WindowEvent};
use zeroize::Zeroizing;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

const KEYRING_SERVICE: &str = "app.aidoo.whisper-lite";
const KEYRING_USER: &str = "openai-api-key";
const TRAY_ID: &str = "aidoo-whisper-lite";

struct AppState {
    settings: Mutex<AppSettings>,
    history: Mutex<Vec<TranscriptEntry>>,
    failed_recording: Mutex<Option<FailedRecording>>,
    recorder: audio::RecorderService,
    shortcut_capture: Mutex<Option<String>>,
    recording_status: Mutex<String>,
    recording_progress: Mutex<RecordingProgress>,
    recording_started_at: Mutex<Option<std::time::Instant>>,
    recording_active: AtomicBool,
    operation_active: AtomicBool,
    stop_requested: AtomicBool,
    status_generation: AtomicU64,
    last_recording_error: Mutex<Option<String>>,
    api_key: Mutex<Option<Zeroizing<String>>>,
}

impl AppState {
    fn load() -> Self {
        let _ = storage::ensure_directories();
        let api_key = keyring_entry()
            .ok()
            .and_then(|entry| entry.get_password().ok())
            .map(Zeroizing::new);
        Self {
            settings: Mutex::new(storage::load_settings()),
            history: Mutex::new(storage::load_history()),
            failed_recording: Mutex::new(storage::load_failed_recording()),
            recorder: audio::RecorderService::new(),
            shortcut_capture: Mutex::new(None),
            recording_status: Mutex::new("idle".into()),
            recording_progress: Mutex::new(RecordingProgress::default()),
            recording_started_at: Mutex::new(None),
            recording_active: AtomicBool::new(false),
            operation_active: AtomicBool::new(false),
            stop_requested: AtomicBool::new(false),
            status_generation: AtomicU64::new(0),
            last_recording_error: Mutex::new(None),
            api_key: Mutex::new(api_key),
        }
    }
}

fn keyring_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

pub(crate) fn accessibility_granted() -> bool {
    #[cfg(target_os = "macos")]
    unsafe {
        AXIsProcessTrusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

fn recording_snapshot(state: &AppState) -> RecordingSnapshot {
    let state_name = state
        .recording_status
        .lock()
        .map(|value| value.clone())
        .unwrap_or_else(|_| "error".into());
    let progress = state
        .recording_progress
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    let elapsed_seconds = state
        .recording_started_at
        .lock()
        .ok()
        .and_then(|value| value.as_ref().map(std::time::Instant::elapsed))
        .map(|value| value.as_secs_f64())
        .unwrap_or(0.0);
    let error = state
        .last_recording_error
        .lock()
        .ok()
        .and_then(|value| value.clone());
    RecordingSnapshot {
        state: state_name,
        progress,
        elapsed_seconds,
        error,
    }
}

fn emit_snapshot(app: &AppHandle) {
    let _ = app.emit(
        "recording:snapshot",
        recording_snapshot(&app.state::<AppState>()),
    );
}

fn uses_english_ui(app: &AppHandle) -> bool {
    let preference = app
        .state::<AppState>()
        .settings
        .lock()
        .map(|settings| settings.ui_language.clone())
        .unwrap_or_else(|_| "auto".into());
    preference == "en"
        || (preference == "auto"
            && !sys_locale::get_locale()
                .unwrap_or_default()
                .to_lowercase()
                .starts_with("bg"))
}

fn status_label(state: &str, english: bool) -> &'static str {
    match (english, state) {
        (true, "starting") => "Status: starting the microphone",
        (true, "recording") => "Status: recording · release the shortcut to finish",
        (true, "transcribing") => "Status: transcribing",
        (true, "done") => "Status: text is ready",
        (true, "error") => "Status: error",
        (true, "permission") => "Status: permission required",
        (true, _) => "Status: ready for dictation",
        (false, "starting") => "Състояние: стартирам микрофона",
        (false, "recording") => "Състояние: записвам · отпуснете shortcut-а за край",
        (false, "transcribing") => "Състояние: транскрибирам",
        (false, "done") => "Състояние: текстът е готов",
        (false, "error") => "Състояние: грешка",
        (false, "permission") => "Състояние: нужно е разрешение",
        (false, _) => "Състояние: готов за диктовка",
    }
}

fn compact_error(error: &str) -> String {
    let normalized = error.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= 150 {
        normalized
    } else {
        format!("{}…", normalized.chars().take(149).collect::<String>())
    }
}

fn build_tray_menu(app: &AppHandle, current: &str) -> tauri::Result<Menu<tauri::Wry>> {
    let english = uses_english_ui(app);
    let status = MenuItem::with_id(
        app,
        "status",
        status_label(current, english),
        false,
        None::<&str>,
    )?;
    let show = MenuItem::with_id(
        app,
        "show",
        if english {
            "Open AIDOO Whisper Lite"
        } else {
            "Отвори AIDOO Whisper Lite"
        },
        true,
        None::<&str>,
    )?;
    let stop = MenuItem::with_id(
        app,
        "stop",
        if english {
            "Stop and transcribe"
        } else {
            "Спри и транскрибирай"
        },
        matches!(current, "starting" | "recording"),
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(
        app,
        "settings",
        if english {
            "Settings"
        } else {
            "Настройки"
        },
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        "quit",
        if english { "Quit" } else { "Изход" },
        true,
        None::<&str>,
    )?;
    let error = app
        .state::<AppState>()
        .last_recording_error
        .lock()
        .ok()
        .and_then(|value| value.clone());
    if let Some(error) = error {
        let error_item = MenuItem::with_id(
            app,
            "last-error",
            format!(
                "{}: {}",
                if english {
                    "Last error"
                } else {
                    "Последна грешка"
                },
                compact_error(&error)
            ),
            false,
            None::<&str>,
        )?;
        let copy_error = MenuItem::with_id(
            app,
            "copy-error",
            if english {
                "Copy error"
            } else {
                "Копирай грешката"
            },
            true,
            None::<&str>,
        )?;
        Menu::with_items(
            app,
            &[
                &status,
                &error_item,
                &copy_error,
                &show,
                &settings,
                &stop,
                &quit,
            ],
        )
    } else {
        Menu::with_items(app, &[&status, &show, &settings, &stop, &quit])
    }
}

fn update_tray_menu(app: &AppHandle, current: &str) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let english = uses_english_ui(app);
        let tooltip = match (english, current) {
            (true, "recording") => "AIDOO Whisper Lite — recording",
            (true, "transcribing") => "AIDOO Whisper Lite — transcribing",
            (true, "error") => "AIDOO Whisper Lite — error",
            (true, _) => "AIDOO Whisper Lite — ready",
            (false, "recording") => "AIDOO Whisper Lite — записвам",
            (false, "transcribing") => "AIDOO Whisper Lite — транскрибирам",
            (false, "error") => "AIDOO Whisper Lite — грешка",
            (false, _) => "AIDOO Whisper Lite — готов",
        };
        let _ = tray.set_tooltip(Some(tooltip));
        if let Ok(menu) = build_tray_menu(app, current) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn refresh_tray_menu(app: &AppHandle) {
    let granted = accessibility_granted();
    let current = app
        .state::<AppState>()
        .recording_status
        .lock()
        .map(|value| value.clone())
        .unwrap_or_else(|_| "idle".into());
    let tray_state = if matches!(
        current.as_str(),
        "starting" | "recording" | "transcribing" | "done" | "error"
    ) {
        current.as_str()
    } else if granted {
        "idle"
    } else {
        "permission"
    };
    update_tray_menu(app, tray_state);
}

fn set_recording_state(app: &AppHandle, next: &str) {
    let state = app.state::<AppState>();
    if let Ok(mut current) = state.recording_status.lock() {
        *current = next.into();
    }
    if let Ok(mut started) = state.recording_started_at.lock() {
        match next {
            "recording" if started.is_none() => *started = Some(std::time::Instant::now()),
            "recording" => {}
            _ => *started = None,
        }
    }
    let generation = state.status_generation.fetch_add(1, Ordering::Relaxed) + 1;
    update_tray_menu(app, next);
    let _ = app.emit("recording:state", next);
    match next {
        "starting" | "recording" | "transcribing" | "done" | "error" => show_recording_overlay(app),
        "idle" => {
            if let Some(window) = app.get_webview_window("overlay") {
                let _ = window.hide();
            }
        }
        _ => {}
    }
    emit_snapshot(app);
    if matches!(next, "done" | "error") {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            if app
                .state::<AppState>()
                .status_generation
                .load(Ordering::Relaxed)
                == generation
            {
                set_recording_state(&app, "idle");
            }
        });
    }
}

fn set_progress(app: &AppHandle, percent: u8, stage: &str, determinate: bool) {
    let progress = RecordingProgress {
        percent: percent.min(100),
        stage: stage.into(),
        determinate,
    };
    if let Ok(mut current) = app.state::<AppState>().recording_progress.lock() {
        *current = progress.clone();
    }
    let _ = app.emit("recording:progress", &progress);
    emit_snapshot(app);
}

fn set_error(app: &AppHandle, error: &str) {
    storage::append_diagnostic(&format!("dictation error: {error}"));
    if let Ok(mut current) = app.state::<AppState>().last_recording_error.lock() {
        *current = Some(error.into());
    }
    let _ = app.emit("recording:error", error);
    set_recording_state(app, "error");
}

pub(crate) fn show_recording_overlay(app: &AppHandle) {
    reposition_overlay_inner(app);
    if let Some(window) = app.get_webview_window("overlay") {
        let _ = window.show();
    }
    emit_snapshot(app);
}

fn reposition_overlay_inner(app: &AppHandle) {
    let Some(window) = app.get_webview_window("overlay") else {
        return;
    };
    let Ok(cursor) = app.cursor_position() else {
        return;
    };
    let Ok(Some(monitor)) = app.monitor_from_point(cursor.x, cursor.y) else {
        return;
    };
    let Ok(size) = window.outer_size() else {
        return;
    };
    let position = monitor.position();
    let monitor_size = monitor.size();
    let x = position.x + (monitor_size.width.saturating_sub(size.width) / 2) as i32;
    // 76px keeps the overlay clear of a normally sized Dock.
    let y = position.y + monitor_size.height.saturating_sub(size.height + 76) as i32;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

#[tauri::command]
fn reposition_overlay(app: AppHandle) {
    reposition_overlay_inner(&app);
}

fn selected_output_dir(settings: &AppSettings) -> PathBuf {
    settings
        .output_directory
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(storage::default_output_dir)
}

fn api_key_from_state(state: &AppState) -> Result<Zeroizing<String>, String> {
    if let Ok(cache) = state.api_key.lock() {
        if let Some(value) = cache.as_ref() {
            return Ok(Zeroizing::new(value.to_string()));
        }
    }
    let password = keyring_entry()?.get_password().map_err(|_| {
        "Няма достъпен OpenAI API ключ. Отворете настройките и го добавете.".to_string()
    })?;
    if let Ok(mut cache) = state.api_key.lock() {
        *cache = Some(Zeroizing::new(password.clone()));
    }
    Ok(Zeroizing::new(password))
}

fn readiness_error(state: &AppState) -> Option<String> {
    let settings = state.settings.lock().ok()?.clone();
    if !settings.onboarding_complete {
        return Some("Завършете началната настройка, преди да използвате диктовката.".into());
    }
    if api_key_from_state(state).is_err() {
        return Some("Добавете и проверете OpenAI API ключ.".into());
    }
    if audio::microphone_names().is_empty() {
        return Some("Не е намерен микрофон.".into());
    }
    if !accessibility_granted() {
        return Some(
            "Разрешете Accessibility, за да работят shortcut-ът и автоматичното поставяне.".into(),
        );
    }
    if state
        .failed_recording
        .lock()
        .map(|recording| recording.is_some())
        .unwrap_or(true)
    {
        return Some(
            "Има запазен неуспешен запис. Изберете „Опитай отново“ или „Изтрий“, преди да започнете нова диктовка."
                .into(),
        );
    }
    None
}

struct OperationGuard<'a>(&'a AtomicBool);

impl Drop for OperationGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn acquire_operation(state: &AppState) -> Result<OperationGuard<'_>, String> {
    state
        .operation_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| OperationGuard(&state.operation_active))
        .map_err(|_| "Изчакайте текущата диктовка да приключи.".into())
}

fn start_recording_inner(app: &AppHandle) -> Result<audio::AudioStartInfo, String> {
    let state = app.state::<AppState>();
    let operation = acquire_operation(&state)?;
    if state.recording_active.swap(true, Ordering::AcqRel) {
        return Err("Вече има активен запис.".into());
    }
    if let Some(error) = readiness_error(&state) {
        state.recording_active.store(false, Ordering::Release);
        return Err(error);
    }
    state.stop_requested.store(false, Ordering::Release);
    if let Ok(mut error) = state.last_recording_error.lock() {
        *error = None;
    }
    set_progress(app, 0, "starting_microphone", false);
    set_recording_state(app, "starting");
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")?
        .clone();
    let routing = audio::MicrophoneRoutingConfig {
        preferred_name: settings.microphone_name,
        priority: settings.microphone_priority,
        automatic_fallback: settings.automatic_microphone_fallback,
    };
    match state.recorder.start(routing) {
        Ok(info) => {
            if info.used_fallback {
                let message = format!(
                    "Избраният микрофон не е наличен. Използвам „{}“.",
                    info.device_name
                );
                let _ = app.emit("toast", message);
            }
            set_recording_state(app, "recording");
            std::mem::forget(operation);
            Ok(info)
        }
        Err(error) => {
            state.recording_active.store(false, Ordering::Release);
            Err(error)
        }
    }
}

pub(crate) fn request_dictation_start(app: &AppHandle) -> bool {
    match start_recording_inner(app) {
        Ok(_) => true,
        Err(error) => {
            set_error(app, &error);
            false
        }
    }
}

pub(crate) fn request_dictation_stop(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !state.recording_active.load(Ordering::Acquire) {
        return;
    }
    if state.stop_requested.swap(true, Ordering::AcqRel) {
        return;
    }
    set_recording_state(app, "transcribing");
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = stop_and_transcribe_inner(&app).await {
            set_error(&app, &error);
        }
    });
}

#[tauri::command]
fn start_recording(app: AppHandle) -> Result<audio::AudioStartInfo, String> {
    start_recording_inner(&app).inspect_err(|error| set_error(&app, error))
}

#[tauri::command]
async fn stop_and_transcribe(app: AppHandle) -> Result<TranscriptionCompleted, String> {
    if app
        .state::<AppState>()
        .stop_requested
        .swap(true, Ordering::AcqRel)
    {
        return Err("Транскрипцията вече е стартирана.".into());
    }
    set_recording_state(&app, "transcribing");
    stop_and_transcribe_inner(&app)
        .await
        .inspect_err(|error| set_error(&app, error))
}

async fn stop_and_transcribe_inner(app: &AppHandle) -> Result<TranscriptionCompleted, String> {
    let state = app.state::<AppState>();
    let _operation = OperationGuard(&state.operation_active);
    let captured = state.recorder.finish();
    state.recording_active.store(false, Ordering::Release);
    let captured = captured?;
    if captured.duration_seconds < 0.20 {
        let _ = std::fs::remove_file(&captured.path);
        return Err(
            "Записът е прекалено кратък. Задръжте shortcut-а и говорете поне половин секунда."
                .into(),
        );
    }
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")?
        .clone();
    let api_key = api_key_from_state(&state)?;
    set_progress(app, 5, "compressing_audio", false);
    let staged = match prepare_flac(&captured.path).await {
        Ok(path) => path,
        Err(error) => {
            let failed =
                retain_failed_recording(&captured.path, captured.duration_seconds, &error)?;
            store_failed_recording(app, &state, failed)?;
            return Err(error);
        }
    };
    let app_for_progress = app.clone();
    let callback: transcription::ProgressCallback = Arc::new(move |percent, stage, determinate| {
        set_progress(&app_for_progress, percent.max(10), stage, determinate);
    });
    let result = transcription::transcribe(&staged, &api_key, &settings, Some(callback)).await;
    match result {
        Ok(text) => {
            let completed =
                match finalize_success(app, &settings, &staged, captured.duration_seconds, text) {
                    Ok(completed) => completed,
                    Err(error) => {
                        let failed =
                            retain_failed_recording(&staged, captured.duration_seconds, &error)?;
                        store_failed_recording(app, &state, failed)?;
                        let _ = std::fs::remove_file(&captured.path);
                        return Err(error);
                    }
                };
            let _ = std::fs::remove_file(&captured.path);
            let _ = std::fs::remove_file(&staged);
            set_recording_state(app, "done");
            let _ = app.emit("transcription:completed", &completed);
            Ok(completed)
        }
        Err(error) => {
            let failed = retain_failed_recording(&staged, captured.duration_seconds, &error)?;
            let _ = std::fs::remove_file(&captured.path);
            store_failed_recording(app, &state, failed)?;
            Err(error)
        }
    }
}

async fn prepare_flac(wav: &Path) -> Result<PathBuf, String> {
    let wav = wav.to_path_buf();
    let target = std::env::temp_dir().join(format!("aidoo-lite-{}.flac", uuid::Uuid::new_v4()));
    let target_for_task = target.clone();
    tokio::task::spawn_blocking(move || transcription::encode_wav_to_flac(&wav, &target_for_task))
        .await
        .map_err(|error| format!("FLAC процесът беше прекъснат: {error}"))??;
    Ok(target)
}

fn safe_file_stem() -> String {
    format!(
        "AIDOO-Whisper-{}-{}",
        Local::now().format("%Y-%m-%d_%H-%M-%S"),
        &uuid::Uuid::new_v4().simple().to_string()[..6]
    )
}

fn finalize_success(
    app: &AppHandle,
    settings: &AppSettings,
    staged_audio: &Path,
    duration_seconds: f64,
    text: String,
) -> Result<TranscriptionCompleted, String> {
    text_insertion::copy(&text)
        .map_err(|error| format!("Текстът е готов, но клипбордът не е достъпен: {error}"))?;

    let output_dir = selected_output_dir(settings);
    if settings.save_audio || settings.save_text {
        std::fs::create_dir_all(&output_dir)
            .map_err(|error| format!("Папката не може да бъде създадена: {error}"))?;
    }
    let stem = safe_file_stem();
    let audio_path = if settings.save_audio {
        let path = output_dir.join(format!("{stem}.flac"));
        std::fs::copy(staged_audio, &path)
            .map_err(|error| format!("FLAC файлът не може да бъде запазен: {error}"))?;
        Some(path)
    } else {
        None
    };
    let text_path = if settings.save_text {
        let path = output_dir.join(format!("{stem}.txt"));
        if let Err(error) = std::fs::write(&path, format!("{text}\n")) {
            remove_created_output(audio_path.as_deref(), None);
            return Err(format!("TXT файлът не може да бъде запазен: {error}"));
        }
        Some(path)
    } else {
        None
    };

    let entry = TranscriptEntry {
        id: uuid::Uuid::new_v4().to_string(),
        text: text.clone(),
        created_at: Utc::now().to_rfc3339(),
        duration_seconds,
        model: settings.model.clone(),
        language: settings.language.clone(),
        audio_path: audio_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string()),
        text_path: text_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string()),
    };
    let history_entry = if settings.history_enabled {
        let state = app.state::<AppState>();
        let mut history = match state.history.lock() {
            Ok(history) => history,
            Err(_) => {
                remove_created_output(audio_path.as_deref(), text_path.as_deref());
                return Err("Историята е заключена.".into());
            }
        };
        let mut next_history = history.clone();
        next_history.insert(0, entry.clone());
        next_history.truncate(10);
        if let Err(error) = storage::save_history(&next_history) {
            remove_created_output(audio_path.as_deref(), text_path.as_deref());
            return Err(format!("Историята не можа да бъде запазена: {error}"));
        }
        *history = next_history;
        Some(entry)
    } else {
        None
    };

    let (paste_succeeded, paste_error) = if settings.auto_paste {
        match text_insertion::paste() {
            Ok(()) => (true, None),
            Err(error) => {
                let message =
                    "Текстът не можа да бъде поставен. Копиран е в клипборда.".to_string();
                let _ = app.emit("toast", &message);
                storage::append_diagnostic(&format!("paste failed: {error}"));
                (false, Some(message))
            }
        }
    } else {
        (false, None)
    };

    Ok(TranscriptionCompleted {
        entry: history_entry,
        text,
        paste_succeeded,
        paste_error,
    })
}

fn remove_created_output(audio_path: Option<&Path>, text_path: Option<&Path>) {
    for path in [audio_path, text_path].into_iter().flatten() {
        let _ = std::fs::remove_file(path);
    }
}

fn retain_failed_recording(
    path: &Path,
    duration_seconds: f64,
    error: &str,
) -> Result<FailedRecording, String> {
    storage::ensure_directories()?;
    for extension in ["flac", "wav"] {
        let _ = std::fs::remove_file(
            storage::recovery_dir().join(format!("last-failed-dictation.{extension}")),
        );
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("flac");
    let target = storage::recovery_dir().join(format!("last-failed-dictation.{extension}"));
    std::fs::rename(path, &target)
        .or_else(|_| {
            std::fs::copy(path, &target)
                .map(|_| ())
                .and_then(|_| std::fs::remove_file(path))
        })
        .map_err(|error| format!("Неуспешният запис не можа да бъде запазен: {error}"))?;
    Ok(FailedRecording {
        path: target.to_string_lossy().to_string(),
        created_at: Utc::now().to_rfc3339(),
        duration_seconds,
        error: error.into(),
    })
}

fn store_failed_recording(
    app: &AppHandle,
    state: &AppState,
    failed: FailedRecording,
) -> Result<(), String> {
    let mut current = state
        .failed_recording
        .lock()
        .map_err(|_| "Recovery състоянието е заключено.")?;
    storage::save_failed_recording(&failed)?;
    *current = Some(failed.clone());
    let _ = app.emit("failed-recording:changed", &failed);
    Ok(())
}

fn clear_failed_recording_state(state: &AppState, remove_audio: bool) -> Result<(), String> {
    let mut current = state
        .failed_recording
        .lock()
        .map_err(|_| "Recovery състоянието е заключено.")?;
    let previous = current.clone();
    storage::clear_failed_recording()?;
    *current = None;
    if remove_audio {
        if let Some(previous) = previous {
            match std::fs::remove_file(previous.path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!("Recovery аудиото не можа да бъде изтрито: {error}"))
                }
            }
        }
    }
    Ok(())
}

#[tauri::command]
async fn retry_failed_transcription(app: AppHandle) -> Result<TranscriptionCompleted, String> {
    let state = app.state::<AppState>();
    let _operation = acquire_operation(&state)?;
    let failed = state
        .failed_recording
        .lock()
        .map_err(|_| "Recovery състоянието е заключено.")?
        .clone()
        .ok_or_else(|| "Няма неуспешен запис за повторен опит.".to_string())?;
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")?
        .clone();
    let key = api_key_from_state(&state)?;
    let source = PathBuf::from(&failed.path);
    if !source.is_file() {
        return Err("Запазеният неуспешен аудио файл не е намерен.".into());
    }
    set_recording_state(&app, "transcribing");
    let temporary_flac = source
        .extension()
        .and_then(|value| value.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("flac"));
    let staged = if temporary_flac {
        set_progress(&app, 5, "compressing_audio", false);
        match prepare_flac(&source).await {
            Ok(path) => path,
            Err(error) => {
                update_failed_recording_error(&state, &error);
                set_error(&app, &error);
                return Err(error);
            }
        }
    } else {
        source.clone()
    };
    let app_for_progress = app.clone();
    let callback: transcription::ProgressCallback = Arc::new(move |percent, stage, determinate| {
        set_progress(&app_for_progress, percent, stage, determinate)
    });
    match transcription::transcribe(&staged, &key, &settings, Some(callback)).await {
        Ok(text) => {
            let completed =
                match finalize_success(&app, &settings, &staged, failed.duration_seconds, text) {
                    Ok(completed) => completed,
                    Err(error) => {
                        if temporary_flac {
                            let _ = std::fs::remove_file(&staged);
                        }
                        update_failed_recording_error(&state, &error);
                        set_error(&app, &error);
                        return Err(error);
                    }
                };
            if let Err(error) = clear_failed_recording_state(&state, true) {
                storage::append_diagnostic(&format!("recovery cleanup failed: {error}"));
            }
            if temporary_flac {
                let _ = std::fs::remove_file(&staged);
            }
            set_recording_state(&app, "done");
            let _ = app.emit("failed-recording:changed", Option::<FailedRecording>::None);
            let _ = app.emit("transcription:completed", &completed);
            Ok(completed)
        }
        Err(error) => {
            if temporary_flac {
                let _ = std::fs::remove_file(&staged);
            }
            update_failed_recording_error(&state, &error);
            set_error(&app, &error);
            Err(error)
        }
    }
}

fn update_failed_recording_error(state: &AppState, error: &str) {
    if let Ok(mut current) = state.failed_recording.lock() {
        if let Some(value) = current.as_mut() {
            value.error = error.into();
            let _ = storage::save_failed_recording(value);
        }
    }
}

#[tauri::command]
async fn retranscribe_history_item(
    id: String,
    app: AppHandle,
) -> Result<TranscriptionCompleted, String> {
    let state = app.state::<AppState>();
    let _operation = acquire_operation(&state)?;
    let entry = state
        .history
        .lock()
        .map_err(|_| "Историята е заключена.")?
        .iter()
        .find(|entry| entry.id == id)
        .cloned()
        .ok_or_else(|| "Записът вече не е в историята.".to_string())?;
    let audio_path = entry
        .audio_path
        .ok_or_else(|| "За тази транскрипция няма запазен аудио файл.".to_string())?;
    if !Path::new(&audio_path).is_file() {
        return Err("Свързаният аудио файл не е намерен.".into());
    }
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")?
        .clone();
    let key = api_key_from_state(&state)?;
    set_recording_state(&app, "transcribing");
    let app_for_progress = app.clone();
    let callback: transcription::ProgressCallback = Arc::new(move |percent, stage, determinate| {
        set_progress(&app_for_progress, percent, stage, determinate)
    });
    match transcription::transcribe(Path::new(&audio_path), &key, &settings, Some(callback)).await {
        Ok(text) => {
            let completed = finalize_success(
                &app,
                &settings,
                Path::new(&audio_path),
                entry.duration_seconds,
                text,
            )?;
            set_recording_state(&app, "done");
            let _ = app.emit("transcription:completed", &completed);
            Ok(completed)
        }
        Err(error) => {
            set_error(&app, &error);
            Err(error)
        }
    }
}

#[tauri::command]
fn delete_failed_recording(app: AppHandle) -> Result<(), String> {
    clear_failed_recording_state(&app.state::<AppState>(), true)?;
    let _ = app.emit("failed-recording:changed", Option::<FailedRecording>::None);
    Ok(())
}

#[tauri::command]
fn bootstrap(app: AppHandle, state: State<'_, AppState>) -> BootstrapState {
    let settings = state
        .settings
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    let history = state
        .history
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    let failed_recording = state
        .failed_recording
        .lock()
        .ok()
        .and_then(|value| value.clone());
    let has_api_key = state
        .api_key
        .lock()
        .map(|value| value.is_some())
        .unwrap_or(false);
    BootstrapState {
        settings,
        history,
        failed_recording,
        microphones: audio::microphone_names(),
        has_api_key,
        accessibility_granted: accessibility_granted(),
        app_version: app.package_info().version.to_string(),
        default_output_directory: storage::default_output_dir().to_string_lossy().to_string(),
        recording: recording_snapshot(&state),
    }
}

#[tauri::command]
fn update_settings(
    settings: AppSettings,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSettings, String> {
    let mut settings = settings;
    settings.normalize();
    shortcuts::validate_settings(&settings)?;
    if settings.save_audio || settings.save_text {
        let directory = selected_output_dir(&settings);
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("Папката не може да бъде използвана: {error}"))?;
    }
    storage::save_settings(&settings)?;
    *state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")? = settings.clone();
    refresh_tray_menu(&app);
    let _ = app.emit("settings:changed", &settings);
    Ok(settings)
}

#[tauri::command]
async fn save_api_key(api_key: String, state: State<'_, AppState>) -> Result<(), String> {
    let key = Zeroizing::new(api_key.trim().to_string());
    transcription::validate_api_key(&key).await?;
    keyring_entry()?
        .set_password(&key)
        .map_err(|error| format!("Ключът не можа да бъде запазен в Keychain: {error}"))?;
    *state
        .api_key
        .lock()
        .map_err(|_| "API key cache е заключен.")? = Some(key);
    Ok(())
}

#[tauri::command]
fn delete_api_key(state: State<'_, AppState>) -> Result<(), String> {
    match keyring_entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(error) => return Err(format!("Ключът не можа да бъде изтрит: {error}")),
    }
    *state
        .api_key
        .lock()
        .map_err(|_| "API key cache е заключен.")? = None;
    Ok(())
}

#[tauri::command]
fn begin_shortcut_capture(state: State<'_, AppState>) -> Result<(), String> {
    shortcuts::begin_capture("dictation".into(), &state)
}

#[tauri::command]
fn cancel_shortcut_capture(state: State<'_, AppState>) -> Result<(), String> {
    shortcuts::cancel_capture(&state)
}

#[tauri::command]
async fn test_microphone(state: State<'_, AppState>) -> Result<audio::MicrophoneProbe, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")?
        .clone();
    let recorder = state.recorder.clone();
    tokio::task::spawn_blocking(move || {
        recorder.probe(audio::MicrophoneRoutingConfig {
            preferred_name: settings.microphone_name,
            priority: settings.microphone_priority,
            automatic_fallback: settings.automatic_microphone_fallback,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn current_recording_snapshot(state: State<'_, AppState>) -> RecordingSnapshot {
    recording_snapshot(&state)
}

#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    text_insertion::copy(&text)
}

#[tauri::command]
fn delete_history_item(
    id: String,
    delete_files: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut history = state.history.lock().map_err(|_| "Историята е заключена.")?;
    let removed = history.iter().find(|entry| entry.id == id).cloned();
    let mut next_history = history.clone();
    next_history.retain(|entry| entry.id != id);
    storage::save_history(&next_history)?;
    *history = next_history;
    if delete_files {
        if let Some(entry) = removed {
            let mut failures = Vec::new();
            for path in [entry.audio_path, entry.text_path].into_iter().flatten() {
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => failures.push(format!("{path}: {error}")),
                }
            }
            if !failures.is_empty() {
                return Err(format!(
                    "Записът е изтрит от историята, но някои файлове не можаха да бъдат изтрити: {}",
                    failures.join(" · ")
                ));
            }
        }
    }
    Ok(())
}

#[tauri::command]
fn open_accessibility_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .status()
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn refresh_accessibility_status(app: AppHandle) -> bool {
    let granted = accessibility_granted();
    refresh_tray_menu(&app);
    granted
}

#[tauri::command]
fn create_diagnostic_bundle(app: AppHandle) -> Result<String, String> {
    storage::ensure_directories()?;
    let path = storage::data_dir().join(format!(
        "AIDOO-Whisper-Lite-Diagnostics-{}.zip",
        Local::now().format("%Y%m%d-%H%M%S")
    ));
    let file = File::create(&path).map_err(|error| error.to_string())?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    if let Ok(log) = std::fs::read(storage::diagnostics_path()) {
        zip.start_file("diagnostics.log", options)
            .map_err(|error| error.to_string())?;
        zip.write_all(&log).map_err(|error| error.to_string())?;
    }
    if let Some(home) = dirs::home_dir() {
        let crash_directory = home.join("Library/Logs/DiagnosticReports");
        let mut crash_reports = std::fs::read_dir(crash_directory)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                name.contains("aidoo whisper lite") || name.contains("aidoo-whisper-lite")
            })
            .filter_map(|entry| {
                let modified = entry.metadata().ok()?.modified().ok()?;
                Some((modified, entry.path()))
            })
            .collect::<Vec<_>>();
        crash_reports.sort_by(|left, right| right.0.cmp(&left.0));
        for (_, report) in crash_reports.into_iter().take(3) {
            let Some(name) = report.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if let Ok(contents) = std::fs::read(&report) {
                let bounded = &contents[..contents.len().min(1_000_000)];
                let sanitized = storage::redact_openai_keys(&String::from_utf8_lossy(bounded));
                zip.start_file(format!("crash-reports/{name}"), options)
                    .map_err(|error| error.to_string())?;
                zip.write_all(sanitized.as_bytes())
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    let settings = app
        .state::<AppState>()
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")?
        .clone();
    zip.start_file("settings.json", options)
        .map_err(|error| error.to_string())?;
    zip.write_all(&serde_json::to_vec_pretty(&settings).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    zip.start_file("system.txt", options)
        .map_err(|error| error.to_string())?;
    zip.write_all(
        format!(
            "App: {}\nVersion: {}\nOS: {}\nArch: {}\n",
            app.package_info().name,
            app.package_info().version,
            std::env::consts::OS,
            std::env::consts::ARCH
        )
        .as_bytes(),
    )
    .map_err(|error| error.to_string())?;
    zip.finish().map_err(|error| error.to_string())?;
    Ok(path.to_string_lossy().to_string())
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
    let status = if accessibility_granted() {
        "idle"
    } else {
        "permission"
    };
    let menu = build_tray_menu(app.handle(), status)?;
    let initial_tooltip = if uses_english_ui(app.handle()) {
        "AIDOO Whisper Lite — ready"
    } else {
        "AIDOO Whisper Lite — готов"
    };
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(initial_tooltip)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app, false),
            "settings" => show_main_window(app, true),
            "stop" => request_dictation_stop(app),
            "copy-error" => {
                if let Some(error) = app
                    .state::<AppState>()
                    .last_recording_error
                    .lock()
                    .ok()
                    .and_then(|value| value.clone())
                {
                    let _ = text_insertion::copy(&error);
                }
            }
            "quit" => app.exit(0),
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
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .manage(AppState::load())
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            install_tray(app)?;
            if let Some(overlay) = app.get_webview_window("overlay") {
                let _ = overlay.set_ignore_cursor_events(true);
                let _ = overlay.set_shadow(false);
            }
            shortcuts::install(app.handle().clone());
            storage::append_diagnostic("application started");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            update_settings,
            save_api_key,
            delete_api_key,
            begin_shortcut_capture,
            cancel_shortcut_capture,
            test_microphone,
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
            create_diagnostic_bundle
        ])
        .run(tauri::generate_context!())
        .expect("error while running AIDOO Whisper Lite");
}
