use crate::models::AppSettings;
use futures_util::{StreamExt, TryStreamExt};
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;

pub type ProgressCallback = Arc<dyn Fn(u8, &str, bool) + Send + Sync>;
// OpenAI documents this limit as 25 MB. Use the decimal boundary so a file that
// passes the local guard is never larger than the documented upload maximum.
const MAX_TRANSCRIPTION_FILE_BYTES: u64 = 25_000_000;
const MAX_TRANSCRIPTION_RESPONSE_BYTES: usize = 2_000_000;
const MAX_API_ERROR_BYTES: usize = 64_000;
const MAX_API_ERROR_MESSAGE_CHARS: usize = 500;
const FILE_TOO_LARGE_PREFIX: &str = "Аудио файлът е по-голям от лимита на OpenAI от 25 MB.";

#[derive(Debug)]
pub struct TranscriptionFailure {
    pub message: String,
    pub retryable: bool,
}

impl TranscriptionFailure {
    fn before_request(message: String) -> Self {
        Self {
            retryable: !message.starts_with(FILE_TOO_LARGE_PREFIX),
            message,
        }
    }

    fn after_request(message: String, retryable: bool) -> Self {
        Self { message, retryable }
    }
}

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
}

pub fn encode_wav_to_flac(input: &Path, output: &Path) -> Result<(), String> {
    let mut reader =
        hound::WavReader::open(input).map_err(|error| format!("Невалиден WAV файл: {error}"))?;
    let spec = reader.spec();
    if !(1..=2).contains(&spec.channels) {
        return Err(format!(
            "FLAC поддържа mono/stereo, а записът има {} канала.",
            spec.channels
        ));
    }
    let mut writer = flexaudio_encode::FlacWriter::create(output, spec.sample_rate, spec.channels)
        .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    if let Err(error) = std::fs::set_permissions(output, std::fs::Permissions::from_mode(0o600)) {
        drop(writer);
        let _ = std::fs::remove_file(output);
        return Err(format!(
            "Временният аудио файл не може да бъде защитен: {error}"
        ));
    }
    let chunk_samples = 8_192 * usize::from(spec.channels);
    let mut chunk = Vec::with_capacity(chunk_samples);

    macro_rules! encode_samples {
        ($sample_type:ty, $scale:expr) => {{
            for sample in reader.samples::<$sample_type>() {
                let sample = sample.map_err(|error| error.to_string())?;
                chunk.push(sample as f32 / $scale);
                if chunk.len() == chunk_samples {
                    writer
                        .write_chunk(&chunk)
                        .map_err(|error| error.to_string())?;
                    chunk.clear();
                }
            }
        }};
    }

    match spec.sample_format {
        hound::SampleFormat::Float => encode_samples!(f32, 1.0_f32),
        hound::SampleFormat::Int if spec.bits_per_sample <= 16 => {
            encode_samples!(i16, 32_768.0_f32)
        }
        hound::SampleFormat::Int => {
            let scale = 2.0_f32.powi(i32::from(spec.bits_per_sample.saturating_sub(1)));
            encode_samples!(i32, scale);
        }
    }
    if !chunk.is_empty() {
        writer
            .write_chunk(&chunk)
            .map_err(|error| error.to_string())?;
    }
    writer.finalize().map_err(|error| error.to_string())
}

async fn streamed_audio_part(
    path: &Path,
    progress: Option<ProgressCallback>,
) -> Result<Part, String> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|error| error.to_string())?;
    let total = file
        .metadata()
        .await
        .map_err(|error| error.to_string())?
        .len();
    if total > MAX_TRANSCRIPTION_FILE_BYTES {
        return Err(format!(
            "{FILE_TOO_LARGE_PREFIX} Направете по-кратък запис."
        ));
    }
    let total = total.max(1);
    let progress_for_stream = progress.clone();
    let stream = futures_util::stream::try_unfold((file, 0_u64), move |(mut file, uploaded)| {
        let progress = progress_for_stream.clone();
        async move {
            let mut chunk = vec![0_u8; 64 * 1024];
            let read = file.read(&mut chunk).await?;
            if read == 0 {
                if let Some(callback) = progress.as_ref() {
                    callback(72, "openai_transcribing", false);
                }
                return Ok::<_, std::io::Error>(None);
            }
            chunk.truncate(read);
            let uploaded = uploaded.saturating_add(read as u64).min(total);
            let percent = ((uploaded.saturating_mul(70)) / total) as u8;
            if let Some(callback) = progress.as_ref() {
                callback(percent, "uploading_audio", true);
            }
            Ok(Some((chunk, (file, uploaded))))
        }
    });
    let body = reqwest::Body::wrap_stream(stream.map_err(std::io::Error::other));
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("dictation.flac");
    Part::stream_with_length(body, total)
        .file_name(name.to_string())
        .mime_str(
            if path.extension().and_then(|v| v.to_str()) == Some("flac") {
                "audio/flac"
            } else {
                "audio/wav"
            },
        )
        .map_err(|error| error.to_string())
}

pub async fn validate_api_key(api_key: &str) -> Result<(), String> {
    if !api_key.trim().starts_with("sk-") {
        return Err("Ключът трябва да започва с sk-.".into());
    }
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("OpenAI връзката не можа да бъде подготвена: {error}"))?;
    let response = client
        .get("https://api.openai.com/v1/models")
        .bearer_auth(api_key.trim())
        .send()
        .await
        .map_err(|error| format!("Няма връзка с OpenAI: {error}"))?;
    if response.status().is_success() || response.status() == reqwest::StatusCode::FORBIDDEN {
        return Ok(());
    }
    Err(api_error(response, "API ключът не беше приет").await)
}

