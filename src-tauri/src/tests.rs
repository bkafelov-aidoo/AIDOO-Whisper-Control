use super::{
    commit_staged_history_deletion, diagnostic_settings, is_managed_output_path,
    localized_native_error, operation_allows_quit, overlay_accepts_pointer_input,
    overlay_visible_for_state, path_is_authorized_for_open, prepare_history_files_for_deletion,
    preserve_completed_recovery_with, recording_watchdog_should_stop,
    recover_pending_history_deletion, recovery_plan, resolve_failed_recording_after_success_with,
    resolved_tray_state, restore_staged_history_files, save_local_transcription_files,
    save_local_transcription_files_with_stem, stage_history_files_for_deletion, tray_tooltip,
    voice_watchdog_action, AppSettings, AssistantStartRequest, FailedRecording,
    PendingDiagnosticFile, RecoveryPlan, TranscriptEntry, VoiceWatchdogAction,
    CHARGED_RECOVERY_ERROR,
};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[test]
fn wake_request_survives_a_lost_ui_event_and_is_consumed_once() {
    let request = AssistantStartRequest::default();
    assert!(!request.take());
    request.request();
    assert!(request.take());
    assert!(!request.take());
}

#[test]
fn voice_watchdog_waits_for_speech_then_transcribes_after_the_pause() {
    let waiting = crate::audio::RecordingActivity {
        duration_seconds: 4.9,
        silence_seconds: 4.9,
        speech_detected: false,
    };
    assert_eq!(
        voice_watchdog_action(waiting),
        VoiceWatchdogAction::Continue
    );

    let empty = crate::audio::RecordingActivity {
        duration_seconds: 5.0,
        silence_seconds: 5.0,
        speech_detected: false,
    };
    assert_eq!(voice_watchdog_action(empty), VoiceWatchdogAction::Cancel);

    let speaking = crate::audio::RecordingActivity {
        duration_seconds: 2.0,
        silence_seconds: 1.49,
        speech_detected: true,
    };
    assert_eq!(
        voice_watchdog_action(speaking),
        VoiceWatchdogAction::Continue
    );

    let finished = crate::audio::RecordingActivity {
        duration_seconds: 3.0,
        silence_seconds: 1.5,
        speech_detected: true,
    };
    assert_eq!(
        voice_watchdog_action(finished),
        VoiceWatchdogAction::Transcribe
    );
}

fn history_entry() -> TranscriptEntry {
    TranscriptEntry {
        id: "entry".into(),
        text: "text".into(),
        created_at: "2026-09-14T00:00:00Z".into(),
        duration_seconds: 1.0,
        model: "gpt-4o-mini-transcribe".into(),
        language: "bg".into(),
        audio_path: Some(
            "/Volumes/External/AIDOO/AIDOO-Whisper-2026-09-14_00-00-00-abcdef.flac".into(),
        ),
        text_path: Some(
            "/Volumes/External/AIDOO/AIDOO-Whisper-2026-09-14_00-00-00-abcdef.txt".into(),
        ),
    }
}

fn recovery_recording(path: &Path, retryable: bool) -> FailedRecording {
    FailedRecording {
        path: path.to_string_lossy().to_string(),
        created_at: "2026-09-15T00:00:00Z".into(),
        duration_seconds: 2.0,
        error: "network".into(),
        retryable,
        completed_text: None,
    }
}

#[test]
fn recovery_policy_has_exactly_one_safe_next_action() {
    let retryable = recovery_recording(Path::new("/tmp/retryable.flac"), true);
    assert_eq!(recovery_plan(&retryable).unwrap(), RecoveryPlan::Transcribe);

    let mut completed = retryable.clone();
    completed.retryable = false;
    completed.completed_text = Some("already paid text".into());
    assert_eq!(
        recovery_plan(&completed).unwrap(),
        RecoveryPlan::FinishLocally("already paid text".into())
    );

    let charged = recovery_recording(Path::new("/tmp/charged.flac"), false);
    assert_eq!(recovery_plan(&charged).unwrap_err(), CHARGED_RECOVERY_ERROR);
}

