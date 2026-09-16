use crate::audio::{microphone_names, MicrophoneRoutingConfig};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use livekit_wakeword::WakeWordModel;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const INFERENCE_INTERVAL: Duration = Duration::from_millis(250);
// The model produces a short cluster of high scores for one spoken phrase. A 1.2 s guard
// suppresses that cluster without swallowing the next deliberate calibration attempt.
const DETECTION_DEBOUNCE: Duration = Duration::from_millis(1_200);
const VOICE_RMS_GATE: f32 = 0.006;
const AUDIO_LEVEL_INTERVAL: Duration = Duration::from_millis(100);
const TRAILING_INFERENCE_COUNT: usize = 4;
const PRIMARY_MODEL_NAME: &str = "hey_aidoo";
const CONFIRMATION_MODEL_NAME: &str = "hey_aidoo_confirmation";
const CONFIRMATION_HISTORY: usize = 3;

struct ConfirmationState {
    recent_primary: VecDeque<bool>,
    recent_confirmation: VecDeque<bool>,
}

struct AudioLevelReporter {
    frames: usize,
    peak: f32,
    interval_frames: usize,
}

impl AudioLevelReporter {
    fn new(sample_rate: u32) -> Self {
        Self {
            frames: 0,
            peak: 0.0,
            interval_frames: ((sample_rate as u64 * AUDIO_LEVEL_INTERVAL.as_millis() as u64)
                / 1_000)
                .max(1) as usize,
        }
    }

    fn observe(&mut self, sample_count: usize, level: f32) -> Option<f32> {
        self.frames = self.frames.saturating_add(sample_count);
        self.peak = self.peak.max(level);
        if self.frames < self.interval_frames {
            return None;
        }
        self.frames = 0;
        Some(std::mem::take(&mut self.peak))
    }
}

impl ConfirmationState {
    fn new() -> Self {
        Self {
            recent_primary: VecDeque::with_capacity(CONFIRMATION_HISTORY),
            recent_confirmation: VecDeque::with_capacity(CONFIRMATION_HISTORY),
        }
    }

    fn pending(&self) -> bool {
        self.recent_primary.iter().any(|detected| *detected)
            || self.recent_confirmation.iter().any(|detected| *detected)
    }

    fn observe(&mut self, primary: bool, confirmation: bool) -> bool {
        let confirmed = (confirmation
            && (primary || self.recent_primary.iter().any(|detected| *detected)))
            || (primary && self.recent_confirmation.iter().any(|detected| *detected));
        self.recent_primary.push_back(primary);
        self.recent_confirmation.push_back(confirmation);
        while self.recent_primary.len() > CONFIRMATION_HISTORY {
            self.recent_primary.pop_front();
        }
        while self.recent_confirmation.len() > CONFIRMATION_HISTORY {
            self.recent_confirmation.pop_front();
        }
        confirmed
    }
}

