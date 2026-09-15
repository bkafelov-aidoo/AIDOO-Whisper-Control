#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

mod audio;
mod models;
mod shortcuts;
mod storage;
mod text_insertion;
mod transcription;

use chrono::{Local, Utc};
use models::{
    AppSettings, BootstrapState, FailedRecording, OverlayBootstrapState, RecordingProgress,
    RecordingSnapshot, TranscriptEntry, TranscriptionCompleted,
};
use std::fs::File;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
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
const MAX_RECORDING_DURATION: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const IN_FLIGHT_RECOVERY_ERROR: &str = "Възстановен е запис след прекъсване. Не може да бъде изпратен повторно автоматично, за да се избегне повторно API таксуване.";
const CHARGED_RECOVERY_ERROR: &str =
    "Този recovery запис вече е транскрибиран. Изберете „Изтрий“, за да не бъде таксуван повторно.";

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecoveryPlan {
    FinishLocally(String),
    Transcribe,
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
        audio::cleanup_stale_temporary_audio();
        let mut history = storage::load_history();
        recover_pending_history_deletion(&mut history);
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
            failed_recording: Mutex::new(failed_recording),
            recorder: audio::RecorderService::new(),
            shortcut_capture: Mutex::new(None),
            recording_status: Mutex::new(if recovery_error.is_some() {
                "error".into()
            } else {
                "idle".into()
            }),
            recording_progress: Mutex::new(RecordingProgress::default()),
            recording_started_at: Mutex::new(None),
            recording_active: AtomicBool::new(false),
            operation_active: AtomicBool::new(false),
            stop_requested: AtomicBool::new(false),
            status_generation: AtomicU64::new(0),
            last_recording_error: Mutex::new(recovery_error),
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
        (true, "recovery") => "Status: action required",
        (true, "setup") => "Status: finish setup",
        (true, "permission") => "Status: permission required",
        (true, _) => "Status: ready for dictation",
        (false, "starting") => "Състояние: стартирам микрофона",
        (false, "recording") => "Състояние: записвам · отпуснете shortcut-а за край",
        (false, "transcribing") => "Състояние: транскрибирам",
        (false, "done") => "Състояние: текстът е готов",
        (false, "error") => "Състояние: грешка",
        (false, "recovery") => "Състояние: нужно е действие",
        (false, "setup") => "Състояние: довършете настройката",
        (false, "permission") => "Състояние: нужно е разрешение",
        (false, _) => "Състояние: готов за диктовка",
    }
}

