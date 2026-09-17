export const MAX_ASSISTANT_TRANSCRIPT_CHARS = 320;
export const ASSISTANT_CLOSE_GRACE_MS = 1_200;

const DICTATION_COMMANDS = [
  "започни транскрипция",
  "стартирай транскрипция",
  "започни да записваш",
  "стартирай запис",
  "запиши транскрипция",
  "start transcription",
  "start dictation",
] as const;

const END_SESSION_PATTERNS = [
  /(?:^| )край$/u,
  /(?:^| )край на (?:разговора|сесията)$/u,
  /(?:^| )затвори(?: ми)?$/u,
  /(?:^| )затвори (?:разговора|сесията|асистента)$/u,
  /(?:^| )приключи(?: разговора| сесията)?$/u,
  /(?:^| )приключваме(?: разговора| със сесията)?$/u,
  /(?:^| )прекрати (?:разговора|сесията)$/u,
  /(?:^| )спри (?:разговора|сесията|асистента)$/u,
  /(?:^| )довиждане$/u,
  /(?:^| )(?:end|goodbye)$/u,
  /(?:^| )(?:end|close|stop) (?:the )?(?:conversation|session|assistant)$/u,
] as const;

export type AssistantVoiceCommand = "start-dictation" | "end-session";

export function normalizeAssistantCommand(value: string) {
  return value
    .toLocaleLowerCase("bg-BG")
    .normalize("NFKC")
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim();
}

export function matchesDictationCommand(value: string) {
  const normalized = normalizeAssistantCommand(value);
  return DICTATION_COMMANDS.some((command) => normalized.includes(command));
}

export function detectAssistantVoiceCommand(value: string): AssistantVoiceCommand | null {
  if (matchesDictationCommand(value)) return "start-dictation";
  const normalized = normalizeAssistantCommand(value);
  return END_SESSION_PATTERNS.some((pattern) => pattern.test(normalized)) ? "end-session" : null;
}

/**
 * Collects fragmented GPT-Live transcript deltas and emits a command once.
 * Live transcript text stays in memory and is bounded so a long conversation
 * cannot grow the renderer's retained buffer indefinitely.
 */
export class AssistantCommandDetector {
  private transcript = "";
  private triggered = false;

  push(delta: string) {
    if (this.triggered || !delta) return false;
    this.transcript = `${this.transcript}${delta}`.slice(-MAX_ASSISTANT_TRANSCRIPT_CHARS);
    if (!matchesDictationCommand(this.transcript)) return false;
    this.triggered = true;
    return true;
  }

  reset() {
    this.transcript = "";
    this.triggered = false;
  }

  bufferedCharacterCount() {
    return this.transcript.length;
  }
}

export class AssistantVoiceCommandDetector {
  private transcript = "";
  private triggered = false;

  push(delta: string): AssistantVoiceCommand | null {
    if (this.triggered || !delta) return null;
    this.transcript = `${this.transcript}${delta}`.slice(-MAX_ASSISTANT_TRANSCRIPT_CHARS);
    const command = detectAssistantVoiceCommand(this.transcript);
    if (!command) return null;
    this.triggered = true;
    return command;
  }

  reset() {
    this.transcript = "";
    this.triggered = false;
  }

  bufferedCharacterCount() {
    return this.transcript.length;
  }
}

export interface LiveInputTranscriptEvent {
  type?: string;
  delta?: string;
}

export function detectAssistantVoiceCommandFromLiveEvent(
  event: LiveInputTranscriptEvent,
  detector: AssistantVoiceCommandDetector,
) {
  if (event.type !== "session.input_transcript.delta" || !event.delta) return null;
  return detector.push(event.delta);
}
