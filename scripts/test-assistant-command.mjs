import assert from "node:assert/strict";
import test from "node:test";
import {
  AssistantCommandDetector,
  MAX_ASSISTANT_TRANSCRIPT_CHARS,
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
