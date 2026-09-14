use crate::models::{AppSettings, FailedRecording, TranscriptEntry};
use chrono::Utc;
use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
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
    let data = data_dir();
    let recovery = recovery_dir();
    ensure_private_directory(&data)?;
    ensure_private_directory(&recovery)?;
    Ok(())
}

fn ensure_private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| error.to_string())?;
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_dir() {
        return Err(format!(
            "Частната папка на приложението не е валидна: {}",
            path.display()
        ));
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
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
    let recording: FailedRecording = read_json(&failed_recording_path())?;
    let path = PathBuf::from(&recording.path);
    let name = path.file_name()?.to_str()?;
    let extension = path.extension()?.to_str()?;
    let valid_name = name.starts_with("failed-dictation-")
        || matches!(
            name,
            "last-failed-dictation.flac" | "last-failed-dictation.wav"
        );
    let recovery = recovery_dir();
    let is_regular_file = fs::symlink_metadata(&path)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_file());
    if path.parent() == Some(recovery.as_path())
        && valid_name
        && matches!(extension.to_ascii_lowercase().as_str(), "flac" | "wav")
        && is_regular_file
    {
        Some(recording)
    } else {
        None
    }
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
    let path = diagnostics_path();
    if fs::symlink_metadata(&path)
        .ok()
        .is_some_and(|metadata| !metadata.file_type().is_file())
    {
        return;
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    options.mode(0o600);
    if let Ok(mut file) = options.open(path) {
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

pub(crate) fn sanitize_support_text(message: &str, transcripts: &[String]) -> String {
    let mut value = redact_openai_keys(message);
    if let Some(home) = dirs::home_dir().and_then(|path| path.to_str().map(str::to_owned)) {
        value = value.replace(&home, "~");
    }
    let mut transcripts = transcripts
        .iter()
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>();
    transcripts.sort_by_key(|text| std::cmp::Reverse(text.len()));
    for transcript in transcripts {
        value = value.replace(transcript, "[transcript redacted]");
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
    if !path.parent().is_some_and(|parent| {
        fs::symlink_metadata(parent)
            .ok()
            .is_some_and(|metadata| metadata.file_type().is_dir())
    }) {
        return None;
    }
    if !fs::symlink_metadata(path)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_file())
    {
        return None;
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).ok()?;
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_json_atomic<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        ensure_private_directory(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("aidoo-data");
    let temporary = path.with_file_name(format!(
        ".{file_name}.tmp-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
        Ok::<(), std::io::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{sanitize_diagnostic, sanitize_support_text};

    #[test]
    fn diagnostics_redact_openai_keys() {
        let key = ["sk", "example-secret-value"].join("-");
        let value = sanitize_diagnostic(&format!("request failed for key={key}, retry"));
        assert!(!value.contains("sk-example"));
        assert!(value.contains("[redacted]"));
    }

    #[test]
    fn support_text_redacts_keys_transcripts_and_home_path() {
        let key = ["sk", "example-secret-value"].join("-");
        let transcript = "A private dictated sentence".to_string();
        let home = dirs::home_dir().unwrap();
        let value = sanitize_support_text(
            &format!(
                "failure in {}/Documents: {transcript}; key={key}",
                home.display()
            ),
            std::slice::from_ref(&transcript),
        );
        assert!(!value.contains(&key));
        assert!(!value.contains(&transcript));
        assert!(!value.contains(&home.to_string_lossy().to_string()));
        assert!(value.contains("[transcript redacted]"));
        assert!(value.contains("~/Documents"));
    }
}
