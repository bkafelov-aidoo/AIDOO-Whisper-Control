use crate::audio::{microphone_names, MicrophoneRoutingConfig};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use livekit_wakeword::WakeWordModel;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const INFERENCE_INTERVAL: Duration = Duration::from_millis(250);
const DETECTION_DEBOUNCE: Duration = Duration::from_secs(3);
const VOICE_RMS_GATE: f32 = 0.006;
const PRIMARY_MODEL_NAME: &str = "hey_aidoo";
const CONFIRMATION_MODEL_NAME: &str = "hey_aidoo_confirmation";
const CONFIRMATION_HISTORY: usize = 3;

struct ConfirmationState {
    recent_primary: VecDeque<bool>,
}

impl ConfirmationState {
    fn new() -> Self {
        Self {
            recent_primary: VecDeque::with_capacity(CONFIRMATION_HISTORY),
        }
    }

    fn pending(&self) -> bool {
        self.recent_primary.iter().any(|detected| *detected)
    }

    fn observe(&mut self, primary: bool, confirmation: bool) -> bool {
        let confirmed = confirmation && self.pending();
        self.recent_primary.push_back(primary);
        while self.recent_primary.len() > CONFIRMATION_HISTORY {
            self.recent_primary.pop_front();
        }
        confirmed
    }
}

#[derive(Debug, Clone)]
pub enum WakeWordEvent {
    Detected { confidence: f32 },
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
    let model = WakeWordModel::new(&[primary_model_path, confirmation_model_path], sample_rate)
        .map_err(|error| format!("Wake-word моделът не може да се зареди: {error}"))?;
    let (audio_tx, audio_rx) = mpsc::sync_channel::<Vec<i16>>(8);
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker_events = events.clone();
    let worker = std::thread::Builder::new()
        .name("aidoo-wakeword-inference".into())
        .spawn(move || {
            inference_loop(
                model,
                audio_rx,
                worker_stop,
                worker_events,
                sample_rate,
                primary_threshold,
                confirmation_threshold,
            )
        })
        .map_err(|error| error.to_string())?;

    let error_events = events;
    let error_callback = move |error| {
        let _ = error_events.send(WakeWordEvent::Failed(format!(
            "Микрофонът за гласово активиране прекъсна: {error}"
        )));
    };
    let stream_result = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _| {
                let _ = audio_tx.try_send(downmix_f32(data, channels));
            },
            error_callback,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| {
                let _ = audio_tx.try_send(downmix_i16(data, channels));
            },
            error_callback,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _| {
                let converted = data
                    .iter()
                    .map(|sample| {
                        ((*sample as f32 / u16::MAX as f32) * 2.0 - 1.0) * i16::MAX as f32
                    })
                    .map(|sample| sample as i16)
                    .collect::<Vec<_>>();
                let _ = audio_tx.try_send(downmix_i16(&converted, channels));
            },
            error_callback,
            None,
        ),
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

fn inference_loop(
    mut model: WakeWordModel,
    audio: mpsc::Receiver<Vec<i16>>,
    stop: Arc<AtomicBool>,
    events: mpsc::Sender<WakeWordEvent>,
    sample_rate: u32,
    primary_threshold: f32,
    confirmation_threshold: f32,
) {
    let window_samples = sample_rate as usize * 2;
    let mut window = VecDeque::with_capacity(window_samples);
    let mut last_inference = Instant::now()
        .checked_sub(INFERENCE_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut last_detection = Instant::now()
        .checked_sub(DETECTION_DEBOUNCE)
        .unwrap_or_else(Instant::now);
    let mut recent_voice = false;
    let mut confirmation_state = ConfirmationState::new();
    while !stop.load(Ordering::Acquire) {
        let samples = match audio.recv_timeout(Duration::from_millis(100)) {
            Ok(samples) => samples,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if rms(&samples) >= VOICE_RMS_GATE {
            recent_voice = true;
        }
        window.extend(samples);
        while window.len() > window_samples {
            window.pop_front();
        }
        let confirmation_pending = confirmation_state.pending();
        if window.len() < window_samples
            || (!recent_voice && !confirmation_pending)
            || last_inference.elapsed() < INFERENCE_INTERVAL
        {
            continue;
        }
        last_inference = Instant::now();
        recent_voice = false;
        let contiguous = window.iter().copied().collect::<Vec<_>>();
        match model.predict(&contiguous) {
            Ok(scores) => {
                let primary = scores.get(PRIMARY_MODEL_NAME).copied().unwrap_or(0.0);
                let confirmation = scores.get(CONFIRMATION_MODEL_NAME).copied().unwrap_or(0.0);
                let confirmed = confirmation_state.observe(
                    primary >= primary_threshold,
                    confirmation >= confirmation_threshold,
                );
                if confirmed && last_detection.elapsed() >= DETECTION_DEBOUNCE {
                    last_detection = Instant::now();
                    let _ = events.send(WakeWordEvent::Detected {
                        confidence: confirmation,
                    });
                }
            }
            Err(error) => {
                let _ = events.send(WakeWordEvent::Failed(format!(
                    "Wake-word разпознаването спря: {error}"
                )));
                break;
            }
        }
    }
}

#[cfg(test)]
mod confirmation_tests {
    use super::ConfirmationState;

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
    fn confirmation_rejects_same_interval_and_expired_candidates() {
        let mut same_interval = ConfirmationState::new();
        assert!(!same_interval.observe(true, true));

        let mut expired = ConfirmationState::new();
        assert!(!expired.observe(true, false));
        for _ in 0..3 {
            assert!(!expired.observe(false, false));
        }
        assert!(!expired.observe(false, true));
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
    fn downmix_preserves_mono_and_averages_channels() {
        assert_eq!(downmix_i16(&[100, -100, 300, 100], 2), vec![0, 200]);
        assert_eq!(downmix_i16(&[7, -9], 1), vec![7, -9]);
    }

    #[test]
    fn voice_gate_rejects_silence_and_accepts_speech_level_audio() {
        assert_eq!(rms(&[0; 32]), 0.0);
        assert!(rms(&[1000; 32]) > VOICE_RMS_GATE);
    }
}