fn progress_status_label(stage: &str, english: bool) -> Option<&'static str> {
    match (english, stage) {
        (true, "preparing_audio") => Some("Status: preparing audio"),
        (true, "starting_microphone") => Some("Status: starting the microphone"),
        (true, "compressing_audio") => Some("Status: compressing to FLAC"),
        (true, "uploading_audio") => Some("Status: uploading audio"),
        (true, "openai_transcribing") => Some("Status: OpenAI is transcribing"),
        (true, "text_ready") => Some("Status: text is ready"),
        (true, "finishing_locally") => Some("Status: finishing locally"),
        (false, "preparing_audio") => Some("Състояние: подготвям аудиото"),
        (false, "starting_microphone") => Some("Състояние: стартирам микрофона"),
        (false, "compressing_audio") => Some("Състояние: компресирам в FLAC"),
        (false, "uploading_audio") => Some("Състояние: изпращам аудиото"),
        (false, "openai_transcribing") => Some("Състояние: OpenAI транскрибира"),
        (false, "text_ready") => Some("Състояние: текстът е готов"),
        (false, "finishing_locally") => Some("Състояние: завършвам локално"),
        _ => None,
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

fn localized_native_error(error: &str, english: bool) -> String {
    if !english {
        return error.into();
    }
    let exact = match error {
        "Ключът трябва да започва с sk-." => Some("The key must start with sk-."),
        "Не е намерен микрофон." => Some("No microphone was found."),
        "Не е намерен микрофон. Свържете устройство и опитайте отново." => {
            Some("No microphone was found. Connect an input device and try again.")
        }
        "Вече има активен запис." => Some("A recording is already active."),
        "Транскрипцията вече е стартирана." => Some("Transcription has already started."),
        "Няма активен запис." => Some("There is no active recording."),
        "Не беше разпозната реч." => Some("No speech was detected."),
        "Няма неуспешен запис за повторен опит." => {
            Some("There is no failed recording to retry.")
        }
        "Запазеният неуспешен аудио файл не е намерен." => {
            Some("The saved failed audio recording could not be found.")
        }
        "Текстът не можа да бъде поставен. Копиран е в клипборда." => {
            Some("The text could not be pasted. It remains copied to the clipboard.")
        }
        "Аудио файлът е по-голям от лимита на OpenAI от 25 MB. Направете по-кратък запис." => {
            Some("The audio file exceeds OpenAI's 25 MB limit. Make a shorter recording.")
        }
        "Достигнат е максималният запис от 5 минути. Спирам и транскрибирам." => {
            Some("The 5-minute recording limit was reached. Stopping and transcribing.")
        }
        "Има запазен неуспешен запис. Изберете „Опитай отново“ или „Изтрий“, преди да започнете нова диктовка." => {
            Some("A failed recording is saved. Choose “Try again” or “Delete” before starting a new dictation.")
        }
        "Има запазен запис за възстановяване. Завършете го или го изтрийте, преди да започнете нова диктовка." => {
            Some("A recovery item is saved. Finish or delete it before starting a new dictation.")
        }
        "Има запазен запис за възстановяване. Завършете го или го изтрийте, преди да започнете нова транскрипция." => {
            Some("A recovery item is saved. Finish or delete it before starting another transcription.")
        }
        "Транскрипцията е готова, но старият recovery запис не можа да бъде изчистен. Изберете „Изтрий“; нов опит може да доведе до повторно API таксуване." => {
            Some("The transcription succeeded, but the old recovery item could not be cleared. Choose Delete; another retry may create another API charge.")
        }
        "Възстановен е неуспешен запис след прекъсване. Можете да опитате отново." => {
            Some("A failed recording was recovered after an interruption. You can try again.")
        }
        "Възстановен е запис след прекъсване. Не може да бъде изпратен повторно автоматично, за да се избегне повторно API таксуване." => {
            Some("A recording was recovered after an interruption. It cannot be sent again automatically because that could create another API charge.")
        }
        "Завършете или отменете избора на shortcut, преди да започнете диктовка." => {
            Some("Finish or cancel shortcut selection before starting dictation.")
        }
        "Изчакайте текущата операция да приключи." => {
            Some("Wait for the current operation to finish.")
        }
        "Записът е прекалено кратък. Задръжте shortcut-а и говорете поне половин секунда." => {
            Some("The recording is too short. Hold the shortcut and speak for at least half a second.")
        }
        "Записът е прекалено кратък. Задръжте клавиша и говорете." => {
            Some("The recording is too short. Hold the key and speak.")
        }
        "Завършете началната настройка, преди да използвате диктовката." => {
            Some("Finish the initial setup before using dictation.")
        }
        "Разрешете Accessibility, за да работят shortcut-ът и автоматичното поставяне." => {
            Some("Grant Accessibility permission so the shortcut and automatic paste can work.")
        }
        "Няма достъпен OpenAI API ключ. Отворете настройките и го добавете." => {
            Some("No OpenAI API key is available. Open Settings and add one.")
        }
        "Добавете и проверете OpenAI API ключ." => {
            Some("Add and verify an OpenAI API key.")
        }
        "Настройките са заключени." => {
            Some("Settings are temporarily unavailable. Try again.")
        }
        "Историята е заключена." => {
            Some("History is temporarily unavailable. Try again.")
        }
        "Recovery състоянието е заключено." => {
            Some("Recovery is temporarily unavailable. Try again.")
        }
        "Recovery аудио файлът не е валиден." => Some("The recovery audio file is invalid."),
        "Аудио услугата не работи." => {
            Some("The audio service is unavailable. Restart the app and try again.")
        }
        "Аудио услугата не отговори." => {
            Some("The audio service did not respond. Restart the app and try again.")
        }
        "Аудио файлът е заключен." => {
            Some("The audio file is temporarily unavailable. Try again.")
        }
        "Липсва Accessibility разрешение за автоматично поставяне. Натиснете „Разреши Accessibility“ в Aidoo; разпознатият текст е запазен в Историята и clipboard." => {
            Some("Accessibility permission for automatic paste is missing. Grant Accessibility permission in AIDOO; the recognized text remains in History and the clipboard.")
        }
        _ => None,
    };
    if let Some(translated) = exact {
        return translated.into();
    }
    let prefixes = [
        ("Няма връзка с OpenAI:", "Could not connect to OpenAI:"),
        (
            "OpenAI връзката не можа да бъде подготвена:",
            "The OpenAI connection could not be prepared:",
        ),
        ("API ключът не беше приет:", "The API key was not accepted:"),
        ("Транскрипцията не успя:", "Transcription failed:"),
        (
            "OpenAI върна невалиден отговор:",
            "OpenAI returned an invalid response:",
        ),
        ("Не е намерен микрофон.", "No microphone was found."),
        (
            "Нито един микрофон не можа да стартира.",
            "No microphone could be started.",
        ),
        (
            "Избраният микрофон не е наличен. Използвам",
            "The selected microphone is unavailable. Using",
        ),
        ("Микрофонът", "Microphone"),
        (
            "Папката не може да бъде създадена:",
            "The folder could not be created:",
        ),
        (
            "Папката не може да бъде използвана:",
            "The folder could not be used:",
        ),
        (
            "Частната папка на приложението не е валидна:",
            "The app's private data folder is invalid:",
        ),
        ("Неподдържан аудио формат:", "Unsupported audio format:"),
        (
            "Записът не можа да се запише на диска:",
            "The recording could not be written to disk:",
        ),
        ("Невалиден WAV файл:", "Invalid WAV file:"),
        (
            "FLAC поддържа mono/stereo, а записът има",
            "FLAC supports mono/stereo, but the recording has",
        ),
        (
            "FLAC процесът беше прекъснат:",
            "The FLAC process was interrupted:",
        ),
        (
            "FLAC файлът не може да бъде запазен:",
            "The FLAC file could not be saved:",
        ),
        (
            "TXT файлът не може да бъде запазен:",
            "The TXT file could not be saved:",
        ),
        (
            "Историята не можа да бъде запазена:",
            "History could not be saved:",
        ),
        (
            "Recovery аудиото не можа да бъде изтрито:",
            "The recovery audio could not be deleted:",
        ),
        (
            "Recovery състоянието не можа да бъде запазено:",
            "The recovery state could not be saved:",
        ),
        (
            "Неуспешният запис не можа да бъде запазен:",
            "The failed recording could not be retained:",
        ),
        (
            "Завършеният запис не можа да бъде запазен:",
            "The completed recording could not be retained:",
        ),
        (
            "Recovery копието не можа да бъде запазено:",
            "The recovery copy could not be retained:",
        ),
        (
            "Recovery аудио файлът не може да бъде защитен:",
            "The recovery audio file could not be protected:",
        ),
        (
            "Recovery защитата не можа да бъде обновена:",
            "The recovery retry protection could not be updated:",
        ),
        (
            "Текстът е готов, но клипбордът не е достъпен:",
            "The text is ready, but the clipboard is unavailable:",
        ),
        (
            "Транскрипцията е завършена и текстът остава в клипборда, но",
            "The transcription is complete and the text remains in the clipboard, but",
        ),
    ];
    let mut translated = error.to_string();
    for (source, target) in prefixes {
        if let Some(remainder) = error.strip_prefix(source) {
            translated = format!("{target}{remainder}");
            break;
        }
    }
    translated
        .replace(
            "невалиден или изтрит API ключ.",
            "invalid or deleted API key.",
        )
        .replace(
            "няма наличен API баланс или е достигнат лимитът.",
            "no API balance is available or the limit has been reached.",
        )
        .replace("не е наличен.", "is unavailable.")
        .replace("канала.", "channels.")
        .replace(
            "папката не може да бъде създадена:",
            "the folder could not be created:",
        )
        .replace(
            "FLAC файлът не може да бъде запазен:",
            "the FLAC file could not be saved:",
        )
        .replace(
            "TXT файлът не може да бъде запазен:",
            "the TXT file could not be saved:",
        )
        .replace(
            "историята е временно недостъпна.",
            "history is temporarily unavailable.",
        )
        .replace(
            "историята не можа да бъде запазена:",
            "history could not be saved:",
        )
        .replace(
            "Създадените локални файлове не са изтрити.",
            "Created local files were not deleted.",
        )
        .replace(
            "отговорът надвишава безопасния лимит.",
            "the response exceeds the safe limit.",
        )
        .replace(
            "Recovery аудио файлът не може да бъде защитен:",
            "The recovery audio file could not be protected:",
        )
        .replace(
            "Recovery защитата не можа да бъде обновена:",
            "The recovery retry protection could not be updated:",
        )
}

fn build_tray_menu(app: &AppHandle, current: &str) -> tauri::Result<Menu<tauri::Wry>> {
    let english = uses_english_ui(app);
    let progress_stage = app
        .state::<AppState>()
        .recording_progress
        .lock()
        .map(|progress| progress.stage.clone())
        .unwrap_or_default();
    let status_text = if matches!(current, "starting" | "transcribing") {
        progress_status_label(&progress_stage, english)
            .unwrap_or_else(|| status_label(current, english))
    } else {
        status_label(current, english)
    };
    let operation_active = app
        .state::<AppState>()
        .operation_active
        .load(Ordering::Acquire)
        || matches!(current, "starting" | "recording" | "transcribing");
    let status = MenuItem::with_id(app, "status", status_text, false, None::<&str>)?;
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
        !operation_active,
        None::<&str>,
    )?;
    let error = app
        .state::<AppState>()
        .last_recording_error
        .lock()
        .ok()
        .and_then(|value| value.clone());
    if let Some(error) = error {
        let error = localized_native_error(&error, english);
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
        let tooltip = tray_tooltip(current, english);
        let _ = tray.set_tooltip(Some(tooltip));
        if let Ok(menu) = build_tray_menu(app, current) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn tray_tooltip(current: &str, english: bool) -> &'static str {
    match (english, current) {
        (true, "starting") => "AIDOO Whisper Lite — starting microphone",
        (true, "recording") => "AIDOO Whisper Lite — recording",
        (true, "transcribing") => "AIDOO Whisper Lite — transcribing",
        (true, "done") => "AIDOO Whisper Lite — transcription ready",
        (true, "error") => "AIDOO Whisper Lite — error",
        (true, "recovery") => "AIDOO Whisper Lite — action required",
        (true, "setup") => "AIDOO Whisper Lite — finish setup",
        (true, "permission") => "AIDOO Whisper Lite — permission required",
        (true, _) => "AIDOO Whisper Lite — ready",
        (false, "starting") => "AIDOO Whisper Lite — стартирам микрофона",
        (false, "recording") => "AIDOO Whisper Lite — записвам",
        (false, "transcribing") => "AIDOO Whisper Lite — транскрибирам",
        (false, "done") => "AIDOO Whisper Lite — транскрипцията е готова",
        (false, "error") => "AIDOO Whisper Lite — грешка",
        (false, "recovery") => "AIDOO Whisper Lite — нужно е действие",
        (false, "setup") => "AIDOO Whisper Lite — довършете настройката",
        (false, "permission") => "AIDOO Whisper Lite — нужно е разрешение",
        (false, _) => "AIDOO Whisper Lite — готов",
    }
}

fn refresh_tray_menu(app: &AppHandle) {
    let granted = accessibility_granted();
    let state = app.state::<AppState>();
    let current = state
        .recording_status
        .lock()
        .map(|value| value.clone())
        .unwrap_or_else(|_| "idle".into());
    let has_recovery = state
        .failed_recording
        .lock()
        .map(|recording| recording.is_some())
        .unwrap_or(true);
    let setup_ready = if matches!(current.as_str(), "idle") {
        tray_setup_ready(&state)
    } else {
        true
    };
    let tray_state = resolved_tray_state(&current, granted, setup_ready, has_recovery);
    update_tray_menu(app, tray_state);
}

fn tray_setup_ready(state: &AppState) -> bool {
    let onboarding_complete = state
        .settings
        .lock()
        .map(|settings| settings.onboarding_complete)
        .unwrap_or(false);
    let has_api_key = state
        .api_key
        .lock()
        .map(|api_key| api_key.is_some())
        .unwrap_or(false);
    onboarding_complete && has_api_key && !audio::microphone_names().is_empty()
}

fn resolved_tray_state(
    current: &str,
    accessibility_granted: bool,
    setup_ready: bool,
    has_recovery: bool,
) -> &str {
    if matches!(current, "starting" | "recording" | "transcribing") {
        current
    } else if has_recovery {
        "recovery"
    } else if matches!(current, "done" | "error") {
        current
    } else if !setup_ready {
        "setup"
    } else if accessibility_granted {
        "idle"
    } else {
        "permission"
    }
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
    refresh_tray_menu(app);
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
    let stage_changed = if let Ok(mut current) = app.state::<AppState>().recording_progress.lock() {
        let changed = current.stage != progress.stage;
        *current = progress.clone();
        changed
    } else {
        false
    };
    let _ = app.emit("recording:progress", &progress);
    emit_snapshot(app);
    if stage_changed {
        refresh_tray_menu(app);
    }
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
    let work_area = monitor.work_area();
    let x = work_area.position.x + (work_area.size.width.saturating_sub(size.width) / 2) as i32;
    let y = work_area.position.y + work_area.size.height.saturating_sub(size.height + 18) as i32;
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

fn ensure_output_directory_writable(directory: &Path) -> Result<(), String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("Папката не може да бъде използвана: {error}"))?;
    let probe = directory.join(format!(
        ".aidoo-whisper-write-test-{}",
        uuid::Uuid::new_v4()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(&probe)
        .map_err(|error| format!("Папката не може да бъде използвана: {error}"))?;
    drop(file);
    std::fs::remove_file(&probe)
        .map_err(|error| format!("Папката не може да бъде използвана: {error}"))
}

fn api_key_from_state(state: &AppState) -> Result<Zeroizing<String>, String> {
    if let Ok(cache) = state.api_key.lock() {
        if let Some(value) = cache.as_ref() {
            return Ok(value.clone());
        }
    }
    let password = Zeroizing::new(keyring_entry()?.get_password().map_err(|_| {
        "Няма достъпен OpenAI API ключ. Отворете настройките и го добавете.".to_string()
    })?);
    if let Ok(mut cache) = state.api_key.lock() {
        *cache = Some(password.clone());
    }
    Ok(password)
}

fn ready_dictation_settings(state: &AppState) -> Result<AppSettings, String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")?
        .clone();
    if !settings.onboarding_complete {
        return Err("Завършете началната настройка, преди да използвате диктовката.".into());
    }
    if api_key_from_state(state).is_err() {
        return Err("Добавете и проверете OpenAI API ключ.".into());
    }
    if audio::microphone_names().is_empty() {
        return Err("Не е намерен микрофон.".into());
    }
    if !accessibility_granted() {
        return Err(
            "Разрешете Accessibility, за да работят shortcut-ът и автоматичното поставяне.".into(),
        );
    }
    if state
        .shortcut_capture
        .lock()
        .map(|capture| capture.is_some())
        .unwrap_or(true)
    {
        return Err(
            "Завършете или отменете избора на shortcut, преди да започнете диктовка.".into(),
        );
    }
    if state
        .failed_recording
        .lock()
        .map(|recording| recording.is_some())
        .unwrap_or(true)
    {
        return Err(
            "Има запазен запис за възстановяване. Завършете го или го изтрийте, преди да започнете нова диктовка."
                .into(),
        );
    }
    Ok(settings)
}

struct OperationGuard<'a> {
    active: &'a AtomicBool,
    app: AppHandle,
    release_on_drop: bool,
}

impl OperationGuard<'_> {
    fn disarm(mut self) {
        self.release_on_drop = false;
    }
}

impl Drop for OperationGuard<'_> {
    fn drop(&mut self) {
        if self.release_on_drop {
            self.active.store(false, Ordering::Release);
            refresh_tray_menu(&self.app);
        }
    }
}

fn acquire_operation<'a>(
    app: &AppHandle,
    state: &'a AppState,
) -> Result<OperationGuard<'a>, String> {
    state
        .operation_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| {
            refresh_tray_menu(app);
            OperationGuard {
                active: &state.operation_active,
                app: app.clone(),
                release_on_drop: true,
            }
        })
        .map_err(|_| "Изчакайте текущата операция да приключи.".into())
}

fn release_active_operation<'a>(app: &AppHandle, state: &'a AppState) -> OperationGuard<'a> {
    OperationGuard {
        active: &state.operation_active,
        app: app.clone(),
        release_on_drop: true,
    }
}

pub(crate) fn release_shortcut_capture_operation(app: &AppHandle) {
    app.state::<AppState>()
        .operation_active
        .store(false, Ordering::Release);
    refresh_tray_menu(app);
}

