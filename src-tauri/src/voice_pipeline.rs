use futures_util::StreamExt;
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::Duration;

use crate::models::{
    LiveBackendUsage, ASSISTANT_REASONING_MODEL, ASSISTANT_SPEECH_MODEL,
    ASSISTANT_TRANSCRIPTION_MODEL,
};
use crate::voice_audio::pcm_to_wav;
pub(crate) use crate::voice_audio::wav_duration_seconds;

const TRANSCRIPTION_ENDPOINT: &str = "https://api.openai.com/v1/audio/transcriptions";
const RESPONSES_ENDPOINT: &str = "https://api.openai.com/v1/responses";
const SPEECH_ENDPOINT: &str = "https://api.openai.com/v1/audio/speech";
const MAX_AUDIO_BYTES: usize = 4 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SPEECH_BYTES: usize = 8 * 1024 * 1024;
const MAX_API_ERROR_BYTES: usize = 64 * 1024;
const MAX_TOOL_ROUNDS: u8 = 8;
const MAX_HISTORY_ITEMS: usize = 24;

const RESPONSE_INSTRUCTIONS: &str = "Отговаряй на български кратко и естествено. Използвай инструментите за всяко AIDOO действие. Не твърди, че действие е извършено преди успешен резултат. Питай само при реална двусмисленост.";
const ORCHESTRATOR_PROTOCOL: &str = r#"Ти си гласовият AIDOO Control асистент. Управляваш AIDOO само чрез предоставените function tools. Никога не измисляй пациент, ID, статус, диагноза, процедура, повърхност, свободен час или резултат. Винаги изчакай резултата от инструмента. Отговаряй на български с едно кратко изречение, подходящо за гласово прочитане.

Нормализирай FDI номер, изговорен като две отделни цифри: „едно шест“ е 16, „две шест“ е 26, „три шест“ е 36 и „четири шест“ е 46. Прилагай това за всички валидни FDI номера.

Работен протокол:
1. „Намери/отвори пациент X“ използва search_aidoo_patients. Един резултат се избира и показва автоматично. При няколко резултата поискай едно кратко уточнение и после select_aidoo_patient. „Зареди следващ пациент“ използва load_next_aidoo_patient. Запомни избрания patientId.
2. „Попълни/отвори статус“ използва begin_aidoo_status. Ако needsVisit=true, попитай само „Частен прием или НЗОК?“, после start_aidoo_status_visit без второ потвърждение. Всяка продиктувана промяна се записва веднага с apply_aidoo_status. Не подготвяй чернова и не искай „Да“. Подай основния статус и regions отделно. За корекция използвай replaceStatus. Кажи spokenSummary. При uncertain, rejected или staleDraft кажи ясно, че записът не е потвърден и не повтаряй автоматично.
3. „Запиши статуса“ използва finish_aidoo_status и показва Лечение.
4. „Запиши процедура X“ използва add_aidoo_procedure. Ако липсва зъб, попитай само „Кой зъб или звездичка?“. При няколко treatment реда използвай get_aidoo_active_treatments и поискай избор. Не искай допълнително потвърждение.
5. „Добави официална забележка“: поискай зъб или звездичка ако липсва, кажи „Диктувайте забележката“, после изпрати точния текст чрез write_aidoo_official_note.
6. За диагноза използвай write_aidoo_diagnosis по същия директен протокол.
7. За първи свободен час използвай find_aidoo_schedule_slot. date е YYYY-MM-DD само ако е посочена; иначе null. durationMinutes е null за 30 минути, doctor е null за свързания лекар. Инструментът отваря точната страница в График.
8. „Запиши пациент X в този час“ използва book_aidoo_schedule_slot с последния slotId, без допълнително „Да“. При няколко пациента поискай едно уточнение. При uncertain не повтаряй автоматично.
9. Всички write инструменти правят read-back и обновяват правилния екран. Не карай потребителя да навигира ръчно.
10. „Започни транскрипция“ и фразите за край се обработват локално преди теб."#;

#[derive(Default)]
pub struct VoicePipelineRuntime {
    history: Mutex<Vec<Value>>,
    pending: Mutex<Option<PendingTurn>>,
}

