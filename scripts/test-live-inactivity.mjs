import assert from "node:assert/strict";
import test from "node:test";
import {
  ASSISTANT_INACTIVITY_MS,
  LiveInactivityTimer,
  assistantGoodbyeInstruction,
  assistantGoodbyePrompt,
} from "../src/lib/live-inactivity.ts";

function fakeClock() {
  let nextId = 0;
  const pending = new Map();
  return {
    schedule(callback, delayMs) {
      const id = ++nextId;
      pending.set(id, { callback, delayMs });
      return id;
    },
    cancel(id) { pending.delete(id); },
    fire() {
      const entry = [...pending.values()][0];
      pending.clear();
      entry?.callback();
    },
    pending() { return [...pending.values()]; },
  };
}

test("ends a live conversation after exactly 20 seconds without activity", () => {
  const clock = fakeClock();
  let closes = 0;
  const timer = new LiveInactivityTimer(() => { closes += 1; }, clock.schedule, clock.cancel);
  timer.start();
  assert.equal(clock.pending()[0].delayMs, ASSISTANT_INACTIVITY_MS);
  clock.fire();
  assert.equal(closes, 1);
});

test("activity restarts the inactivity window and pause suppresses it", () => {
  const clock = fakeClock();
  let closes = 0;
  const timer = new LiveInactivityTimer(() => { closes += 1; }, clock.schedule, clock.cancel);
  timer.start();
  const first = clock.pending()[0];
  timer.touch();
  assert.notEqual(clock.pending()[0], first);
  timer.pause();
  timer.touch();
  clock.fire();
  assert.equal(closes, 0);
  timer.start();
  clock.fire();
  assert.equal(closes, 1);
});

test("idle close asks GPT-Live to say only goodbye before closing", () => {
  assert.deepEqual(assistantGoodbyeInstruction("idle_1"), {
    type: "session.instructions.append",
    event_id: "idle_1",
    delegation_id: null,
    content: "След 20 секунди без активност разговорът приключва. Кажи веднага само „Чао!“ на български, без допълнителен текст, след което замълчи.",
  });
  assert.deepEqual(assistantGoodbyePrompt("idle_1_prompt"), {
    type: "session.commentary.append",
    event_id: "idle_1_prompt",
    delegation_id: null,
    content: "Кажи сега само „Чао!“.",
  });
});
