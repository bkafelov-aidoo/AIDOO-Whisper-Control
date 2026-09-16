use super::*;

const MAX_LIVE_SESSION_DURATION: std::time::Duration = std::time::Duration::from_secs(10 * 60);

pub(super) fn release_live_session(app: &AppHandle, generation: Option<u64>) -> bool {
    let state = app.state::<AppState>();
    if generation
        .is_some_and(|expected| state.live_session_generation.load(Ordering::Acquire) != expected)
    {
        return false;
    }
    if !state.live_session_active.swap(false, Ordering::AcqRel) {
        return false;
    }
    finish_live_usage(app);
    state.operation_active.store(false, Ordering::Release);
    if let Ok(mut phase) = state.live_phase.lock() {
        *phase = "idle".into();
    }
    refresh_tray_menu(app);
    schedule_wake_word_reconcile(app, std::time::Duration::from_millis(300));
    true
}

#[tauri::command]
pub(super) fn prepare_live_session(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if state.live_session_active.load(Ordering::Acquire) {
        return Err("Вече има активен разговор с AIDOO.".into());
    }
    let operation = acquire_operation(&app, &state)?;
    stop_wake_word_listener(&state);
    api_key_from_state(&state)?;
    state
        .live_backend_response_ids
        .lock()
        .map_err(|_| "Локалният отчет за разходите е заключен.")?
        .clear();
    state.live_session_active.store(true, Ordering::Release);
    let generation = state
        .live_session_generation
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    operation.disarm();
    refresh_tray_menu(&app);
    storage::append_diagnostic("GPT-Live session preparation started");

    let timeout_app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(MAX_LIVE_SESSION_DURATION).await;
        if release_live_session(&timeout_app, Some(generation)) {
            storage::append_diagnostic("GPT-Live session reached the 10-minute safety limit");
            let _ = timeout_app.emit("live:force-close", "duration-limit");
        }
    });
    Ok(())
}

