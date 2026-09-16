import assert from "node:assert/strict";
import test from "node:test";
import {
  ASSISTANT_CLOSE_GRACE_MS,
  AssistantCommandDetector,
  AssistantVoiceCommandDetector,
  MAX_ASSISTANT_TRANSCRIPT_CHARS,
  detectAssistantVoiceCommand,
  detectAssistantVoiceCommandFromLiveEvent,
  matchesDictationCommand,
  normalizeAssistantCommand,
} from "../src/lib/assistant-command.ts";

test("recognizes the Bulgarian commands despite case and punctuation", () => {
  assert.equal(matchesDictationCommand("AIDOO, ЗАПОЧНИ транскрипция!"), true);
  assert.equal(matchesDictationCommand("Моля, стартирай запис."), true);
  assert.equal(matchesDictationCommand("Започни да записваш"), true);
});

test("recognizes the English commands", () => {
  assert.equal(matchesDictationCommand("AIDOO, start transcription."), true);
  assert.equal(matchesDictationCommand("Please start dictation now"), true);
});

test("does not mistake nearby phrases for the command", () => {
  assert.equal(matchesDictationCommand("Не започвай транскрипция"), false);
  assert.equal(matchesDictationCommand("Транскрипцията вече започна"), false);
  assert.equal(matchesDictationCommand("Стартирай приложението"), false);
  assert.equal(matchesDictationCommand("Запиши тази бележка"), false);
});

test("recognizes a command split across Live transcript deltas exactly once", () => {
  const detector = new AssistantCommandDetector();
  assert.equal(detector.push("Моля, започни тран"), false);
  assert.equal(detector.push("скрипция."), true);
  assert.equal(detector.push(" Започни транскрипция отново."), false);
});

test("reset starts a new assistant conversation", () => {
  const detector = new AssistantCommandDetector();
  assert.equal(detector.push("Start dictation"), true);
  detector.reset();
  assert.equal(detector.push("Start dictation"), true);
});

test("the in-memory transcript buffer remains bounded", () => {
  const detector = new AssistantCommandDetector();
  assert.equal(detector.push("x".repeat(MAX_ASSISTANT_TRANSCRIPT_CHARS * 3)), false);
  assert.equal(detector.bufferedCharacterCount(), MAX_ASSISTANT_TRANSCRIPT_CHARS);
  assert.equal(detector.push(" start transcription"), true);
  assert.ok(detector.bufferedCharacterCount() <= MAX_ASSISTANT_TRANSCRIPT_CHARS);
});

test("normalization preserves words and removes separator differences", () => {
  assert.equal(normalizeAssistantCommand("  Start—TRANSCRIPTION… "), "start transcription");
});

test("recognizes natural commands that end the AI conversation", () => {
  for (const phrase of [
    "Край",
    "Затвори",
    "Затвори ми",
    "Приключи разговора",
    "Приключваме",
    "Приключваме разговора",
    "Спри асистента",
    "Прекрати сесията",
    "Довиждане",
    "End conversation",
    "Close the session",
    "Goodbye",
  ]) {
    assert.equal(detectAssistantVoiceCommand(phrase), "end-session", phrase);
  }
});

test("does not close on words that merely resemble an end command", () => {
  assert.equal(detectAssistantVoiceCommand("В крайна сметка продължаваме"), null);
  assert.equal(detectAssistantVoiceCommand("Спри да говориш толкова бързо"), null);
  assert.equal(detectAssistantVoiceCommand("Затворих вратата"), null);
});

test("recognizes a fragmented end command once", () => {
  const detector = new AssistantVoiceCommandDetector();
  assert.equal(detector.push("Моля, приключи раз"), null);
  assert.equal(detector.push("говора"), "end-session");
  assert.equal(detector.push(" край"), null);
});

test("maps a real Live input-transcript event to the end-session command", () => {
  for (const phrase of ["Край.", "Затвори!", "Приключваме."]) {
    const detector = new AssistantVoiceCommandDetector();
    assert.equal(detectAssistantVoiceCommandFromLiveEvent({
      type: "session.input_transcript.delta",
      delta: phrase,
    }, detector), "end-session", phrase);
  }
});

test("recognizes fragmented приключваме from Live transcript deltas", () => {
  const detector = new AssistantVoiceCommandDetector();
  assert.equal(detectAssistantVoiceCommandFromLiveEvent({
    type: "session.input_transcript.delta",
    delta: "Приключ",
  }, detector), null);
  assert.equal(detectAssistantVoiceCommandFromLiveEvent({
    type: "session.input_transcript.delta",
    delta: "ваме.",
  }, detector), "end-session");
});

test("voice-requested close has a short bounded fallback", () => {
  assert.ok(ASSISTANT_CLOSE_GRACE_MS <= 1_500, `close fallback was ${ASSISTANT_CLOSE_GRACE_MS} ms`);
});
