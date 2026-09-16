use super::*;

fn safe_file_stem() -> String {
    format!(
        "AIDOO-Whisper-{}-{}",
        Local::now().format("%Y-%m-%d_%H-%M-%S"),
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    )
}

#[derive(Debug)]
pub(super) struct FinalizationError {
    pub(super) message: String,
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

pub(super) fn save_local_transcription_files(
    settings: &AppSettings,
    staged_audio: &Path,
    text: &str,
) -> Result<(Option<PathBuf>, Option<PathBuf>), FinalizationError> {
    save_local_transcription_files_with_stem(settings, staged_audio, text, &safe_file_stem())
}

pub(super) fn save_local_transcription_files_with_stem(
    settings: &AppSettings,
    staged_audio: &Path,
    text: &str,
    stem: &str,
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

pub(super) fn finalize_success(
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

pub(super) fn retain_failed_recording(
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

pub(super) fn retain_captured_failure(
    app: &AppHandle,
    state: &AppState,
    captured: &audio::CapturedAudio,
    error: &str,
) -> Result<(), String> {
    let failed =
        retain_failed_recording(&captured.path, captured.duration_seconds, error, true, None)?;
    store_failed_recording(app, state, failed)
}

pub(super) fn store_failed_recording(
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

pub(super) fn clear_failed_recording_state(
    state: &AppState,
    remove_audio: bool,
) -> Result<(), String> {
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

pub(super) fn resolve_failed_recording_after_success_with(
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
pub(super) async fn retry_failed_transcription(
    app: AppHandle,
) -> Result<TranscriptionCompleted, String> {
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
            record_transcription_usage(&app, failed.duration_seconds, &settings.model);
            set_progress(&app, 100, "finishing_locally", true);
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

pub(super) fn emit_current_failed_recording(app: &AppHandle, state: &AppState) {
    if let Ok(current) = state.failed_recording.lock() {
        if let Some(failed) = current.as_ref() {
            let _ = app.emit("failed-recording:changed", failed);
        }
    }
}

pub(super) fn publish_recovery_completion(
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

pub(super) fn set_failed_recording_retryability(
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

pub(super) fn preserve_completed_recovery(state: &AppState, error: &str, completed_text: String) {
    preserve_completed_recovery_with(&state.failed_recording, error, completed_text);
}

pub(super) fn preserve_completed_recovery_with(
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
pub(super) async fn retranscribe_history_item(
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
            record_transcription_usage(&app, entry.duration_seconds, &settings.model);
            set_progress(&app, 100, "finishing_locally", true);
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
