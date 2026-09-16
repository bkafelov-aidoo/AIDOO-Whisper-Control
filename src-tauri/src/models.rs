use serde::{Deserialize, Serialize};
use std::path::Path;

pub const ECONOMY_MODEL: &str = "gpt-4o-mini-transcribe";
pub const ACCURACY_MODEL: &str = "gpt-transcribe";
pub const LIVE_MODEL: &str = "gpt-live-1";
pub const LIVE_BACKEND_MODEL: &str = "gpt-5.6-terra";
pub const ECONOMY_RATE_NANO_USD_PER_MINUTE: u64 = 3_000_000;
pub const ACCURACY_RATE_NANO_USD_PER_MINUTE: u64 = 4_500_000;
pub const LIVE_RATE_NANO_USD_PER_MINUTE: u64 = 50_000_000;
const TERRA_INPUT_NANO_USD_PER_TOKEN: u64 = 2_000;
const TERRA_CACHED_INPUT_NANO_USD_PER_TOKEN: u64 = 200;
const TERRA_CACHE_WRITE_NANO_USD_PER_TOKEN: u64 = 2_500;
const TERRA_OUTPUT_NANO_USD_PER_TOKEN: u64 = 12_000;
const TERRA_LONG_CONTEXT_THRESHOLD: u64 = 272_000;
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
    pub aidoo_browser_sync_enabled: bool,
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
            aidoo_browser_sync_enabled: true,
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
    use super::{
        cost_nano_usd, live_backend_cost_nano_usd, AppSettings, FailedRecording, LiveBackendUsage,
        UsageLedger, ACCURACY_MODEL, ACCURACY_RATE_NANO_USD_PER_MINUTE, ECONOMY_MODEL,
        ECONOMY_RATE_NANO_USD_PER_MINUTE, LIVE_BACKEND_MODEL, LIVE_MODEL,
        LIVE_RATE_NANO_USD_PER_MINUTE,
    };

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

    #[test]
    fn usage_cost_uses_millisecond_duration_without_minute_rounding() {
        assert_eq!(cost_nano_usd(1_000, LIVE_RATE_NANO_USD_PER_MINUTE), 833_333);
        assert_eq!(
            cost_nano_usd(60_000, LIVE_RATE_NANO_USD_PER_MINUTE),
            50_000_000
        );
        assert_eq!(
            cost_nano_usd(60_000, ECONOMY_RATE_NANO_USD_PER_MINUTE),
            3_000_000
        );
        assert_eq!(
            cost_nano_usd(60_000, ACCURACY_RATE_NANO_USD_PER_MINUTE),
            4_500_000
        );
    }

    #[test]
    fn usage_ledger_keeps_separate_and_combined_totals() {
        let mut usage = UsageLedger::default();
        usage.record(
            "live",
            "2026-09-16T10:00:00Z".into(),
            30_000,
            LIVE_MODEL,
            false,
        );
        usage.record(
            "transcription",
            "2026-09-16T10:01:00Z".into(),
            120_000,
            ECONOMY_MODEL,
            false,
        );

        assert_eq!(usage.live_session_count, 1);
        assert_eq!(usage.transcription_count, 1);
        assert_eq!(usage.live_cost_nano_usd, 25_000_000);
        assert_eq!(usage.transcription_cost_nano_usd, 6_000_000);
        assert_eq!(usage.entries.len(), 2);
        assert_eq!(usage.entries[0].kind, "transcription");
    }

    #[test]
    fn backend_cost_uses_uncached_cached_write_and_output_rates() {
        let backend = LiveBackendUsage {
            input_tokens: 1_000,
            cached_input_tokens: 200,
            cache_write_tokens: 100,
            output_tokens: 50,
        };
        assert_eq!(live_backend_cost_nano_usd(&backend), Some(2_290_000));

        let mut usage = UsageLedger::default();
        let entry = usage
            .record_live_backend("2026-09-17T10:00:00Z".into(), LIVE_BACKEND_MODEL, &backend)
            .unwrap();
        assert_eq!(entry.kind, "liveBackend");
        assert_eq!(usage.live_backend_cost_nano_usd, 2_290_000);
        assert_eq!(usage.live_backend_response_count, 1);
        assert_eq!(usage.live_backend_input_tokens, 1_000);
        assert_eq!(usage.live_backend_output_tokens, 50);
    }

    #[test]
    fn backend_cost_rejects_invalid_token_breakdowns_and_prices_long_context() {
        let invalid = LiveBackendUsage {
            input_tokens: 10,
            cached_input_tokens: 8,
            cache_write_tokens: 3,
            output_tokens: 0,
        };
        assert_eq!(live_backend_cost_nano_usd(&invalid), None);

        let long = LiveBackendUsage {
            input_tokens: 272_001,
            cached_input_tokens: 0,
            cache_write_tokens: 0,
            output_tokens: 10,
        };
        assert_eq!(live_backend_cost_nano_usd(&long), Some(1_088_184_000));
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageEntry {
    pub id: String,
    pub kind: String,
    pub created_at: String,
    pub duration_millis: u64,
    pub model: String,
    pub rate_nano_usd_per_minute: u64,
    pub cost_nano_usd: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub imported_from_history: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LiveBackendUsage {
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct UsageLedger {
    pub entries: Vec<UsageEntry>,
    pub live_duration_millis: u64,
    pub transcription_duration_millis: u64,
    pub live_cost_nano_usd: u64,
    pub live_backend_cost_nano_usd: u64,
    pub transcription_cost_nano_usd: u64,
    pub live_session_count: u64,
    pub live_backend_response_count: u64,
    pub live_backend_input_tokens: u64,
    pub live_backend_output_tokens: u64,
    pub transcription_count: u64,
}

impl UsageLedger {
    pub fn record(
        &mut self,
        kind: &str,
        created_at: String,
        duration_millis: u64,
        model: &str,
        imported_from_history: bool,
    ) -> Option<UsageEntry> {
        let rate = match (kind, model) {
            ("live", LIVE_MODEL) => LIVE_RATE_NANO_USD_PER_MINUTE,
            ("transcription", ECONOMY_MODEL) => ECONOMY_RATE_NANO_USD_PER_MINUTE,
            ("transcription", ACCURACY_MODEL) => ACCURACY_RATE_NANO_USD_PER_MINUTE,
            _ => return None,
        };
        let cost = cost_nano_usd(duration_millis, rate);
        let entry = UsageEntry {
            id: uuid::Uuid::new_v4().to_string(),
            kind: kind.into(),
            created_at,
            duration_millis,
            model: model.into(),
            rate_nano_usd_per_minute: rate,
            cost_nano_usd: cost,
            input_tokens: 0,
            cached_input_tokens: 0,
            cache_write_tokens: 0,
            output_tokens: 0,
            imported_from_history,
        };
        match kind {
            "live" => {
                self.live_duration_millis =
                    self.live_duration_millis.saturating_add(duration_millis);
                self.live_cost_nano_usd = self.live_cost_nano_usd.saturating_add(cost);
                self.live_session_count = self.live_session_count.saturating_add(1);
            }
            "transcription" => {
                self.transcription_duration_millis = self
                    .transcription_duration_millis
                    .saturating_add(duration_millis);
                self.transcription_cost_nano_usd =
                    self.transcription_cost_nano_usd.saturating_add(cost);
                self.transcription_count = self.transcription_count.saturating_add(1);
            }
            _ => return None,
        }
        self.entries.insert(0, entry.clone());
        self.entries.truncate(500);
        Some(entry)
    }

    pub fn record_live_backend(
        &mut self,
        created_at: String,
        model: &str,
        usage: &LiveBackendUsage,
    ) -> Option<UsageEntry> {
        if !is_live_backend_model(model) {
            return None;
        }
        let cost = live_backend_cost_nano_usd(usage)?;
        let entry = UsageEntry {
            id: uuid::Uuid::new_v4().to_string(),
            kind: "liveBackend".into(),
            created_at,
            duration_millis: 0,
            model: model.into(),
            rate_nano_usd_per_minute: 0,
            cost_nano_usd: cost,
            input_tokens: usage.input_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            cache_write_tokens: usage.cache_write_tokens,
            output_tokens: usage.output_tokens,
            imported_from_history: false,
        };
        self.live_backend_cost_nano_usd = self.live_backend_cost_nano_usd.saturating_add(cost);
        self.live_backend_response_count = self.live_backend_response_count.saturating_add(1);
        self.live_backend_input_tokens = self
            .live_backend_input_tokens
            .saturating_add(usage.input_tokens);
        self.live_backend_output_tokens = self
            .live_backend_output_tokens
            .saturating_add(usage.output_tokens);
        self.entries.insert(0, entry.clone());
        self.entries.truncate(500);
        Some(entry)
    }
}

pub fn duration_millis(duration_seconds: f64) -> u64 {
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return 0;
    }
    (duration_seconds * 1_000.0).round().min(u64::MAX as f64) as u64
}

pub fn cost_nano_usd(duration_millis: u64, rate_nano_usd_per_minute: u64) -> u64 {
    let numerator = u128::from(duration_millis) * u128::from(rate_nano_usd_per_minute);
    ((numerator + 30_000) / 60_000).min(u128::from(u64::MAX)) as u64
}

pub fn live_backend_cost_nano_usd(usage: &LiveBackendUsage) -> Option<u64> {
    let discounted = usage
        .cached_input_tokens
        .checked_add(usage.cache_write_tokens)?;
    let uncached = usage.input_tokens.checked_sub(discounted)?;
    let long_context = usage.input_tokens > TERRA_LONG_CONTEXT_THRESHOLD;
    let input_multiplier = if long_context { 2_u128 } else { 1 };
    let output_numerator = if long_context { 3_u128 } else { 2 };
    let cost = u128::from(uncached) * u128::from(TERRA_INPUT_NANO_USD_PER_TOKEN) * input_multiplier
        + u128::from(usage.cached_input_tokens)
            * u128::from(TERRA_CACHED_INPUT_NANO_USD_PER_TOKEN)
            * input_multiplier
        + u128::from(usage.cache_write_tokens)
            * u128::from(TERRA_CACHE_WRITE_NANO_USD_PER_TOKEN)
            * input_multiplier
        + u128::from(usage.output_tokens)
            * u128::from(TERRA_OUTPUT_NANO_USD_PER_TOKEN)
            * output_numerator
            / 2;
    Some(cost.min(u128::from(u64::MAX)) as u64)
}

pub fn is_live_backend_model(model: &str) -> bool {
    model == LIVE_BACKEND_MODEL
        || model
            .strip_prefix(LIVE_BACKEND_MODEL)
            .is_some_and(|suffix| suffix.starts_with('-'))
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
    pub usage: UsageLedger,
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