fn start_recording_inner(app: &AppHandle) -> Result<audio::AudioStartInfo, String> {
    let state = app.state::<AppState>();
    let operation = acquire_operation(app, &state)?;
    let settings = ready_dictation_settings(&state)?;
    if state.recording_active.swap(true, Ordering::AcqRel) {
        return Err("Вече има активен запис.".into());
    }
    state.stop_requested.store(false, Ordering::Release);
    if let Ok(mut error) = state.last_recording_error.lock() {
        *error = None;
    }
    set_progress(app, 0, "starting_microphone", false);
    set_recording_state(app, "starting");
    let routing = audio::MicrophoneRoutingConfig {
        preferred_name: settings.microphone_name,
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
            if !state.stop_requested.load(Ordering::Acquire) {
                set_recording_state(app, "recording");
                let recording_generation = state.status_generation.load(Ordering::Acquire);
                let timeout_app = app.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(MAX_RECORDING_DURATION).await;
                    let timeout_state = timeout_app.state::<AppState>();
                    if recording_watchdog_should_stop(
                        timeout_state.recording_active.load(Ordering::Acquire),
                        timeout_state.status_generation.load(Ordering::Acquire),
                        recording_generation,
                    ) {
                        let _ = timeout_app.emit(
                            "toast",
                            "Достигнат е максималният запис от 5 минути. Спирам и транскрибирам.",
                        );
                        request_dictation_stop(&timeout_app);
                    }
                });
            }
            operation.disarm();
            Ok(info)
        }
        Err(error) => {
            state.recording_active.store(false, Ordering::Release);
            Err(error)
        }
    }
}

fn recording_watchdog_should_stop(
    recording_active: bool,
    current_generation: u64,
    scheduled_generation: u64,
) -> bool {
    recording_active && current_generation == scheduled_generation
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
    set_progress(app, 1, "preparing_audio", false);
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
    if !app
        .state::<AppState>()
        .recording_active
        .load(Ordering::Acquire)
    {
        return Err("Няма активен запис.".into());
    }
    if app
        .state::<AppState>()
        .stop_requested
        .swap(true, Ordering::AcqRel)
    {
        return Err("Транскрипцията вече е стартирана.".into());
    }
    set_progress(&app, 1, "preparing_audio", false);
    set_recording_state(&app, "transcribing");
    stop_and_transcribe_inner(&app)
        .await
        .inspect_err(|error| set_error(&app, error))
}

async fn stop_and_transcribe_inner(app: &AppHandle) -> Result<TranscriptionCompleted, String> {
    let state = app.state::<AppState>();
    let _operation = release_active_operation(app, &state);
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
    let settings = match state.settings.lock() {
        Ok(settings) => settings.clone(),
        Err(_) => {
            let error = "Настройките са заключени.".to_string();
            retain_captured_failure(app, &state, &captured, &error)?;
            return Err(error);
        }
    };
    let api_key = match api_key_from_state(&state) {
        Ok(api_key) => api_key,
        Err(error) => {
            retain_captured_failure(app, &state, &captured, &error)?;
            return Err(error);
        }
    };
    set_progress(app, 5, "compressing_audio", false);
    let staged = match prepare_flac(&captured.path).await {
        Ok(path) => path,
        Err(error) => {
            let failed = retain_failed_recording(
                &captured.path,
                captured.duration_seconds,
                &error,
                true,
                None,
            )?;
            store_failed_recording(app, &state, failed)?;
            return Err(error);
        }
    };
    // Persist a fail-safe Recovery item before the request can reach OpenAI. If the process
    // exits at any point after this, startup must assume that the request may have been charged.
    let pending = retain_failed_recording(
        &staged,
        captured.duration_seconds,
        IN_FLIGHT_RECOVERY_ERROR,
        false,
        None,
    )?;
    let request_audio = PathBuf::from(&pending.path);
    store_failed_recording(app, &state, pending)?;
    let _ = std::fs::remove_file(&captured.path);
    let app_for_progress = app.clone();
    let callback: transcription::ProgressCallback = Arc::new(move |percent, stage, determinate| {
        set_progress(&app_for_progress, percent.max(10), stage, determinate);
    });
    let result =
        transcription::transcribe(&request_audio, &api_key, &settings, Some(callback)).await;
    match result {
        Ok(text) => {
            let completed_text = text.clone();
            let completed = match finalize_success(
                app,
                &settings,
                &request_audio,
                captured.duration_seconds,
                text,
            ) {
                Ok(completed) => completed,
                Err(error) => {
                    preserve_completed_recovery(&state, &error.message, completed_text);
                    emit_current_failed_recording(app, &state);
                    return Err(error.message);
                }
            };
            Ok(publish_recovery_completion(
                app,
                &state,
                &request_audio,
                false,
                completed,
            ))
        }
        Err(failure) => {
            set_failed_recording_retryability(app, &state, failure.retryable, &failure.message)?;
            Err(failure.message)
        }
    }
}

async fn prepare_flac(wav: &Path) -> Result<PathBuf, String> {
    let wav = wav.to_path_buf();
    let target = std::env::temp_dir().join(format!("aidoo-lite-{}.flac", uuid::Uuid::new_v4()));
    let target_for_task = target.clone();
    let result = tokio::task::spawn_blocking(move || {
        transcription::encode_wav_to_flac(&wav, &target_for_task)
    })
    .await
    .map_err(|error| format!("FLAC процесът беше прекъснат: {error}"))
    .and_then(|result| result);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&target);
        return Err(error);
    }
    Ok(target)
}

fn safe_file_stem() -> String {
    format!(
        "AIDOO-Whisper-{}-{}",
        Local::now().format("%Y-%m-%d_%H-%M-%S"),
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    )
}

#[derive(Debug)]
struct FinalizationError {
    message: String,
}

impl FinalizationError {
    fn new(message: String) -> Self {
        Self { message }
    }

    fn completed(message: String, files_created: bool) -> Self {
        let preservation = if files_created {
            " Създадените локални файлове не са изтрити."
        } else {
            ""
        };
        Self {
            message: format!(
                "Транскрипцията е завършена и текстът остава в клипборда, но {message}{preservation}"
            ),
        }
    }
}

fn save_local_transcription_files(
    settings: &AppSettings,
    staged_audio: &Path,
    text: &str,
) -> Result<(Option<PathBuf>, Option<PathBuf>), FinalizationError> {
    let output_dir = selected_output_dir(settings);
    if settings.save_audio || settings.save_text {
        std::fs::create_dir_all(&output_dir).map_err(|error| {
            FinalizationError::completed(
                format!("папката не може да бъде създадена: {error}"),
                false,
            )
        })?;
    }
    let stem = safe_file_stem();
    let audio_path = if settings.save_audio {
        let path = output_dir.join(format!("{stem}.flac"));
        copy_output_atomic(staged_audio, &path).map_err(|error| {
            FinalizationError::completed(
                format!("FLAC файлът не може да бъде запазен: {error}"),
                false,
            )
        })?;
        Some(path)
    } else {
        None
    };
    let text_path = if settings.save_text {
        let path = output_dir.join(format!("{stem}.txt"));
        if let Err(error) = write_output_atomic(&path, format!("{text}\n").as_bytes()) {
            return Err(FinalizationError::completed(
                format!("TXT файлът не може да бъде запазен: {error}"),
                audio_path.is_some(),
            ));
        }
        Some(path)
    } else {
        None
    };
    Ok((audio_path, text_path))
}

fn finalize_success(
    app: &AppHandle,
    settings: &AppSettings,
    staged_audio: &Path,
    duration_seconds: f64,
    text: String,
) -> Result<TranscriptionCompleted, FinalizationError> {
    text_insertion::copy(&text).map_err(|error| {
        FinalizationError::new(format!(
            "Текстът е готов, но клипбордът не е достъпен: {error}"
        ))
    })?;

    let (audio_path, text_path) = save_local_transcription_files(settings, staged_audio, &text)?;

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
                return Err(FinalizationError::completed(
                    "историята е временно недостъпна.".into(),
                    audio_path.is_some() || text_path.is_some(),
                ));
            }
        };
        let mut next_history = history.clone();
        next_history.insert(0, entry.clone());
        next_history.truncate(10);
        if let Err(error) = storage::save_history(&next_history) {
            return Err(FinalizationError::completed(
                format!("историята не можа да бъде запазена: {error}"),
                audio_path.is_some() || text_path.is_some(),
            ));
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
                if let Ok(mut current) = app.state::<AppState>().last_recording_error.lock() {
                    *current = Some(message.clone());
                }
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

fn temporary_output_path(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("aidoo-output");
    target.with_file_name(format!(".{name}.tmp-{}", uuid::Uuid::new_v4()))
}

fn copy_output_atomic(source: &Path, target: &Path) -> std::io::Result<()> {
    let temporary = temporary_output_path(target);
    let result = (|| {
        let mut source = std::fs::File::open(source)?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary)?;
        std::io::copy(&mut source, &mut file)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn write_output_atomic(target: &Path, contents: &[u8]) -> std::io::Result<()> {
    let temporary = temporary_output_path(target);
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn retain_failed_recording(
    path: &Path,
    duration_seconds: f64,
    error: &str,
    retryable: bool,
    completed_text: Option<String>,
) -> Result<FailedRecording, String> {
    storage::ensure_directories()?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("flac");
    // A completed OpenAI response always gets the fail-safe on-disk marker. If metadata is
    // lost, startup must disable retry rather than risk sending already charged audio again.
    let retry_status = if completed_text.is_some() {
        "nonretryable"
    } else if retryable {
        "retryable"
    } else {
        "nonretryable"
    };
    let target = storage::recovery_dir().join(format!(
        "failed-dictation-{retry_status}-{}.{extension}",
        uuid::Uuid::new_v4()
    ));
    if std::fs::rename(path, &target).is_err() {
        copy_output_atomic(path, &target)
            .map_err(|error| format!("Неуспешният запис не можа да бъде запазен: {error}"))?;
        if let Err(error) = std::fs::remove_file(path) {
            storage::append_diagnostic(&format!(
                "temporary recording cleanup failed after recovery copy: {error}"
            ));
        }
    }
    #[cfg(unix)]
    if let Err(error) = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)) {
        storage::append_diagnostic(&format!("recovery permission update failed: {error}"));
    }
    Ok(FailedRecording {
        path: target.to_string_lossy().to_string(),
        created_at: Utc::now().to_rfc3339(),
        duration_seconds,
        error: error.into(),
        retryable,
        completed_text,
    })
}

fn retain_history_recording_copy(
    source: &Path,
    duration_seconds: f64,
    error: &str,
    retryable: bool,
    completed_text: Option<String>,
) -> Result<FailedRecording, String> {
    storage::ensure_directories()?;
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("flac");
    let retry_status = if completed_text.is_some() || !retryable {
        "nonretryable"
    } else {
        "retryable"
    };
    let target = storage::recovery_dir().join(format!(
        "failed-dictation-{retry_status}-{}.{extension}",
        uuid::Uuid::new_v4()
    ));
    copy_output_atomic(source, &target)
        .map_err(|error| format!("Recovery копието не можа да бъде запазено: {error}"))?;
    #[cfg(unix)]
    if let Err(error) = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)) {
        storage::append_diagnostic(&format!(
            "history recovery permission update failed: {error}"
        ));
    }
    Ok(FailedRecording {
        path: target.to_string_lossy().to_string(),
        created_at: Utc::now().to_rfc3339(),
        duration_seconds,
        error: error.into(),
        retryable,
        completed_text,
    })
}

