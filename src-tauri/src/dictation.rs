use super::*;

pub(super) fn selected_output_dir(settings: &AppSettings) -> PathBuf {
    settings
        .output_directory
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(storage::default_output_dir)
}

pub(super) fn ensure_output_directory_writable(directory: &Path) -> Result<(), String> {
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

pub(super) fn api_key_from_state(state: &AppState) -> Result<Zeroizing<String>, String> {
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

pub(super) struct OperationGuard<'a> {
    active: &'a AtomicBool,
    app: AppHandle,
    release_on_drop: bool,
}

impl OperationGuard<'_> {
    pub(super) fn disarm(mut self) {
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

pub(super) fn acquire_operation<'a>(
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

pub(super) fn recording_watchdog_should_stop(
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
pub(super) fn start_recording(app: AppHandle) -> Result<audio::AudioStartInfo, String> {
    start_recording_inner(&app).inspect_err(|error| set_error(&app, error))
}

#[tauri::command]
pub(super) async fn stop_and_transcribe(app: AppHandle) -> Result<TranscriptionCompleted, String> {
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
            set_progress(app, 100, "finishing_locally", true);
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

pub(super) async fn prepare_flac(wav: &Path) -> Result<PathBuf, String> {
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
