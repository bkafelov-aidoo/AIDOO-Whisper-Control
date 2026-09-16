use super::*;
use tauri::path::BaseDirectory;

const WAKE_WORD_PRIMARY_THRESHOLD: f32 = 0.68;
const WAKE_WORD_CONFIRMATION_THRESHOLD: f32 = 0.76;
const VOICE_SILENCE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1_500);
const VOICE_NO_SPEECH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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
            schedule_wake_word_reconcile(&self.app, std::time::Duration::from_millis(300));
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
    schedule_wake_word_reconcile(app, std::time::Duration::from_millis(300));
}

pub(super) fn wake_word_model_paths(app: &AppHandle) -> Result<(PathBuf, PathBuf), String> {
    let primary = app
        .path()
        .resolve("wakeword/hey_aidoo.onnx", BaseDirectory::Resource)
        .map_err(|error| format!("Пътят до основния wake-word модел не е достъпен: {error}"))?;
    let confirmation = app
        .path()
        .resolve(
            "wakeword/hey_aidoo_confirmation.onnx",
            BaseDirectory::Resource,
        )
        .map_err(|error| {
            format!("Пътят до потвърждаващия wake-word модел не е достъпен: {error}")
        })?;
    Ok((primary, confirmation))
}

pub(super) fn wake_word_should_listen(state: &AppState) -> bool {
    let calibrating = state.wake_word_calibrating.load(Ordering::Acquire);
    let settings_ready = state
        .settings
        .lock()
        .map(|settings| calibrating || (settings.onboarding_complete && settings.wake_word_enabled))
        .unwrap_or(false);
    let has_api_key = state
        .api_key
        .lock()
        .map(|key| key.is_some())
        .unwrap_or(false);
    let has_recovery = state
        .failed_recording
        .lock()
        .map(|recording| recording.is_some())
        .unwrap_or(true);
    let idle = state
        .recording_status
        .lock()
        .map(|status| status.as_str() == "idle")
        .unwrap_or(false);
    settings_ready
        && (calibrating || (has_api_key && accessibility_granted()))
        && (calibrating || !has_recovery)
        && (calibrating || idle)
        && !state.operation_active.load(Ordering::Acquire)
        && !state.recording_active.load(Ordering::Acquire)
        && !state.live_session_active.load(Ordering::Acquire)
}

pub(super) fn stop_wake_word_listener(state: &AppState) {
    if state.wake_word_listening.swap(false, Ordering::AcqRel) {
        state.wake_word.stop();
    }
}

pub(super) fn reconcile_wake_word_listener(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !wake_word_should_listen(&state) {
        stop_wake_word_listener(&state);
        return;
    }
    if state
        .wake_word_listening
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let settings = match state.settings.lock() {
        Ok(settings) => settings.clone(),
        Err(_) => {
            state.wake_word_listening.store(false, Ordering::Release);
            return;
        }
    };
    let routing = audio::MicrophoneRoutingConfig {
        preferred_name: settings.microphone_name,
        automatic_fallback: settings.automatic_microphone_fallback,
    };
    let result = wake_word_model_paths(app).and_then(|(primary, confirmation)| {
        state.wake_word.start(
            routing,
            primary,
            confirmation,
            WAKE_WORD_PRIMARY_THRESHOLD,
            WAKE_WORD_CONFIRMATION_THRESHOLD,
        )
    });
    match result {
        Ok(device) => {
            if let Ok(mut error) = state.wake_word_error.lock() {
                *error = None;
            }
            storage::append_diagnostic(&format!("wake word listener started; device={device}"));
            let _ = app.emit("wake-word:status", "listening");
            refresh_tray_menu(app);
        }
        Err(error) => {
            state.wake_word_listening.store(false, Ordering::Release);
            let is_new_error = state
                .wake_word_error
                .lock()
                .map(|mut current| {
                    let changed = current.as_deref() != Some(error.as_str());
                    *current = Some(error.clone());
                    changed
                })
                .unwrap_or(true);
            storage::append_diagnostic(&format!("wake word listener failed: {error}"));
            let _ = app.emit("wake-word:status", "error");
            if is_new_error {
                let _ = app.emit("toast", &error);
            }
            refresh_tray_menu(app);
            schedule_wake_word_reconcile(app, std::time::Duration::from_secs(5));
        }
    }
}

