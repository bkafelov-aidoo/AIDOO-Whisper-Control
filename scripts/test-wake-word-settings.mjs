import assert from "node:assert/strict";
import test from "node:test";
import { persistSuccessfulWakeCalibration } from "../src/lib/wake-word-settings.ts";

test("successful calibration enables and persists wake-word listening", async () => {
  const original = {
    wakeWordEnabled: false,
    wakeWordAutoStop: true,
    aidooAssistantMode: "economy",
    microphoneName: null,
  };
  let persisted;

  const enabled = await persistSuccessfulWakeCalibration(original, async (settings) => {
    persisted = settings;
  });

  assert.equal(enabled.wakeWordEnabled, true);
  assert.deepEqual(persisted, enabled);
  assert.equal(enabled.wakeWordAutoStop, original.wakeWordAutoStop);
  assert.equal(enabled.aidooAssistantMode, original.aidooAssistantMode);
});

test("failed persistence does not report calibration as enabled", async () => {
  await assert.rejects(
    persistSuccessfulWakeCalibration({ wakeWordEnabled: false }, async () => {
      throw new Error("save failed");
    }),
    /save failed/,
  );
});
