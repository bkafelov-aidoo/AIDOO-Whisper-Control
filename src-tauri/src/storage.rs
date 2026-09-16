use crate::models::{duration_millis, AppSettings, FailedRecording, TranscriptEntry, UsageLedger};
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[cfg(test)]
thread_local! {
    static TEST_DATA_DIR: std::cell::RefCell<Option<PathBuf>> = const {
        std::cell::RefCell::new(None)
    };
}

const MAX_DIAGNOSTIC_BYTES: usize = 1_000_000;
const RETAINED_DIAGNOSTIC_BYTES: usize = 500_000;
const MAX_SETTINGS_JSON_BYTES: u64 = 1_000_000;
const MAX_HISTORY_JSON_BYTES: u64 = 10_000_000;
const MAX_USAGE_JSON_BYTES: u64 = 10_000_000;
const MAX_HISTORY_DELETION_JSON_BYTES: u64 = 1_000_000;
// A bounded OpenAI response can contain up to 2 MB of JSON. Re-serializing its decoded text can
// expand escaped characters, so Recovery metadata gets a separate still-bounded allowance.
const MAX_FAILED_RECORDING_JSON_BYTES: u64 = 16_000_000;

pub fn data_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_DATA_DIR.with(|value| value.borrow().clone()) {
        return path;
    }
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("AIDOO Whisper Lite")
}

