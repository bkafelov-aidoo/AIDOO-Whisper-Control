export const ASSISTANT_INACTIVITY_MS = 20_000;
export const ASSISTANT_GOODBYE_START_TIMEOUT_MS = 5_000;
export const ASSISTANT_GOODBYE_SILENCE_MS = 900;

type Schedule = (callback: () => void, delayMs: number) => number;
type Cancel = (handle: number) => void;

export class LiveInactivityTimer {
  private handle: number | null = null;
  private active = false;
  private readonly onInactive: () => void;
  private readonly schedule: Schedule;
  private readonly cancel: Cancel;

  constructor(
    onInactive: () => void,
    schedule: Schedule = (callback, delayMs) => window.setTimeout(callback, delayMs),
    cancel: Cancel = (handle) => window.clearTimeout(handle),
  ) {
    this.onInactive = onInactive;
    this.schedule = schedule;
    this.cancel = cancel;
  }

  start() {
    this.active = true;
    this.arm();
  }

  touch() {
    if (this.active) this.arm();
  }

  pause() {
    this.active = false;
    this.clear();
  }

  stop() {
    this.active = false;
    this.clear();
  }

  private arm() {
    this.clear();
    this.handle = this.schedule(() => {
      this.handle = null;
      if (!this.active) return;
      this.active = false;
      this.onInactive();
    }, ASSISTANT_INACTIVITY_MS);
  }

  private clear() {
    if (this.handle !== null) this.cancel(this.handle);
    this.handle = null;
  }
}

export function assistantGoodbyeInstruction(eventId: string) {
  return {
    type: "session.instructions.append",
    event_id: eventId,
    delegation_id: null,
    content: "След 20 секунди без активност разговорът приключва. Кажи веднага само „Чао!“ на български, без допълнителен текст, след което замълчи.",
  } as const;
}

export function assistantGoodbyePrompt(eventId: string) {
  return {
    type: "session.commentary.append",
    event_id: eventId,
    delegation_id: null,
    content: "Кажи сега само „Чао!“.",
  } as const;
}