fn retain_captured_failure(
    app: &AppHandle,
    state: &AppState,
    captured: &audio::CapturedAudio,
    error: &str,
) -> Result<(), String> {
    let failed =
        retain_failed_recording(&captured.path, captured.duration_seconds, error, true, None)?;
    store_failed_recording(app, state, failed)
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
    *current = Some(failed.clone());
    drop(current);
    let _ = app.emit("failed-recording:changed", &failed);
    refresh_tray_menu(app);
    storage::save_failed_recording(&failed)
        .map_err(|error| format!("Recovery състоянието не можа да бъде запазено: {error}"))
}

fn clear_failed_recording_state(state: &AppState, remove_audio: bool) -> Result<(), String> {
    let mut current = state
        .failed_recording
        .lock()
        .map_err(|_| "Recovery състоянието е заключено.")?;
    let previous = current.clone();
    let staged_deletion = if remove_audio {
        previous
            .as_ref()
            .map(|recording| PathBuf::from(&recording.path))
            .filter(|path| path.exists())
            .map(|path| {
                let file_name = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("recovery-audio");
                let staged =
                    path.with_file_name(format!(".{file_name}.deleting-{}", uuid::Uuid::new_v4()));
                std::fs::rename(&path, &staged)
                    .map(|_| (path, staged))
                    .map_err(|error| format!("Recovery аудиото не можа да бъде изтрито: {error}"))
            })
            .transpose()?
    } else {
        None
    };
    if let Err(error) = storage::clear_failed_recording() {
        if let Some((original, staged)) = staged_deletion.as_ref() {
            let _ = std::fs::rename(staged, original);
        }
        return Err(error);
    }
    *current = None;
    if let Some((_, staged)) = staged_deletion {
        if let Err(error) = std::fs::remove_file(staged) {
            storage::append_diagnostic(&format!("recovery staged cleanup failed: {error}"));
        }
    }
    Ok(())
}

fn resolve_failed_recording_after_success(state: &AppState) -> Result<(), String> {
    resolve_failed_recording_after_success_with(&state.failed_recording, |path| {
        std::fs::remove_file(path)
    })
}

fn resolve_failed_recording_after_success_with(
    failed_recording: &Mutex<Option<FailedRecording>>,
    remove_file: impl Fn(&Path) -> std::io::Result<()>,
) -> Result<(), String> {
    let mut current = failed_recording
        .lock()
        .map_err(|_| "Recovery състоянието е заключено.")?;
    let Some(mut recording) = current.clone() else {
        storage::clear_failed_recording()?;
        return Ok(());
    };

    // Persist the charged/non-retryable state before deleting anything. A crash or cleanup
    // failure must never make the same audio look safe for another OpenAI request.
    mark_recovery_file_non_retryable(&mut recording);
    recording.retryable = false;
    recording.completed_text = None;
    *current = Some(recording.clone());
    storage::save_failed_recording(&recording)?;

    let path = PathBuf::from(&recording.path);
    match remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            let message = format!("Recovery аудиото не можа да бъде изтрито: {error}");
            recording.error = message.clone();
            *current = Some(recording.clone());
            if let Err(metadata_error) = storage::save_failed_recording(&recording) {
                storage::append_diagnostic(&format!(
                    "resolved recovery metadata restore failed: {metadata_error}"
                ));
            }
            return Err(message);
        }
    }
    storage::clear_failed_recording()?;
    *current = None;
    Ok(())
}

#[tauri::command]
async fn retry_failed_transcription(app: AppHandle) -> Result<TranscriptionCompleted, String> {
    let state = app.state::<AppState>();
    let _operation = acquire_operation(&app, &state)?;
    let failed = state
        .failed_recording
        .lock()
        .map_err(|_| "Recovery състоянието е заключено.")?
        .clone()
        .ok_or_else(|| "Няма неуспешен запис за повторен опит.".to_string())?;
    let plan = recovery_plan(&failed)?;
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")?
        .clone();
    let source = PathBuf::from(&failed.path);
    if !is_regular_file_with_extension(&source, "flac")
        && !is_regular_file_with_extension(&source, "wav")
    {
        return Err("Запазеният неуспешен аудио файл не е намерен.".into());
    }
    if let Ok(mut error) = state.last_recording_error.lock() {
        *error = None;
    }
    set_progress(&app, 1, "preparing_audio", false);
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
    let key = match plan {
        RecoveryPlan::FinishLocally(text) => {
            set_progress(&app, 100, "finishing_locally", true);
            return match finalize_success(&app, &settings, &staged, failed.duration_seconds, text) {
                Ok(completed) => Ok(publish_recovery_completion(
                    &app,
                    &state,
                    &staged,
                    temporary_flac,
                    completed,
                )),
                Err(error) => {
                    if temporary_flac {
                        let _ = std::fs::remove_file(&staged);
                    }
                    update_failed_recording_error(&state, &error.message);
                    emit_current_failed_recording(&app, &state);
                    set_error(&app, &error.message);
                    Err(error.message)
                }
            };
        }
        RecoveryPlan::Transcribe => api_key_from_state(&state)?,
    };
    let request_source =
        match set_failed_recording_retryability(&app, &state, false, IN_FLIGHT_RECOVERY_ERROR) {
            Ok(path) => path,
            Err(error) => {
                if temporary_flac {
                    let _ = std::fs::remove_file(&staged);
                }
                set_error(&app, &error);
                return Err(error);
            }
        };
    let staged = if temporary_flac {
        staged
    } else {
        request_source
    };
    let app_for_progress = app.clone();
    let callback: transcription::ProgressCallback = Arc::new(move |percent, stage, determinate| {
        set_progress(&app_for_progress, percent, stage, determinate)
    });
    match transcription::transcribe(&staged, &key, &settings, Some(callback)).await {
        Ok(text) => {
            let completed_text = text.clone();
            let completed =
                match finalize_success(&app, &settings, &staged, failed.duration_seconds, text) {
                    Ok(completed) => completed,
                    Err(error) => {
                        if temporary_flac {
                            let _ = std::fs::remove_file(&staged);
                        }
                        preserve_completed_recovery(&state, &error.message, completed_text);
                        emit_current_failed_recording(&app, &state);
                        set_error(&app, &error.message);
                        return Err(error.message);
                    }
                };
            Ok(publish_recovery_completion(
                &app,
                &state,
                &staged,
                temporary_flac,
                completed,
            ))
        }
        Err(failure) => {
            if temporary_flac {
                let _ = std::fs::remove_file(&staged);
            }
            let message = match set_failed_recording_retryability(
                &app,
                &state,
                failure.retryable,
                &failure.message,
            ) {
                Ok(_) => failure.message,
                Err(recovery_error) => format!("{} {recovery_error}", failure.message),
            };
            set_error(&app, &message);
            Err(message)
        }
    }
}

fn emit_current_failed_recording(app: &AppHandle, state: &AppState) {
    if let Ok(current) = state.failed_recording.lock() {
        if let Some(failed) = current.as_ref() {
            let _ = app.emit("failed-recording:changed", failed);
        }
    }
}

fn publish_recovery_completion(
    app: &AppHandle,
    state: &AppState,
    staged: &Path,
    temporary_flac: bool,
    completed: TranscriptionCompleted,
) -> TranscriptionCompleted {
    let recovery_cleared = match resolve_failed_recording_after_success(state) {
        Ok(()) => true,
        Err(error) => {
            let error_message = "Транскрипцията е готова, но старият recovery запис не можа да бъде изчистен. Изберете „Изтрий“; нов опит може да доведе до повторно API таксуване.";
            let toast_message = if uses_english_ui(app) {
                "The transcription succeeded, but the old recovery item could not be cleared. Choose Delete; another retry may create another API charge."
            } else {
                error_message
            };
            mark_failed_recording_non_retryable(state, error_message);
            if let Ok(mut current) = state.last_recording_error.lock() {
                *current = Some(error_message.into());
            }
            let _ = app.emit("toast", toast_message);
            storage::append_diagnostic(&format!("recovery cleanup failed: {error}"));
            false
        }
    };
    if temporary_flac {
        let _ = std::fs::remove_file(staged);
    }
    if recovery_cleared && completed.paste_error.is_none() {
        if let Ok(mut error) = state.last_recording_error.lock() {
            *error = None;
        }
    }
    set_recording_state(app, "done");
    if recovery_cleared {
        let _ = app.emit("failed-recording:changed", Option::<FailedRecording>::None);
    }
    let _ = app.emit("transcription:completed", &completed);
    completed
}

fn update_failed_recording_error(state: &AppState, error: &str) {
    if let Ok(mut current) = state.failed_recording.lock() {
        if let Some(value) = current.as_mut() {
            value.error = error.into();
            if let Err(error) = storage::save_failed_recording(value) {
                storage::append_diagnostic(&format!(
                    "recovery metadata error update failed: {error}"
                ));
            }
        }
    }
}

fn set_failed_recording_retryability(
    app: &AppHandle,
    state: &AppState,
    retryable: bool,
    error: &str,
) -> Result<PathBuf, String> {
    let failed = {
        let mut current = state
            .failed_recording
            .lock()
            .map_err(|_| "Recovery състоянието е заключено.")?;
        let failed = current
            .as_mut()
            .ok_or_else(|| "Няма неуспешен запис за повторен опит.".to_string())?;
        rename_recovery_file_for_retryability(failed, retryable)?;
        failed.retryable = retryable;
        failed.completed_text = None;
        failed.error = error.into();
        storage::save_failed_recording(failed).map_err(|storage_error| {
            format!("Recovery състоянието не можа да бъде запазено: {storage_error}")
        })?;
        failed.clone()
    };
    let path = PathBuf::from(&failed.path);
    let _ = app.emit("failed-recording:changed", &failed);
    refresh_tray_menu(app);
    Ok(path)
}

fn preserve_completed_recovery(state: &AppState, error: &str, completed_text: String) {
    preserve_completed_recovery_with(&state.failed_recording, error, completed_text);
}

