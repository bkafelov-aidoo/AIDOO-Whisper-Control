import assert from "node:assert/strict";
import test from "node:test";
import { encodeMonoWav, rootMeanSquare } from "../src/lib/turn-audio.ts";

test("encodes mono PCM WAV at the requested output sample rate", () => {
  const input = new Float32Array(48_000);
  for (let index = 0; index < input.length; index += 1) input[index] = Math.sin(index / 12) * 0.25;
  const wav = encodeMonoWav(input, 48_000, 16_000);
  const view = new DataView(wav.buffer, wav.byteOffset, wav.byteLength);
  assert.equal(new TextDecoder().decode(wav.slice(0, 4)), "RIFF");
  assert.equal(new TextDecoder().decode(wav.slice(8, 12)), "WAVE");
  assert.equal(view.getUint16(22, true), 1);
  assert.equal(view.getUint32(24, true), 16_000);
  assert.equal(view.getUint32(40, true), 16_000 * 2);
});

test("RMS distinguishes silence from speech energy", () => {
  assert.equal(rootMeanSquare(new Float32Array(100)), 0);
  assert.ok(rootMeanSquare(Float32Array.from({ length: 100 }, (_, index) => index % 2 ? 0.2 : -0.2)) > 0.19);
});
