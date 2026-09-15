fn main() {
    const COMMANDS: &[&str] = &[
        "bootstrap",
        "overlay_bootstrap",
        "update_settings",
        "save_api_key",
        "delete_api_key",
        "begin_shortcut_capture",
        "cancel_shortcut_capture",
        "test_microphone",
        "start_recording",
        "stop_and_transcribe",
        "retry_failed_transcription",
        "retranscribe_history_item",
        "delete_failed_recording",
        "current_recording_snapshot",
        "copy_text",
        "delete_history_item",
        "open_accessibility_settings",
        "refresh_accessibility_status",
        "reposition_overlay",
        "open_local_path",
        "create_diagnostic_bundle",
    ];
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to build AIDOO Whisper Lite permissions");
}