fn wake_phrase_detected(
    state: &mut ConfirmationState,
    primary_score: f32,
    confirmation_score: f32,
    primary_threshold: f32,
    confirmation_threshold: f32,
) -> bool {
    let primary = primary_score >= primary_threshold;
    let confirmation = confirmation_score >= confirmation_threshold;
    state.observe(primary, confirmation)
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WakeWordEvent {
    Detected {
        confidence: f32,
    },
    Scores {
        rms: f32,
        primary: f32,
        confirmation: f32,
    },
    Level {
        rms: f32,
    },
    Failed(String),
}

enum Command {
    Start {
        routing: MicrophoneRoutingConfig,
        primary_model_path: PathBuf,
        confirmation_model_path: PathBuf,
        primary_threshold: f32,
        confirmation_threshold: f32,
        reply: mpsc::Sender<Result<String, String>>,
    },
    Stop(mpsc::Sender<()>),
}

struct ActiveListener {
    stream: Option<Stream>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

struct InferenceContext {
    audio_buffer: Arc<Mutex<VecDeque<i16>>>,
    recent_voice: Arc<AtomicBool>,
    audio: mpsc::Receiver<()>,
    stop: Arc<AtomicBool>,
    events: mpsc::Sender<WakeWordEvent>,
    sample_rate: u32,
    primary_threshold: f32,
    confirmation_threshold: f32,
}

impl Drop for ActiveListener {
    fn drop(&mut self) {
        self.stream.take();
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub struct WakeWordService {
    commands: mpsc::Sender<Command>,
    events: std::sync::Mutex<Option<mpsc::Receiver<WakeWordEvent>>>,
}

impl WakeWordService {
    pub fn new() -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("aidoo-wakeword-control".into())
            .spawn(move || {
                let mut active: Option<ActiveListener> = None;
                while let Ok(command) = command_rx.recv() {
                    match command {
                        Command::Start {
                            routing,
                            primary_model_path,
                            confirmation_model_path,
                            primary_threshold,
                            confirmation_threshold,
                            reply,
                        } => {
                            let result = if active.is_some() {
                                Ok("already-listening".into())
                            } else {
                                start(
                                    &routing,
                                    primary_model_path,
                                    confirmation_model_path,
                                    primary_threshold,
                                    confirmation_threshold,
                                    event_tx.clone(),
                                )
                                .map(|(listener, device_name)| {
                                    active = Some(listener);
                                    device_name
                                })
                            };
                            let _ = reply.send(result);
                        }
                        Command::Stop(reply) => {
                            active.take();
                            let _ = reply.send(());
                        }
                    }
                }
            })
            .expect("wake word control thread must start");
        Self {
            commands,
            events: std::sync::Mutex::new(Some(event_rx)),
        }
    }

    pub fn take_events(&self) -> Option<mpsc::Receiver<WakeWordEvent>> {
        self.events.lock().ok()?.take()
    }

    pub fn start(
        &self,
        routing: MicrophoneRoutingConfig,
        primary_model_path: PathBuf,
        confirmation_model_path: PathBuf,
        primary_threshold: f32,
        confirmation_threshold: f32,
    ) -> Result<String, String> {
        let (reply, response) = mpsc::channel();
        self.commands
            .send(Command::Start {
                routing,
                primary_model_path,
                confirmation_model_path,
                primary_threshold,
                confirmation_threshold,
                reply,
            })
            .map_err(|_| "Гласовото активиране не работи.".to_string())?;
        response
            .recv_timeout(COMMAND_TIMEOUT)
            .map_err(|_| "Гласовото активиране не отговори.".to_string())?
    }

    pub fn stop(&self) {
        let (reply, response) = mpsc::channel();
        if self.commands.send(Command::Stop(reply)).is_ok() {
            let _ = response.recv_timeout(COMMAND_TIMEOUT);
        }
    }
}

fn start(
    routing: &MicrophoneRoutingConfig,
    primary_model_path: PathBuf,
    confirmation_model_path: PathBuf,
    primary_threshold: f32,
    confirmation_threshold: f32,
    events: mpsc::Sender<WakeWordEvent>,
) -> Result<(ActiveListener, String), String> {
    if !primary_model_path.is_file() {
        return Err(format!(
            "Основният модел за „Hey, AIDOO“ не е намерен: {}",
            primary_model_path.display()
        ));
    }
    if !confirmation_model_path.is_file() {
        return Err(format!(
            "Потвърждаващият модел за „Hey, AIDOO“ не е намерен: {}",
            confirmation_model_path.display()
        ));
    }
    let host = cpal::default_host();
    let available = microphone_names();
    let default_name = host
        .default_input_device()
        .and_then(|device| device.name().ok());
    let plan = microphone_plan(
        routing.preferred_name.as_deref(),
        &available,
        default_name.as_deref(),
        routing.automatic_fallback,
    );
    let mut failures = Vec::new();
    for name in plan {
        let result = device_named(&name).and_then(|device| {
            start_on_device(
                device,
                &primary_model_path,
                &confirmation_model_path,
                primary_threshold,
                confirmation_threshold,
                events.clone(),
            )
        });
        match result {
            Ok(listener) => return Ok((listener, name)),
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }
    if failures.is_empty() {
        Err("Не е намерен микрофон за гласово активиране.".into())
    } else {
        Err(format!(
            "Гласовото активиране не можа да стартира. {}",
            failures.join(" · ")
        ))
    }
}

fn start_on_device(
    device: cpal::Device,
    primary_model_path: &std::path::Path,
    confirmation_model_path: &std::path::Path,
    primary_threshold: f32,
    confirmation_threshold: f32,
    events: mpsc::Sender<WakeWordEvent>,
) -> Result<ActiveListener, String> {
    let supported = device
        .default_input_config()
        .map_err(|error| error.to_string())?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let sample_rate = config.sample_rate.0;
    let channels = config.channels;
    let model = load_wake_word_model(primary_model_path, confirmation_model_path, sample_rate)?;
    let audio_buffer = Arc::new(Mutex::new(VecDeque::<i16>::with_capacity(
        sample_rate as usize * 3,
    )));
    let recent_voice = Arc::new(AtomicBool::new(false));
    let (audio_tx, audio_rx) = mpsc::sync_channel::<()>(1);
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker_events = events.clone();
    let worker_audio_buffer = audio_buffer.clone();
    let worker_recent_voice = recent_voice.clone();
    let worker = std::thread::Builder::new()
        .name("aidoo-wakeword-inference".into())
        .spawn(move || {
            inference_loop(
                model,
                InferenceContext {
                    audio_buffer: worker_audio_buffer,
                    recent_voice: worker_recent_voice,
                    audio: audio_rx,
                    stop: worker_stop,
                    events: worker_events,
                    sample_rate,
                    primary_threshold,
                    confirmation_threshold,
                },
            )
        })
        .map_err(|error| error.to_string())?;

    let error_events = events.clone();
    let error_callback = move |error| {
        let _ = error_events.send(WakeWordEvent::Failed(format!(
            "Микрофонът за гласово активиране прекъсна: {error}"
        )));
    };
    let max_buffer_samples = sample_rate as usize * 3;
    let stream_result = match sample_format {
        SampleFormat::F32 => {
            let buffer = audio_buffer.clone();
            let voice = recent_voice.clone();
            let level_events = events.clone();
            let mut level_reporter = AudioLevelReporter::new(sample_rate);
            device.build_input_stream(
                &config,
                move |data: &[f32], _| {
                    push_audio(
                        &buffer,
                        &voice,
                        &audio_tx,
                        downmix_f32(data, channels),
                        max_buffer_samples,
                        &mut level_reporter,
                        &level_events,
                    );
                },
                error_callback,
                None,
            )
        }
        SampleFormat::I16 => {
            let buffer = audio_buffer.clone();
            let voice = recent_voice.clone();
            let level_events = events.clone();
            let mut level_reporter = AudioLevelReporter::new(sample_rate);
            device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    push_audio(
                        &buffer,
                        &voice,
                        &audio_tx,
                        downmix_i16(data, channels),
                        max_buffer_samples,
                        &mut level_reporter,
                        &level_events,
                    );
                },
                error_callback,
                None,
            )
        }
        SampleFormat::U16 => {
            let level_events = events;
            let mut level_reporter = AudioLevelReporter::new(sample_rate);
            device.build_input_stream(
                &config,
                move |data: &[u16], _| {
                    let converted = data
                        .iter()
                        .map(|sample| {
                            ((*sample as f32 / u16::MAX as f32) * 2.0 - 1.0) * i16::MAX as f32
                        })
                        .map(|sample| sample as i16)
                        .collect::<Vec<_>>();
                    push_audio(
                        &audio_buffer,
                        &recent_voice,
                        &audio_tx,
                        downmix_i16(&converted, channels),
                        max_buffer_samples,
                        &mut level_reporter,
                        &level_events,
                    );
                },
                error_callback,
                None,
            )
        }
        other => return Err(format!("Неподдържан аудио формат: {other:?}")),
    };
    let stream = match stream_result {
        Ok(stream) => stream,
        Err(error) => {
            stop.store(true, Ordering::Release);
            let _ = worker.join();
            return Err(error.to_string());
        }
    };
    if let Err(error) = stream.play() {
        drop(stream);
        stop.store(true, Ordering::Release);
        let _ = worker.join();
        return Err(error.to_string());
    }
    Ok(ActiveListener {
        stream: Some(stream),
        stop,
        worker: Some(worker),
    })
}

fn load_wake_word_model(
    primary_model_path: &std::path::Path,
    confirmation_model_path: &std::path::Path,
    sample_rate: u32,
) -> Result<WakeWordModel, String> {
    WakeWordModel::new(&[primary_model_path, confirmation_model_path], sample_rate)
        .map_err(|error| format!("Wake-word моделът не може да се зареди: {error}"))
}

fn push_audio(
    buffer: &Mutex<VecDeque<i16>>,
    recent_voice: &AtomicBool,
    signal: &mpsc::SyncSender<()>,
    samples: Vec<i16>,
    max_samples: usize,
    level_reporter: &mut AudioLevelReporter,
    events: &mpsc::Sender<WakeWordEvent>,
) {
    let level = rms(&samples);
    if level >= VOICE_RMS_GATE {
        recent_voice.store(true, Ordering::Release);
    }
    if let Some(rms) = level_reporter.observe(samples.len(), level) {
        let _ = events.send(WakeWordEvent::Level { rms });
    }
    if let Ok(mut buffer) = buffer.lock() {
        buffer.extend(samples);
        while buffer.len() > max_samples {
            buffer.pop_front();
        }
    }
    let _ = signal.try_send(());
}

fn inference_loop(mut model: WakeWordModel, context: InferenceContext) {
    let window_samples = context.sample_rate as usize * 2;
    let mut last_inference = Instant::now()
        .checked_sub(INFERENCE_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut last_detection = Instant::now()
        .checked_sub(DETECTION_DEBOUNCE)
        .unwrap_or_else(Instant::now);
    let mut confirmation_state = ConfirmationState::new();
    let mut trailing_inferences = 0;
    while !context.stop.load(Ordering::Acquire) {
        match context.audio.recv_timeout(Duration::from_millis(100)) {
            Ok(()) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        let confirmation_pending = confirmation_state.pending();
        if !should_run_inference(
            last_inference.elapsed() >= INFERENCE_INTERVAL,
            &context.recent_voice,
            confirmation_pending,
            &mut trailing_inferences,
        ) {
            continue;
        }
        let contiguous = match context.audio_buffer.lock() {
            Ok(buffer) if buffer.len() >= window_samples => buffer
                .iter()
                .skip(buffer.len() - window_samples)
                .copied()
                .collect::<Vec<_>>(),
            _ => continue,
        };
        last_inference = Instant::now();
        match model.predict(&contiguous) {
            Ok(scores) => {
                let primary = scores.get(PRIMARY_MODEL_NAME).copied().unwrap_or(0.0);
                let confirmation = scores.get(CONFIRMATION_MODEL_NAME).copied().unwrap_or(0.0);
                let _ = context.events.send(WakeWordEvent::Scores {
                    rms: rms(&contiguous),
                    primary,
                    confirmation,
                });
                let confirmed = wake_phrase_detected(
                    &mut confirmation_state,
                    primary,
                    confirmation,
                    context.primary_threshold,
                    context.confirmation_threshold,
                );
                if confirmed && last_detection.elapsed() >= DETECTION_DEBOUNCE {
                    last_detection = Instant::now();
                    let _ = context.events.send(WakeWordEvent::Detected {
                        confidence: primary.max(confirmation),
                    });
                }
            }
            Err(error) => {
                let _ = context.events.send(WakeWordEvent::Failed(format!(
                    "Wake-word разпознаването спря: {error}"
                )));
                break;
            }
        }
    }
}

fn should_run_inference(
    interval_ready: bool,
    recent_voice: &AtomicBool,
    confirmation_pending: bool,
    trailing_inferences: &mut usize,
) -> bool {
    if !interval_ready {
        return false;
    }
    if recent_voice.swap(false, Ordering::AcqRel) {
        *trailing_inferences = TRAILING_INFERENCE_COUNT;
    }
    let should_run = confirmation_pending || *trailing_inferences > 0;
    if should_run && *trailing_inferences > 0 {
        *trailing_inferences -= 1;
    }
    should_run
}

#[cfg(test)]
mod confirmation_tests {
    use super::{wake_phrase_detected, ConfirmationState};

    #[test]
    fn confirmation_accepts_any_of_the_previous_three_intervals() {
        for delay in 1..=3 {
            let mut state = ConfirmationState::new();
            assert!(!state.observe(true, false));
            for _ in 1..delay {
                assert!(!state.observe(false, false));
            }
            assert!(state.observe(false, true), "delay {delay} must confirm");
        }
    }

    #[test]
    fn confirmation_accepts_same_interval_and_reverse_adjacent_order() {
        let mut same_interval = ConfirmationState::new();
        assert!(same_interval.observe(true, true));

        let mut reverse_order = ConfirmationState::new();
        assert!(!reverse_order.observe(false, true));
        assert!(reverse_order.observe(true, false));
    }

    #[test]
    fn repeated_primary_detection_without_confirmation_is_rejected() {
        let mut state = ConfirmationState::new();
        assert!(!state.observe(true, false));
        assert!(!state.observe(true, false));
    }

    #[test]
    fn strong_primary_detection_without_confirmation_is_rejected() {
        let mut state = ConfirmationState::new();
        assert!(!wake_phrase_detected(&mut state, 0.96, 0.01, 0.68, 0.76));
    }

    #[test]
    fn isolated_medium_primary_score_is_rejected_as_a_false_positive() {
        let mut state = ConfirmationState::new();
        assert!(!wake_phrase_detected(&mut state, 0.84, 0.01, 0.68, 0.76));
    }

    #[test]
    fn repeated_medium_primary_scores_without_confirmation_are_rejected() {
        let mut state = ConfirmationState::new();
        assert!(!wake_phrase_detected(&mut state, 0.84, 0.01, 0.68, 0.76));
        assert!(!wake_phrase_detected(&mut state, 0.84, 0.01, 0.68, 0.76));
    }

    #[test]
    fn confirmation_rejects_expired_and_isolated_candidates() {
        let mut expired = ConfirmationState::new();
        assert!(!expired.observe(true, false));
        for _ in 0..3 {
            assert!(!expired.observe(false, false));
        }
        assert!(!expired.observe(false, true));

        let mut isolated = ConfirmationState::new();
        assert!(!isolated.observe(true, false));
        let mut isolated = ConfirmationState::new();
        assert!(!isolated.observe(false, true));
    }
}

fn rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum = samples
        .iter()
        .map(|sample| {
            let value = *sample as f64 / i16::MAX as f64;
            value * value
        })
        .sum::<f64>();
    (sum / samples.len() as f64).sqrt() as f32
}

fn downmix_f32(interleaved: &[f32], channels: u16) -> Vec<i16> {
    let channels = usize::from(channels.max(1));
    interleaved
        .chunks(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / frame.len() as f32)
        .map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect()
}

fn downmix_i16(interleaved: &[i16], channels: u16) -> Vec<i16> {
    let channels = usize::from(channels.max(1));
    interleaved
        .chunks(channels)
        .map(|frame| {
            let sum = frame.iter().map(|sample| i32::from(*sample)).sum::<i32>();
            (sum / frame.len() as i32) as i16
        })
        .collect()
}

fn device_named(name: &str) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    if let Ok(devices) = host.input_devices() {
        for device in devices {
            if device.name().ok().as_deref() == Some(name) {
                return Ok(device);
            }
        }
    }
    Err(format!("Микрофонът „{name}“ не е наличен."))
}

fn microphone_plan(
    preferred: Option<&str>,
    available: &[String],
    default_name: Option<&str>,
    automatic_fallback: bool,
) -> Vec<String> {
    let mut plan = Vec::new();
    let mut push_unique = |name: &str| {
        if !name.trim().is_empty() && !plan.iter().any(|item: &String| item == name) {
            plan.push(name.to_string());
        }
    };
    if let Some(preferred) = preferred {
        push_unique(preferred);
    } else if let Some(default_name) = default_name {
        push_unique(default_name);
    }
    if automatic_fallback {
        if let Some(default_name) = default_name {
            push_unique(default_name);
        }
        for name in available {
            push_unique(name);
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_detector_exposes_both_required_classifier_scores() {
        let resource_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("wakeword");
        let primary = resource_dir.join("hey_aidoo.onnx");
        let confirmation = resource_dir.join("hey_aidoo_confirmation.onnx");
        let mut model = load_wake_word_model(&primary, &confirmation, 16_000).unwrap();

        let scores = model.predict(&vec![0; 32_000]).unwrap();

        assert!(scores.contains_key(PRIMARY_MODEL_NAME));
        assert!(
            scores.contains_key(CONFIRMATION_MODEL_NAME),
            "the temporal detector cannot recognize any phrase without its confirmation classifier"
        );
    }

    #[test]
    fn downmix_preserves_mono_and_averages_channels() {
        assert_eq!(downmix_i16(&[100, -100, 300, 100], 2), vec![0, 200]);
        assert_eq!(downmix_i16(&[7, -9], 1), vec![7, -9]);
    }

    #[test]
    fn voice_gate_rejects_silence_and_accepts_speech_level_audio() {
        assert_eq!(rms(&[0; 32]), 0.0);
        assert!(rms(&[1000; 32]) > VOICE_RMS_GATE);
    }

    #[test]
    fn ring_buffer_keeps_contiguous_audio_when_the_wake_signal_is_full() {
        let buffer = Mutex::new(VecDeque::new());
        let recent_voice = AtomicBool::new(false);
        let (signal, receiver) = mpsc::sync_channel(1);
        let (events, _event_receiver) = mpsc::channel();
        let mut level_reporter = AudioLevelReporter::new(100);

        push_audio(
            &buffer,
            &recent_voice,
            &signal,
            vec![1, 2, 3],
            4,
            &mut level_reporter,
            &events,
        );
        push_audio(
            &buffer,
            &recent_voice,
            &signal,
            vec![4, 5, 6],
            4,
            &mut level_reporter,
            &events,
        );

        assert_eq!(
            buffer.lock().unwrap().iter().copied().collect::<Vec<_>>(),
            vec![3, 4, 5, 6]
        );
        assert_eq!(receiver.try_recv(), Ok(()));
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn audio_level_reporter_emits_frequent_direct_microphone_feedback() {
        let mut reporter = AudioLevelReporter::new(1_000);
        assert_eq!(reporter.observe(40, 0.01), None);
        assert_eq!(reporter.observe(60, 0.04), Some(0.04));
        assert_eq!(reporter.observe(100, 0.02), Some(0.02));
    }

    #[test]
    fn inference_keeps_voice_pending_until_the_interval_is_ready() {
        let recent_voice = AtomicBool::new(true);
        let mut trailing = 0;
        assert!(!should_run_inference(
            false,
            &recent_voice,
            false,
            &mut trailing
        ));
        assert!(recent_voice.load(Ordering::Acquire));
        assert!(should_run_inference(
            true,
            &recent_voice,
            false,
            &mut trailing
        ));
        assert!(!recent_voice.load(Ordering::Acquire));
        assert_eq!(trailing, TRAILING_INFERENCE_COUNT - 1);

        for expected in (0..TRAILING_INFERENCE_COUNT - 1).rev() {
            assert!(should_run_inference(
                true,
                &recent_voice,
                false,
                &mut trailing
            ));
            assert_eq!(trailing, expected);
        }
        assert!(!should_run_inference(
            true,
            &recent_voice,
            false,
            &mut trailing
        ));
    }
}
