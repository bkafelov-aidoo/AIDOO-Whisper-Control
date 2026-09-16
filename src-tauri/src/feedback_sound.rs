use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FeedbackSound {
    Start,
    Stop,
}

pub(super) fn for_recording_transition(previous: &str, next: &str) -> Option<FeedbackSound> {
    match (previous, next) {
        ("recording", "recording") => None,
        (_, "recording") => Some(FeedbackSound::Start),
        ("recording", _) => Some(FeedbackSound::Stop),
        _ => None,
    }
}

pub(super) fn for_live_transition(previous: &str, next: &str) -> Option<FeedbackSound> {
    match (previous, next) {
        ("idle" | "error", "preparing") => Some(FeedbackSound::Start),
        ("switching", "idle") => None,
        ("idle" | "error", _) => None,
        (_, "idle" | "error") => Some(FeedbackSound::Stop),
        _ => None,
    }
}

pub(super) fn play(app: &AppHandle, sound: FeedbackSound) {
    #[cfg(target_os = "macos")]
    macos::play(app, sound);

    #[cfg(not(target_os = "macos"))]
    let _ = (app, sound);
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2::rc::Retained;
    use objc2::AnyThread;
    use objc2_app_kit::NSSound;
    use objc2_foundation::NSString;
    use std::cell::RefCell;
    use tauri::path::BaseDirectory;

    struct Players {
        start: Option<Retained<NSSound>>,
        stop: Option<Retained<NSSound>>,
    }

    thread_local! {
        static PLAYERS: RefCell<Players> = const {
            RefCell::new(Players { start: None, stop: None })
        };
    }

    pub(super) fn play(app: &AppHandle, sound: FeedbackSound) {
        let file = match sound {
            FeedbackSound::Start => "sounds/recording-start.wav",
            FeedbackSound::Stop => "sounds/recording-stop.wav",
        };
        let Ok(path) = app.path().resolve(file, BaseDirectory::Resource) else {
            return;
        };
        let Some(path) = path.to_str().map(str::to_owned) else {
            return;
        };
        let _ = app.run_on_main_thread(move || {
            PLAYERS.with_borrow_mut(|players| {
                let cached = match sound {
                    FeedbackSound::Start => &mut players.start,
                    FeedbackSound::Stop => &mut players.stop,
                };
                if cached.is_none() {
                    *cached = load(&path);
                }
                if let Some(player) = cached {
                    player.stop();
                    player.play();
                }
            });
        });
    }

    fn load(path: &str) -> Option<Retained<NSSound>> {
        let path = NSString::from_str(path);
        let player = NSSound::initWithContentsOfFile_byReference(NSSound::alloc(), &path, false)?;
        player.setVolume(0.82);
        Some(player)
    }
}

#[cfg(test)]
mod tests {
    use super::{for_live_transition, for_recording_transition, FeedbackSound};

    #[test]
    fn feedback_tracks_real_recording_boundaries_only() {
        assert_eq!(
            for_recording_transition("starting", "recording"),
            Some(FeedbackSound::Start)
        );
        assert_eq!(
            for_recording_transition("recording", "transcribing"),
            Some(FeedbackSound::Stop)
        );
        assert_eq!(
            for_recording_transition("recording", "idle"),
            Some(FeedbackSound::Stop)
        );
        assert_eq!(for_recording_transition("starting", "error"), None);
        assert_eq!(for_recording_transition("recording", "recording"), None);
    }

    #[test]
    fn feedback_tracks_live_conversation_boundaries() {
        assert_eq!(
            for_live_transition("idle", "preparing"),
            Some(FeedbackSound::Start)
        );
        assert_eq!(for_live_transition("preparing", "connecting"), None);
        assert_eq!(for_live_transition("listening", "speaking"), None);
        assert_eq!(
            for_live_transition("closing", "idle"),
            Some(FeedbackSound::Stop)
        );
        assert_eq!(for_live_transition("switching", "idle"), None);
    }
}
