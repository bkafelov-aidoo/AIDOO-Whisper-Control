import assert from "node:assert/strict";
import test from "node:test";
import { acquireMicrophone, mediaRequestWithTimeout } from "../src/lib/live-microphone.ts";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function fakeStream() {
  const track = { stopped: false, stop() { this.stopped = true; } };
  return { stream: { getTracks: () => [track] }, track };
}

test("stops a media stream that arrives after its request timed out", async () => {
  const pending = deferred();
  const late = fakeStream();
  await assert.rejects(
    mediaRequestWithTimeout(pending.promise, 5, "timeout"),
    /timeout/,
  );
  pending.resolve(late.stream);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(late.track.stopped, true);
});

test("retries one timed-out microphone handoff and returns the fresh stream", async () => {
  const first = deferred();
  const second = fakeStream();
  let attempts = 0;
  const acquired = acquireMicrophone(
    () => {
      attempts += 1;
      return attempts === 1 ? first.promise : Promise.resolve(second.stream);
    },
    { audio: true },
    { attemptTimeoutMs: 5, retryDelayMs: 0 },
  );
  assert.equal(await acquired, second.stream);
  assert.equal(attempts, 2);

  const late = fakeStream();
  first.resolve(late.stream);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(late.track.stopped, true);
});

test("does not retry a permission rejection", async () => {
  let attempts = 0;
  await assert.rejects(
    acquireMicrophone(
      () => {
        attempts += 1;
        return Promise.reject(Object.assign(new Error("denied"), { name: "NotAllowedError" }));
      },
      { audio: true },
      { attemptTimeoutMs: 5, retryDelayMs: 0 },
    ),
    /denied/,
  );
  assert.equal(attempts, 1);
});
