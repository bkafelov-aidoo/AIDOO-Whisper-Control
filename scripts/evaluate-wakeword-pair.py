#!/usr/bin/env python3
"""Evaluate the two-stage Hey, AIDOO detector on a continuous speech stream."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path

import numpy as np
import onnxruntime as ort


PRIMARY_THRESHOLD = 0.68
CONFIRMATION_THRESHOLD = 0.76
CONFIRMATION_OFFSETS = (1, 2, 3)
EMBEDDING_STRIDE = 3
INFERENCE_INTERVAL_SECONDS = 0.24


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def predict(session: ort.InferenceSession, features: np.ndarray) -> np.ndarray:
    input_name = session.get_inputs()[0].name
    scores: list[np.ndarray] = []
    for start in range(0, len(features), 1_024):
        batch = np.asarray(features[start : start + 1_024], dtype=np.float32)
        scores.append(session.run(None, {input_name: batch})[0].squeeze(-1))
    return np.concatenate(scores)


def rolling_predict(
    session: ort.InferenceSession, embeddings: np.ndarray
) -> np.ndarray:
    starts = np.arange(0, len(embeddings) - 15, EMBEDDING_STRIDE)
    input_name = session.get_inputs()[0].name
    scores: list[np.ndarray] = []
    for offset in range(0, len(starts), 1_024):
        batch_starts = starts[offset : offset + 1_024]
        batch = np.stack(
            [embeddings[start : start + 16] for start in batch_starts]
        ).astype(np.float32, copy=False)
        scores.append(session.run(None, {input_name: batch})[0].squeeze(-1))
    return np.concatenate(scores)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--primary-model", type=Path, required=True)
    parser.add_argument("--confirmation-model", type=Path, required=True)
    parser.add_argument("--model-output-directory", type=Path, required=True)
    parser.add_argument("--validation-features", type=Path, required=True)
    parser.add_argument("--confirmation-positive-features", type=Path, required=True)
    parser.add_argument("--confirmation-positive-metadata", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()

    primary_session = ort.InferenceSession(
        str(arguments.primary_model), providers=["CPUExecutionProvider"]
    )
    confirmation_session = ort.InferenceSession(
        str(arguments.confirmation_model), providers=["CPUExecutionProvider"]
    )

    positive_features = np.load(
        arguments.model_output_directory / "positive_features_test.npy", mmap_mode="r"
    )
    positive_audio_directory = arguments.model_output_directory / "positive_test"
    augmented = sorted(
        path
        for path in positive_audio_directory.glob("*.wav")
        if re.fullmatch(r"clip_\d{6}_r\d+\.wav", path.name)
    )
    if len(augmented) != len(positive_features):
        raise SystemExit("positive feature rows do not match augmented audio files")
    round_zero_indices = np.array(
        [index for index, path in enumerate(augmented) if path.stem.endswith("_r0")]
    )
    primary_positive = predict(primary_session, positive_features[round_zero_indices])

    confirmation_metadata = json.loads(
        arguments.confirmation_positive_metadata.read_text()
    )
    if confirmation_metadata.get("shiftsMilliseconds") != [500]:
        raise SystemExit("confirmation positive test features must use the 500 ms shift")
    if confirmation_metadata.get("artifact", {}).get("sha256") != sha256(
        arguments.confirmation_positive_features
    ):
        raise SystemExit("confirmation positive features differ from their metadata hash")
    confirmation_positive_features = np.load(
        arguments.confirmation_positive_features, mmap_mode="r"
    )
    confirmation_positive = predict(
        confirmation_session, confirmation_positive_features
    )
    if len(primary_positive) != len(confirmation_positive):
        raise SystemExit("primary and confirmation positive sets differ in size")

    continuous_embeddings = np.load(arguments.validation_features, mmap_mode="r")
    primary_negative = rolling_predict(primary_session, continuous_embeddings)
    confirmation_negative = rolling_predict(
        confirmation_session, continuous_embeddings
    )
    if len(primary_negative) != len(confirmation_negative):
        raise SystemExit("primary and confirmation continuous scores differ in size")

    positive_detections = (primary_positive >= PRIMARY_THRESHOLD) & (
        confirmation_positive >= CONFIRMATION_THRESHOLD
    )
    negative_detections = np.zeros(len(primary_negative), dtype=bool)
    for offset in CONFIRMATION_OFFSETS:
        negative_detections[offset:] |= (
            (primary_negative[:-offset] >= PRIMARY_THRESHOLD)
            & (confirmation_negative[offset:] >= CONFIRMATION_THRESHOLD)
        )
    event_starts = negative_detections & np.r_[True, ~negative_detections[:-1]]
    false_positives = int(np.count_nonzero(event_starts))
    validation_hours = (
        len(primary_negative) * INFERENCE_INTERVAL_SECONDS / 3600.0
    )
    recall = float(np.mean(positive_detections))
    fpph = false_positives / validation_hours
    results = {
        "schemaVersion": 2,
        "detector": "two_stage_temporal_confirmation",
        "primary_threshold": PRIMARY_THRESHOLD,
        "confirmation_threshold": CONFIRMATION_THRESHOLD,
        "confirmation_offsets": list(CONFIRMATION_OFFSETS),
        "inference_interval_ms": 250,
        "evaluation_interval_ms": int(INFERENCE_INTERVAL_SECONDS * 1_000),
        "optimal_threshold": PRIMARY_THRESHOLD,
        "optimal_recall": recall,
        "optimal_fpph": fpph,
        "false_positives": false_positives,
        "n_positive": len(primary_positive),
        "n_negative": len(primary_negative),
        "validation_hours": round(validation_hours, 6),
        "primary_positive_recall": float(
            np.mean(primary_positive >= PRIMARY_THRESHOLD)
        ),
        "confirmation_positive_recall": float(
            np.mean(confirmation_positive >= CONFIRMATION_THRESHOLD)
        ),
        "models": {
            "primary": {
                "sha256": sha256(arguments.primary_model),
                "sizeBytes": arguments.primary_model.stat().st_size,
            },
            "confirmation": {
                "sha256": sha256(arguments.confirmation_model),
                "sizeBytes": arguments.confirmation_model.stat().st_size,
            },
        },
        "validationFeaturesSha256": sha256(arguments.validation_features),
        "confirmationPositiveFeaturesSha256": sha256(
            arguments.confirmation_positive_features
        ),
    }
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(json.dumps(results, indent=2, sort_keys=True) + "\n")
    print(json.dumps(results, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