fn preserve_completed_recovery_with(
    failed_recording: &Mutex<Option<FailedRecording>>,
    error: &str,
    completed_text: String,
) {
    if let Ok(mut current) = failed_recording.lock() {
        if let Some(value) = current.as_mut() {
            mark_recovery_file_non_retryable(value);
            value.error = error.into();
            value.retryable = false;
            value.completed_text = Some(completed_text);
            if let Err(error) = storage::save_failed_recording(value) {
                storage::append_diagnostic(&format!(
                    "completed recovery metadata update failed: {error}"
                ));
            }
        }
    }
}

fn mark_failed_recording_non_retryable(state: &AppState, error: &str) {
    if let Ok(mut current) = state.failed_recording.lock() {
        if let Some(value) = current.as_mut() {
            mark_recovery_file_non_retryable(value);
            value.error = error.into();
            value.retryable = false;
            value.completed_text = None;
            if let Err(error) = storage::save_failed_recording(value) {
                storage::append_diagnostic(&format!(
                    "recovery non-retryable metadata update failed: {error}"
                ));
            }
        }
    }
}

fn mark_recovery_file_non_retryable(value: &mut FailedRecording) {
    if let Err(error) = rename_recovery_file_for_retryability(value, false) {
        storage::append_diagnostic(&format!("recovery retry-safety move failed: {error}"));
    }
}

fn rename_recovery_file_for_retryability(
    value: &mut FailedRecording,
    retryable: bool,
) -> Result<(), String> {
    let source = PathBuf::from(&value.path);
    let expected_prefix = if retryable {
        "failed-dictation-retryable-"
    } else {
        "failed-dictation-nonretryable-"
    };
    let already_marked = source
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(expected_prefix));
    if source.parent() != Some(storage::recovery_dir().as_path())
        || !source
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("flac") || extension.eq_ignore_ascii_case("wav")
            })
    {
        return Err("Recovery аудио файлът не е валиден.".into());
    }
    if !is_regular_local_file(&source) {
        return Err("Запазеният неуспешен аудио файл не е намерен.".into());
    }
    #[cfg(unix)]
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("Recovery аудио файлът не може да бъде защитен: {error}"))?;
    if already_marked {
        value.retryable = retryable;
        return Ok(());
    }
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("flac");
    let target = storage::recovery_dir().join(format!(
        "{expected_prefix}{}.{extension}",
        uuid::Uuid::new_v4()
    ));
    // This is a same-directory rename, so success is atomic. Do not fall back to a copy: leaving
    // both a retryable and non-retryable name could resurrect the unsafe one after later cleanup.
    std::fs::rename(&source, &target)
        .map_err(|error| format!("Recovery защитата не можа да бъде обновена: {error}"))?;
    value.path = target.to_string_lossy().to_string();
    value.retryable = retryable;
    Ok(())
}

#[tauri::command]
async fn retranscribe_history_item(
    id: String,
    app: AppHandle,
) -> Result<TranscriptionCompleted, String> {
    let state = app.state::<AppState>();
    let _operation = acquire_operation(&app, &state)?;
    if state
        .failed_recording
        .lock()
        .map_err(|_| "Recovery състоянието е заключено.")?
        .is_some()
    {
        return Err("Има запазен запис за възстановяване. Завършете го или го изтрийте, преди да започнете нова транскрипция.".into());
    }
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
    if !is_managed_output_path(Path::new(&audio_path), "flac")
        || !is_regular_file_with_extension(Path::new(&audio_path), "flac")
    {
        return Err("Свързаният аудио файл не е намерен.".into());
    }
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")?
        .clone();
    let key = api_key_from_state(&state)?;
    if let Ok(mut error) = state.last_recording_error.lock() {
        *error = None;
    }
    set_progress(&app, 1, "preparing_audio", false);
    set_recording_state(&app, "transcribing");
    // Keep the user's history FLAC untouched, but create and persist a non-retryable Recovery
    // copy before OpenAI can receive this retranscription. A crash must block another request.
    let pending = match retain_history_recording_copy(
        Path::new(&audio_path),
        entry.duration_seconds,
        IN_FLIGHT_RECOVERY_ERROR,
        false,
        None,
    ) {
        Ok(pending) => pending,
        Err(error) => {
            set_error(&app, &error);
            return Err(error);
        }
    };
    let request_audio = PathBuf::from(&pending.path);
    if let Err(error) = store_failed_recording(&app, &state, pending) {
        set_error(&app, &error);
        return Err(error);
    }
    let app_for_progress = app.clone();
    let callback: transcription::ProgressCallback = Arc::new(move |percent, stage, determinate| {
        set_progress(&app_for_progress, percent, stage, determinate)
    });
    match transcription::transcribe(&request_audio, &key, &settings, Some(callback)).await {
        Ok(text) => {
            let completed_text = text.clone();
            let completed = match finalize_success(
                &app,
                &settings,
                &request_audio,
                entry.duration_seconds,
                text,
            ) {
                Ok(completed) => completed,
                Err(error) => {
                    preserve_completed_recovery(&state, &error.message, completed_text);
                    emit_current_failed_recording(&app, &state);
                    set_error(&app, &error.message);
                    return Err(error.message);
                }
            };
            Ok(publish_recovery_completion(
                &app,
                &state,
                &request_audio,
                false,
                completed,
            ))
        }
        Err(failure) => {
            let message = match set_failed_recording_retryability(
                &app,
                &state,
                failure.retryable,
                &failure.message,
            ) {
                Ok(_) => failure.message,
                Err(recovery_error) => format!("{} {recovery_error}", failure.message),
            };
            set_error(&app, &message);
            Err(message)
        }
    }
}

