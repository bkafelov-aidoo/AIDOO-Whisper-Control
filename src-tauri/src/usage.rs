use super::*;

pub(super) struct LiveUsageTiming {
    started_at: std::time::Instant,
    created_at: String,
}

pub(super) fn start_live_usage(state: &AppState) {
    if let Ok(mut timing) = state.live_usage_timing.lock() {
        *timing = Some(LiveUsageTiming {
            started_at: std::time::Instant::now(),
            created_at: Utc::now().to_rfc3339(),
        });
    }
}

pub(super) fn finish_live_usage(app: &AppHandle) {
    let state = app.state::<AppState>();
    let timing = state
        .live_usage_timing
        .lock()
        .ok()
        .and_then(|mut timing| timing.take());
    let Some(timing) = timing else {
        return;
    };
    let millis = timing
        .started_at
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    record_usage(app, "live", timing.created_at, millis, models::LIVE_MODEL);
}

pub(super) fn record_transcription_usage(app: &AppHandle, duration_seconds: f64, model: &str) {
    record_usage(
        app,
        "transcription",
        Utc::now().to_rfc3339(),
        models::duration_millis(duration_seconds),
        model,
    );
}

fn record_usage(
    app: &AppHandle,
    kind: &str,
    created_at: String,
    duration_millis: u64,
    model: &str,
) {
    if duration_millis == 0 {
        return;
    }
    let state = app.state::<AppState>();
    let mut usage = match state.usage.lock() {
        Ok(usage) => usage,
        Err(_) => {
            storage::append_diagnostic("usage ledger lock failed");
            return;
        }
    };
    let mut next = usage.clone();
    if next
        .record(kind, created_at, duration_millis, model, false)
        .is_none()
    {
        storage::append_diagnostic(&format!("usage rate missing for {kind}/{model}"));
        return;
    }
    if let Err(error) = storage::save_usage(&next) {
        storage::append_diagnostic(&format!("usage ledger save failed: {error}"));
        return;
    }
    *usage = next.clone();
    drop(usage);
    let _ = app.emit("usage:changed", next);
}
