use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const LIVE_MODEL: &str = "gpt-live-1";
pub const LIVE_BACKEND_MODEL: &str = "gpt-5.6-terra";
const LIVE_SESSION_ENDPOINT: &str = "https://api.openai.com/v1/live/sessions";
const MAX_SDP_BYTES: usize = 128 * 1024;
const MAX_LIVE_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_API_ERROR_BYTES: usize = 64 * 1024;

const LIVE_INSTRUCTIONS: &str = "Говори на български, освен ако потребителят не поиска друг език. Бъди кратък, естествен и ясен. Това е разговор с AIDOO асистента, а не диктовка. Когато потребителят каже „Започни транскрипция“, приложението ще премине към отделния режим за запис. Когато каже „Край“, „Затвори“, „Приключи разговора“, „Приключваме“, „Спри асистента“ или „Довиждане“, приложението ще затвори сесията. Приемай FDI номер на зъб, изговорен като две отделни цифри: „едно шест“ означава 16, „две шест“ означава 26, „три шест“ означава 36 и „четири шест“ означава 46; прилагай същото правило за всички валидни FDI номера. Делегирай всяка задача за AIDOO Kontrol към backend модела. Не твърди, че действие е извършено, преди инструментът да върне успех. Преди създаване на посещение или клиничен запис кажи с глас точно какво ще направиш и поискай ясно „Да“ или „Потвърждавам“. При „Запиши официална забележка“ изслушай текста, уточни зъба или реда за лечение и го подготви като забележка до процедурите.";
const BACKEND_INSTRUCTIONS: &str = "Управляваш AIDOO Kontrol чрез предоставените инструменти. Отговаряй на български, кратко и проверимо. Никога не измисляй пациент, ID, статус, диагноза, лечение, процедура, повърхност или резултат. Нормализирай FDI номер, изговорен като две отделни цифри: „едно шест“ е 16, „две шест“ е 26, „три шест“ е 36 и „четири шест“ е 46; подавай към инструментите двуцифрения низ. При избор на пациент първо търси и при повече от един резултат поискай уточнение. За посещение за статус уточни дали е по НЗОК или частно, опиши действието и поискай гласово потвърждение; извикай create_aidoo_status_visit едва след ясно „Да“, „Потвърждавам“ или „Потвърди“. За статус първо вземи актуалния каталог, после подготви чернова. Прочети дословно spokenSummary и изчакай отделно гласово потвърждение. Едва тогава извикай confirm_aidoo_status. За корекция използвай operation=replace и existingStatusId. За няколко промени ги подай заедно. Преди диагноза, процедура или официална забележка извикай get_aidoo_active_treatments. При повече от един подходящ ред за същия зъб опиши ги кратко и поискай избор; не избирай сам. Вземи каталога за диагнози само при диагноза и каталога за процедури само при процедури. Не задавай нов treatmentId, защото няма проверим каталог за съвместимост; използвай null и точния existingTreatmentId. При фразата „Запиши официална забележка“ поискай текста и точния зъб или ред за лечение; note е точният продиктуван текст. Прочети дословно spokenSummary и потвърди чрез confirm_aidoo_treatment само след отделно гласово потвърждение. Ако процедура вече съществува или AIDOO отхвърли несъвместима комбинация, съобщи резултата и не повтаряй автоматично. Ако проверката върне uncertain, rejected или staleDraft, съобщи ясно, че записът не е потвърден, и не повтаряй автоматично.";

#[derive(Debug, Serialize)]
struct LiveCreateRequest<'a> {
    session: LiveSessionConfig,
    transport: LiveTransportOffer<'a>,
}

#[derive(Debug, Serialize)]
struct LiveSessionConfig {
    model: &'static str,
    instructions: &'static str,
    client: LiveClientConfig,
    delegation: LiveDelegation,
    store: bool,
}

#[derive(Debug, Serialize)]
struct LiveClientConfig {
    data_channel: LiveDataChannelConfig,
}

#[derive(Debug, Serialize)]
struct LiveDataChannelConfig {
    allowed_client_events: Vec<&'static str>,
    allowed_server_events: Vec<LiveServerEventSelector>,
}