#[tauri::command]
fn delete_failed_recording(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _operation = acquire_operation(&app, &state)?;
    clear_failed_recording_state(&state, true)?;
    if let Ok(mut error) = state.last_recording_error.lock() {
        *error = None;
    }
    set_recording_state(&app, "idle");
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
fn overlay_bootstrap(state: State<'_, AppState>) -> OverlayBootstrapState {
    let ui_language = state
        .settings
        .lock()
        .map(|settings| settings.ui_language.clone())
        .unwrap_or_else(|_| "auto".into());
    OverlayBootstrapState {
        ui_language,
        recording: recording_snapshot(&state),
    }
}

#[tauri::command]
fn update_settings(
    settings: AppSettings,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSettings, String> {
    let _operation = acquire_operation(&app, &state)?;
    let mut settings = settings;
    settings.normalize();
    shortcuts::validate_settings(&settings)?;
    if settings.save_audio || settings.save_text {
        let directory = selected_output_dir(&settings);
        ensure_output_directory_writable(&directory)?;
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
async fn save_api_key(
    api_key: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _operation = acquire_operation(&app, &state)?;
    let api_key = Zeroizing::new(api_key);
    let key = Zeroizing::new(api_key.trim().to_string());
    transcription::validate_api_key(&key).await?;
    keyring_entry()?
        .set_password(&key)
        .map_err(|error| format!("Ключът не можа да бъде запазен в Keychain: {error}"))?;
    *state
        .api_key
        .lock()
        .map_err(|_| "API key cache е заключен.")? = Some(key);
    refresh_tray_menu(&app);
    Ok(())
}

#[tauri::command]
fn delete_api_key(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let _operation = acquire_operation(&app, &state)?;
    match keyring_entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(error) => return Err(format!("Ключът не можа да бъде изтрит: {error}")),
    }
    *state
        .api_key
        .lock()
        .map_err(|_| "API key cache е заключен.")? = None;
    refresh_tray_menu(&app);
    Ok(())
}

#[tauri::command]
fn begin_shortcut_capture(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let operation = acquire_operation(&app, &state)?;
    shortcuts::begin_capture("dictation".into(), &state)?;
    operation.disarm();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let state = app.state::<AppState>();
        if shortcuts::cancel_capture(&state).unwrap_or(false) {
            release_shortcut_capture_operation(&app);
            let _ = app.emit("shortcut:capture-cancelled", "dictation");
        }
    });
    Ok(())
}

#[tauri::command]
fn cancel_shortcut_capture(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    if shortcuts::cancel_capture(&state)? {
        release_shortcut_capture_operation(&app);
    }
    Ok(())
}

#[tauri::command]
async fn test_microphone(
    microphone_name: Option<String>,
    automatic_fallback: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<audio::MicrophoneProbe, String> {
    let _operation = acquire_operation(&app, &state)?;
    let recorder = state.recorder.clone();
    tokio::task::spawn_blocking(move || {
        recorder.probe(audio::MicrophoneRoutingConfig {
            preferred_name: microphone_name,
            automatic_fallback,
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

fn valid_history_deletion_file(file: &storage::PendingHistoryDeletionFile) -> bool {
    let original = Path::new(&file.original);
    let staged = Path::new(&file.staged);
    let Some(original_name) = original.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let Some(staged_name) = staged.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let valid_original =
        is_managed_output_path(original, "flac") || is_managed_output_path(original, "txt");
    let Some(identifier) = staged_name
        .strip_prefix(&format!(".{original_name}.deleting-"))
        .filter(|value| value.len() == 32)
    else {
        return false;
    };
    valid_original
        && original.parent() == staged.parent()
        && identifier
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn restore_staged_history_files(deletion: &storage::PendingHistoryDeletion) -> Vec<String> {
    let mut failures = Vec::new();
    for file in deletion.files.iter().rev() {
        if !valid_history_deletion_file(file) {
            failures.push(format!("{}: invalid deletion journal", file.original));
            continue;
        }
        let original = Path::new(&file.original);
        let staged = Path::new(&file.staged);
        if original.exists() || !staged.exists() {
            continue;
        }
        if let Err(error) = std::fs::rename(staged, original) {
            failures.push(format!("{}: {error}", original.display()));
        }
    }
    failures
}

fn prepare_history_files_for_deletion(
    entry: &TranscriptEntry,
) -> Result<storage::PendingHistoryDeletion, String> {
    let mut files = Vec::new();
    for (path, expected_extension) in [
        (entry.audio_path.as_deref(), "flac"),
        (entry.text_path.as_deref(), "txt"),
    ]
    .into_iter()
    .filter_map(|(path, extension)| path.map(|path| (Path::new(path), extension)))
    {
        if !is_managed_output_path(path, expected_extension) {
            return Err(format!("{}: invalid linked file", path.display()));
        }
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{}: {error}", path.display())),
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err(format!("{}: invalid linked file", path.display()));
            }
            Ok(_) => {}
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("history-file");
        let temporary = path.with_file_name(format!(
            ".{name}.deleting-{}",
            uuid::Uuid::new_v4().simple()
        ));
        files.push(storage::PendingHistoryDeletionFile {
            original: path.to_string_lossy().to_string(),
            staged: temporary.to_string_lossy().to_string(),
        });
    }
    Ok(storage::PendingHistoryDeletion {
        entry: entry.clone(),
        history_committed: false,
        files,
    })
}

fn stage_history_files_for_deletion(
    deletion: &storage::PendingHistoryDeletion,
) -> Result<(), String> {
    for file in &deletion.files {
        if !valid_history_deletion_file(file) {
            let failures = restore_staged_history_files(deletion);
            return Err(format!(
                "{}: invalid deletion journal{}",
                file.original,
                if failures.is_empty() {
                    String::new()
                } else {
                    format!(" · rollback failed: {}", failures.join(" · "))
                }
            ));
        }
        let original = Path::new(&file.original);
        let staged = Path::new(&file.staged);
        if staged.exists() && !original.exists() {
            continue;
        }
        if let Err(error) = std::fs::rename(original, staged) {
            let failures = restore_staged_history_files(deletion);
            return Err(format!(
                "{}: {error}{}",
                original.display(),
                if failures.is_empty() {
                    String::new()
                } else {
                    format!(" · rollback failed: {}", failures.join(" · "))
                }
            ));
        }
    }
    Ok(())
}

fn commit_staged_history_deletion(deletion: &storage::PendingHistoryDeletion) -> Vec<String> {
    let mut failures = Vec::new();
    for file in &deletion.files {
        if !valid_history_deletion_file(file) {
            failures.push(format!("{}: invalid deletion journal", file.original));
            continue;
        }
        for path in [&file.staged, &file.original] {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => failures.push(format!("{path}: {error}")),
            }
        }
    }
    failures
}

fn recover_pending_history_deletion(history: &mut Vec<TranscriptEntry>) {
    let Some(deletion) = storage::load_pending_history_deletion() else {
        return;
    };
    let mut failures = if deletion.history_committed {
        commit_staged_history_deletion(&deletion)
    } else {
        restore_staged_history_files(&deletion)
    };
    if !deletion.history_committed
        && failures.is_empty()
        && !history.iter().any(|entry| entry.id == deletion.entry.id)
    {
        let mut restored_history = history.clone();
        restored_history.insert(0, deletion.entry.clone());
        restored_history.truncate(10);
        match storage::save_history(&restored_history) {
            Ok(()) => *history = restored_history,
            Err(error) => failures.push(format!("history restore: {error}")),
        }
    }
    if failures.is_empty() {
        if let Err(error) = storage::clear_pending_history_deletion() {
            storage::append_diagnostic(&format!(
                "history deletion journal cleanup failed: {error}"
            ));
        }
    } else {
        storage::append_diagnostic(&format!(
            "history deletion recovery failed: {}",
            failures.join(" · ")
        ));
    }
}

#[tauri::command]
fn delete_history_item(
    id: String,
    delete_files: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _operation = acquire_operation(&app, &state)?;
    let mut history = state.history.lock().map_err(|_| "Историята е заключена.")?;
    let removed = history.iter().find(|entry| entry.id == id).cloned();
    let mut next_history = history.clone();
    next_history.retain(|entry| entry.id != id);
    let mut deletion = if delete_files {
        removed
            .as_ref()
            .map(prepare_history_files_for_deletion)
            .transpose()?
            .filter(|deletion| !deletion.files.is_empty())
    } else {
        None
    };
    if let Some(deletion) = deletion.as_ref() {
        storage::save_pending_history_deletion(deletion)?;
        if let Err(error) = stage_history_files_for_deletion(deletion) {
            let _ = storage::clear_pending_history_deletion();
            return Err(error);
        }
    }
    if let Err(error) = storage::save_history(&next_history) {
        let rollback_failures = deletion
            .as_ref()
            .map(restore_staged_history_files)
            .unwrap_or_default();
        if rollback_failures.is_empty() {
            let _ = storage::clear_pending_history_deletion();
        }
        return Err(if rollback_failures.is_empty() {
            error
        } else {
            format!(
                "{error} · Файловете не можаха да бъдат възстановени: {}",
                rollback_failures.join(" · ")
            )
        });
    }
    if let Some(deletion) = deletion.as_mut() {
        deletion.history_committed = true;
        if let Err(error) = storage::save_pending_history_deletion(deletion) {
            let mut failures = restore_staged_history_files(deletion);
            if let Err(history_error) = storage::save_history(&history) {
                failures.push(format!("history restore: {history_error}"));
            }
            if failures.is_empty() {
                let _ = storage::clear_pending_history_deletion();
            }
            return Err(format!(
                "Изтриването не можа да бъде потвърдено: {error}{}",
                if failures.is_empty() {
                    String::new()
                } else {
                    format!(" · rollback failed: {}", failures.join(" · "))
                }
            ));
        }
    }
    *history = next_history;
    let failures = deletion
        .as_ref()
        .map(commit_staged_history_deletion)
        .unwrap_or_default();
    if !failures.is_empty() {
        return Err(format!(
            "Записът е изтрит от историята, но някои файлове ще бъдат изчистени при следващото стартиране: {}",
            failures.join(" · ")
        ));
    }
    if deletion.is_some() {
        storage::clear_pending_history_deletion().map_err(|error| {
            format!(
                "Файловете са изтрити, но cleanup състоянието ще бъде проверено отново: {error}"
            )
        })?;
    }
    Ok(())
}

#[tauri::command]
fn open_accessibility_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .status()
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err("Accessibility настройките не можаха да бъдат отворени.".into());
        }
    }
    Ok(())
}

#[tauri::command]
fn refresh_accessibility_status(app: AppHandle) -> bool {
    let granted = accessibility_granted();
    refresh_tray_menu(&app);
    granted
}

fn path_is_authorized_for_open(
    requested: &Path,
    history: &[TranscriptEntry],
    data_directory: &Path,
) -> bool {
    let is_history_file = history.iter().any(|entry| {
        entry.audio_path.as_deref().is_some_and(|saved| {
            let saved = Path::new(saved);
            saved == requested && is_managed_output_path(saved, "flac")
        }) || entry.text_path.as_deref().is_some_and(|saved| {
            let saved = Path::new(saved);
            saved == requested && is_managed_output_path(saved, "txt")
        })
    });
    let is_diagnostic_bundle = is_managed_diagnostic_path(requested, data_directory);
    is_history_file || is_diagnostic_bundle
}

fn is_managed_diagnostic_path(path: &Path, data_directory: &Path) -> bool {
    if path.parent() != Some(data_directory)
        || !path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        return false;
    }

    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return false;
    };
    let Some(suffix) = stem.strip_prefix("AIDOO-Whisper-Lite-Diagnostics-") else {
        return false;
    };
    let Some((timestamp, identifier)) = suffix.rsplit_once('-') else {
        return false;
    };

    chrono::NaiveDateTime::parse_from_str(timestamp, "%Y%m%d-%H%M%S").is_ok()
        && is_lower_hex_identifier(identifier, 12)
}

fn is_regular_file_with_extension(path: &Path, expected_extension: &str) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected_extension))
        && std::fs::symlink_metadata(path)
            .ok()
            .is_some_and(|metadata| metadata.file_type().is_file())
}

fn is_managed_output_path(path: &Path, expected_extension: &str) -> bool {
    if !path.is_absolute()
        || !path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case(expected_extension))
    {
        return false;
    }

    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return false;
    };
    let Some(suffix) = stem.strip_prefix("AIDOO-Whisper-") else {
        return false;
    };
    let Some((timestamp, identifier)) = suffix.rsplit_once('-') else {
        return false;
    };

    chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d_%H-%M-%S").is_ok()
        && (is_lower_hex_identifier(identifier, 6) || is_lower_hex_identifier(identifier, 12))
}

fn is_lower_hex_identifier(value: &str, expected_length: usize) -> bool {
    value.len() == expected_length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_regular_local_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_file())
}

fn diagnostic_settings(settings: &AppSettings) -> serde_json::Value {
    serde_json::json!({
        "onboardingComplete": settings.onboarding_complete,
        "uiLanguage": settings.ui_language,
        "language": settings.language,
        "model": settings.model,
        "autoPaste": settings.auto_paste,
        "saveAudio": settings.save_audio,
        "saveText": settings.save_text,
        "historyEnabled": settings.history_enabled,
        "outputDirectory": if settings.output_directory.is_some() { "custom" } else { "default" },
        "launchAtLogin": settings.launch_at_login,
        "microphone": if settings.microphone_name.is_some() { "custom" } else { "system-default" },
        "automaticMicrophoneFallback": settings.automatic_microphone_fallback,
        "dictationShortcut": settings.dictation_shortcut,
    })
}

#[cfg(test)]
mod local_path_tests {
    use super::{
        commit_staged_history_deletion, diagnostic_settings, is_managed_output_path,
        localized_native_error, path_is_authorized_for_open, prepare_history_files_for_deletion,
        preserve_completed_recovery_with, recording_watchdog_should_stop,
        recover_pending_history_deletion, recovery_plan,
        resolve_failed_recording_after_success_with, resolved_tray_state,
        restore_staged_history_files, save_local_transcription_files,
        stage_history_files_for_deletion, tray_tooltip, AppSettings, FailedRecording, RecoveryPlan,
        TranscriptEntry, CHARGED_RECOVERY_ERROR,
    };
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    fn history_entry() -> TranscriptEntry {
        TranscriptEntry {
            id: "entry".into(),
            text: "text".into(),
            created_at: "2026-09-14T00:00:00Z".into(),
            duration_seconds: 1.0,
            model: "gpt-4o-mini-transcribe".into(),
            language: "bg".into(),
            audio_path: Some(
                "/Volumes/External/AIDOO/AIDOO-Whisper-2026-09-14_00-00-00-abcdef.flac".into(),
            ),
            text_path: Some(
                "/Volumes/External/AIDOO/AIDOO-Whisper-2026-09-14_00-00-00-abcdef.txt".into(),
            ),
        }
    }

    fn recovery_recording(path: &Path, retryable: bool) -> FailedRecording {
        FailedRecording {
            path: path.to_string_lossy().to_string(),
            created_at: "2026-09-15T00:00:00Z".into(),
            duration_seconds: 2.0,
            error: "network".into(),
            retryable,
            completed_text: None,
        }
    }

    #[test]
    fn recovery_policy_has_exactly_one_safe_next_action() {
        let retryable = recovery_recording(Path::new("/tmp/retryable.flac"), true);
        assert_eq!(recovery_plan(&retryable).unwrap(), RecoveryPlan::Transcribe);

        let mut completed = retryable.clone();
        completed.retryable = false;
        completed.completed_text = Some("already paid text".into());
        assert_eq!(
            recovery_plan(&completed).unwrap(),
            RecoveryPlan::FinishLocally("already paid text".into())
        );

        let charged = recovery_recording(Path::new("/tmp/charged.flac"), false);
        assert_eq!(recovery_plan(&charged).unwrap_err(), CHARGED_RECOVERY_ERROR);
    }

