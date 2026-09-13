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
  microphonePriority: string[];
  automaticMicrophoneFallback: boolean;
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

export interface FailedRecording {
  path: string;
  createdAt: string;
  durationSeconds: number;
  error: string;
}

export interface MicrophoneProbe {
  deviceName: string;
  preferredName: string | null;
  usedFallback: boolean;
  peakLevel: number;
  heardAudio: boolean;
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
}

export interface BootstrapState {
  settings: AppSettings;
  history: TranscriptEntry[];
  failedRecording: FailedRecording | null;
  microphones: string[];
  hasApiKey: boolean;
  accessibilityGranted: boolean;
  appVersion: string;
  defaultOutputDirectory: string;
  recording: RecordingSnapshot;
}

export interface TranscriptionCompleted {
  entry: TranscriptEntry | null;
  text: string;
  pasteSucceeded: boolean;
  pasteError: string | null;
}

export type AppLanguage = "bg" | "en";
