use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::models::{LIVE_BACKEND_MODEL, LIVE_MODEL};

const LIVE_SESSION_ENDPOINT: &str = "https://api.openai.com/v1/live/sessions";
const MAX_SDP_BYTES: usize = 128 * 1024;
const MAX_LIVE_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_API_ERROR_BYTES: usize = 64 * 1024;

const LIVE_INSTRUCTIONS: &str = "Говори на български, освен ако потребителят не поиска друг език. Бъди кратък, естествен и ясен. Това е разговор с AIDOO асистента, а не диктовка. Когато потребителят каже „Започни транскрипция“, приложението ще премине към отделния режим за запис. Когато каже „Край“, „Затвори“, „Приключи разговора“, „Приключваме“, „Спри асистента“ или „Довиждане“, приложението ще затвори сесията. Приемай FDI номер на зъб, изговорен като две отделни цифри: „едно шест“ означава 16, „две шест“ означава 26, „три шест“ означава 36 и „четири шест“ означава 46; прилагай същото правило за всички валидни FDI номера. Делегирай всяка задача за AIDOO Control към backend модела. Не твърди, че действие е извършено, преди инструментът да върне резултат. В клиничния режим не искай „Да“ или „Потвърждавам“ за всеки статус, диагноза, процедура или забележка. След успешен запис повтори накратко какво е разпознато и записано; потребителят ще прекъсне и ще коригира, ако не е съгласен. Питай само когато пациентът, видът на новото посещение, зъбът, процедурата или treatment редът са действително двусмислени. При „Добави официална забележка“ покани потребителя да продиктува текста, изслушай го дословно и го изпрати за директен запис.";
const BACKEND_INSTRUCTIONS: &str = "Управляваш AIDOO Control чрез предоставените инструменти. Отговаряй на български, възможно най-кратко и проверимо. Никога не измисляй пациент, ID, статус, диагноза, процедура, повърхност, свободен час или резултат. Нормализирай FDI номер, изговорен като две отделни цифри: „едно шест“ е 16, „две шест“ е 26, „три шест“ е 36 и „четири шест“ е 46. Протокол: 1) При „Намери пациент X“ извикай search_aidoo_patients. Единственият резултат се избира и показва автоматично; при няколко резултата поискай едно кратко уточнение и извикай select_aidoo_patient. При „Зареди следващ пациент“ извикай load_next_aidoo_patient. Запомни избрания patientId за следващите действия. 2) При „Попълни статус“, „Отвори статус“ и сходни фрази извикай begin_aidoo_status. Ако резултатът needsVisit=true, попитай само „Частен прием или НЗОК?“ и след отговора извикай start_aidoo_status_visit без допълнително потвърждение. За всяка продиктувана статусна промяна веднага извикай apply_aidoo_status. Не подготвяй чернова и не искай „Да“. За статус с повърхности подай основното име, например „Кариес“, и regions отделно; OCCLUSAL е оклузално, LINGUAL е палатинално/лингвално, CERVICAL_LINGUAL е цервикално-палатинално. За корекция подай стария статус в replaceStatus. След резултата кажи само краткото spokenSummary; при verified или verifiedAfterAmbiguousWrite промяната е записана и екранът вече е обновен. При uncertain, rejected или staleDraft кажи ясно, че записът не е потвърден, и не повтаряй автоматично. 3) При „Запиши статуса“ извикай finish_aidoo_status; това приключва статусния режим и показва Лечение, защото отделните статуси вече са записани и проверени. 4) При „Запиши процедура X“ използвай add_aidoo_procedure. Ако липсва зъб, попитай само „Кой зъб или звездичка?“. Подай името или кода на процедурата; каталогът и цената се проверяват автоматично. Ако има няколко treatment реда за зъба, извикай get_aidoo_active_treatments, опиши ги кратко и поискай избор. Не искай потвърждение след избора. 5) При „Добави официална забележка“ поискай зъб или звездичка, ако липсва, после кажи „Диктувайте забележката“. Изпрати точния продиктуван текст чрез write_aidoo_official_note и след успех кажи само „Официалната забележка е записана.“ 6) При диагноза използвай write_aidoo_diagnosis по същия директен протокол. 7) При „Намери първия свободен час след 12:00“ и сходни фрази извикай find_aidoo_schedule_slot. Подай date като YYYY-MM-DD само ако потребителят е посочил дата; иначе null, за да се търси от днес напред. Подай durationMinutes или null за стандартни 30 минути и doctor или null за свързания лекар. Инструментът проверява работните интервали, лекаря и кабинета и веднага отваря точната дата в График. Кажи намерените дата, час, лекар и продължителност. 8) „Запиши пациент X в този час“ е изрично нареждане за запис: извикай book_aidoo_schedule_slot с последния slotId и patientQuery=X, без допълнително „Да“. При няколко пациента поискай едно уточнение и извикай инструмента пак със slotId, patientQuery=null и избрания patientId. Инструментът проверява повторно дали слотът е свободен, записва веднъж, прочита обратно и обновява същата страница на графика. Кажи само spokenSummary. При uncertain не повтаряй автоматично. Всички write инструменти правят независимо read-back и обновяват правилния екран в Chrome. Не карай потребителя да навигира ръчно и не добавяй междинни потвърждения.";

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
                        "session.instructions.append",
                        "session.commentary.append",
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
                            r#type: "session.instructions.appended",
                            response_event: None,
                        },
                        LiveServerEventSelector {
                            r#type: "session.commentary.appended",
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
                        LiveServerEventSelector {
                            r#type: "response.event",
                            response_event: Some("response.completed"),
                        },
                        LiveServerEventSelector {
                            r#type: "response.event",
                            response_event: Some("response.incomplete"),
                        },
                        LiveServerEventSelector {
                            r#type: "response.event",
                            response_event: Some("response.failed"),
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
            serde_json::json!([
                "session.close",
                "session.instructions.append",
                "session.commentary.append",
                "response.item.create",
                "response.create"
            ])
        );
        assert_eq!(
            value["session"]["client"]["data_channel"]["allowed_server_events"],
            serde_json::json!([
                {"type": "session.started"},
                {"type": "session.input_transcript.delta"},
                {"type": "session.instructions.appended"},
                {"type": "session.commentary.appended"},
                {"type": "session.closed"},
                {"type": "error"},
                {"type": "response.event", "response_event": "response.output_item.done"},
                {"type": "response.event", "response_event": "response.completed"},
                {"type": "response.event", "response_event": "response.incomplete"},
                {"type": "response.event", "response_event": "response.failed"}
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
        assert_eq!(tools.len(), 13);
        assert!(tools.iter().all(|tool| tool["strict"] == true));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "begin_aidoo_status"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "apply_aidoo_status"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "start_aidoo_status_visit"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "add_aidoo_procedure"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "get_aidoo_active_treatments"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "write_aidoo_official_note"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "find_aidoo_schedule_slot"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "book_aidoo_schedule_slot"));
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
        assert!(value["session"]["delegation"]["responses"]["instructions"]
            .as_str()
            .unwrap()
            .contains("apply_aidoo_status"));
        assert!(value["session"]["delegation"]["responses"]["instructions"]
            .as_str()
            .unwrap()
            .contains("не искай „Да“"));
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
