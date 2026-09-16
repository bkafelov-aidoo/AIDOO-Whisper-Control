#!/usr/bin/env python3
"""Create positive wake-word features at several positions in the 2 s window."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
from pathlib import Path

import numpy as np


DEFAULT_SEED = 20_260_916
DEFAULT_CLIPS = 4_000
SHIFTS_MS = (250, 500, 750, 1_000)
SAMPLE_RATE = 16_000
WINDOW_SAMPLES = SAMPLE_RATE * 2


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--training-repository", type=Path, required=True)
    parser.add_argument("--audio-directory", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--clips", type=int, default=DEFAULT_CLIPS)
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED)
    parser.add_argument(
        "--shifts-ms",
        default=",".join(str(value) for value in SHIFTS_MS),
        help="Comma-separated positive shifts inside the two-second window",
    )
    arguments = parser.parse_args()
    shifts_ms = tuple(int(value) for value in arguments.shifts_ms.split(","))
    if not shifts_ms or any(value <= 0 or value >= 2_000 for value in shifts_ms):
        raise SystemExit("--shifts-ms values must be between 1 and 1999")

    source_path = arguments.training_repository / "src"
    if not source_path.is_dir():
        raise SystemExit(f"LiveKit wake-word source directory is missing: {source_path}")
    sys.path.insert(0, str(source_path))
    import soundfile as sf
    from onnxruntime import SessionOptions
    from livekit.wakeword.models.feature_extractor import (
        MelSpectrogramFrontend,
        SpeechEmbedding,
    )
    from livekit.wakeword.resources import get_embedding_model_path, get_mel_model_path

    candidates = sorted(arguments.audio_directory.glob("clip_*_r0.wav"))
    if arguments.clips <= 0 or arguments.clips > len(candidates):
        raise SystemExit(
            f"--clips must be between 1 and {len(candidates)}, received {arguments.clips}"
        )
    randomizer = np.random.default_rng(arguments.seed)
    selected_indices = np.sort(
        randomizer.choice(len(candidates), size=arguments.clips, replace=False)
    )
    selected = [candidates[int(index)] for index in selected_indices]

    options = SessionOptions()
    mel_frontend = MelSpectrogramFrontend(get_mel_model_path(), options)
    speech_embedding = SpeechEmbedding(get_embedding_model_path(), options)
    shape = (len(selected) * len(shifts_ms), 16, 96)
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    output = np.lib.format.open_memmap(
        arguments.output, mode="w+", dtype=np.float32, shape=shape
    )
    started = time.time()

    for clip_index, path in enumerate(selected):
        audio, sample_rate = sf.read(path, dtype="float32")
        if sample_rate != SAMPLE_RATE:
            raise SystemExit(f"unexpected sample rate in {path}: {sample_rate}")
        if audio.ndim > 1:
            audio = audio[:, 0]
        if len(audio) != WINDOW_SAMPLES:
            raise SystemExit(f"unexpected clip length in {path}: {len(audio)}")

        for shift_index, shift_ms in enumerate(shifts_ms):
            shift_samples = shift_ms * SAMPLE_RATE // 1_000
            shifted = np.pad(audio[shift_samples:], (0, shift_samples))
            mel = mel_frontend(shifted)
            embeddings = speech_embedding.extract_embeddings(mel)[0]
            if len(embeddings) >= 16:
                features = embeddings[-16:]
            else:
                features = np.concatenate(
                    [
                        np.zeros((16 - len(embeddings), 96), dtype=np.float32),
                        embeddings,
                    ]
                )
            output[clip_index * len(shifts_ms) + shift_index] = features

        if (clip_index + 1) % 100 == 0:
            output.flush()
            print(
                f"features {clip_index + 1}/{len(selected)} "
                f"elapsed={time.time() - started:.1f}s",
                flush=True,
            )
    output.flush()
    del output

    selected_names = "\n".join(path.name for path in selected).encode()
    metadata = {
        "schemaVersion": 1,
        "seed": arguments.seed,
        "sourceDirectory": arguments.audio_directory.name,
        "sourceClipCount": len(selected),
        "sourceFilenamesSha256": hashlib.sha256(selected_names).hexdigest(),
        "shiftsMilliseconds": list(shifts_ms),
        "artifact": {
            "path": arguments.output.name,
            "shape": list(shape),
            "dtype": "<f4",
            "sizeBytes": arguments.output.stat().st_size,
            "sha256": sha256(arguments.output),
        },
    }
    arguments.metadata.write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n")
    print(f"Wrote {arguments.output}")
    print(f"Wrote {arguments.metadata}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