#[test]
fn failed_cleanup_after_success_can_only_be_deleted() {
    let root = std::env::temp_dir().join(format!(
        "aidoo-lite-recovery-cleanup-test-{}",
        uuid::Uuid::new_v4()
    ));
    let private = root.join("private");

    crate::storage::with_test_data_dir(private, || {
        crate::storage::ensure_directories().unwrap();
        let source = crate::storage::recovery_dir().join(format!(
            "failed-dictation-retryable-{}.flac",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&source, b"fLaC charged audio").unwrap();
        let failed_recording = Mutex::new(Some(recovery_recording(&source, true)));

        let error = resolve_failed_recording_after_success_with(&failed_recording, |_| {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        })
        .unwrap_err();

        assert!(error.contains("не можа да бъде изтрито"));
        let current = failed_recording.lock().unwrap().clone().unwrap();
        assert!(!current.retryable);
        assert!(current.completed_text.is_none());
        assert!(Path::new(&current.path).is_file());
        assert!(Path::new(&current.path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("failed-dictation-nonretryable-"));
        assert_eq!(recovery_plan(&current).unwrap_err(), CHARGED_RECOVERY_ERROR);

        let persisted = crate::storage::load_failed_recording().unwrap();
        assert_eq!(persisted.path, current.path);
        assert!(!persisted.retryable);
        assert_eq!(
            recovery_plan(&persisted).unwrap_err(),
            CHARGED_RECOVERY_ERROR
        );
    });

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn completed_openai_text_is_finished_locally_without_another_request() {
    let root = std::env::temp_dir().join(format!(
        "aidoo-lite-completed-recovery-test-{}",
        uuid::Uuid::new_v4()
    ));
    let private = root.join("private");

    crate::storage::with_test_data_dir(private, || {
        crate::storage::ensure_directories().unwrap();
        let source = crate::storage::recovery_dir().join(format!(
            "failed-dictation-nonretryable-{}.flac",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&source, b"fLaC charged audio").unwrap();
        let failed_recording = Mutex::new(Some(recovery_recording(&source, false)));

        preserve_completed_recovery_with(
            &failed_recording,
            "local save failed",
            "already paid text".into(),
        );

        let current = failed_recording.lock().unwrap().clone().unwrap();
        assert!(!current.retryable);
        assert_eq!(
            recovery_plan(&current).unwrap(),
            RecoveryPlan::FinishLocally("already paid text".into())
        );
        let persisted = crate::storage::load_failed_recording().unwrap();
        assert_eq!(
            persisted.completed_text.as_deref(),
            Some("already paid text")
        );
        assert!(!persisted.retryable);
    });

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_open_scope_accepts_only_history_files_and_diagnostic_bundles() {
    let history = [history_entry()];
    let data = Path::new("/Users/example/Library/Application Support/AIDOO Whisper Lite");

    assert!(path_is_authorized_for_open(
        Path::new("/Volumes/External/AIDOO/AIDOO-Whisper-2026-09-14_00-00-00-abcdef.flac"),
        &history,
        data
    ));
    assert!(is_managed_output_path(
        Path::new("/Volumes/External/AIDOO/AIDOO-Whisper-2026-09-14_00-00-00-abcdef123456.flac"),
        "flac"
    ));
    assert!(path_is_authorized_for_open(
        &data.join("AIDOO-Whisper-Lite-Diagnostics-20260914-000000-abcdef123456.zip"),
        &history,
        data
    ));
    assert!(!path_is_authorized_for_open(
        Path::new("/Users/example/secret.txt"),
        &history,
        data
    ));
    assert!(!path_is_authorized_for_open(
        &data.join("settings.json"),
        &history,
        data
    ));

    let mut tampered = history_entry();
    tampered.audio_path = Some("/Users/example/secret.flac".into());
    assert!(!path_is_authorized_for_open(
        Path::new("/Users/example/secret.flac"),
        &[tampered],
        data
    ));
    assert!(!is_managed_output_path(
        Path::new("AIDOO-Whisper-relative.flac"),
        "flac"
    ));
    assert!(!is_managed_output_path(
        Path::new("/Users/example/AIDOO-Whisper-secret.flac"),
        "flac"
    ));
    assert!(!is_managed_output_path(
        Path::new("/Users/example/AIDOO-Whisper-2026-99-99_00-00-00-abcdef.flac"),
        "flac"
    ));
    assert!(!is_managed_output_path(
        Path::new("/Users/example/AIDOO-Whisper-2026-09-14_00-00-00-abcdeg.flac"),
        "flac"
    ));
    assert!(!is_managed_output_path(
        Path::new("/Users/example/AIDOO-Whisper-2026-09-14_00-00-00-abcdef0.flac"),
        "flac"
    ));
    assert!(!is_managed_output_path(
        Path::new("/Users/example/AIDOO-Whisper-2026-09-14_00-00-00-ABCDEF.flac"),
        "flac"
    ));
    assert!(!path_is_authorized_for_open(
        &data.join("AIDOO-Whisper-Lite-Diagnostics-20260914-000000.zip"),
        &history,
        data
    ));
    assert!(!path_is_authorized_for_open(
        &data.join("AIDOO-Whisper-Lite-Diagnostics-20261314-000000-abcdef123456.zip"),
        &history,
        data
    ));
    assert!(!path_is_authorized_for_open(
        &data.join("AIDOO-Whisper-Lite-Diagnostics-20260914-000000-abcdef12345g.zip"),
        &history,
        data
    ));
}

#[test]
fn diagnostic_settings_exclude_private_device_and_path_details() {
    let settings = AppSettings {
        output_directory: Some("/Users/example/Private Transcripts".into()),
        microphone_name: Some("Owner's Studio Microphone".into()),
        ..AppSettings::default()
    };

    let serialized = diagnostic_settings(&settings).to_string();

    assert!(serialized.contains("\"outputDirectory\":\"custom\""));
    assert!(serialized.contains("\"microphone\":\"custom\""));
    assert!(!serialized.contains("Private Transcripts"));
    assert!(!serialized.contains("Owner's Studio Microphone"));
    assert!(!serialized.contains("/Users/example"));
}

#[test]
fn local_file_preferences_create_only_the_requested_private_artifacts() {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    for (save_audio, save_text) in [(false, false), (true, false), (false, true), (true, true)] {
        let root = std::env::temp_dir().join(format!(
            "aidoo-lite-local-files-test-{}",
            uuid::Uuid::new_v4()
        ));
        let output = root.join("custom-output");
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("source.flac");
        std::fs::write(&source, b"fLaC private audio").unwrap();
        let settings = AppSettings {
            save_audio,
            save_text,
            output_directory: Some(output.to_string_lossy().to_string()),
            ..AppSettings::default()
        };

        let (audio, text) =
            save_local_transcription_files(&settings, &source, "Private text").unwrap();

        assert_eq!(audio.is_some(), save_audio);
        assert_eq!(text.is_some(), save_text);
        assert_eq!(output.exists(), save_audio || save_text);
        if let Some(path) = audio.as_ref() {
            assert_eq!(path.parent(), Some(output.as_path()));
            assert_eq!(std::fs::read(path).unwrap(), b"fLaC private audio");
            #[cfg(unix)]
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        if let Some(path) = text.as_ref() {
            assert_eq!(path.parent(), Some(output.as_path()));
            assert_eq!(std::fs::read(path).unwrap(), b"Private text\n");
            #[cfg(unix)]
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        if let (Some(audio), Some(text)) = (audio.as_ref(), text.as_ref()) {
            assert_eq!(audio.file_stem(), text.file_stem());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn txt_failure_keeps_completed_audio_and_removes_temporary_output() {
    let root = std::env::temp_dir().join(format!(
        "aidoo-lite-partial-local-save-test-{}",
        uuid::Uuid::new_v4()
    ));
    let output = root.join("output");
    std::fs::create_dir_all(output.join("fixed.txt")).unwrap();
    let source = root.join("source.flac");
    std::fs::write(&source, b"fLaC private audio").unwrap();
    let settings = AppSettings {
        save_audio: true,
        save_text: true,
        output_directory: Some(output.to_string_lossy().to_string()),
        ..AppSettings::default()
    };

    let error = save_local_transcription_files_with_stem(
        &settings,
        &source,
        "completed private text",
        "fixed",
    )
    .unwrap_err();

    assert!(error.message.contains("TXT файлът не може да бъде запазен"));
    assert!(error
        .message
        .contains("Създадените локални файлове не са изтрити"));
    assert_eq!(
        std::fs::read(output.join("fixed.flac")).unwrap(),
        b"fLaC private audio"
    );
    assert!(std::fs::read_dir(&output).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".fixed.txt.tmp-")
    }));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn diagnostic_failure_guard_removes_only_uncommitted_temporary_file() {
    let root = std::env::temp_dir().join(format!(
        "aidoo-lite-diagnostic-cleanup-test-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let abandoned = root.join("abandoned.zip.tmp");
    std::fs::write(&abandoned, b"partial private diagnostics").unwrap();
    drop(PendingDiagnosticFile {
        path: abandoned.clone(),
        committed: false,
    });
    assert!(!abandoned.exists());

    let committed = root.join("committed.zip");
    std::fs::write(&committed, b"complete private diagnostics").unwrap();
    drop(PendingDiagnosticFile {
        path: committed.clone(),
        committed: true,
    });
    assert!(committed.is_file());
    std::fs::remove_dir_all(root).unwrap();
}

fn temporary_history_entry(root: &Path) -> TranscriptEntry {
    let mut entry = history_entry();
    entry.audio_path = Some(
        root.join("AIDOO-Whisper-2026-09-15_12-00-00-abcdef123456.flac")
            .to_string_lossy()
            .to_string(),
    );
    entry.text_path = Some(
        root.join("AIDOO-Whisper-2026-09-15_12-00-00-abcdef123456.txt")
            .to_string_lossy()
            .to_string(),
    );
    entry
}

#[test]
fn history_file_deletion_is_reversible_until_history_is_committed() {
    let root = std::env::temp_dir().join(format!(
        "aidoo-lite-history-delete-test-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let entry = temporary_history_entry(&root);
    let originals = [
        PathBuf::from(entry.audio_path.as_ref().unwrap()),
        PathBuf::from(entry.text_path.as_ref().unwrap()),
    ];
    std::fs::write(&originals[0], b"fLaC").unwrap();
    std::fs::write(&originals[1], b"text").unwrap();

    let deletion = prepare_history_files_for_deletion(&entry).unwrap();
    stage_history_files_for_deletion(&deletion).unwrap();

    assert_eq!(deletion.files.len(), 2);
    assert!(originals.iter().all(|path| !path.exists()));
    assert!(deletion
        .files
        .iter()
        .all(|file| Path::new(&file.staged).is_file()));
    assert!(restore_staged_history_files(&deletion).is_empty());
    assert!(originals.iter().all(|path| path.is_file()));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn committed_history_file_deletion_removes_both_linked_files() {
    let root = std::env::temp_dir().join(format!(
        "aidoo-lite-history-delete-commit-test-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let entry = temporary_history_entry(&root);
    let originals = [
        PathBuf::from(entry.audio_path.as_ref().unwrap()),
        PathBuf::from(entry.text_path.as_ref().unwrap()),
    ];
    std::fs::write(&originals[0], b"fLaC").unwrap();
    std::fs::write(&originals[1], b"text").unwrap();
    let deletion = prepare_history_files_for_deletion(&entry).unwrap();
    stage_history_files_for_deletion(&deletion).unwrap();

    assert!(commit_staged_history_deletion(&deletion).is_empty());

    assert!(originals.iter().all(|path| !path.exists()));
    assert!(deletion
        .files
        .iter()
        .all(|file| !Path::new(&file.staged).exists()));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn interrupted_history_deletion_rolls_back_when_history_still_contains_the_entry() {
    let root = std::env::temp_dir().join(format!(
        "aidoo-lite-history-delete-recovery-test-{}",
        uuid::Uuid::new_v4()
    ));
    let output = root.join("output");
    let private = root.join("private");
    std::fs::create_dir_all(&output).unwrap();
    let entry = temporary_history_entry(&output);
    let originals = [
        PathBuf::from(entry.audio_path.as_ref().unwrap()),
        PathBuf::from(entry.text_path.as_ref().unwrap()),
    ];
    std::fs::write(&originals[0], b"fLaC").unwrap();
    std::fs::write(&originals[1], b"text").unwrap();
    let deletion = prepare_history_files_for_deletion(&entry).unwrap();

    crate::storage::with_test_data_dir(private, || {
        crate::storage::ensure_directories().unwrap();
        crate::storage::save_pending_history_deletion(&deletion).unwrap();
        stage_history_files_for_deletion(&deletion).unwrap();
        let mut history = vec![entry.clone()];
        recover_pending_history_deletion(&mut history);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, entry.id);
        assert!(crate::storage::load_pending_history_deletion().is_none());
    });

    assert!(originals.iter().all(|path| path.is_file()));
    assert!(deletion
        .files
        .iter()
        .all(|file| !Path::new(&file.staged).exists()));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn interrupted_history_deletion_finishes_when_history_commit_is_present() {
    let root = std::env::temp_dir().join(format!(
        "aidoo-lite-history-delete-finish-test-{}",
        uuid::Uuid::new_v4()
    ));
    let output = root.join("output");
    let private = root.join("private");
    std::fs::create_dir_all(&output).unwrap();
    let entry = temporary_history_entry(&output);
    let originals = [
        PathBuf::from(entry.audio_path.as_ref().unwrap()),
        PathBuf::from(entry.text_path.as_ref().unwrap()),
    ];
    std::fs::write(&originals[0], b"fLaC").unwrap();
    std::fs::write(&originals[1], b"text").unwrap();
    let mut deletion = prepare_history_files_for_deletion(&entry).unwrap();
    deletion.history_committed = true;

    crate::storage::with_test_data_dir(private, || {
        crate::storage::ensure_directories().unwrap();
        crate::storage::save_pending_history_deletion(&deletion).unwrap();
        stage_history_files_for_deletion(&deletion).unwrap();
        let mut history = Vec::new();
        recover_pending_history_deletion(&mut history);
        assert!(history.is_empty());
        assert!(crate::storage::load_pending_history_deletion().is_none());
    });

    assert!(originals.iter().all(|path| !path.exists()));
    assert!(deletion
        .files
        .iter()
        .all(|file| !Path::new(&file.staged).exists()));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn prepared_deletion_restores_the_history_entry_if_commit_phase_was_not_saved() {
    let root = std::env::temp_dir().join(format!(
        "aidoo-lite-history-delete-phase-test-{}",
        uuid::Uuid::new_v4()
    ));
    let output = root.join("output");
    let private = root.join("private");
    std::fs::create_dir_all(&output).unwrap();
    let entry = temporary_history_entry(&output);
    let originals = [
        PathBuf::from(entry.audio_path.as_ref().unwrap()),
        PathBuf::from(entry.text_path.as_ref().unwrap()),
    ];
    std::fs::write(&originals[0], b"fLaC").unwrap();
    std::fs::write(&originals[1], b"text").unwrap();
    let deletion = prepare_history_files_for_deletion(&entry).unwrap();

    crate::storage::with_test_data_dir(private, || {
        crate::storage::ensure_directories().unwrap();
        crate::storage::save_pending_history_deletion(&deletion).unwrap();
        stage_history_files_for_deletion(&deletion).unwrap();
        let mut history = Vec::new();
        recover_pending_history_deletion(&mut history);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, entry.id);
        assert_eq!(crate::storage::load_history()[0].id, entry.id);
        assert!(crate::storage::load_pending_history_deletion().is_none());
    });

    assert!(originals.iter().all(|path| path.is_file()));
    assert!(deletion
        .files
        .iter()
        .all(|file| !Path::new(&file.staged).exists()));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn recovery_remains_visible_in_the_tray_until_it_is_resolved() {
    assert_eq!(resolved_tray_state("idle", true, true, true), "recovery");
    assert_eq!(resolved_tray_state("error", true, true, true), "recovery");
    assert_eq!(
        resolved_tray_state("transcribing", true, true, true),
        "transcribing"
    );
    assert_eq!(resolved_tray_state("idle", true, true, false), "idle");
    assert_eq!(
        resolved_tray_state("idle", false, true, false),
        "permission"
    );
    assert_eq!(resolved_tray_state("idle", true, false, false), "setup");
}

#[test]
fn tray_tooltip_covers_starting_processing_and_completion() {
    assert_eq!(
        tray_tooltip("starting", true),
        "AIDOO Whisper Lite — starting microphone"
    );
    assert_eq!(
        tray_tooltip("transcribing", false),
        "AIDOO Whisper Lite — транскрибирам"
    );
    assert_eq!(
        tray_tooltip("done", false),
        "AIDOO Whisper Lite — транскрипцията е готова"
    );
    assert_eq!(
        tray_tooltip("wake-listening", true),
        "AIDOO Whisper Lite — listening for Hey, AIDOO"
    );
    assert_eq!(
        tray_tooltip("wake-error", false),
        "AIDOO Whisper Lite — проблем с гласовото активиране"
    );
}

#[test]
fn overlay_stays_visible_until_dictation_is_ready_again() {
    for state in ["starting", "recording", "transcribing", "done", "error"] {
        assert!(overlay_visible_for_state(state), "{state}");
    }
    assert!(!overlay_visible_for_state("idle"));
    assert!(!overlay_visible_for_state("unknown"));
}

#[test]
fn overlay_accepts_clicks_only_while_its_stop_button_is_actionable() {
    assert!(overlay_accepts_pointer_input("starting"));
    assert!(overlay_accepts_pointer_input("recording"));
    for state in ["idle", "transcribing", "done", "error"] {
        assert!(!overlay_accepts_pointer_input(state), "{state}");
    }
}

#[test]
fn quitting_is_blocked_for_the_full_native_operation() {
    assert!(!operation_allows_quit(true));
    assert!(operation_allows_quit(false));
}

#[test]
fn stale_recording_watchdog_cannot_stop_a_new_recording() {
    assert!(recording_watchdog_should_stop(true, 7, 7));
    assert!(!recording_watchdog_should_stop(false, 7, 7));
    assert!(!recording_watchdog_should_stop(true, 8, 7));
}

#[test]
fn tray_reports_every_processing_stage_in_both_languages() {
    for stage in [
        "preparing_audio",
        "starting_microphone",
        "compressing_audio",
        "uploading_audio",
        "openai_transcribing",
        "text_ready",
        "finishing_locally",
    ] {
        assert!(
            super::progress_status_label(stage, true).is_some(),
            "{stage}"
        );
        assert!(
            super::progress_status_label(stage, false).is_some(),
            "{stage}"
        );
    }
    assert_eq!(super::progress_status_label("unknown", true), None);
}

#[test]
fn tray_errors_follow_the_selected_interface_language() {
    assert_eq!(
        localized_native_error("Транскрипцията не успя: HTTP 500", true),
        "Transcription failed: HTTP 500"
    );
    assert_eq!(
        localized_native_error(
            "Транскрипцията не успя: няма наличен API баланс или е достигнат лимитът.",
            true
        ),
        "Transcription failed: no API balance is available or the limit has been reached."
    );
    assert_eq!(
        localized_native_error("Не беше разпозната реч.", false),
        "Не беше разпозната реч."
    );
    assert_eq!(
        localized_native_error(
            "Моделът за „Hey, AIDOO“ не е намерен: /missing/model.onnx",
            true
        ),
        "The “Hey, AIDOO” model was not found: /missing/model.onnx"
    );
}