#[cfg(test)]
pub(crate) fn with_test_data_dir<T>(path: PathBuf, test: impl FnOnce() -> T) -> T {
    struct ResetTestDataDir(Option<PathBuf>);

    impl Drop for ResetTestDataDir {
        fn drop(&mut self) {
            TEST_DATA_DIR.with(|value| {
                value.replace(self.0.take());
            });
        }
    }

    let previous = TEST_DATA_DIR.with(|value| value.replace(Some(path)));
    let _reset = ResetTestDataDir(previous);
    test()
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

fn usage_path() -> PathBuf {
    data_dir().join("usage.json")
}

fn pending_history_deletion_path() -> PathBuf {
    data_dir().join("pending-history-deletion.json")
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
    let mut settings: AppSettings =
        read_json(&settings_path(), MAX_SETTINGS_JSON_BYTES).unwrap_or_default();
    settings.normalize();
    settings
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    write_json_atomic(&settings_path(), settings)
}

pub fn load_history() -> Vec<TranscriptEntry> {
    let mut history: Vec<TranscriptEntry> =
        read_json(&history_path(), MAX_HISTORY_JSON_BYTES).unwrap_or_default();
    history.truncate(10);
    history
}

pub fn save_history(history: &[TranscriptEntry]) -> Result<(), String> {
    let bounded = history.iter().take(10).cloned().collect::<Vec<_>>();
    write_json_atomic(&history_path(), &bounded)
}

pub fn load_usage(history: &[TranscriptEntry]) -> UsageLedger {
    if let Some(mut ledger) = read_json::<UsageLedger>(&usage_path(), MAX_USAGE_JSON_BYTES) {
        ledger.entries.truncate(500);
        return ledger;
    }

    let mut ledger = UsageLedger::default();
    // Seed the new ledger once from the local history so existing users do not start with an
    // empty dashboard. History is bounded, so the UI identifies these rows as imported estimates.
    for entry in history.iter().rev() {
        let _ = ledger.record(
            "transcription",
            entry.created_at.clone(),
            duration_millis(entry.duration_seconds),
            &entry.model,
            true,
        );
    }
    let _ = save_usage(&ledger);
    ledger
}

pub fn save_usage(usage: &UsageLedger) -> Result<(), String> {
    write_json_atomic(&usage_path(), usage)
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingHistoryDeletionFile {
    pub original: String,
    pub staged: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingHistoryDeletion {
    pub entry: TranscriptEntry,
    #[serde(default)]
    pub history_committed: bool,
    pub files: Vec<PendingHistoryDeletionFile>,
}

pub fn load_pending_history_deletion() -> Option<PendingHistoryDeletion> {
    read_json(
        &pending_history_deletion_path(),
        MAX_HISTORY_DELETION_JSON_BYTES,
    )
}

pub fn save_pending_history_deletion(deletion: &PendingHistoryDeletion) -> Result<(), String> {
    write_json_atomic(&pending_history_deletion_path(), deletion)
}

pub fn clear_pending_history_deletion() -> Result<(), String> {
    match fs::remove_file(pending_history_deletion_path()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn load_failed_recording() -> Option<FailedRecording> {
    let persisted =
        read_json::<FailedRecording>(&failed_recording_path(), MAX_FAILED_RECORDING_JSON_BYTES)
            .filter(valid_failed_recording);
    if let Some(recording) = persisted {
        if let Some(recovered) = newest_recovery_audio() {
            let persisted_path = PathBuf::from(&recording.path);
            let recovered_path = PathBuf::from(&recovered.path);
            let recovered_is_newer = persisted_path != recovered_path
                && file_modified(&recovered_path)
                    .zip(file_modified(&persisted_path))
                    .is_some_and(|(recovered, persisted)| recovered > persisted);
            if recovered_is_newer {
                let _ = save_failed_recording(&recovered);
                return Some(recovered);
            }
        }
        return Some(recording);
    }

    let recovered = newest_recovery_audio()?;
    let _ = save_failed_recording(&recovered);
    Some(recovered)
}

fn valid_failed_recording(recording: &FailedRecording) -> bool {
    let path = PathBuf::from(&recording.path);
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let recovery = recovery_dir();
    let is_regular_file = fs::symlink_metadata(&path)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_file());
    path.parent() == Some(recovery.as_path())
        && recovery_file_matches_metadata(name, recording)
        && is_regular_file
}

fn file_modified(path: &Path) -> Option<std::time::SystemTime> {
    fs::symlink_metadata(path)
        .ok()
        .filter(|metadata| metadata.file_type().is_file())?
        .modified()
        .ok()
}

fn newest_recovery_audio() -> Option<FailedRecording> {
    let mut newest = None;
    for entry in fs::read_dir(recovery_dir()).ok()?.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(retryable) = recovery_file_retryability(name) else {
            continue;
        };
        let Some(modified) = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
        else {
            continue;
        };
        if newest
            .as_ref()
            .is_none_or(|(current, _, _)| modified > *current)
        {
            newest = Some((modified, entry.path(), retryable));
        }
    }

    let (modified, path, retryable) = newest?;
    let error = if retryable {
        "Възстановен е неуспешен запис след прекъсване. Можете да опитате отново."
    } else {
        "Възстановен е запис след прекъсване. Не може да бъде изпратен повторно автоматично, за да се избегне повторно API таксуване."
    };
    Some(FailedRecording {
        path: path.to_string_lossy().to_string(),
        created_at: DateTime::<Utc>::from(modified).to_rfc3339(),
        duration_seconds: 0.0,
        error: error.into(),
        retryable,
        completed_text: None,
    })
}

fn recovery_file_retryability(name: &str) -> Option<bool> {
    let path = Path::new(name);
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(extension.as_str(), "flac" | "wav") {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    for (prefix, retryable) in [
        ("failed-dictation-retryable-", true),
        ("failed-dictation-nonretryable-", false),
        ("failed-dictation-", false),
    ] {
        if let Some(identifier) = stem.strip_prefix(prefix) {
            return uuid::Uuid::parse_str(identifier).ok().map(|_| retryable);
        }
    }
    matches!(
        name,
        "last-failed-dictation.flac" | "last-failed-dictation.wav"
    )
    .then_some(false)
}

fn recovery_file_matches_metadata(name: &str, recording: &FailedRecording) -> bool {
    match recovery_file_retryability(name) {
        Some(true) => recording.retryable || recording.completed_text.is_some(),
        Some(false) if name.starts_with("failed-dictation-nonretryable-") => {
            !recording.retryable || recording.completed_text.is_some()
        }
        Some(false) => true,
        None => false,
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
        #[cfg(unix)]
        let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
        let _ = writeln!(file, "{} {}", Utc::now().to_rfc3339(), sanitized);
    }
    trim_diagnostics();
}

pub fn read_diagnostics_for_support() -> Option<Vec<u8>> {
    read_regular_file_tail(&diagnostics_path(), MAX_DIAGNOSTIC_BYTES)
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
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return;
    };
    if !metadata.file_type().is_file() || metadata.len() <= MAX_DIAGNOSTIC_BYTES as u64 {
        return;
    }
    if let Some(contents) = read_regular_file_tail(&path, RETAINED_DIAGNOSTIC_BYTES) {
        let _ = write_private_bytes_atomic(&path, &contents);
    }
}

fn read_regular_file_tail(path: &Path, maximum_bytes: usize) -> Option<Vec<u8>> {
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
    let mut file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.file_type().is_file() {
        return None;
    }
    let start = metadata.len().saturating_sub(maximum_bytes as u64);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut contents = Vec::with_capacity(maximum_bytes.min(metadata.len() as usize));
    file.take(maximum_bytes as u64)
        .read_to_end(&mut contents)
        .ok()?;
    Some(contents)
}

fn write_private_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
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
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
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

fn read_json<T: DeserializeOwned>(path: &Path, maximum_bytes: u64) -> Option<T> {
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
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > maximum_bytes {
        quarantine_invalid_json(path, "size limit exceeded");
        return None;
    }
    let bytes = fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(value) => Some(value),
        Err(_) => {
            quarantine_invalid_json(path, "invalid JSON");
            None
        }
    }
}

fn quarantine_invalid_json(path: &Path, reason: &str) {
    let Some(parent) = path.parent() else {
        return;
    };
    let should_log = parent == data_dir();
    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("private-data");
    let target = parent.join(format!(
        ".{name}.corrupt-{}-{}.json",
        Utc::now().format("%Y%m%dT%H%M%S"),
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    ));
    match fs::rename(path, &target) {
        Ok(()) => {
            #[cfg(unix)]
            let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o600));
            #[cfg(unix)]
            let _ = File::open(parent).and_then(|directory| directory.sync_all());
            if should_log {
                append_diagnostic(&format!("quarantined {name}.json: {reason}"));
            }
        }
        Err(error) => {
            if should_log {
                append_diagnostic(&format!(
                    "failed to quarantine {name}.json ({reason}): {error}"
                ));
            }
        }
    }
}

