use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use serde::Serialize;
use std::fs::File;
use std::io::BufWriter;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

const STALE_TEMPORARY_AUDIO_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const AUDIO_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct MicrophoneRoutingConfig {
    pub preferred_name: Option<String>,
    pub automatic_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStartInfo {
    pub device_name: String,
    pub preferred_name: Option<String>,
    pub used_fallback: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicrophoneProbe {
    pub device_name: String,
    pub preferred_name: Option<String>,
    pub used_fallback: bool,
    pub peak_level: f32,
    pub heard_audio: bool,
}

struct ActiveRecording {
    stream: Option<Stream>,
    writer: Arc<Mutex<Option<RecordingWriter>>>,
    path: PathBuf,
    captured_frames: Arc<AtomicU64>,
    peak_level_bits: Arc<AtomicU32>,
    write_error: Arc<Mutex<Option<String>>>,
    sample_rate: u32,
    keep_file: bool,
}

type RecordingWriter = hound::WavWriter<BufWriter<File>>;

impl Drop for ActiveRecording {
    fn drop(&mut self) {
        self.stream.take();
        if let Ok(mut writer) = self.writer.lock() {
            if let Some(writer) = writer.take() {
                let _ = writer.finalize();
            }
        }
        if !self.keep_file {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub struct CapturedAudio {
    pub path: PathBuf,
    pub duration_seconds: f64,
    cleanup_on_drop: bool,
}

impl Drop for CapturedAudio {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

enum RecorderCommand {
    Start(
        MicrophoneRoutingConfig,
        mpsc::Sender<Result<AudioStartInfo, String>>,
    ),
    Finish(mpsc::Sender<Result<CapturedAudio, String>>),
    Probe(
        MicrophoneRoutingConfig,
        mpsc::Sender<Result<MicrophoneProbe, String>>,
    ),
}

#[derive(Clone)]
pub struct RecorderService {
    sender: mpsc::Sender<RecorderCommand>,
}

impl RecorderService {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel::<RecorderCommand>();
        std::thread::spawn(move || {
            let mut active: Option<ActiveRecording> = None;
            while let Ok(command) = receiver.recv() {
                match command {
                    RecorderCommand::Start(routing, reply) => {
                        let result = if active.is_some() {
                            Err("Вече има активен запис.".to_string())
                        } else {
                            start(&routing).map(|(recording, info)| {
                                active = Some(recording);
                                info
                            })
                        };
                        if reply.send(result).is_err() {
                            active.take();
                        }
                    }
                    RecorderCommand::Finish(reply) => {
                        let result = active
                            .take()
                            .ok_or_else(|| "Няма активен запис.".to_string())
                            .and_then(finish);
                        let _ = reply.send(result);
                    }
                    RecorderCommand::Probe(routing, reply) => {
                        let result = if active.is_some() {
                            Err("Вече има активен запис.".to_string())
                        } else {
                            probe(&routing)
                        };
                        let _ = reply.send(result);
                    }
                }
            }
        });
        Self { sender }
    }

    pub fn start(&self, routing: MicrophoneRoutingConfig) -> Result<AudioStartInfo, String> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send(RecorderCommand::Start(routing, reply))
            .map_err(|_| "Аудио услугата не работи.".to_string())?;
        match response.recv_timeout(AUDIO_COMMAND_TIMEOUT) {
            Ok(result) => result,
            Err(_) => {
                self.request_stop_without_waiting();
                Err("Аудио услугата не отговори.".into())
            }
        }
    }

    pub fn finish(&self) -> Result<CapturedAudio, String> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send(RecorderCommand::Finish(reply))
            .map_err(|_| "Аудио услугата не работи.".to_string())?;
        match response.recv_timeout(AUDIO_COMMAND_TIMEOUT) {
            Ok(Ok(mut captured)) => {
                captured.cleanup_on_drop = false;
                Ok(captured)
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Err("Аудио услугата не отговори.".into()),
        }
    }

    pub fn probe(&self, routing: MicrophoneRoutingConfig) -> Result<MicrophoneProbe, String> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send(RecorderCommand::Probe(routing, reply))
            .map_err(|_| "Аудио услугата не работи.".to_string())?;
        response
            .recv_timeout(AUDIO_COMMAND_TIMEOUT)
            .map_err(|_| "Аудио услугата не отговори.".to_string())?
    }

    fn request_stop_without_waiting(&self) {
        let (reply, response) = mpsc::channel();
        drop(response);
        let _ = self.sender.send(RecorderCommand::Finish(reply));
    }
}

pub fn microphone_names() -> Vec<String> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|device| device.name().ok());
    let mut names: Vec<String> = host
        .input_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    if let Some(default_name) = default_name {
        if let Some(index) = names.iter().position(|name| name == &default_name) {
            names.remove(index);
        }
        names.insert(0, default_name);
    }
    names
}

pub fn cleanup_stale_temporary_audio() {
    let temporary_directory = std::env::temp_dir();
    let Ok(entries) = std::fs::read_dir(&temporary_directory) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !is_aidoo_temporary_audio_name(name)
            || !entry
                .file_type()
                .ok()
                .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }
        let is_stale = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= STALE_TEMPORARY_AUDIO_AGE);
        if is_stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn is_aidoo_temporary_audio_name(name: &str) -> bool {
    let identifier = name
        .strip_prefix("aidoo-")
        .and_then(|value| value.strip_suffix(".wav"))
        .or_else(|| {
            name.strip_prefix("aidoo-lite-")
                .and_then(|value| value.strip_suffix(".flac"))
        });
    identifier.is_some_and(|value| uuid::Uuid::parse_str(value).is_ok())
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

fn start(routing: &MicrophoneRoutingConfig) -> Result<(ActiveRecording, AudioStartInfo), String> {
    let host = cpal::default_host();
    let available = microphone_names();
    let default_name = host
        .default_input_device()
        .and_then(|device| device.name().ok());
    let primary_name = routing
        .preferred_name
        .as_deref()
        .or(default_name.as_deref())
        .map(str::to_owned);
    let plan = microphone_plan(
        routing.preferred_name.as_deref(),
        &available,
        default_name.as_deref(),
        routing.automatic_fallback,
    );
    if plan.is_empty() {
        return Err("Не е намерен микрофон. Свържете устройство и опитайте отново.".into());
    }
    let mut failures = Vec::new();
    for candidate in plan {
        let result = device_named(&candidate).and_then(start_on_device);
        match result {
            Ok(recording) => {
                let used_fallback = uses_fallback(primary_name.as_deref(), &candidate);
                return Ok((
                    recording,
                    AudioStartInfo {
                        device_name: candidate,
                        preferred_name: routing.preferred_name.clone(),
                        used_fallback,
                    },
                ));
            }
            Err(error) => failures.push(format!("{candidate}: {error}")),
        }
    }
    Err(format!(
        "Нито един микрофон не можа да стартира. {}",
        failures.join(" · ")
    ))
}

fn start_on_device(device: cpal::Device) -> Result<ActiveRecording, String> {
    let supported = device.default_input_config().map_err(|e| e.to_string())?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let sample_rate = config.sample_rate.0;
    let channel_count = config.channels;
    let path = std::env::temp_dir().join(format!("aidoo-{}.wav", uuid::Uuid::new_v4()));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let writer = hound::WavWriter::create(&path, spec).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    if let Err(error) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
        drop(writer);
        let _ = std::fs::remove_file(&path);
        return Err(format!(
            "Временният аудио файл не може да бъде защитен: {error}"
        ));
    }
    let writer = Arc::new(Mutex::new(Some(writer)));
    let captured_frames = Arc::new(AtomicU64::new(0));
    let peak_level_bits = Arc::new(AtomicU32::new(0.0_f32.to_bits()));
    let write_error = Arc::new(Mutex::new(None));
    let error_callback = |error| eprintln!("audio stream error: {error}");

    let stream_result = match sample_format {
        SampleFormat::F32 => {
            let writer = writer.clone();
            let frames = captured_frames.clone();
            let peak = peak_level_bits.clone();
            let write_error = write_error.clone();
            device.build_input_stream(
                &config,
                move |data: &[f32], _| {
                    write_mono_frames(&writer, &frames, &peak, &write_error, data, channel_count);
                },
                error_callback,
                None,
            )
        }
        SampleFormat::I16 => {
            let writer = writer.clone();
            let frames = captured_frames.clone();
            let peak = peak_level_bits.clone();
            let write_error = write_error.clone();
            device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    let converted = data
                        .iter()
                        .map(|value| *value as f32 / i16::MAX as f32)
                        .collect::<Vec<_>>();
                    write_mono_frames(
                        &writer,
                        &frames,
                        &peak,
                        &write_error,
                        &converted,
                        channel_count,
                    );
                },
                error_callback,
                None,
            )
        }
        SampleFormat::U16 => {
            let writer = writer.clone();
            let frames = captured_frames.clone();
            let peak = peak_level_bits.clone();
            let write_error = write_error.clone();
            device.build_input_stream(
                &config,
                move |data: &[u16], _| {
                    let converted = data
                        .iter()
                        .map(|value| (*value as f32 / u16::MAX as f32) * 2.0 - 1.0)
                        .collect::<Vec<_>>();
                    write_mono_frames(
                        &writer,
                        &frames,
                        &peak,
                        &write_error,
                        &converted,
                        channel_count,
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
            discard_recording_file(&writer, &path);
            return Err(error.to_string());
        }
    };

    if let Err(error) = stream.play() {
        drop(stream);
        discard_recording_file(&writer, &path);
        return Err(error.to_string());
    }
    Ok(ActiveRecording {
        stream: Some(stream),
        writer,
        path,
        captured_frames,
        peak_level_bits,
        write_error,
        sample_rate,
        keep_file: false,
    })
}

fn probe(routing: &MicrophoneRoutingConfig) -> Result<MicrophoneProbe, String> {
    let (mut recording, info) = start(routing)?;
    std::thread::sleep(std::time::Duration::from_millis(1_500));
    recording.stream.take();
    let peak_level = f32::from_bits(recording.peak_level_bits.load(Ordering::Relaxed));
    let captured_frames = recording.captured_frames.load(Ordering::Relaxed);
    let heard_audio =
        captured_frames >= u64::from(recording.sample_rate / 4) && peak_level >= 0.008;
    drop(recording);
    Ok(MicrophoneProbe {
        device_name: info.device_name,
        preferred_name: info.preferred_name,
        used_fallback: info.used_fallback,
        peak_level,
        heard_audio,
    })
}

fn write_mono_frames(
    writer: &Arc<Mutex<Option<RecordingWriter>>>,
    captured_frames: &Arc<AtomicU64>,
    peak_level_bits: &Arc<AtomicU32>,
    write_error: &Arc<Mutex<Option<String>>>,
    interleaved: &[f32],
    channels: u16,
) {
    let channels = usize::from(channels.max(1));
    let Ok(mut writer_guard) = writer.lock() else {
        remember_write_error(write_error, "Аудио файлът е заключен.");
        return;
    };
    let Some(writer) = writer_guard.as_mut() else {
        return;
    };
    let mut written = 0_u64;
    for frame in interleaved.chunks(channels) {
        let mono = frame.iter().copied().sum::<f32>() / frame.len() as f32;
        let level = mono.abs().clamp(0.0, 1.0);
        peak_level_bits.fetch_max(level.to_bits(), Ordering::Relaxed);
        let value = (mono.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        if let Err(error) = writer.write_sample(value) {
            remember_write_error(write_error, &error.to_string());
            break;
        }
        written += 1;
    }
    captured_frames.fetch_add(written, Ordering::Relaxed);
}

fn remember_write_error(target: &Arc<Mutex<Option<String>>>, error: &str) {
    if let Ok(mut current) = target.lock() {
        if current.is_none() {
            *current = Some(error.to_string());
        }
    }
}

fn discard_recording_file(writer: &Arc<Mutex<Option<RecordingWriter>>>, path: &std::path::Path) {
    if let Ok(mut writer) = writer.lock() {
        if let Some(writer) = writer.take() {
            let _ = writer.finalize();
        }
    }
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
fn microphone_activity(samples: &[f32], sample_rate: u32, channels: u16) -> (f32, bool) {
    let channels = usize::from(channels.max(1));
    let peak_level = samples
        .iter()
        .copied()
        .filter(|sample| sample.is_finite())
        .map(f32::abs)
        .fold(0.0_f32, f32::max)
        .clamp(0.0, 1.0);
    let captured_frames = samples.len() / channels;
    let enough_audio = captured_frames >= (sample_rate as usize / 4);
    (peak_level, enough_audio && peak_level >= 0.008)
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

fn uses_fallback(primary_name: Option<&str>, selected_name: &str) -> bool {
    primary_name.is_some_and(|primary| primary != selected_name)
}

fn finish(mut recording: ActiveRecording) -> Result<CapturedAudio, String> {
    recording.stream.take();
    if let Some(error) = recording
        .write_error
        .lock()
        .map_err(|_| "Аудио файлът е заключен.")?
        .take()
    {
        return Err(format!("Записът не можа да се запише на диска: {error}"));
    }
    let writer = recording
        .writer
        .lock()
        .map_err(|_| "Аудио файлът е заключен.")?
        .take();
    if let Some(writer) = writer {
        writer.finalize().map_err(|error| error.to_string())?;
    }
    let captured_frames = recording.captured_frames.load(Ordering::Relaxed);
    if captured_frames < u64::from(recording.sample_rate / 5) {
        return Err("Записът е прекалено кратък. Задръжте клавиша и говорете.".into());
    }
    let duration_seconds = captured_frames as f64 / recording.sample_rate as f64;
    recording.keep_file = true;
    Ok(CapturedAudio {
        path: recording.path.clone(),
        duration_seconds,
        cleanup_on_drop: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_device_leads_then_default_and_remaining_devices() {
        let plan = microphone_plan(
            Some("AirPods"),
            &[
                "MacBook Microphone".into(),
                "Studio USB".into(),
                "Webcam Mic".into(),
            ],
            Some("MacBook Microphone"),
            true,
        );
        assert_eq!(
            plan,
            vec!["AirPods", "MacBook Microphone", "Studio USB", "Webcam Mic"]
        );
    }

    #[test]
    fn fallback_off_tries_only_the_explicit_preference() {
        let plan = microphone_plan(
            Some("AirPods"),
            &["Studio USB".into(), "MacBook Microphone".into()],
            Some("MacBook Microphone"),
            false,
        );
        assert_eq!(plan, vec!["AirPods"]);
    }

    #[test]
    fn automatic_mode_uses_system_default_before_other_devices() {
        let plan = microphone_plan(
            None,
            &["MacBook Microphone".into(), "Studio USB".into()],
            Some("MacBook Microphone"),
            true,
        );
        assert_eq!(plan, vec!["MacBook Microphone", "Studio USB"]);
    }

    #[test]
    fn automatic_mode_can_identify_a_non_default_fallback() {
        assert!(!uses_fallback(
            Some("MacBook Microphone"),
            "MacBook Microphone"
        ));
        assert!(uses_fallback(Some("MacBook Microphone"), "Studio USB"));
        assert!(!uses_fallback(None, "Studio USB"));
    }

    #[test]
    fn microphone_probe_requires_duration_and_audible_signal() {
        let silent = vec![0.0; 12_000];
        assert_eq!(microphone_activity(&silent, 48_000, 1), (0.0, false));

        let too_short = vec![0.2; 4_000];
        assert!(!microphone_activity(&too_short, 48_000, 1).1);

        let mut audible = vec![0.001; 24_000];
        audible[20_000] = -0.25;
        let (peak, heard) = microphone_activity(&audible, 48_000, 2);
        assert_eq!(peak, 0.25);
        assert!(heard);
    }

    #[test]
    fn microphone_chunks_are_spooled_as_counted_mono_frames() {
        let root = std::env::temp_dir().join(format!("aidoo-spool-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("recording.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let writer = Arc::new(Mutex::new(Some(
            hound::WavWriter::create(&path, spec).unwrap(),
        )));
        let captured_frames = Arc::new(AtomicU64::new(0));
        let peak = Arc::new(AtomicU32::new(0.0_f32.to_bits()));
        let error = Arc::new(Mutex::new(None));

        write_mono_frames(
            &writer,
            &captured_frames,
            &peak,
            &error,
            &[0.5, 0.5, -0.25, -0.25],
            2,
        );
        writer.lock().unwrap().take().unwrap().finalize().unwrap();

        assert_eq!(captured_frames.load(Ordering::Relaxed), 2);
        assert_eq!(f32::from_bits(peak.load(Ordering::Relaxed)), 0.5);
        assert!(error.lock().unwrap().is_none());
        let samples = hound::WavReader::open(&path)
            .unwrap()
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(samples.len(), 2);
        assert!((samples[0] - i16::MAX / 2).abs() <= 1);
        assert!((samples[1] + i16::MAX / 4).abs() <= 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
