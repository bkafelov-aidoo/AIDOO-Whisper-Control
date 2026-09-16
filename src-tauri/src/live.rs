use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const LIVE_MODEL: &str = "gpt-live-1";
pub const LIVE_BACKEND_MODEL: &str = "gpt-5.6-terra";
const LIVE_SESSION_ENDPOINT: &str = "https://api.openai.com/v1/live/sessions";
const MAX_SDP_BYTES: usize = 128 * 1024;
const MAX_LIVE_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_API_ERROR_BYTES: usize = 64 * 1024;

const LIVE_INSTRUCTIONS: &str = "Говори на български, освен ако потребителят не поиска друг език. Бъди кратък, естествен и ясен. Това е тестов режим на AIDOO асистента: не твърди, че си променил стоматологични данни и не измисляй пациенти, статуси или резултати. Делегирай към backend модела, когато задачата изисква повече разсъждение. Кажи ясно, че интеграцията с AIDOO Kontrol още не е активна, ако потребителят поиска действие в нея.";
const BACKEND_INSTRUCTIONS: &str = "Отговаряй на български с кратък, проверим резултат, подходящ за гласов разговор. В този тест няма свързани AIDOO Kontrol инструменти. Не твърди, че са извършени действия и не създавай пациентски или клинични данни.";

#[derive(Debug, Serialize)]
struct LiveCreateRequest<'a> {
    session: LiveSessionConfig,
    transport: LiveTransportOffer<'a>,
}

#[derive(Debug, Serialize)]
struct LiveSessionConfig {
    model: &'static str,
    instructions: &'static str,
    delegation: LiveDelegation,
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
            delegation: LiveDelegation {
                r#type: "responses",
                responses: LiveResponsesConfig {
                    model: LIVE_BACKEND_MODEL,
                    instructions: BACKEND_INSTRUCTIONS,
                },
            },
        },
        transport: LiveTransportOffer {
            r#type: "webrtc",
            sdp,
        },
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
        assert_eq!(value["session"]["delegation"]["type"], "responses");
        assert_eq!(
            value["session"]["delegation"]["responses"]["model"],
            LIVE_BACKEND_MODEL
        );
        assert_eq!(value["transport"]["type"], "webrtc");
        assert_eq!(value["transport"]["sdp"], "v=0\r\ns=test\r\n");
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