fn write_json_atomic<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    write_private_bytes_atomic(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_directories, failed_recording_path, load_failed_recording, load_usage, read_json,
        recovery_dir, recovery_file_matches_metadata, recovery_file_retryability,
        sanitize_diagnostic, sanitize_support_text, with_test_data_dir, write_json_atomic,
    };
    use crate::models::{FailedRecording, TranscriptEntry, ECONOMY_MODEL};
    use serde_json::json;

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

    #[test]
    fn private_json_round_trips_atomically() {
        let root = std::env::temp_dir().join(format!(
            "aidoo-lite-private-json-test-{}",
            uuid::Uuid::new_v4()
        ));
        let path = root.join("settings.json");
        let expected = json!({"language": "bg", "history": true});

        write_json_atomic(&path, &expected).unwrap();

        assert_eq!(
            read_json::<serde_json::Value>(&path, 1_000_000),
            Some(expected)
        );
        let files = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(files, vec![std::ffi::OsString::from("settings.json")]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn first_usage_load_imports_existing_history_once() {
        let root = std::env::temp_dir().join(format!(
            "aidoo-lite-usage-import-test-{}",
            uuid::Uuid::new_v4()
        ));
        let history = vec![TranscriptEntry {
            id: "history-1".into(),
            text: "private text must not enter usage.json".into(),
            created_at: "2026-09-16T10:00:00Z".into(),
            duration_seconds: 60.0,
            model: ECONOMY_MODEL.into(),
            language: "bg".into(),
            audio_path: None,
            text_path: None,
        }];

        with_test_data_dir(root.clone(), || {
            let imported = load_usage(&history);
            assert_eq!(imported.transcription_count, 1);
            assert_eq!(imported.transcription_cost_nano_usd, 3_000_000);
            assert!(imported.entries[0].imported_from_history);

            let loaded_again = load_usage(&[]);
            assert_eq!(loaded_again, imported);
            let raw = std::fs::read_to_string(root.join("usage.json")).unwrap();
            assert!(!raw.contains("private text"));
        });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_private_json_is_preserved_before_defaults_are_used() {
        let root = std::env::temp_dir().join(format!(
            "aidoo-lite-invalid-json-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("history.json");
        std::fs::write(&path, b"{not valid json").unwrap();

        assert!(read_json::<serde_json::Value>(&path, 1_000_000).is_none());
        assert!(!path.exists());
        let preserved = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .any(|name| name.starts_with(".history.corrupt-") && name.ends_with(".json"));
        assert!(preserved);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_file_names_preserve_retry_safety() {
        let identifier = uuid::Uuid::new_v4();
        let failed = |retryable, completed_text: Option<&str>| FailedRecording {
            path: String::new(),
            created_at: String::new(),
            duration_seconds: 0.0,
            error: String::new(),
            retryable,
            completed_text: completed_text.map(str::to_owned),
        };
        assert_eq!(
            recovery_file_retryability(&format!("failed-dictation-retryable-{identifier}.flac")),
            Some(true)
        );
        assert_eq!(
            recovery_file_retryability(&format!("failed-dictation-nonretryable-{identifier}.wav")),
            Some(false)
        );
        assert_eq!(
            recovery_file_retryability(&format!("failed-dictation-{identifier}.flac")),
            Some(false)
        );
        assert_eq!(
            recovery_file_retryability("failed-dictation-junk.flac"),
            None
        );
        assert_eq!(recovery_file_retryability("someone-else.flac"), None);
        assert!(recovery_file_matches_metadata(
            &format!("failed-dictation-retryable-{identifier}.flac"),
            &failed(true, None)
        ));
        assert!(!recovery_file_matches_metadata(
            &format!("failed-dictation-retryable-{identifier}.flac"),
            &failed(false, None)
        ));
        assert!(recovery_file_matches_metadata(
            &format!("failed-dictation-retryable-{identifier}.flac"),
            &failed(true, Some("already transcribed"))
        ));
        assert!(recovery_file_matches_metadata(
            &format!("failed-dictation-retryable-{identifier}.flac"),
            &failed(false, Some("already transcribed"))
        ));
        assert!(!recovery_file_matches_metadata(
            &format!("failed-dictation-nonretryable-{identifier}.flac"),
            &failed(true, None)
        ));
        assert!(recovery_file_matches_metadata(
            &format!("failed-dictation-nonretryable-{identifier}.flac"),
            &failed(true, Some("already transcribed"))
        ));
    }

    #[test]
    fn missing_recovery_metadata_is_rebuilt_from_the_audio_marker() {
        let root = std::env::temp_dir().join(format!(
            "aidoo-lite-missing-recovery-metadata-test-{}",
            uuid::Uuid::new_v4()
        ));
        with_test_data_dir(root.clone(), || {
            ensure_directories().unwrap();
            let audio = recovery_dir().join(format!(
                "failed-dictation-nonretryable-{}.flac",
                uuid::Uuid::new_v4()
            ));
            std::fs::write(&audio, b"fLaC recovery").unwrap();

            let recovered = load_failed_recording().expect("audio marker must be recovered");

            assert_eq!(recovered.path, audio.to_string_lossy());
            assert!(!recovered.retryable);
            assert!(recovered.completed_text.is_none());
            assert!(failed_recording_path().is_file());
            let persisted: FailedRecording =
                read_json(&failed_recording_path(), 16_000_000).unwrap();
            assert_eq!(persisted.path, recovered.path);
            assert!(!persisted.retryable);
        });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_recovery_metadata_is_quarantined_before_fail_safe_recovery() {
        let root = std::env::temp_dir().join(format!(
            "aidoo-lite-corrupt-recovery-metadata-test-{}",
            uuid::Uuid::new_v4()
        ));
        with_test_data_dir(root.clone(), || {
            ensure_directories().unwrap();
            let audio = recovery_dir().join(format!(
                "failed-dictation-retryable-{}.flac",
                uuid::Uuid::new_v4()
            ));
            std::fs::write(&audio, b"fLaC recovery").unwrap();
            std::fs::write(failed_recording_path(), b"{invalid metadata").unwrap();

            let recovered = load_failed_recording().expect("audio marker must survive metadata");

            assert_eq!(recovered.path, audio.to_string_lossy());
            assert!(recovered.retryable);
            assert!(failed_recording_path().is_file());
            let quarantined = std::fs::read_dir(&root)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .any(|name| {
                    name.starts_with(".failed-recording.corrupt-") && name.ends_with(".json")
                });
            assert!(quarantined);
        });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_json_rejects_a_symlinked_parent_directory() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "aidoo-lite-private-link-test-{}",
            uuid::Uuid::new_v4()
        ));
        let real = root.join("real");
        let linked = root.join("linked");
        std::fs::create_dir_all(&real).unwrap();
        symlink(&real, &linked).unwrap();
        let linked_path = linked.join("history.json");
        let real_path = real.join("history.json");
        std::fs::write(&real_path, b"[]").unwrap();

        assert!(read_json::<serde_json::Value>(&linked_path, 1_000_000).is_none());
        assert!(write_json_atomic(&linked_path, &json!([])).is_err());
        assert_eq!(std::fs::read(&real_path).unwrap(), b"[]");
        std::fs::remove_dir_all(root).unwrap();
    }
}
