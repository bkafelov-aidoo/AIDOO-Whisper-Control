export const VAD_PRE_ROLL_MS = 360;
export const VAD_END_SILENCE_MS = 680;
export const VAD_MIN_SPEECH_MS = 220;
export const VAD_MAX_TURN_MS = 25_000;

export type CaptureMode = "listening" | "barge-in" | "paused";

export interface VoiceTurn {
  audio: Uint8Array;
  durationSeconds: number;
}

export interface VoiceTurnCaptureCallbacks {
  onSpeechStart: () => void;
  onActivity: () => void;
  onTurn: (turn: VoiceTurn) => void;
}

export interface VoiceTurnCaptureOptions {
  minimumThreshold?: number;
  speechMultiplier?: number;
  bargeInMultiplier?: number;
}

export class VoiceTurnCapture {
  private readonly callbacks: VoiceTurnCaptureCallbacks;
  private context: AudioContext | null = null;
  private source: MediaStreamAudioSourceNode | null = null;
  private processor: ScriptProcessorNode | null = null;
  private silentGain: GainNode | null = null;
  private stream: MediaStream | null = null;
  private mode: CaptureMode = "paused";
  private noiseFloor = 0.004;
  private preRoll: Float32Array[] = [];
  private preRollSamples = 0;
  private captured: Float32Array[] = [];
  private capturedSamples = 0;
  private speechSamples = 0;
  private silenceSamples = 0;
  private candidateSamples = 0;
  private speaking = false;
  private readonly minimumThreshold: number;
  private readonly speechMultiplier: number;
  private readonly bargeInMultiplier: number;

  constructor(
    callbacks: VoiceTurnCaptureCallbacks,
    options: VoiceTurnCaptureOptions = {},
  ) {
    this.callbacks = callbacks;
    this.minimumThreshold = options.minimumThreshold ?? 0.012;
    this.speechMultiplier = options.speechMultiplier ?? 3.1;
    this.bargeInMultiplier = options.bargeInMultiplier ?? 4.4;
  }

  async start(stream: MediaStream) {
    this.stop();
    const context = new AudioContext();
    await context.resume();
    const source = context.createMediaStreamSource(stream);
    const processor = context.createScriptProcessor(2048, 1, 1);
    const silentGain = context.createGain();
    silentGain.gain.value = 0;
    processor.onaudioprocess = (event) => this.consume(event.inputBuffer.getChannelData(0), context.sampleRate);
    source.connect(processor);
    processor.connect(silentGain);
    silentGain.connect(context.destination);
    this.context = context;
    this.source = source;
    this.processor = processor;
    this.silentGain = silentGain;
    this.stream = stream;
    this.resetTurn();
    this.mode = "listening";
  }

  setMode(mode: CaptureMode) {
    this.mode = mode;
    if (mode === "paused") this.resetTurn();
  }

  stop() {
    this.mode = "paused";
    this.processor?.disconnect();
    this.source?.disconnect();
    this.silentGain?.disconnect();
    if (this.processor) this.processor.onaudioprocess = null;
    this.stream?.getTracks().forEach((track) => track.stop());
    void this.context?.close().catch(() => undefined);
    this.context = null;
    this.source = null;
    this.processor = null;
    this.silentGain = null;
    this.stream = null;
    this.resetTurn();
  }