    #[test]
    fn failed_cleanup_after_success_can_only_be_deleted() {
        let root = std::env::temp_dir().join(format!(
            "aidoo-lite-recovery-cleanup-test-{}",
            uuid::Uuid::new_v4()
        ));
        let private = root.join("private");

        crate::storage::with_test_data_dir(private, || {
            crate::storage::ensure_directories().unwrap();
            let source = crate::storage::recovery_dir().join(format!(
                "failed-dictation-retryable-{}.flac",
                uuid::Uuid::new_v4()
            ));
            std::fs::write(&source, b"fLaC charged audio").unwrap();
            let failed_recording = Mutex::new(Some(recovery_recording(&source, true)));

            let error = resolve_failed_recording_after_success_with(&failed_recording, |_| {
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            })
            .unwrap_err();

            assert!(error.contains("не можа да бъде изтрито"));
            let current = failed_recording.lock().unwrap().clone().unwrap();
            assert!(!current.retryable);
            assert!(current.completed_text.is_none());
            assert!(Path::new(&current.path).is_file());
            assert!(Path::new(&current.path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("failed-dictation-nonretryable-"));
            assert_eq!(recovery_plan(&current).unwrap_err(), CHARGED_RECOVERY_ERROR);

            let persisted = crate::storage::load_failed_recording().unwrap();
            assert_eq!(persisted.path, current.path);
            assert!(!persisted.retryable);
            assert_eq!(
                recovery_plan(&persisted).unwrap_err(),
                CHARGED_RECOVERY_ERROR
            );
        });

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn completed_openai_text_is_finished_locally_without_another_request() {
        let root = std::env::temp_dir().join(format!(
            "aidoo-lite-completed-recovery-test-{}",
            uuid::Uuid::new_v4()
        ));
        let private = root.join("private");

        crate::storage::with_test_data_dir(private, || {
            crate::storage::ensure_directories().unwrap();
            let source = crate::storage::recovery_dir().join(format!(
                "failed-dictation-nonretryable-{}.flac",
                uuid::Uuid::new_v4()
            ));
            std::fs::write(&source, b"fLaC charged audio").unwrap();
            let failed_recording = Mutex::new(Some(recovery_recording(&source, false)));

            preserve_completed_recovery_with(
                &failed_recording,
                "local save failed",
                "already paid text".into(),
            );

            let current = failed_recording.lock().unwrap().clone().unwrap();
            assert!(!current.retryable);
            assert_eq!(
                recovery_plan(&current).unwrap(),
                RecoveryPlan::FinishLocally("already paid text".into())
            );
            let persisted = crate::storage::load_failed_recording().unwrap();
            assert_eq!(
                persisted.completed_text.as_deref(),
                Some("already paid text")
            );
            assert!(!persisted.retryable);
        });

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_open_scope_accepts_only_history_files_and_diagnostic_bundles() {
        let history = [history_entry()];
        let data = Path::new("/Users/example/Library/Application Support/AIDOO Whisper Lite");

        assert!(path_is_authorized_for_open(
            Path::new("/Volumes/External/AIDOO/AIDOO-Whisper-2026-09-14_00-00-00-abcdef.flac"),
            &history,
            data
        ));
        assert!(is_managed_output_path(
            Path::new(
                "/Volumes/External/AIDOO/AIDOO-Whisper-2026-09-14_00-00-00-abcdef123456.flac"
            ),
            "flac"
        ));
        assert!(path_is_authorized_for_open(
            &data.join("AIDOO-Whisper-Lite-Diagnostics-20260914-000000-abcdef123456.zip"),
            &history,
            data
        ));
        assert!(!path_is_authorized_for_open(
            Path::new("/Users/example/secret.txt"),
            &history,
            data
        ));
        assert!(!path_is_authorized_for_open(
            &data.join("settings.json"),
            &history,
            data
        ));

        let mut tampered = history_entry();
        tampered.audio_path = Some("/Users/example/secret.flac".into());
        assert!(!path_is_authorized_for_open(
            Path::new("/Users/example/secret.flac"),
            &[tampered],
            data
        ));
        assert!(!is_managed_output_path(
            Path::new("AIDOO-Whisper-relative.flac"),
            "flac"
        ));
        assert!(!is_managed_output_path(
            Path::new("/Users/example/AIDOO-Whisper-secret.flac"),
            "flac"
        ));
        assert!(!is_managed_output_path(
            Path::new("/Users/example/AIDOO-Whisper-2026-99-99_00-00-00-abcdef.flac"),
            "flac"
        ));
        assert!(!is_managed_output_path(
            Path::new("/Users/example/AIDOO-Whisper-2026-09-14_00-00-00-abcdeg.flac"),
            "flac"
        ));
        assert!(!is_managed_output_path(
            Path::new("/Users/example/AIDOO-Whisper-2026-09-14_00-00-00-abcdef0.flac"),
            "flac"
        ));
        assert!(!is_managed_output_path(
            Path::new("/Users/example/AIDOO-Whisper-2026-09-14_00-00-00-ABCDEF.flac"),
            "flac"
        ));
        assert!(!path_is_authorized_for_open(
            &data.join("AIDOO-Whisper-Lite-Diagnostics-20260914-000000.zip"),
            &history,
            data
        ));
        assert!(!path_is_authorized_for_open(
            &data.join("AIDOO-Whisper-Lite-Diagnostics-20261314-000000-abcdef123456.zip"),
            &history,
            data
        ));
        assert!(!path_is_authorized_for_open(
            &data.join("AIDOO-Whisper-Lite-Diagnostics-20260914-000000-abcdef12345g.zip"),
            &history,
            data
        ));
    }

    #[test]
    fn diagnostic_settings_exclude_private_device_and_path_details() {
        let settings = AppSettings {
            output_directory: Some("/Users/example/Private Transcripts".into()),
            microphone_name: Some("Owner's Studio Microphone".into()),
            ..AppSettings::default()
        };

        let serialized = diagnostic_settings(&settings).to_string();

        assert!(serialized.contains("\"outputDirectory\":\"custom\""));
        assert!(serialized.contains("\"microphone\":\"custom\""));
        assert!(!serialized.contains("Private Transcripts"));
        assert!(!serialized.contains("Owner's Studio Microphone"));
        assert!(!serialized.contains("/Users/example"));
    }

    #[test]
    fn local_file_preferences_create_only_the_requested_private_artifacts() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;

        for (save_audio, save_text) in [(false, false), (true, false), (false, true), (true, true)]
        {
            let root = std::env::temp_dir().join(format!(
                "aidoo-lite-local-files-test-{}",
                uuid::Uuid::new_v4()
            ));
            let output = root.join("custom-output");
            std::fs::create_dir_all(&root).unwrap();
            let source = root.join("source.flac");
            std::fs::write(&source, b"fLaC private audio").unwrap();
            let settings = AppSettings {
                save_audio,
                save_text,
                output_directory: Some(output.to_string_lossy().to_string()),
                ..AppSettings::default()
            };

            let (audio, text) =
                save_local_transcription_files(&settings, &source, "Private text").unwrap();

            assert_eq!(audio.is_some(), save_audio);
            assert_eq!(text.is_some(), save_text);
            assert_eq!(output.exists(), save_audio || save_text);
            if let Some(path) = audio.as_ref() {
                assert_eq!(path.parent(), Some(output.as_path()));
                assert_eq!(std::fs::read(path).unwrap(), b"fLaC private audio");
                #[cfg(unix)]
                assert_eq!(
                    std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
            if let Some(path) = text.as_ref() {
                assert_eq!(path.parent(), Some(output.as_path()));
                assert_eq!(std::fs::read(path).unwrap(), b"Private text\n");
                #[cfg(unix)]
                assert_eq!(
                    std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
            if let (Some(audio), Some(text)) = (audio.as_ref(), text.as_ref()) {
                assert_eq!(audio.file_stem(), text.file_stem());
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    fn temporary_history_entry(root: &Path) -> TranscriptEntry {
        let mut entry = history_entry();
        entry.audio_path = Some(
            root.join("AIDOO-Whisper-2026-09-15_12-00-00-abcdef123456.flac")
                .to_string_lossy()
                .to_string(),
        );
        entry.text_path = Some(
            root.join("AIDOO-Whisper-2026-09-15_12-00-00-abcdef123456.txt")
                .to_string_lossy()
                .to_string(),
        );
        entry
    }

    #[test]
    fn history_file_deletion_is_reversible_until_history_is_committed() {
        let root = std::env::temp_dir().join(format!(
            "aidoo-lite-history-delete-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let entry = temporary_history_entry(&root);
        let originals = [
            PathBuf::from(entry.audio_path.as_ref().unwrap()),
            PathBuf::from(entry.text_path.as_ref().unwrap()),
        ];
        std::fs::write(&originals[0], b"fLaC").unwrap();
        std::fs::write(&originals[1], b"text").unwrap();

        let deletion = prepare_history_files_for_deletion(&entry).unwrap();
        stage_history_files_for_deletion(&deletion).unwrap();

        assert_eq!(deletion.files.len(), 2);
        assert!(originals.iter().all(|path| !path.exists()));
        assert!(deletion
            .files
            .iter()
            .all(|file| Path::new(&file.staged).is_file()));
        assert!(restore_staged_history_files(&deletion).is_empty());
        assert!(originals.iter().all(|path| path.is_file()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn committed_history_file_deletion_removes_both_linked_files() {
        let root = std::env::temp_dir().join(format!(
            "aidoo-lite-history-delete-commit-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let entry = temporary_history_entry(&root);
        let originals = [
            PathBuf::from(entry.audio_path.as_ref().unwrap()),
            PathBuf::from(entry.text_path.as_ref().unwrap()),
        ];
        std::fs::write(&originals[0], b"fLaC").unwrap();
        std::fs::write(&originals[1], b"text").unwrap();
        let deletion = prepare_history_files_for_deletion(&entry).unwrap();
        stage_history_files_for_deletion(&deletion).unwrap();

        assert!(commit_staged_history_deletion(&deletion).is_empty());

        assert!(originals.iter().all(|path| !path.exists()));
        assert!(deletion
            .files
            .iter()
            .all(|file| !Path::new(&file.staged).exists()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_history_deletion_rolls_back_when_history_still_contains_the_entry() {
        let root = std::env::temp_dir().join(format!(
            "aidoo-lite-history-delete-recovery-test-{}",
            uuid::Uuid::new_v4()
        ));
        let output = root.join("output");
        let private = root.join("private");
        std::fs::create_dir_all(&output).unwrap();
        let entry = temporary_history_entry(&output);
        let originals = [
            PathBuf::from(entry.audio_path.as_ref().unwrap()),
            PathBuf::from(entry.text_path.as_ref().unwrap()),
        ];
        std::fs::write(&originals[0], b"fLaC").unwrap();
        std::fs::write(&originals[1], b"text").unwrap();
        let deletion = prepare_history_files_for_deletion(&entry).unwrap();

        crate::storage::with_test_data_dir(private, || {
            crate::storage::ensure_directories().unwrap();
            crate::storage::save_pending_history_deletion(&deletion).unwrap();
            stage_history_files_for_deletion(&deletion).unwrap();
            let mut history = vec![entry.clone()];
            recover_pending_history_deletion(&mut history);
            assert_eq!(history.len(), 1);
            assert_eq!(history[0].id, entry.id);
            assert!(crate::storage::load_pending_history_deletion().is_none());
        });

        assert!(originals.iter().all(|path| path.is_file()));
        assert!(deletion
            .files
            .iter()
            .all(|file| !Path::new(&file.staged).exists()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_history_deletion_finishes_when_history_commit_is_present() {
        let root = std::env::temp_dir().join(format!(
            "aidoo-lite-history-delete-finish-test-{}",
            uuid::Uuid::new_v4()
        ));
        let output = root.join("output");
        let private = root.join("private");
        std::fs::create_dir_all(&output).unwrap();
        let entry = temporary_history_entry(&output);
        let originals = [
            PathBuf::from(entry.audio_path.as_ref().unwrap()),
            PathBuf::from(entry.text_path.as_ref().unwrap()),
        ];
        std::fs::write(&originals[0], b"fLaC").unwrap();
        std::fs::write(&originals[1], b"text").unwrap();
        let mut deletion = prepare_history_files_for_deletion(&entry).unwrap();
        deletion.history_committed = true;

        crate::storage::with_test_data_dir(private, || {
            crate::storage::ensure_directories().unwrap();
            crate::storage::save_pending_history_deletion(&deletion).unwrap();
            stage_history_files_for_deletion(&deletion).unwrap();
            let mut history = Vec::new();
            recover_pending_history_deletion(&mut history);
            assert!(history.is_empty());
            assert!(crate::storage::load_pending_history_deletion().is_none());
        });

        assert!(originals.iter().all(|path| !path.exists()));
        assert!(deletion
            .files
            .iter()
            .all(|file| !Path::new(&file.staged).exists()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn prepared_deletion_restores_the_history_entry_if_commit_phase_was_not_saved() {
        let root = std::env::temp_dir().join(format!(
            "aidoo-lite-history-delete-phase-test-{}",
            uuid::Uuid::new_v4()
        ));
        let output = root.join("output");
        let private = root.join("private");
        std::fs::create_dir_all(&output).unwrap();
        let entry = temporary_history_entry(&output);
        let originals = [
            PathBuf::from(entry.audio_path.as_ref().unwrap()),
            PathBuf::from(entry.text_path.as_ref().unwrap()),
        ];
        std::fs::write(&originals[0], b"fLaC").unwrap();
        std::fs::write(&originals[1], b"text").unwrap();
        let deletion = prepare_history_files_for_deletion(&entry).unwrap();

        crate::storage::with_test_data_dir(private, || {
            crate::storage::ensure_directories().unwrap();
            crate::storage::save_pending_history_deletion(&deletion).unwrap();
            stage_history_files_for_deletion(&deletion).unwrap();
            let mut history = Vec::new();
            recover_pending_history_deletion(&mut history);
            assert_eq!(history.len(), 1);
            assert_eq!(history[0].id, entry.id);
            assert_eq!(crate::storage::load_history()[0].id, entry.id);
            assert!(crate::storage::load_pending_history_deletion().is_none());
        });

        assert!(originals.iter().all(|path| path.is_file()));
        assert!(deletion
            .files
            .iter()
            .all(|file| !Path::new(&file.staged).exists()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_remains_visible_in_the_tray_until_it_is_resolved() {
        assert_eq!(resolved_tray_state("idle", true, true, true), "recovery");
        assert_eq!(resolved_tray_state("error", true, true, true), "recovery");
        assert_eq!(
            resolved_tray_state("transcribing", true, true, true),
            "transcribing"
        );
        assert_eq!(resolved_tray_state("idle", true, true, false), "idle");
        assert_eq!(
            resolved_tray_state("idle", false, true, false),
            "permission"
        );
        assert_eq!(resolved_tray_state("idle", true, false, false), "setup");
    }

    #[test]
    fn tray_tooltip_covers_starting_processing_and_completion() {
        assert_eq!(
            tray_tooltip("starting", true),
            "AIDOO Whisper Lite — starting microphone"
        );
        assert_eq!(
            tray_tooltip("transcribing", false),
            "AIDOO Whisper Lite — транскрибирам"
        );
        assert_eq!(
            tray_tooltip("done", false),
            "AIDOO Whisper Lite — транскрипцията е готова"
        );
    }

    #[test]
    fn stale_recording_watchdog_cannot_stop_a_new_recording() {
        assert!(recording_watchdog_should_stop(true, 7, 7));
        assert!(!recording_watchdog_should_stop(false, 7, 7));
        assert!(!recording_watchdog_should_stop(true, 8, 7));
    }

    #[test]
    fn tray_reports_every_processing_stage_in_both_languages() {
        for stage in [
            "preparing_audio",
            "starting_microphone",
            "compressing_audio",
            "uploading_audio",
            "openai_transcribing",
            "text_ready",
            "finishing_locally",
        ] {
            assert!(
                super::progress_status_label(stage, true).is_some(),
                "{stage}"
            );
            assert!(
                super::progress_status_label(stage, false).is_some(),
                "{stage}"
            );
        }
        assert_eq!(super::progress_status_label("unknown", true), None);
    }

    #[test]
    fn tray_errors_follow_the_selected_interface_language() {
        assert_eq!(
            localized_native_error("Транскрипцията не успя: HTTP 500", true),
            "Transcription failed: HTTP 500"
        );
        assert_eq!(
            localized_native_error(
                "Транскрипцията не успя: няма наличен API баланс или е достигнат лимитът.",
                true
            ),
            "Transcription failed: no API balance is available or the limit has been reached."
        );
        assert_eq!(
            localized_native_error("Не беше разпозната реч.", false),
            "Не беше разпозната реч."
        );
    }
}

#[tauri::command]
fn open_local_path(path: String, reveal: bool, state: State<'_, AppState>) -> Result<(), String> {
    let requested = PathBuf::from(path);
    let history = state.history.lock().map_err(|_| "Историята е заключена.")?;
    if !path_is_authorized_for_open(&requested, &history, &storage::data_dir()) {
        return Err("Този локален файл не е разрешен за отваряне.".into());
    }
    if !is_regular_local_file(&requested) {
        return Err("Локалният файл вече не съществува.".into());
    }
    let mut command = std::process::Command::new("open");
    if reveal {
        command.arg("-R");
    }
    let status = command
        .arg(&requested)
        .status()
        .map_err(|error| format!("Файлът не можа да бъде отворен: {error}"))?;
    if !status.success() {
        return Err("Файлът не можа да бъде отворен.".into());
    }
    Ok(())
}

struct PendingDiagnosticFile {
    path: PathBuf,
    committed: bool,
}

impl Drop for PendingDiagnosticFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[tauri::command]
fn create_diagnostic_bundle(app: AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let _operation = acquire_operation(&app, &state)?;
    storage::ensure_directories()?;
    let transcript_texts = state
        .history
        .lock()
        .map_err(|_| "Историята е заключена.")?
        .iter()
        .map(|entry| entry.text.clone())
        .collect::<Vec<_>>();
    let path = storage::data_dir().join(format!(
        "AIDOO-Whisper-Lite-Diagnostics-{}-{}.zip",
        Local::now().format("%Y%m%d-%H%M%S"),
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    ));
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("diagnostics.zip");
    let temporary = path.with_file_name(format!(
        ".{file_name}.tmp-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let mut pending = PendingDiagnosticFile {
        path: temporary.clone(),
        committed: false,
    };
    let mut file_options = File::options();
    file_options.write(true).create_new(true);
    #[cfg(unix)]
    file_options.mode(0o600);
    let file = file_options
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    if let Some(log) = storage::read_diagnostics_for_support() {
        zip.start_file("diagnostics.log", options)
            .map_err(|error| error.to_string())?;
        let sanitized =
            storage::sanitize_support_text(&String::from_utf8_lossy(&log), &transcript_texts);
        zip.write_all(sanitized.as_bytes())
            .map_err(|error| error.to_string())?;
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
                if !entry.file_type().ok()?.is_file() {
                    return None;
                }
                let modified = entry.metadata().ok()?.modified().ok()?;
                Some((modified, entry.path()))
            })
            .collect::<Vec<_>>();
        crash_reports.sort_by(|left, right| right.0.cmp(&left.0));
        for (_, report) in crash_reports.into_iter().take(3) {
            let Some(name) = report.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let mut bounded = Vec::with_capacity(1_000_000);
            if File::open(&report)
                .and_then(|file| file.take(1_000_000).read_to_end(&mut bounded))
                .is_ok()
            {
                let sanitized = storage::sanitize_support_text(
                    &String::from_utf8_lossy(&bounded),
                    &transcript_texts,
                );
                zip.start_file(format!("crash-reports/{name}"), options)
                    .map_err(|error| error.to_string())?;
                zip.write_all(sanitized.as_bytes())
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Настройките са заключени.")?
        .clone();
    zip.start_file("settings.json", options)
        .map_err(|error| error.to_string())?;
    let diagnostic_settings = diagnostic_settings(&settings);
    zip.write_all(
        &serde_json::to_vec_pretty(&diagnostic_settings).map_err(|error| error.to_string())?,
    )
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
    let file = zip.finish().map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    std::fs::rename(&temporary, &path).map_err(|error| error.to_string())?;
    pending.committed = true;
    #[cfg(unix)]
    if let Err(error) = File::open(storage::data_dir()).and_then(|directory| directory.sync_all()) {
        storage::append_diagnostic(&format!("diagnostic directory sync failed: {error}"));
    }
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
            "stop" => request_dictation_stop(app),
            "copy-error" => {
                if let Some(error) = app
                    .state::<AppState>()
                    .last_recording_error
                    .lock()
                    .ok()
                    .and_then(|value| value.clone())
                {
                    let localized = localized_native_error(&error, uses_english_ui(app));
                    let _ = text_insertion::copy(&localized);
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
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
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
            overlay_bootstrap,
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
        _ => {}
    });
}
