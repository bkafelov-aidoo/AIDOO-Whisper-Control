export type ShortcutBinding = { kind: "key"; code: string; modifiers: string[] };

export interface AppSettings {
  onboardingComplete: boolean;
  uiLanguage: "auto" | "bg" | "en";
  language: string;
  model: "gpt-4o-mini-transcribe" | "gpt-transcribe";
  autoPaste: boolean;
  saveAudio: boolean;
  saveText: boolean;
  historyEnabled: boolean;
  outputDirectory: string | null;
  launchAtLogin: boolean;
  microphoneName: string | null;
  automaticMicrophoneFallback: boolean;
  wakeWordEnabled: boolean;
  wakeWordAutoStop: boolean;
  aidooClinicSlug: string | null;
  aidooClinicUrl: string | null;
  aidooEmail: string | null;
  aidooBrowserSyncEnabled: boolean;
  dictationShortcut: ShortcutBinding;
}

export interface TranscriptEntry {
  id: string;
  text: string;
  createdAt: string;
  durationSeconds: number;
  model: string;
  language: string;
  audioPath: string | null;
  textPath: string | null;
}

export interface UsageEntry {
  id: string;
  kind: "live" | "liveBackend" | "transcription";
  createdAt: string;
  durationMillis: number;
  model: string;
  rateNanoUsdPerMinute: number;
  costNanoUsd: number;
  inputTokens: number;
  cachedInputTokens: number;
  cacheWriteTokens: number;
  outputTokens: number;
  importedFromHistory: boolean;
}

export interface UsageLedger {
  entries: UsageEntry[];
  liveDurationMillis: number;
  transcriptionDurationMillis: number;
  liveCostNanoUsd: number;
  liveBackendCostNanoUsd: number;
  transcriptionCostNanoUsd: number;
  liveSessionCount: number;
  liveBackendResponseCount: number;
  liveBackendInputTokens: number;
  liveBackendOutputTokens: number;
  transcriptionCount: number;
}

export interface FailedRecording {
  path: string;
  createdAt: string;
  durationSeconds: number;
  error: string;
  retryable: boolean;
  completedText?: string | null;
}

export interface MicrophoneProbe {
  deviceName: string;
  preferredName: string | null;
  usedFallback: boolean;
  peakLevel: number;
  heardAudio: boolean;
}

export interface WakeWordCalibrationScore {
  rms: number;
  primary: number;
  confirmation: number;
}

export interface RecordingProgress {
  percent: number;
  stage: string;
  determinate: boolean;
}

export interface RecordingSnapshot {
  state: "idle" | "starting" | "recording" | "transcribing" | "done" | "error";
  progress: RecordingProgress;
  elapsedSeconds: number;
  error: string | null;
  trigger: "shortcut" | "voice" | null;
}

export interface BootstrapState {
  settings: AppSettings;
  history: TranscriptEntry[];
  usage: UsageLedger;
  failedRecording: FailedRecording | null;
  microphones: string[];
  hasApiKey: boolean;
  hasAidooPassword: boolean;
  aidooConnected: boolean;
  aidooConnectionError: string | null;
  accessibilityGranted: boolean;
  appVersion: string;
  defaultOutputDirectory: string;
  recording: RecordingSnapshot;
}

export interface OverlayBootstrapState {
  uiLanguage: "auto" | "bg" | "en";
  recording: RecordingSnapshot;
  assistantPhase: "idle" | "preparing" | "connecting" | "listening" | "speaking" | "working" | "switching" | "closing" | "error";
}

export interface TranscriptionCompleted {
  entry: TranscriptEntry | null;
  text: string;
  pasteSucceeded: boolean;
  pasteError: string | null;
}

export type AppLanguage = "bg" | "en";