pub async fn transcribe(
    path: &Path,
    api_key: &str,
    settings: &AppSettings,
    progress: Option<ProgressCallback>,
) -> Result<String, TranscriptionFailure> {
    let audio = streamed_audio_part(path, progress.clone())
        .await
        .map_err(TranscriptionFailure::before_request)?;
    let mut form = Form::new()
        .part("file", audio)
        .text("model", settings.model.clone())
        .text("response_format", "json");
    if settings.language != "auto" {
        form = form.text("language", settings.language.clone());
    }
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|error| {
            TranscriptionFailure::before_request(format!(
                "OpenAI връзката не можа да бъде подготвена: {error}"
            ))
        })?;
    let response = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
        .map_err(|error| {
            let retryable = error.is_builder() || error.is_connect();
            TranscriptionFailure::after_request(format!("Няма връзка с OpenAI: {error}"), retryable)
        })?;
    if !response.status().is_success() {
        let retryable = response_status_is_retry_safe(response.status());
        return Err(TranscriptionFailure::after_request(
            api_error(response, "Транскрипцията не успя").await,
            retryable,
        ));
    }
    let body = read_response_body(response, MAX_TRANSCRIPTION_RESPONSE_BYTES)
        .await
        .map_err(|error| {
            TranscriptionFailure::after_request(
                format!("OpenAI върна невалиден отговор: {error}"),
                false,
            )
        })?;
    let result: TranscriptionResponse = serde_json::from_slice(&body).map_err(|error| {
        TranscriptionFailure::after_request(
            format!("OpenAI върна невалиден отговор: {error}"),
            false,
        )
    })?;
    let text = result.text.trim().to_string();
    if text.is_empty() {
        return Err(TranscriptionFailure::after_request(
            "Не беше разпозната реч.".into(),
            false,
        ));
    }
    if let Some(callback) = progress {
        callback(100, "text_ready", true);
    }
    Ok(text)
}

fn response_status_is_retry_safe(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 400 | 401 | 403 | 404 | 409 | 422 | 429)
}

async fn api_error(response: reqwest::Response, prefix: &str) -> String {
    let status = response.status();
    let body = read_response_body(response, MAX_API_ERROR_BYTES)
        .await
        .unwrap_or_default();
    let message = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| value.pointer("/error/message")?.as_str().map(str::to_owned))
        .map(|message| compact_api_message(&message))
        .unwrap_or_else(|| format!("HTTP {status}"));
    match status.as_u16() {
        401 => format!("{prefix}: невалиден или изтрит API ключ."),
        429 if message.to_lowercase().contains("quota") => {
            format!("{prefix}: няма наличен API баланс или е достигнат лимитът.")
        }
        _ => format!("{prefix}: {message}"),
    }
}

async fn read_response_body(
    response: reqwest::Response,
    maximum_bytes: usize,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum_bytes as u64)
    {
        return Err("отговорът надвишава безопасния лимит.".into());
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| error.to_string())?;
        if body.len().saturating_add(chunk.len()) > maximum_bytes {
            return Err("отговорът надвишава безопасния лимит.".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn compact_api_message(message: &str) -> String {
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= MAX_API_ERROR_MESSAGE_CHARS {
        normalized
    } else {
        format!(
            "{}…",
            normalized
                .chars()
                .take(MAX_API_ERROR_MESSAGE_CHARS - 1)
                .collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        compact_api_message, encode_wav_to_flac, response_status_is_retry_safe,
        TranscriptionFailure,
    };

    #[test]
    fn api_error_messages_are_single_line_and_bounded() {
        let message = format!("  first\n\tsecond {}", "x".repeat(700));
        let compact = compact_api_message(&message);
        assert!(!compact.contains('\n'));
        assert_eq!(compact.chars().count(), 500);
        assert!(compact.ends_with('…'));
    }

    #[test]
    fn retry_safety_distinguishes_preflight_and_ambiguous_api_failures() {
        assert!(
            !TranscriptionFailure::before_request(
                "Аудио файлът е по-голям от лимита на OpenAI от 25 MB. Направете по-кратък запис."
                    .into()
            )
            .retryable
        );
        assert!(TranscriptionFailure::before_request("file unavailable".into()).retryable);
        assert!(response_status_is_retry_safe(
            reqwest::StatusCode::UNAUTHORIZED
        ));
        assert!(response_status_is_retry_safe(
            reqwest::StatusCode::TOO_MANY_REQUESTS
        ));
        assert!(!response_status_is_retry_safe(
            reqwest::StatusCode::REQUEST_TIMEOUT
        ));
        assert!(!response_status_is_retry_safe(
            reqwest::StatusCode::PAYLOAD_TOO_LARGE
        ));
        assert!(!response_status_is_retry_safe(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        ));
    }

    #[test]
    fn wav_is_encoded_as_flac() {
        let root =
            std::env::temp_dir().join(format!("aidoo-lite-flac-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let wav = root.join("sample.wav");
        let flac = root.join("sample.flac");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&wav, spec).unwrap();
        for i in 0..16_000 {
            writer
                .write_sample::<i16>(((i as f32 / 14.0).sin() * 4_000.0) as i16)
                .unwrap();
        }
        writer.finalize().unwrap();
        encode_wav_to_flac(&wav, &flac).unwrap();
        assert!(std::fs::read(&flac).unwrap().starts_with(b"fLaC"));
        let _ = std::fs::remove_dir_all(root);
    }
}