#[derive(Debug, Serialize)]
struct LiveServerEventSelector {
    r#type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_event: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct LiveDelegation {
    r#type: &'static str,
    responses: LiveResponsesConfig,
}

#[derive(Debug, Serialize)]
struct LiveResponsesConfig {
    model: &'static str,
    instructions: &'static str,
    tools: Vec<serde_json::Value>,
    tool_choice: &'static str,
    parallel_tool_calls: bool,
}

#[derive(Debug, Serialize)]
struct LiveTransportOffer<'a> {
    r#type: &'static str,
    sdp: &'a str,
}

#[derive(Debug, Deserialize)]
struct OpenAiLiveCreateResponse {
    session: OpenAiLiveSession,
    transport: OpenAiLiveTransport,
}

#[derive(Debug, Deserialize)]
struct OpenAiLiveSession {
    id: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiLiveTransport {
    r#type: String,
    sdp: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveSessionAnswer {
    pub session_id: String,
    pub sdp: String,
}

fn validate_sdp(sdp: &str) -> Result<&str, String> {
    if sdp.is_empty() || !sdp.starts_with("v=0") {
        return Err("Невалидна WebRTC заявка.".into());
    }
    if sdp.len() > MAX_SDP_BYTES {
        return Err("WebRTC заявката е прекалено голяма.".into());
    }
    Ok(sdp)
}

fn create_request(sdp: &str) -> Result<LiveCreateRequest<'_>, String> {
    let sdp = validate_sdp(sdp)?;
    Ok(LiveCreateRequest {
        session: LiveSessionConfig {
            model: LIVE_MODEL,
            instructions: LIVE_INSTRUCTIONS,
            client: LiveClientConfig {
                data_channel: LiveDataChannelConfig {
                    allowed_client_events: vec![
                        "session.close",
                        "response.item.create",
                        "response.create",
                    ],
                    allowed_server_events: vec![
                        LiveServerEventSelector {
                            r#type: "session.started",
                            response_event: None,
                        },
                        LiveServerEventSelector {
                            r#type: "session.input_transcript.delta",
                            response_event: None,
                        },
                        LiveServerEventSelector {
                            r#type: "session.closed",
                            response_event: None,
                        },
                        LiveServerEventSelector {
                            r#type: "error",
                            response_event: None,
                        },
                        LiveServerEventSelector {
                            r#type: "response.event",
                            response_event: Some("response.output_item.done"),
                        },
                    ],
                },
            },
            delegation: LiveDelegation {
                r#type: "responses",
                responses: LiveResponsesConfig {
                    model: LIVE_BACKEND_MODEL,
                    instructions: BACKEND_INSTRUCTIONS,
                    tools: aidoo_tools(),
                    tool_choice: "auto",
                    parallel_tool_calls: false,
                },
            },
            store: false,
        },
        transport: LiveTransportOffer {
            r#type: "webrtc",
            sdp,
        },
    })
}

fn aidoo_tools() -> Vec<serde_json::Value> {
    vec![
        function_tool(
            "search_aidoo_patients",
            "Търси пациент в AIDOO. Използвай поне четири знака и не избирай при двусмислен резултат.",
            serde_json::json!({
                "type": "object",
                "properties": { "query": { "type": "string", "minLength": 4 } },
                "required": ["query"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "get_aidoo_status_catalog",
            "Връща актуалния каталог от статуси и техните ID. Извикай преди подготовка на зъбен статус.",
            empty_object_schema(),
        ),
        function_tool(
            "create_aidoo_status_visit",
            "Създава посещение за статус по НЗОК или частно и създава статусния запис, само след отделно ясно гласово потвърждение.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "patientId": { "type": "string" },
                    "isNzok": { "type": "boolean" },
                    "confirmation": { "type": "string", "description": "Точната потвърждаваща фраза на потребителя." }
                },
                "required": ["patientId", "isNzok", "confirmation"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "prepare_aidoo_status",
            "Чете актуалното посещение и статус, проверява каталога и подготвя чернова без запис.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "patientId": { "type": "string" },
                    "isNzok": { "type": "boolean" },
                    "changes": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "operation": { "type": "string", "enum": ["add", "replace"] },
                                "tooth": { "type": "string" },
                                "statusId": { "type": "string" },
                                "regions": { "type": "array", "items": { "type": "string", "enum": ["MESIAL", "DISTAL", "OCCLUSAL", "VESTIBULAR", "LINGUAL", "PALATAL", "CERVICAL_LINGUAL", "CERVICAL_VESTIBULAR", "CERVICAL_PALATAL"] } },
                                "existingStatusId": { "type": ["string", "null"] },
                                "isMilkTooth": { "type": "boolean" },
                                "forObservation": { "type": "boolean" },
                                "note": { "type": ["string", "null"] }
                            },
                            "required": ["operation", "tooth", "statusId", "regions", "existingStatusId", "isMilkTooth", "forObservation", "note"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["patientId", "isNzok", "changes"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "confirm_aidoo_status",
            "Записва подготвената чернова и прави независимо read-back потвърждение. Използвай само след гласово потвърждение.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "draftId": { "type": "string" },
                    "confirmation": { "type": "string", "description": "Точната потвърждаваща фраза на потребителя." }
                },
                "required": ["draftId", "confirmation"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "cancel_aidoo_status",
            "Изтрива само локалната непотвърдена чернова. Не променя AIDOO.",
            empty_object_schema(),
        ),
        function_tool(
            "get_aidoo_diagnosis_catalog",
            "Връща актуалния каталог от диагнози и ID.",
            empty_object_schema(),
        ),
        function_tool(
            "get_aidoo_procedure_catalog",
            "Връща актуалния каталог от процедури, ID и цени.",
            empty_object_schema(),
        ),
        function_tool(
            "get_aidoo_active_treatments",
            "Връща treatment редовете от активното посещение. Използвай преди диагноза, процедура или официална забележка и поискай уточнение при повече от един ред за зъба.",
            serde_json::json!({
                "type": "object",
                "properties": { "patientId": { "type": "string" } },
                "required": ["patientId"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "prepare_aidoo_treatment",
            "Подготвя без запис диагноза, процедури и/или официална забележка в реда до процедурите.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "patientId": { "type": "string" },
                    "change": {
                        "type": "object",
                        "properties": {
                            "tooth": { "type": "string" },
                            "existingTreatmentId": { "type": ["string", "null"] },
                            "diagnosisId": { "type": ["string", "null"] },
                            "treatmentId": { "type": ["string", "null"] },
                            "note": { "type": ["string", "null"], "description": "Точният продиктуван текст на официалната забележка." },
                            "procedureIds": { "type": "array", "items": { "type": "string" } }
                        },
                        "required": ["tooth", "existingTreatmentId", "diagnosisId", "treatmentId", "note", "procedureIds"],
                        "additionalProperties": false
                    }
                },
                "required": ["patientId", "change"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "confirm_aidoo_treatment",
            "Записва подготвените диагноза, процедури и официална забележка и прави независимо read-back потвърждение.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "draftId": { "type": "string" },
                    "confirmation": { "type": "string" }
                },
                "required": ["draftId", "confirmation"],
                "additionalProperties": false
            }),
        ),
        function_tool(
            "cancel_aidoo_treatment",
            "Изтрива локалната непотвърдена чернова за диагноза, процедури и забележка.",
            empty_object_schema(),
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

pub async fn create_session(sdp: &str, api_key: &str) -> Result<LiveSessionAnswer, String> {
    let request = create_request(sdp)?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|error| format!("GPT-Live връзката не можа да бъде подготвена: {error}"))?;
    let response = client
        .post(LIVE_SESSION_ENDPOINT)
        .bearer_auth(api_key)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("Няма връзка с GPT-Live: {error}"))?;
    if !response.status().is_success() {
        return Err(live_api_error(response).await);
    }
    let body = read_limited_body(response, MAX_LIVE_RESPONSE_BYTES).await?;
    let response: OpenAiLiveCreateResponse = serde_json::from_slice(&body)
        .map_err(|error| format!("GPT-Live върна невалиден отговор: {error}"))?;
    if response.session.id.trim().is_empty()
        || response.transport.r#type != "webrtc"
        || response.transport.sdp.trim().is_empty()
    {
        return Err("GPT-Live върна непълна WebRTC сесия.".into());
    }
    Ok(LiveSessionAnswer {
        session_id: response.session.id,
        sdp: response.transport.sdp,
    })
}

async fn live_api_error(response: reqwest::Response) -> String {
    let status = response.status();
    let body = read_limited_body(response, MAX_API_ERROR_BYTES)
        .await
        .unwrap_or_default();
    let message = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| value.pointer("/error/message")?.as_str().map(str::to_owned))
        .map(|message| message.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| format!("HTTP {status}"));
    match status.as_u16() {
        401 => "GPT-Live не прие API ключа.".into(),
        403 => "Този OpenAI проект няма достъп до GPT-Live.".into(),
        429 if message.to_lowercase().contains("quota") => {
            "Няма наличен OpenAI API баланс или е достигнат лимитът.".into()
        }
        _ => format!(
            "GPT-Live не можа да стартира: {}",
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
        return Err("GPT-Live отговорът надвишава безопасния лимит.".into());
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("GPT-Live отговорът е прекъснат: {error}"))?;
        if body.len().saturating_add(chunk.len()) > maximum_bytes {
            return Err("GPT-Live отговорът надвишава безопасния лимит.".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_request_uses_reviewed_models_and_webrtc_transport() {
        let request = create_request("v=0\r\ns=test\r\n").unwrap();
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["session"]["model"], LIVE_MODEL);
        assert_eq!(value["session"]["store"], false);
        assert_eq!(
            value["session"]["client"]["data_channel"]["allowed_client_events"],
            serde_json::json!(["session.close", "response.item.create", "response.create"])
        );
        assert_eq!(
            value["session"]["client"]["data_channel"]["allowed_server_events"],
            serde_json::json!([
                {"type": "session.started"},
                {"type": "session.input_transcript.delta"},
                {"type": "session.closed"},
                {"type": "error"},
                {"type": "response.event", "response_event": "response.output_item.done"}
            ])
        );
        assert_eq!(value["session"]["delegation"]["type"], "responses");
        assert_eq!(
            value["session"]["delegation"]["responses"]["model"],
            LIVE_BACKEND_MODEL
        );
        let tools = value["session"]["delegation"]["responses"]["tools"]
            .as_array()
            .unwrap();
        assert_eq!(tools.len(), 12);
        assert!(tools.iter().all(|tool| tool["strict"] == true));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "prepare_aidoo_status"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "confirm_aidoo_status"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "create_aidoo_status_visit"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "prepare_aidoo_treatment"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "get_aidoo_active_treatments"));
        assert_eq!(value["transport"]["type"], "webrtc");
        assert_eq!(value["transport"]["sdp"], "v=0\r\ns=test\r\n");
    }

    #[test]
    fn live_instructions_normalize_spoken_fdi_tooth_numbers() {
        let request = create_request("v=0\r\ns=test\r\n").unwrap();
        let value = serde_json::to_value(request).unwrap();
        assert!(value["session"]["instructions"]
            .as_str()
            .unwrap()
            .contains("„едно шест“ означава 16"));
        assert!(value["session"]["delegation"]["responses"]["instructions"]
            .as_str()
            .unwrap()
            .contains("„едно шест“ е 16"));
    }

    #[test]
    fn live_request_rejects_empty_invalid_and_oversized_sdp() {
        assert!(create_request("").is_err());
        assert!(create_request("not-sdp").is_err());
        let oversized = format!("v=0{}", "x".repeat(MAX_SDP_BYTES));
        assert!(create_request(&oversized).is_err());
    }

    #[test]
    fn live_response_requires_session_id_webrtc_and_sdp() {
        let response: OpenAiLiveCreateResponse = serde_json::from_value(serde_json::json!({
            "session": { "id": "live_123" },
            "transport": { "type": "webrtc", "sdp": "v=0\\r\\n" }
        }))
        .unwrap();
        assert_eq!(response.session.id, "live_123");
        assert_eq!(response.transport.r#type, "webrtc");
        assert!(!response.transport.sdp.is_empty());
    }
}