struct PendingTurn {
    id: String,
    transcript: String,
    input: Vec<Value>,
    tool_context: Vec<String>,
    rounds: u8,
}

impl VoicePipelineRuntime {
    pub fn reset(&self) {
        if let Ok(mut history) = self.history.lock() {
            history.clear();
        }
        if let Ok(mut pending) = self.pending.lock() {
            *pending = None;
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceToolOutput {
    pub call_id: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTurnResult {
    pub turn_id: Option<String>,
    pub transcript: Option<String>,
    pub reply: Option<String>,
    pub action: Option<String>,
    pub tool_calls: Vec<VoiceToolCall>,
    pub response_id: Option<String>,
    pub model: Option<String>,
    pub usage: Option<LiveBackendUsage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechResult {
    pub audio: Vec<u8>,
    pub duration_seconds: f64,
    pub model: &'static str,
}

#[derive(Debug, Deserialize)]
struct TranscriptionResponse {
    text: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    id: String,
    model: String,
    output: Vec<Value>,
    usage: Option<ResponseUsage>,
}

#[derive(Debug, Deserialize)]
struct ResponseUsage {
    input_tokens: u64,
    #[serde(default)]
    input_tokens_details: ResponseInputDetails,
    output_tokens: u64,
}

#[derive(Debug, Default, Deserialize)]
struct ResponseInputDetails {
    #[serde(default)]
    cached_tokens: u64,
    #[serde(default)]
    cache_write_tokens: u64,
}

struct ResponseStep {
    response_id: String,
    model: String,
    output: Vec<Value>,
    reply: Option<String>,
    tool_calls: Vec<VoiceToolCall>,
    usage: Option<LiveBackendUsage>,
}

pub async fn begin_text_turn(
    runtime: &VoicePipelineRuntime,
    transcript: String,
    api_key: &str,
) -> Result<VoiceTurnResult, String> {
    if runtime
        .pending
        .lock()
        .map_err(|_| locked_error())?
        .is_some()
    {
        return Err("Изчакайте текущата AIDOO команда да приключи.".into());
    }
    if transcript.trim().is_empty() {
        return Err("Не беше разпозната реч.".into());
    }
    if let Some(action) = local_session_action(&transcript) {
        return Ok(VoiceTurnResult {
            turn_id: None,
            transcript: Some(transcript),
            reply: (action == "close").then(|| "Чао!".into()),
            action: Some(action.into()),
            tool_calls: Vec::new(),
            response_id: None,
            model: None,
            usage: None,
        });
    }

    let history = runtime.history.lock().map_err(|_| locked_error())?.clone();
    let user_item = message("user", &transcript);
    let mut input = history;
    input.push(user_item);
    if let Some(call) = route_simple_command(&transcript) {
        let turn_id = uuid::Uuid::new_v4().to_string();
        input.push(json!({
            "type": "function_call",
            "call_id": call.call_id,
            "name": call.name,
            "arguments": call.arguments
        }));
        *runtime.pending.lock().map_err(|_| locked_error())? = Some(PendingTurn {
            id: turn_id.clone(),
            transcript: transcript.clone(),
            input,
            tool_context: Vec::new(),
            rounds: 1,
        });
        return Ok(VoiceTurnResult {
            turn_id: Some(turn_id),
            transcript: Some(transcript),
            reply: None,
            action: None,
            tool_calls: vec![call],
            response_id: None,
            model: None,
            usage: None,
        });
    }

    let step = create_response(&input, api_key).await?;
    finish_or_store(runtime, transcript, input, Vec::new(), 1, step)
}

pub async fn continue_turn(
    runtime: &VoicePipelineRuntime,
    turn_id: &str,
    outputs: Vec<VoiceToolOutput>,
    api_key: &str,
) -> Result<VoiceTurnResult, String> {
    let mut pending = runtime
        .pending
        .lock()
        .map_err(|_| locked_error())?
        .take()
        .ok_or_else(|| "Няма AIDOO команда, която чака резултат.".to_string())?;
    if pending.id != turn_id {
        *runtime.pending.lock().map_err(|_| locked_error())? = Some(pending);
        return Err("Получен е резултат за друга AIDOO команда.".into());
    }
    if outputs.is_empty() {
        return Err("AIDOO инструментът не върна резултат.".into());
    }
    for output in outputs {
        let safe = output.output.chars().take(100_000).collect::<String>();
        pending.input.push(json!({
            "type": "function_call_output",
            "call_id": output.call_id,
            "output": safe
        }));
        pending.tool_context.push(safe);
    }
    if pending.rounds >= MAX_TOOL_ROUNDS {
        return Err("AIDOO командата достигна безопасния лимит за последователни действия.".into());
    }
    let step = create_response(&pending.input, api_key).await?;
    finish_or_store(
        runtime,
        pending.transcript,
        pending.input,
        pending.tool_context,
        pending.rounds + 1,
        step,
    )
}

fn finish_or_store(
    runtime: &VoicePipelineRuntime,
    transcript: String,
    mut input: Vec<Value>,
    tool_context: Vec<String>,
    rounds: u8,
    step: ResponseStep,
) -> Result<VoiceTurnResult, String> {
    input.extend(step.output.iter().cloned());
    if !step.tool_calls.is_empty() {
        let turn_id = uuid::Uuid::new_v4().to_string();
        *runtime.pending.lock().map_err(|_| locked_error())? = Some(PendingTurn {
            id: turn_id.clone(),
            transcript: transcript.clone(),
            input,
            tool_context,
            rounds,
        });
        return Ok(VoiceTurnResult {
            turn_id: Some(turn_id),
            transcript: Some(transcript),
            reply: None,
            action: None,
            tool_calls: step.tool_calls,
            response_id: Some(step.response_id),
            model: Some(step.model),
            usage: step.usage,
        });
    }

    let reply = step.reply.unwrap_or_else(|| "Готово.".into());
    let mut history = runtime.history.lock().map_err(|_| locked_error())?;
    history.push(message("user", &transcript));
    if !tool_context.is_empty() {
        let context = tool_context
            .join("\n")
            .chars()
            .take(24_000)
            .collect::<String>();
        history.push(message(
            "developer",
            &format!("Проверен контекст от AIDOO инструментите за предишната команда:\n{context}"),
        ));
    }
    history.push(message("assistant", &reply));
    if history.len() > MAX_HISTORY_ITEMS {
        let remove = history.len() - MAX_HISTORY_ITEMS;
        history.drain(0..remove);
    }
    Ok(VoiceTurnResult {
        turn_id: None,
        transcript: Some(transcript),
        reply: Some(reply),
        action: None,
        tool_calls: Vec::new(),
        response_id: Some(step.response_id),
        model: Some(step.model),
        usage: step.usage,
    })
}

pub async fn transcribe_turn(audio: Vec<u8>, api_key: &str) -> Result<String, String> {
    validate_audio(&audio)?;
    let client = api_client(Duration::from_secs(90))?;
    let audio = Part::bytes(audio)
        .file_name("aidoo-turn.wav")
        .mime_str("audio/wav")
        .map_err(|error| error.to_string())?;
    let form = Form::new()
        .part("file", audio)
        .text("model", ASSISTANT_TRANSCRIPTION_MODEL)
        .text("response_format", "json")
        .text("language", "bg")
        .text("prompt", "AIDOO, НЗОК, пациент, посещение, статус, диагноза, процедура, оклузален, вестибуларен, лингвален, палатинален, цервикален, FDI.");
    let response = client
        .post(TRANSCRIPTION_ENDPOINT)
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
        .map_err(|error| format!("Няма връзка с OpenAI транскрипцията: {error}"))?;
    if !response.status().is_success() {
        return Err(api_error(response, "Транскрипцията на командата не успя").await);
    }
    let body = read_limited_body(response, MAX_RESPONSE_BYTES).await?;
    let parsed: TranscriptionResponse = serde_json::from_slice(&body)
        .map_err(|error| format!("OpenAI върна невалидна транскрипция: {error}"))?;
    Ok(parsed.text.trim().to_string())
}

async fn create_response(input: &[Value], api_key: &str) -> Result<ResponseStep, String> {
    let client = api_client(Duration::from_secs(90))?;
    let request = response_request(input);
    let response = client
        .post(RESPONSES_ENDPOINT)
        .bearer_auth(api_key)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("Няма връзка с OpenAI текстовия модел: {error}"))?;
    if !response.status().is_success() {
        return Err(api_error(response, "AI обработката на командата не успя").await);
    }
    let body = read_limited_body(response, MAX_RESPONSE_BYTES).await?;
    let parsed: OpenAiResponse = serde_json::from_slice(&body)
        .map_err(|error| format!("OpenAI върна невалиден AI отговор: {error}"))?;
    let tool_calls = parse_tool_calls(&parsed.output)?;
    let reply = parse_output_text(&parsed.output);
    let usage = parsed.usage.map(|usage| LiveBackendUsage {
        input_tokens: usage.input_tokens,
        cached_input_tokens: usage.input_tokens_details.cached_tokens,
        cache_write_tokens: usage.input_tokens_details.cache_write_tokens,
        output_tokens: usage.output_tokens,
    });
    Ok(ResponseStep {
        response_id: parsed.id,
        model: parsed.model,
        output: parsed.output,
        reply,
        tool_calls,
        usage,
    })
}

fn response_request(input: &[Value]) -> Value {
    let mut request_input = Vec::with_capacity(input.len() + 1);
    request_input.push(message("developer", ORCHESTRATOR_PROTOCOL));
    request_input.extend(input.iter().cloned());
    json!({
        "model": ASSISTANT_REASONING_MODEL,
        "instructions": RESPONSE_INSTRUCTIONS,
        "input": request_input,
        "tools": aidoo_tools(),
        "tool_choice": "auto",
        "parallel_tool_calls": false,
        "reasoning": { "effort": "none" },
        "include": ["reasoning.encrypted_content"],
        "max_output_tokens": 900,
        "store": false
    })
}

pub async fn synthesize_speech(text: &str, api_key: &str) -> Result<SpeechResult, String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 4_096 {
        return Err("Текстът за гласовия отговор е невалиден.".into());
    }
    let client = api_client(Duration::from_secs(90))?;
    let response = client
        .post(SPEECH_ENDPOINT)
        .bearer_auth(api_key)
        .json(&json!({
            "model": ASSISTANT_SPEECH_MODEL,
            "voice": "marin",
            "input": text,
            "instructions": "Говори на български, спокойно, ясно и кратко. Произнасяй AIDOO като Айдуу.",
            "response_format": "pcm",
            "stream_format": "audio",
            "speed": 1.05
        }))
        .send()
        .await
        .map_err(|error| format!("Няма връзка с OpenAI гласовия модел: {error}"))?;
    if !response.status().is_success() {
        return Err(api_error(response, "Гласовият отговор не успя").await);
    }
    let pcm = read_limited_body(response, MAX_SPEECH_BYTES).await?;
    let audio = pcm_to_wav(&pcm)?;
    let duration_seconds = wav_duration_seconds(&audio)?;
    Ok(SpeechResult {
        audio,
        duration_seconds,
        model: ASSISTANT_SPEECH_MODEL,
    })
}

fn validate_audio(audio: &[u8]) -> Result<(), String> {
    if audio.len() < 44
        || audio.len() > MAX_AUDIO_BYTES
        || &audio[0..4] != b"RIFF"
        || &audio[8..12] != b"WAVE"
    {
        return Err("Невалидна или прекалено дълга аудио реплика.".into());
    }
    Ok(())
}

fn parse_tool_calls(output: &[Value]) -> Result<Vec<VoiceToolCall>, String> {
    output
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
        .map(|item| {
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
            let arguments = item
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if call_id.is_empty() || name.is_empty() || arguments.is_empty() {
                return Err("OpenAI върна непълна заявка за AIDOO инструмент.".into());
            }
            Ok(VoiceToolCall {
                call_id: call_id.into(),
                name: name.into(),
                arguments: arguments.into(),
            })
        })
        .collect()
}

fn parse_output_text(output: &[Value]) -> Option<String> {
    let text = output
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter(|content| content.get("type").and_then(Value::as_str) == Some("output_text"))
        .filter_map(|content| content.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");
    (!text.trim().is_empty()).then(|| text.trim().to_string())
}

fn message(role: &str, text: &str) -> Value {
    let content_type = if role == "assistant" {
        "output_text"
    } else {
        "input_text"
    };
    json!({ "role": role, "content": [{ "type": content_type, "text": text }] })
}

fn route_simple_command(transcript: &str) -> Option<VoiceToolCall> {
    let normalized = normalize_command(transcript);
    if matches!(
        normalized.as_str(),
        "зареди следващ пациент" | "следващ пациент" | "отвори следващия пациент"
    ) {
        return Some(direct_call("load_next_aidoo_patient", json!({})));
    }
    for prefix in ["намери пациент ", "отвори пациент ", "зареди пациент "]
    {
        if let Some(query) = normalized
            .strip_prefix(prefix)
            .map(str::trim)
            .filter(|value| value.chars().count() >= 3)
        {
            return Some(direct_call(
                "search_aidoo_patients",
                json!({ "query": query }),
            ));
        }
    }
    None
}

fn direct_call(name: &str, arguments: Value) -> VoiceToolCall {
    VoiceToolCall {
        call_id: format!("local_{}", uuid::Uuid::new_v4().simple()),
        name: name.into(),
        arguments: arguments.to_string(),
    }
}

fn local_session_action(transcript: &str) -> Option<&'static str> {
    let normalized = normalize_command(transcript);
    let dictation = [
        "започни транскрипция",
        "стартирай транскрипция",
        "започни да записваш",
        "стартирай запис",
        "запиши транскрипция",
        "start transcription",
        "start dictation",
    ];
    if dictation.iter().any(|command| normalized.contains(command)) {
        return Some("dictation");
    }
    let close = [
        "край",
        "край на разговора",
        "край на сесията",
        "затвори",
        "затвори ми",
        "затвори разговора",
        "затвори сесията",
        "затвори асистента",
        "приключи",
        "приключи разговора",
        "приключи сесията",
        "приключваме",
        "спри асистента",
        "довиждане",
        "end",
        "goodbye",
        "close the session",
        "stop the assistant",
    ];
    close
        .iter()
        .find(|command| normalized == **command)
        .map(|_| "close")
}

fn normalize_command(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn api_client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(20))
        .timeout(timeout)
        .build()
        .map_err(|error| format!("OpenAI връзката не можа да бъде подготвена: {error}"))
}

async fn api_error(response: reqwest::Response, prefix: &str) -> String {
    let status = response.status();
    let body = read_limited_body(response, MAX_API_ERROR_BYTES)
        .await
        .unwrap_or_default();
    let message = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|value| value.pointer("/error/message")?.as_str().map(str::to_owned))
        .map(|message| message.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| format!("HTTP {status}"));
    match status.as_u16() {
        401 => "OpenAI не прие API ключа.".into(),
        403 => "Този OpenAI проект няма достъп до избрания AI модел.".into(),
        429 if message.to_lowercase().contains("quota") => {
            "Няма наличен OpenAI API баланс или е достигнат лимитът.".into()
        }
        _ => format!(
            "{prefix}: {}",
            message.chars().take(500).collect::<String>()
        ),
    }
}

async fn read_limited_body(
    response: reqwest::Response,
    maximum_bytes: usize,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum_bytes as u64)
    {
        return Err("OpenAI отговорът надвишава безопасния лимит.".into());
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("OpenAI отговорът е прекъснат: {error}"))?;
        if body.len().saturating_add(chunk.len()) > maximum_bytes {
            return Err("OpenAI отговорът надвишава безопасния лимит.".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn locked_error() -> String {
    "Състоянието на AI разговора е заключено.".into()
}

fn aidoo_tools() -> Vec<serde_json::Value> {
    vec![
        function_tool(
            "search_aidoo_patients",
            "Търси пациент. При един резултат го избира и показва автоматично; при повече резултати върни списъка за уточнение.",
            serde_json::json!({
                "type": "object",
                "properties": { "query": { "type": "string", "minLength": 4 } },
                "required": ["query"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "select_aidoo_patient",
            "Избира един пациент от последното търсене и показва картона му.",
            serde_json::json!({
                "type": "object",
                "properties": { "patientId": { "type": "string" } },
                "required": ["patientId"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "load_next_aidoo_patient",
            "Избира и показва следващия пациент от последните резултати от търсенето.",
            empty_object_schema(),
        ),
        function_tool(
            "begin_aidoo_status",
            "Показва Status за пациента и проверява дали има активно посещение. Не записва клинична промяна.",
            serde_json::json!({
                "type": "object",
                "properties": { "patientId": { "type": "string" } },
                "required": ["patientId"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "start_aidoo_status_visit",
            "Създава липсващо посещение за статус веднага след избора Частен прием или НЗОК. Не изисква второ потвърждение.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "patientId": { "type": "string" },
                    "isNzok": { "type": "boolean" }
                },
                "required": ["patientId", "isNzok"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "apply_aidoo_status",
            "Незабавно проверява, записва и прочита обратно една статусна промяна, след което обновява Status в Chrome. Не изисква потвърждение.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "patientId": { "type": "string" },
                    "isNzok": { "type": "boolean" },
                    "change": {
                        "type": "object",
                        "properties": {
                            "tooth": { "type": "string", "description": "Двуцифрен FDI номер." },
                            "status": { "type": "string", "description": "Основното име или код на AIDOO статуса, без surface mapping суфикс." },
                            "regions": { "type": "array", "items": { "type": "string", "enum": ["MESIAL", "DISTAL", "OCCLUSAL", "VESTIBULAR", "LINGUAL", "CERVICAL_LINGUAL", "CERVICAL_VESTIBULAR"] } },
                            "replaceStatus": { "type": ["string", "null"], "description": "Старият статус при корекция; null при добавяне." },
                            "isMilkTooth": { "type": "boolean" },
                            "forObservation": { "type": "boolean" },
                            "note": { "type": ["string", "null"] }
                        },
                        "required": ["tooth", "status", "regions", "replaceStatus", "isMilkTooth", "forObservation", "note"],
                        "additionalProperties": false
                    }
                },
                "required": ["patientId", "isNzok", "change"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "finish_aidoo_status",
            "Приключва статусния режим и показва Лечение. Предишните статусни промени вече са записани отделно.",
            serde_json::json!({
                "type": "object",
                "properties": { "patientId": { "type": "string" } },
                "required": ["patientId"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "get_aidoo_active_treatments",
            "Връща редовете в активното Лечение за уточнение само когато няколко реда съвпадат със същия зъб или звездичка.",
            serde_json::json!({
                "type": "object",
                "properties": { "patientId": { "type": "string" } },
                "required": ["patientId"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "add_aidoo_procedure",
            "Намира процедурата в актуалния каталог, добавя я директно към единствения ред за зъба или създава ред, проверява записа и обновява Лечение.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "patientId": { "type": "string" },
                    "tooth": { "type": "string", "description": "FDI номер или * за общ ред." },
                    "procedure": { "type": "string", "description": "Име или код на процедурата." },
                    "existingTreatmentId": { "type": ["string", "null"] }
                },
                "required": ["patientId", "tooth", "procedure", "existingTreatmentId"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "write_aidoo_diagnosis",
            "Намира диагнозата в актуалния каталог, записва я директно, проверява резултата и обновява Лечение.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "patientId": { "type": "string" },
                    "tooth": { "type": "string", "description": "FDI номер или * за общ ред." },
                    "diagnosis": { "type": "string", "description": "Име или код на диагнозата." },
                    "existingTreatmentId": { "type": ["string", "null"] }
                },
                "required": ["patientId", "tooth", "diagnosis", "existingTreatmentId"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "write_aidoo_official_note",
            "Записва дословно продиктуваната официална забележка в реда до процедурите, проверява резултата и обновява Лечение.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "patientId": { "type": "string" },
                    "tooth": { "type": "string", "description": "FDI номер или * за общ ред." },
                    "note": { "type": "string", "minLength": 1, "description": "Точният продиктуван текст без преразказ." },
                    "existingTreatmentId": { "type": ["string", "null"] }
                },
                "required": ["patientId", "tooth", "note", "existingTreatmentId"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "find_aidoo_schedule_slot",
            "Намира първия реално свободен работен слот и отваря точната дата и лекар в AIDOO График. Не създава час.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "date": { "type": ["string", "null"], "description": "YYYY-MM-DD; null търси от днес до 30 дни напред." },
                    "afterTime": { "type": "string", "description": "Местен час HH:MM, след който да започне слотът." },
                    "durationMinutes": { "type": ["integer", "null"], "description": "15–240 през 15 минути; null означава 30." },
                    "doctor": { "type": ["string", "null"], "description": "Име на лекар; null използва свързания лекар." }
                },
                "required": ["date", "afterTime", "durationMinutes", "doctor"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "book_aidoo_schedule_slot",
            "Записва пациент в последния предложен слот след повторна проверка, независимо прочитане и видимо обновяване на графика.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "slotId": { "type": "string" },
                    "patientQuery": { "type": ["string", "null"], "description": "Име или търсене за пациент при първия опит." },
                    "patientId": { "type": ["string", "null"], "description": "Избран ID само след двусмислено търсене." }
                },
                "required": ["slotId", "patientQuery", "patientId"],
                "additionalProperties": false
            }),
        ),
    ]
}

fn function_tool(
    name: &'static str,
    description: &'static str,
    parameters: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "name": name,
        "description": description,
        "parameters": parameters,
        "strict": true
    })
}

fn empty_object_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {},
        "required": [],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_actions_are_strict_and_do_not_trigger_on_incidental_words() {
        assert_eq!(local_session_action("Край."), Some("close"));
        assert_eq!(local_session_action("Затвори ми"), Some("close"));
        assert_eq!(
            local_session_action("Започни транскрипция"),
            Some("dictation")
        );
        assert_eq!(local_session_action("Крайният час е следобед"), None);
    }

    #[test]
    fn simple_patient_commands_skip_the_first_model_call() {
        let search = route_simple_command("Отвори пациент Иван Иванов").unwrap();
        assert_eq!(search.name, "search_aidoo_patients");
        assert!(search.arguments.contains("иван иванов"));
        let next = route_simple_command("Зареди следващ пациент").unwrap();
        assert_eq!(next.name, "load_next_aidoo_patient");
        assert!(route_simple_command("Кога е следващият пациент?").is_none());
    }

    #[test]
    fn tool_schema_keeps_all_aidoo_control_operations() {
        let tools = aidoo_tools();
        assert_eq!(tools.len(), 13);
        assert!(tools.iter().all(|tool| tool["strict"] == true));
        for name in [
            "search_aidoo_patients",
            "apply_aidoo_status",
            "add_aidoo_procedure",
            "write_aidoo_official_note",
            "find_aidoo_schedule_slot",
            "book_aidoo_schedule_slot",
        ] {
            assert!(tools.iter().any(|tool| tool["name"] == name));
        }
    }

    #[test]
    fn parses_text_and_function_calls_from_responses_output() {
        let output = vec![
            json!({"type":"function_call","call_id":"call_1","name":"load_next_aidoo_patient","arguments":"{}"}),
            json!({"type":"message","content":[{"type":"output_text","text":"Готово."}]}),
        ];
        assert_eq!(
            parse_tool_calls(&output).unwrap()[0].name,
            "load_next_aidoo_patient"
        );
        assert_eq!(parse_output_text(&output).as_deref(), Some("Готово."));
    }

    #[test]
    fn validates_wav_boundaries() {
        let mut wav = vec![0_u8; 44];
        wav[0..4].copy_from_slice(b"RIFF");
        wav[8..12].copy_from_slice(b"WAVE");
        assert!(validate_audio(&wav).is_ok());
        assert!(validate_audio(b"not audio").is_err());
    }

    #[test]
    fn responses_request_is_private_bounded_and_uses_strict_tools() {
        let request = response_request(&[message("user", "Отвори пациент Иван Иванов")]);
        assert_eq!(request["model"], ASSISTANT_REASONING_MODEL);
        assert_eq!(request["store"], false);
        assert_eq!(request["parallel_tool_calls"], false);
        assert_eq!(request["reasoning"]["effort"], "none");
        assert_eq!(request["include"][0], "reasoning.encrypted_content");
        assert_eq!(request["max_output_tokens"], 900);
        assert_eq!(request["input"][0]["role"], "developer");
        assert_eq!(request["input"][1]["role"], "user");
        assert!(request["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["strict"] == true));
    }
}