#[tauri::command]
pub(super) async fn create_live_session(
    sdp: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<live::LiveSessionAnswer, String> {
    if !state.live_session_active.load(Ordering::Acquire) {
        return Err("GPT-Live режимът не е подготвен.".into());
    }
    let api_key = api_key_from_state(&state)?;
    let answer = match live::create_session(&sdp, &api_key).await {
        Ok(answer) => answer,
        Err(error) => {
            release_live_session(&app, None);
            return Err(error);
        }
    };
    if state.live_session_active.load(Ordering::Acquire) {
        start_live_usage(&state);
    }
    storage::append_diagnostic("GPT-Live session started");
    Ok(answer)
}

#[tauri::command]
pub(super) fn end_live_session(app: AppHandle) {
    if release_live_session(&app, None) {
        storage::append_diagnostic("GPT-Live session ended");
    }
}

#[tauri::command]
pub(super) fn record_live_backend_usage(
    response_id: String,
    model: String,
    usage: models::LiveBackendUsage,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    const MAX_TOKENS_PER_RESPONSE: u64 = 10_000_000;
    let valid_response_id = !response_id.is_empty()
        && response_id.len() <= 160
        && response_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    let valid_usage = usage.input_tokens <= MAX_TOKENS_PER_RESPONSE
        && usage.output_tokens <= MAX_TOKENS_PER_RESPONSE
        && usage.cached_input_tokens <= MAX_TOKENS_PER_RESPONSE
        && usage.cache_write_tokens <= MAX_TOKENS_PER_RESPONSE
        && usage
            .cached_input_tokens
            .checked_add(usage.cache_write_tokens)
            .is_some_and(|discounted| discounted <= usage.input_tokens);
    if !valid_response_id || !models::is_live_backend_model(&model) || !valid_usage {
        return Err("OpenAI върна невалидни backend usage данни.".into());
    }

    let mut recorded = state
        .live_backend_response_ids
        .lock()
        .map_err(|_| "Локалният отчет за разходите е заключен.")?;
    if recorded.contains(&response_id) {
        return Ok(());
    }
    persist_live_backend_usage(&app, &model, &usage)?;
    recorded.insert(response_id);
    Ok(())
}

#[tauri::command]
pub(super) fn set_live_phase(
    phase: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    const ALLOWED: &[&str] = &[
        "idle",
        "preparing",
        "connecting",
        "listening",
        "speaking",
        "working",
        "switching",
        "closing",
        "error",
    ];
    if !ALLOWED.contains(&phase.as_str()) {
        return Err("Невалидно състояние на AIDOO асистента.".into());
    }
    let previous = state.live_phase.lock().ok().map(|mut current| {
        let previous = current.clone();
        *current = phase.clone();
        previous
    });
    if let Some(sound) = previous
        .as_deref()
        .and_then(|previous| feedback_sound::for_live_transition(previous, &phase))
    {
        feedback_sound::play(&app, sound);
    }
    let _ = app.emit("assistant:phase", &phase);
    if phase == "idle" {
        let recording_idle = state
            .recording_status
            .lock()
            .map(|value| value.as_str() == "idle")
            .unwrap_or(false);
        if recording_idle {
            if let Some(window) = app.get_webview_window("overlay") {
                let _ = window.hide();
            }
        }
    } else {
        show_recording_overlay(&app);
        if let Some(window) = app.get_webview_window("overlay") {
            let _ = window.set_ignore_cursor_events(false);
        }
    }
    Ok(())
}

#[tauri::command]
pub(super) fn request_live_stop(app: AppHandle) {
    let _ = app.emit("live:force-close", "user-request");
}

#[tauri::command]
pub(super) fn take_assistant_request(state: State<'_, AppState>) -> bool {
    state.assistant_start_request.take()
}

#[tauri::command]
pub(super) fn start_voice_dictation(app: AppHandle) -> Result<audio::AudioStartInfo, String> {
    start_recording_inner(&app, "voice").inspect_err(|error| set_error(&app, error))
}

#[tauri::command]
pub(super) fn start_wake_word_calibration(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.recording_active.load(Ordering::Acquire)
        || state.operation_active.load(Ordering::Acquire)
    {
        return Err("Изчакайте текущата операция да приключи.".into());
    }
    if audio::microphone_names().is_empty() {
        return Err("Не е намерен микрофон.".into());
    }
    state.wake_word_calibrating.store(true, Ordering::Release);
    reconcile_wake_word_listener(&app);
    if !state.wake_word_listening.load(Ordering::Acquire) {
        state.wake_word_calibrating.store(false, Ordering::Release);
        return Err(state
            .wake_word_error
            .lock()
            .ok()
            .and_then(|error| error.clone())
            .unwrap_or_else(|| "Калибрацията не можа да стартира.".into()));
    }
    Ok(())
}

#[tauri::command]
pub(super) fn stop_wake_word_calibration(app: AppHandle) {
    let state = app.state::<AppState>();
    state.wake_word_calibrating.store(false, Ordering::Release);
    schedule_wake_word_reconcile(&app, std::time::Duration::from_millis(100));
}

#[tauri::command]
pub(super) fn delete_failed_recording(app: AppHandle) -> Result<(), String> {
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
pub(super) fn bootstrap(app: AppHandle, state: State<'_, AppState>) -> BootstrapState {
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
    let usage = state
        .usage
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
    let has_aidoo_password = aidoo_keyring_entry()
        .ok()
        .and_then(|entry| entry.get_password().ok())
        .is_some();
    let aidoo_connection_error = state
        .aidoo_connection_error
        .lock()
        .ok()
        .and_then(|value| value.clone());
    BootstrapState {
        settings,
        history,
        usage,
        failed_recording,
        microphones: audio::microphone_names(),
        has_api_key,
        has_aidoo_password,
        aidoo_connected: state.aidoo.connected(),
        aidoo_connection_error,
        accessibility_granted: accessibility_granted(),
        app_version: app.package_info().version.to_string(),
        default_output_directory: storage::default_output_dir().to_string_lossy().to_string(),
        recording: recording_snapshot(&state),
    }
}

#[tauri::command]
pub(super) fn overlay_bootstrap(state: State<'_, AppState>) -> OverlayBootstrapState {
    let ui_language = state
        .settings
        .lock()
        .map(|settings| settings.ui_language.clone())
        .unwrap_or_else(|_| "auto".into());
    OverlayBootstrapState {
        ui_language,
        recording: recording_snapshot(&state),
        assistant_phase: state
            .live_phase
            .lock()
            .map(|phase| phase.clone())
            .unwrap_or_else(|_| "idle".into()),
    }
}

#[tauri::command]
pub(super) fn update_settings(
    settings: AppSettings,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSettings, String> {
    let _operation = acquire_operation(&app, &state)?;
    stop_wake_word_listener(&state);
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
    if !settings.wake_word_enabled {
        if let Ok(mut error) = state.wake_word_error.lock() {
            *error = None;
        }
    }
    refresh_tray_menu(&app);
    let _ = app.emit("settings:changed", &settings);
    Ok(settings)
}

#[tauri::command]
pub(super) async fn save_api_key(
    api_key: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _operation = acquire_operation(&app, &state)?;
    stop_wake_word_listener(&state);
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
pub(super) fn delete_api_key(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let _operation = acquire_operation(&app, &state)?;
    stop_wake_word_listener(&state);
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
pub(super) fn begin_shortcut_capture(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let operation = acquire_operation(&app, &state)?;
    stop_wake_word_listener(&state);
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
pub(super) fn cancel_shortcut_capture(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if shortcuts::cancel_capture(&state)? {
        release_shortcut_capture_operation(&app);
    }
    Ok(())
}

#[tauri::command]
pub(super) async fn test_microphone(
    microphone_name: Option<String>,
    automatic_fallback: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<audio::MicrophoneProbe, String> {
    let _operation = acquire_operation(&app, &state)?;
    stop_wake_word_listener(&state);
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
pub(super) fn current_recording_snapshot(state: State<'_, AppState>) -> RecordingSnapshot {
    recording_snapshot(&state)
}

#[tauri::command]
pub(super) fn copy_text(text: String) -> Result<(), String> {
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

pub(super) fn restore_staged_history_files(
    deletion: &storage::PendingHistoryDeletion,
) -> Vec<String> {
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

pub(super) fn prepare_history_files_for_deletion(
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

pub(super) fn stage_history_files_for_deletion(
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

pub(super) fn commit_staged_history_deletion(
    deletion: &storage::PendingHistoryDeletion,
) -> Vec<String> {
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

pub(super) fn recover_pending_history_deletion(history: &mut Vec<TranscriptEntry>) {
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
pub(super) fn delete_history_item(
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
pub(super) fn open_accessibility_settings() -> Result<(), String> {
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
pub(super) fn refresh_accessibility_status(app: AppHandle) -> bool {
    let granted = accessibility_granted();
    refresh_tray_menu(&app);
    schedule_wake_word_reconcile(&app, std::time::Duration::from_millis(300));
    granted
}

pub(super) fn path_is_authorized_for_open(
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

pub(super) fn is_regular_file_with_extension(path: &Path, expected_extension: &str) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected_extension))
        && std::fs::symlink_metadata(path)
            .ok()
            .is_some_and(|metadata| metadata.file_type().is_file())
}

pub(super) fn is_managed_output_path(path: &Path, expected_extension: &str) -> bool {
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

pub(super) fn is_regular_local_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_file())
}

pub(super) fn diagnostic_settings(settings: &AppSettings) -> serde_json::Value {
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
        "wakeWordEnabled": settings.wake_word_enabled,
        "wakeWordAutoStop": settings.wake_word_auto_stop,
        "dictationShortcut": settings.dictation_shortcut,
    })
}

#[tauri::command]
pub(super) fn open_local_path(
    path: String,
    reveal: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
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

pub(super) struct PendingDiagnosticFile {
    pub(super) path: PathBuf,
    pub(super) committed: bool,
}

impl Drop for PendingDiagnosticFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[tauri::command]
pub(super) fn create_diagnostic_bundle(app: AppHandle) -> Result<String, String> {
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
