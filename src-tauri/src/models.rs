use serde::{Deserialize, Serialize};
use std::path::Path;

pub const ECONOMY_MODEL: &str = "gpt-4o-mini-transcribe";
pub const ACCURACY_MODEL: &str = "gpt-transcribe";
const SUPPORTED_LANGUAGES: &[&str] = &["auto", "bg", "en", "de", "es", "fr", "it"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ShortcutBinding {
    Key {
        code: String,
        #[serde(default)]
        modifiers: Vec<String>,
    },
}

impl ShortcutBinding {
    pub fn key(code: &str, modifiers: &[&str]) -> Self {
        Self::Key {
            code: code.into(),
            modifiers: modifiers.iter().map(|value| (*value).into()).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub onboarding_complete: bool,
    pub ui_language: String,
    pub language: String,
    pub model: String,
    pub auto_paste: bool,
    pub save_audio: bool,
    pub save_text: bool,
    pub history_enabled: bool,
    pub output_directory: Option<String>,
    pub launch_at_login: bool,
    pub microphone_name: Option<String>,
    pub automatic_microphone_fallback: bool,
    pub wake_word_enabled: bool,
    pub wake_word_auto_stop: bool,
    pub aidoo_clinic_slug: Option<String>,
    pub aidoo_clinic_url: Option<String>,
    pub aidoo_email: Option<String>,
    pub dictation_shortcut: ShortcutBinding,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            onboarding_complete: false,
            ui_language: "auto".into(),
            language: "bg".into(),
            model: ECONOMY_MODEL.into(),
            auto_paste: true,
            save_audio: true,
            save_text: true,
            history_enabled: true,
            output_directory: None,
            launch_at_login: false,
            microphone_name: None,
            automatic_microphone_fallback: true,
            wake_word_enabled: false,
            wake_word_auto_stop: true,
            aidoo_clinic_slug: None,
            aidoo_clinic_url: None,
            aidoo_email: None,
            dictation_shortcut: ShortcutBinding::key("alt_gr", &[]),
        }
    }
}

impl AppSettings {
    pub fn normalize(&mut self) {
        if !matches!(self.model.as_str(), ECONOMY_MODEL | ACCURACY_MODEL) {
            self.model = ECONOMY_MODEL.into();
        }
        if !SUPPORTED_LANGUAGES.contains(&self.language.as_str()) {
            self.language = "auto".into();
        }
        if !matches!(self.ui_language.as_str(), "auto" | "bg" | "en") {
            self.ui_language = "auto".into();
        }
        if self.output_directory.as_deref().is_some_and(|directory| {
            directory.trim().is_empty() || !Path::new(directory).is_absolute()
        }) {
            self.output_directory = None;
        }
        if self
            .microphone_name
            .as_deref()
            .is_some_and(|name| name.trim().is_empty())
        {
            self.microphone_name = None;
        }
        self.aidoo_clinic_slug = normalized_optional(self.aidoo_clinic_slug.take());
        self.aidoo_clinic_url = normalized_optional(self.aidoo_clinic_url.take());
        self.aidoo_email = normalized_optional(self.aidoo_email.take());
    }
}

fn normalized_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{AppSettings, FailedRecording, ACCURACY_MODEL, ECONOMY_MODEL};

    #[test]
    fn production_model_aliases_remain_stable() {
        assert_eq!(ECONOMY_MODEL, "gpt-4o-mini-transcribe");
        assert_eq!(ACCURACY_MODEL, "gpt-transcribe");

        let mut settings = AppSettings {
            model: ACCURACY_MODEL.into(),
            ..AppSettings::default()
        };
        settings.normalize();
        assert_eq!(settings.model, ACCURACY_MODEL);
    }

    #[test]
    fn normalize_rejects_unknown_model_and_language() {
        let mut settings = AppSettings {
            model: "unknown-model".into(),
            language: "made-up-language".into(),
            ui_language: "unsupported".into(),
            output_directory: Some("relative/output".into()),
            microphone_name: Some("   ".into()),
            ..AppSettings::default()
        };

        settings.normalize();

        assert_eq!(settings.model, ECONOMY_MODEL);
        assert_eq!(settings.language, "auto");
        assert_eq!(settings.ui_language, "auto");
        assert_eq!(settings.output_directory, None);
        assert_eq!(settings.microphone_name, None);
    }

    #[test]
    fn legacy_failed_recordings_remain_retryable() {
        let recording: FailedRecording = serde_json::from_value(serde_json::json!({
            "path": "/tmp/failed.flac",
            "createdAt": "2026-09-14T00:00:00Z",
            "durationSeconds": 2.0,
            "error": "network"
        }))
        .unwrap();

        assert!(recording.retryable);
        assert!(recording.completed_text.is_none());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptEntry {
    pub id: String,
    pub text: String,
    pub created_at: String,
    pub duration_seconds: f64,
    pub model: String,
    pub language: String,
    pub audio_path: Option<String>,
    pub text_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedRecording {
    pub path: String,
    pub created_at: String,
    pub duration_seconds: f64,
    pub error: String,
    #[serde(default = "default_true")]
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_text: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingProgress {
    pub percent: u8,
    pub stage: String,
    pub determinate: bool,
}

impl Default for RecordingProgress {
    fn default() -> Self {
        Self {
            percent: 0,
            stage: "preparing_audio".into(),
            determinate: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSnapshot {
    pub state: String,
    pub progress: RecordingProgress,
    pub elapsed_seconds: f64,
    pub error: Option<String>,
    pub trigger: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapState {
    pub settings: AppSettings,
    pub history: Vec<TranscriptEntry>,
    pub failed_recording: Option<FailedRecording>,
    pub microphones: Vec<String>,
    pub has_api_key: bool,
    pub has_aidoo_password: bool,
    pub aidoo_connected: bool,
    pub aidoo_connection_error: Option<String>,
    pub accessibility_granted: bool,
    pub app_version: String,
    pub default_output_directory: String,
    pub recording: RecordingSnapshot,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayBootstrapState {
    pub ui_language: String,
    pub recording: RecordingSnapshot,
    pub assistant_phase: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionCompleted {
    pub entry: Option<TranscriptEntry>,
    pub text: String,
    pub paste_succeeded: bool,
    pub paste_error: Option<String>,
}