pub(super) fn schedule_wake_word_reconcile(app: &AppHandle, delay: std::time::Duration) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        reconcile_wake_word_listener(&app);
    });
}

pub(super) fn start_recording_inner(
    app: &AppHandle,
    trigger: &str,
) -> Result<audio::AudioStartInfo, String> {
    let state = app.state::<AppState>();
    let operation = acquire_operation(app, &state)?;
    stop_wake_word_listener(&state);
    let settings = ready_dictation_settings(&state)?;
    if state.recording_active.swap(true, Ordering::AcqRel) {
        return Err("Вече има активен запис.".into());
    }
    state.stop_requested.store(false, Ordering::Release);
    if let Ok(mut current_trigger) = state.recording_trigger.lock() {
        *current_trigger = Some(trigger.into());
    }
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
                if trigger == "voice" && settings.wake_word_auto_stop {
                    start_voice_auto_stop_watchdog(app, recording_generation);
                }
            }
            operation.disarm();
            Ok(info)
        }
        Err(error) => {
            state.recording_active.store(false, Ordering::Release);
            if let Ok(mut current_trigger) = state.recording_trigger.lock() {
                *current_trigger = None;
            }
            Err(error)
        }
    }
}

fn start_voice_auto_stop_watchdog(app: &AppHandle, recording_generation: u64) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let state = app.state::<AppState>();
            if !recording_watchdog_should_stop(
                state.recording_active.load(Ordering::Acquire),
                state.status_generation.load(Ordering::Acquire),
                recording_generation,
            ) {
                return;
            }
            let Ok(activity) = state.recorder.activity() else {
                return;
            };
            match voice_watchdog_action(activity) {
                VoiceWatchdogAction::Continue => {}
                VoiceWatchdogAction::Transcribe => {
                    request_dictation_stop(&app);
                    return;
                }
                VoiceWatchdogAction::Cancel => {
                    cancel_empty_voice_recording(&app);
                    return;
                }
            }
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VoiceWatchdogAction {
    Continue,
    Transcribe,
    Cancel,
}

pub(super) fn voice_watchdog_action(activity: audio::RecordingActivity) -> VoiceWatchdogAction {
    if activity.speech_detected && activity.silence_seconds >= VOICE_SILENCE_TIMEOUT.as_secs_f64() {
        VoiceWatchdogAction::Transcribe
    } else if !activity.speech_detected
        && activity.duration_seconds >= VOICE_NO_SPEECH_TIMEOUT.as_secs_f64()
    {
        VoiceWatchdogAction::Cancel
    } else {
        VoiceWatchdogAction::Continue
    }
}

fn cancel_empty_voice_recording(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !state.recording_active.swap(false, Ordering::AcqRel)
        || state.stop_requested.swap(true, Ordering::AcqRel)
    {
        return;
    }
    let _operation = release_active_operation(app, &state);
    if let Ok(captured) = state.recorder.finish() {
        let _ = std::fs::remove_file(&captured.path);
    }
    let message = if uses_english_ui(app) {
        "I did not hear speech after “Hey, AIDOO”. The recording was cancelled and not sent."
    } else {
        "Не чух реч след „Hey, AIDOO“. Записът е отменен и не е изпращан."
    };
    let _ = app.emit("toast", message);
    set_recording_state(app, "idle");
}

pub(super) fn recording_watchdog_should_stop(
    recording_active: bool,
    current_generation: u64,
    scheduled_generation: u64,
) -> bool {
    recording_active && current_generation == scheduled_generation
}

pub(crate) fn request_dictation_start(app: &AppHandle) -> bool {
    match start_recording_inner(app, "shortcut") {
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
    start_recording_inner(&app, "shortcut").inspect_err(|error| set_error(&app, error))
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
            record_transcription_usage(app, captured.duration_seconds, &settings.model);
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
