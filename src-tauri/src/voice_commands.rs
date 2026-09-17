use super::*;

const MAX_LIVE_SESSION_DURATION: std::time::Duration = std::time::Duration::from_secs(10 * 60);

fn configured_assistant_mode(state: &State<'_, AppState>) -> Result<String, String> {
    state
        .settings
        .lock()
        .map(|settings| settings.aidoo_assistant_mode.clone())
        .map_err(|_| "Настройките са заключени.".into())
}

fn require_assistant_mode(state: &State<'_, AppState>, expected: &str) -> Result<(), String> {
    if configured_assistant_mode(state)? == expected {
        Ok(())
    } else {
        Err("Избраният AI режим беше променен. Стартирайте разговора отново.".into())
    }
}

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
    state.voice_pipeline.reset();
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
    mode: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if state.live_session_active.load(Ordering::Acquire) {
        return Err("Вече има активен разговор с AIDOO.".into());
    }
    let operation = acquire_operation(&app, &state)?;
    stop_wake_word_listener(&state);
    api_key_from_state(&state)?;
    if !matches!(
        mode.as_str(),
        models::ASSISTANT_MODE_ECONOMY | models::ASSISTANT_MODE_LIVE
    ) || configured_assistant_mode(&state)? != mode
    {
        return Err("Изберете валиден AI режим от настройките.".into());
    }
    state
        .live_backend_response_ids
        .lock()
        .map_err(|_| "Локалният отчет за разходите е заключен.")?
        .clear();
    state.voice_pipeline.reset();
    state.live_session_active.store(true, Ordering::Release);
    let generation = state
        .live_session_generation
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    operation.disarm();
    if mode == models::ASSISTANT_MODE_ECONOMY {
        start_live_usage(&state, models::ASSISTANT_PIPELINE_MODEL);
    }
    refresh_tray_menu(&app);
    storage::append_diagnostic(&format!("{mode} voice session prepared"));

    let timeout_app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(MAX_LIVE_SESSION_DURATION).await;
        if release_live_session(&timeout_app, Some(generation)) {
            storage::append_diagnostic("voice session reached the 10-minute safety limit");
            let _ = timeout_app.emit("live:force-close", "duration-limit");
        }
    });
    Ok(())
}

#[tauri::command]
pub(super) async fn begin_voice_turn(
    audio_data: Vec<u8>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<voice_pipeline::VoiceTurnResult, String> {
    if !state.live_session_active.load(Ordering::Acquire) {
        return Err("AI разговорът не е активен.".into());
    }
    require_assistant_mode(&state, models::ASSISTANT_MODE_ECONOMY)?;
    let duration_seconds = voice_pipeline::wav_duration_seconds(&audio_data)?;
    if !(0.15..=30.0).contains(&duration_seconds) {
        return Err("Аудио репликата е прекалено кратка или дълга.".into());
    }
    let api_key = api_key_from_state(&state)?;
    let transcript = voice_pipeline::transcribe_turn(audio_data, &api_key).await?;
    record_assistant_transcription_usage(&app, duration_seconds);
    let result =
        voice_pipeline::begin_text_turn(&state.voice_pipeline, transcript, &api_key).await?;
    record_turn_response_usage(&app, &state, &result)?;
    Ok(result)
}

#[tauri::command]
pub(super) async fn continue_voice_turn(
    turn_id: String,
    outputs: Vec<voice_pipeline::VoiceToolOutput>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<voice_pipeline::VoiceTurnResult, String> {
    if !state.live_session_active.load(Ordering::Acquire) {
        return Err("AI разговорът не е активен.".into());
    }
    require_assistant_mode(&state, models::ASSISTANT_MODE_ECONOMY)?;
    let api_key = api_key_from_state(&state)?;
    let result =
        voice_pipeline::continue_turn(&state.voice_pipeline, &turn_id, outputs, &api_key).await?;
    record_turn_response_usage(&app, &state, &result)?;
    Ok(result)
}

#[tauri::command]
pub(super) async fn synthesize_voice_reply(
    text: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<voice_pipeline::SpeechResult, String> {
    if !state.live_session_active.load(Ordering::Acquire) {
        return Err("AI разговорът не е активен.".into());
    }
    require_assistant_mode(&state, models::ASSISTANT_MODE_ECONOMY)?;
    let api_key = api_key_from_state(&state)?;
    let result = voice_pipeline::synthesize_speech(&text, &api_key).await?;
    record_assistant_speech_usage(&app, result.duration_seconds);
    Ok(result)
}

#[tauri::command]
pub(super) fn end_live_session(app: AppHandle) {
    if release_live_session(&app, None) {
        storage::append_diagnostic("turn-based voice session ended");
    }
}

#[tauri::command]
pub(super) async fn create_live_session(
    sdp: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<live::LiveSessionAnswer, String> {
    if !state.live_session_active.load(Ordering::Acquire) {
        return Err("GPT Live режимът не е подготвен.".into());
    }
    require_assistant_mode(&state, models::ASSISTANT_MODE_LIVE)?;
    let api_key = api_key_from_state(&state)?;
    let answer = match live::create_session(&sdp, &api_key).await {
        Ok(answer) => answer,
        Err(error) => {
            release_live_session(&app, None);
            return Err(error);
        }
    };
    if state.live_session_active.load(Ordering::Acquire) {
        start_live_usage(&state, models::LIVE_MODEL);
    }
    storage::append_diagnostic("GPT Live session started");
    Ok(answer)
}

#[tauri::command]
pub(super) fn record_live_backend_usage(
    response_id: String,
    model: String,
    usage: models::LiveBackendUsage,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if !state.live_session_active.load(Ordering::Acquire) {
        return Err("AI разговорът не е активен.".into());
    }
    require_assistant_mode(&state, models::ASSISTANT_MODE_LIVE)?;
    record_backend_usage_once(&app, &state, &response_id, &model, &usage)
}

fn record_turn_response_usage(
    app: &AppHandle,
    state: &State<'_, AppState>,
    result: &voice_pipeline::VoiceTurnResult,
) -> Result<(), String> {
    let (Some(response_id), Some(model), Some(usage)) = (
        result.response_id.as_ref(),
        result.model.as_ref(),
        result.usage.as_ref(),
    ) else {
        return Ok(());
    };
    record_backend_usage_once(app, state, response_id, model, usage)
}

fn record_backend_usage_once(
    app: &AppHandle,
    state: &State<'_, AppState>,
    response_id: &str,
    model: &str,
    usage: &models::LiveBackendUsage,
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
    if !valid_response_id || !models::is_live_backend_model(model) || !valid_usage {
        return Err("OpenAI върна невалидни backend usage данни.".into());
    }

    let mut recorded = state
        .live_backend_response_ids
        .lock()
        .map_err(|_| "Локалният отчет за разходите е заключен.")?;
    if recorded.contains(response_id) {
        return Ok(());
    }
    persist_live_backend_usage(app, model, usage)?;
    recorded.insert(response_id.into());
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
        "hearing",
        "transcribing",
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
