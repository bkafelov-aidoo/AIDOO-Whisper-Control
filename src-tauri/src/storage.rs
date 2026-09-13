use crate::models::{AppSettings, FailedRecording, TranscriptEntry};
use chrono::Utc;
use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("AIDOO Whisper Lite")
}

pub fn default_output_dir() -> PathBuf {
    dirs::document_dir()
        .unwrap_or_else(data_dir)
        .join("AIDOO Whisper Lite")
        .join("Transcriptions")
}

pub fn recovery_dir() -> PathBuf {
    data_dir().join("Recovery")
}

fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

fn history_path() -> PathBuf {
    data_dir().join("history.json")
}

fn failed_recording_path() -> PathBuf {
    data_dir().join("failed-recording.json")
}

pub fn diagnostics_path() -> PathBuf {
    data_dir().join("diagnostics.log")
}

pub fn ensure_directories() -> Result<(), String> {
    fs::create_dir_all(data_dir()).map_err(|error| error.to_string())?;
    fs::create_dir_all(recovery_dir()).map_err(|error| error.to_string())?;
    Ok(())
}

pub fn load_settings() -> AppSettings {
    let mut settings: AppSettings = read_json(&settings_path()).unwrap_or_default();
    settings.normalize();
    settings
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    write_json_atomic(&settings_path(), settings)
}

pub fn load_history() -> Vec<TranscriptEntry> {
    let mut history: Vec<TranscriptEntry> = read_json(&history_path()).unwrap_or_default();
    history.truncate(10);
    history
}

pub fn save_history(history: &[TranscriptEntry]) -> Result<(), String> {
    let bounded = history.iter().take(10).cloned().collect::<Vec<_>>();
    write_json_atomic(&history_path(), &bounded)
}

pub fn load_failed_recording() -> Option<FailedRecording> {
    read_json(&failed_recording_path())
}

pub fn save_failed_recording(recording: &FailedRecording) -> Result<(), String> {
    write_json_atomic(&failed_recording_path(), recording)
}

pub fn clear_failed_recording() -> Result<(), String> {
    match fs::remove_file(failed_recording_path()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn append_diagnostic(message: &str) {
    if ensure_directories().is_err() {
        return;
    }
    let sanitized = sanitize_diagnostic(message);
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(diagnostics_path())
    {
        let _ = writeln!(file, "{} {}", Utc::now().to_rfc3339(), sanitized);
    }
    trim_diagnostics();
}

pub fn append_shortcut_diagnostic(message: &str) {
    append_diagnostic(&format!("shortcut: {message}"));
}

pub fn sanitize_diagnostic(message: &str) -> String {
    redact_openai_keys(message)
        .split_whitespace()
        .map(|token| {
            if token.len() > 180 {
                "[redacted]"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn redact_openai_keys(message: &str) -> String {
    let mut value = message.to_string();
    while let Some(start) = value.find("sk-") {
        let length = value[start..]
            .chars()
            .take_while(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
            .map(char::len_utf8)
            .sum::<usize>();
        value.replace_range(start..start + length.max(3), "[redacted]");
    }
    value
}

fn trim_diagnostics() {
    let path = diagnostics_path();
    let Ok(metadata) = fs::metadata(&path) else {
        return;
    };
    if metadata.len() <= 1_000_000 {
        return;
    }
    if let Ok(contents) = fs::read(&path) {
        let keep_from = contents.len().saturating_sub(500_000);
        let _ = fs::write(path, &contents[keep_from..]);
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_json_atomic<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let temporary = path.with_extension("tmp");
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::sanitize_diagnostic;

    #[test]
    fn diagnostics_redact_openai_keys() {
        let key = ["sk", "example-secret-value"].join("-");
        let value = sanitize_diagnostic(&format!("request failed for key={key}, retry"));
        assert!(!value.contains("sk-example"));
        assert!(value.contains("[redacted]"));
    }
}
