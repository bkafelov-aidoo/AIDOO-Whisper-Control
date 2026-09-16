#!/usr/bin/env python3
"""Generate the short, gentle recording feedback sounds bundled with the app."""

from __future__ import annotations

import math
import random
import struct
import wave
from pathlib import Path


SAMPLE_RATE = 44_100
OUTPUT = Path(__file__).resolve().parent.parent / "src-tauri/resources/sounds"


def pluck(frequency: float, duration: float, seed: int) -> list[float]:
    randomizer = random.Random(seed)
    samples: list[float] = []
    for index in range(round(duration * SAMPLE_RATE)):
        time = index / SAMPLE_RATE
        attack = 1.0 - math.exp(-240.0 * time)
        body = (
            math.sin(math.tau * frequency * time) * math.exp(-6.8 * time)
            + 0.42 * math.sin(math.tau * frequency * 2.01 * time + 0.18) * math.exp(-11.0 * time)
            + 0.18 * math.sin(math.tau * frequency * 3.98 * time + 0.41) * math.exp(-17.0 * time)
        )
        pick = randomizer.uniform(-1.0, 1.0) * math.exp(-75.0 * time) * 0.08
        release = min(1.0, (duration - time) / 0.045)
        samples.append((body + pick) * attack * max(0.0, release))
    return samples


def render(name: str, frequencies: tuple[float, float]) -> None:
    duration = 0.48
    audio = [0.0] * round(duration * SAMPLE_RATE)
    for note, (frequency, offset) in enumerate(zip(frequencies, (0.0, 0.15))):
        for index, sample in enumerate(pluck(frequency, 0.33, 8800 + note)):
            position = round(offset * SAMPLE_RATE) + index
            if position < len(audio):
                audio[position] += sample
    peak = max(abs(sample) for sample in audio) or 1.0
    gain = 0.38 / peak
    pcm = b"".join(
        struct.pack("<h", round(max(-1.0, min(1.0, sample * gain)) * 32767))
        for sample in audio
    )
    OUTPUT.mkdir(parents=True, exist_ok=True)
    with wave.open(str(OUTPUT / name), "wb") as target:
        target.setnchannels(1)
        target.setsampwidth(2)
        target.setframerate(SAMPLE_RATE)
        target.writeframes(pcm)


def main() -> None:
    render("recording-start.wav", (392.00, 523.25))
    render("recording-stop.wav", (523.25, 392.00))


if __name__ == "__main__":
    main()