  private consume(input: Float32Array, sampleRate: number) {
    if (this.mode === "paused") return;
    const samples = new Float32Array(input);
    const rms = rootMeanSquare(samples);
    const multiplier = this.mode === "barge-in" ? this.bargeInMultiplier : this.speechMultiplier;
    const threshold = Math.max(this.minimumThreshold, this.noiseFloor * multiplier);
    const voiced = rms >= threshold;

    if (!this.speaking) {
      if (!voiced) this.noiseFloor = Math.min(0.035, this.noiseFloor * 0.985 + rms * 0.015);
      this.pushPreRoll(samples, sampleRate);
      this.candidateSamples = voiced ? this.candidateSamples + samples.length : 0;
      if (this.candidateSamples < millisecondsToSamples(this.mode === "barge-in" ? 150 : 90, sampleRate)) return;
      this.speaking = true;
      this.captured = this.preRoll.map((chunk) => new Float32Array(chunk));
      this.capturedSamples = this.preRollSamples;
      this.speechSamples = this.candidateSamples;
      this.silenceSamples = 0;
      this.callbacks.onSpeechStart();
      this.callbacks.onActivity();
      return;
    }

    this.captured.push(samples);
    this.capturedSamples += samples.length;
    if (voiced) {
      this.speechSamples += samples.length;
      this.silenceSamples = 0;
      this.callbacks.onActivity();
    } else {
      this.silenceSamples += samples.length;
    }

    const reachedSilence = this.silenceSamples >= millisecondsToSamples(VAD_END_SILENCE_MS, sampleRate);
    const reachedMaximum = this.capturedSamples >= millisecondsToSamples(VAD_MAX_TURN_MS, sampleRate);
    if (!reachedSilence && !reachedMaximum) return;
    if (this.speechSamples < millisecondsToSamples(VAD_MIN_SPEECH_MS, sampleRate)) {
      this.resetTurn();
      return;
    }
    const joined = joinSamples(this.captured, this.capturedSamples);
    const wav = encodeMonoWav(joined, sampleRate, 16_000);
    const durationSeconds = joined.length / sampleRate;
    this.resetTurn();
    this.mode = "paused";
    this.callbacks.onTurn({ audio: wav, durationSeconds });
  }

  private pushPreRoll(samples: Float32Array, sampleRate: number) {
    this.preRoll.push(samples);
    this.preRollSamples += samples.length;
    const limit = millisecondsToSamples(VAD_PRE_ROLL_MS, sampleRate);
    while (this.preRollSamples > limit && this.preRoll.length > 1) {
      const removed = this.preRoll.shift();
      this.preRollSamples -= removed?.length ?? 0;
    }
  }

  private resetTurn() {
    this.preRoll = [];
    this.preRollSamples = 0;
    this.captured = [];
    this.capturedSamples = 0;
    this.speechSamples = 0;
    this.silenceSamples = 0;
    this.candidateSamples = 0;
    this.speaking = false;
  }
}

export function rootMeanSquare(samples: Float32Array) {
  if (!samples.length) return 0;
  let sum = 0;
  for (const sample of samples) sum += sample * sample;
  return Math.sqrt(sum / samples.length);
}

export function encodeMonoWav(samples: Float32Array, inputRate: number, outputRate = 16_000) {
  const resampled = resampleMono(samples, inputRate, outputRate);
  const buffer = new ArrayBuffer(44 + resampled.length * 2);
  const view = new DataView(buffer);
  writeAscii(view, 0, "RIFF");
  view.setUint32(4, 36 + resampled.length * 2, true);
  writeAscii(view, 8, "WAVE");
  writeAscii(view, 12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, outputRate, true);
  view.setUint32(28, outputRate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  writeAscii(view, 36, "data");
  view.setUint32(40, resampled.length * 2, true);
  for (let index = 0; index < resampled.length; index += 1) {
    const sample = Math.max(-1, Math.min(1, resampled[index]));
    view.setInt16(44 + index * 2, sample < 0 ? sample * 0x8000 : sample * 0x7fff, true);
  }
  return new Uint8Array(buffer);
}

function resampleMono(samples: Float32Array, inputRate: number, outputRate: number) {
  if (inputRate === outputRate) return samples;
  const ratio = inputRate / outputRate;
  const length = Math.max(1, Math.round(samples.length / ratio));
  const output = new Float32Array(length);
  for (let index = 0; index < length; index += 1) {
    const start = Math.floor(index * ratio);
    const end = Math.min(samples.length, Math.max(start + 1, Math.floor((index + 1) * ratio)));
    let sum = 0;
    for (let source = start; source < end; source += 1) sum += samples[source];
    output[index] = sum / (end - start);
  }
  return output;
}

function joinSamples(chunks: Float32Array[], total: number) {
  const output = new Float32Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    output.set(chunk, offset);
    offset += chunk.length;
  }
  return output;
}

function millisecondsToSamples(milliseconds: number, sampleRate: number) {
  return Math.round((milliseconds / 1000) * sampleRate);
}

function writeAscii(view: DataView, offset: number, value: string) {
  for (let index = 0; index < value.length; index += 1) view.setUint8(offset + index, value.charCodeAt(index));
}
