export const MAX_ASSISTANT_TRANSCRIPT_CHARS = 320;

const DICTATION_COMMANDS = [
  "започни транскрипция",
  "стартирай транскрипция",
  "започни да записваш",
  "стартирай запис",
  "запиши транскрипция",
  "start transcription",
  "start dictation",
] as const;

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
