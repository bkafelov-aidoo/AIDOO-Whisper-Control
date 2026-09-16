#!/usr/bin/env python3
"""Train the second-stage 500 ms Hey, AIDOO confirmation classifier."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
import time
from pathlib import Path

import numpy as np
import torch
import torch.nn.functional as F


SEED = 20_260_916
THRESHOLD = 0.76
BATCH_COUNTS = {
    "positive": 128,
    "adversarial_negative": 64,
    "background_noise": 32,
    "general_speech": 256,
    "validation_speech": 128,
}
LOSS_WEIGHTS = {
    "positive": 1.5,
    "adversarial_negative": 0.4,
    "background_noise": 0.1,
    "general_speech": 1.0,
    "validation_speech": 1.0,
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_features(path: Path) -> np.ndarray:
    array = np.load(path, mmap_mode="r")
    if array.ndim == 2 and array.shape[1] == 96:
        array = array[: len(array) // 16 * 16].reshape(-1, 16, 96)
    if array.ndim != 3 or array.shape[1:] != (16, 96):
        raise ValueError(f"unexpected feature shape for {path}: {array.shape}")
    return array


def predict(
    model: torch.nn.Module, features: np.ndarray, device: torch.device
) -> np.ndarray:
    model.eval()
    scores: list[np.ndarray] = []
    with torch.no_grad():
        for start in range(0, len(features), 1_024):
            batch = np.asarray(features[start : start + 1_024], dtype=np.float32).copy()
            scores.append(
                model(torch.from_numpy(batch).to(device)).squeeze(-1).cpu().numpy()
            )
    return np.concatenate(scores)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--training-repository", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--initial-model", type=Path, required=True)
    parser.add_argument("--general-speech", type=Path, required=True)
    parser.add_argument("--general-speech-metadata", type=Path, required=True)
    parser.add_argument("--shifted-positive", type=Path, required=True)
    parser.add_argument("--shifted-positive-metadata", type=Path, required=True)
    parser.add_argument("--output-directory", type=Path, required=True)
    parser.add_argument("--steps", type=int, default=2_000)
    parser.add_argument("--learning-rate", type=float, default=1e-5)
    arguments = parser.parse_args()

    sys.path.insert(0, str(arguments.training_repository / "src"))
    from livekit.wakeword.config import load_config
    from livekit.wakeword.models.pipeline import WakeWordClassifier
    from livekit.wakeword.utils import get_device

    config = load_config(arguments.config)
    model_directory = config.model_output_dir
    shifted_metadata = json.loads(arguments.shifted_positive_metadata.read_text())
    if shifted_metadata.get("shiftsMilliseconds") != [250, 500, 750, 1000]:
        raise SystemExit("shifted-positive features use unexpected time positions")
    if shifted_metadata.get("artifact", {}).get("sha256") != sha256(
        arguments.shifted_positive
    ):
        raise SystemExit("shifted-positive features differ from their metadata hash")
    general_metadata = json.loads(arguments.general_speech_metadata.read_text())
    if general_metadata.get("artifact", {}).get("sha256") != sha256(
        arguments.general_speech
    ):
        raise SystemExit("general-speech features differ from their metadata hash")

    shifted = load_features(arguments.shifted_positive).reshape(-1, 4, 16, 96)[:, 1]
    randomizer = np.random.default_rng(SEED)
    positive_order = randomizer.permutation(len(shifted))
    positive_train = np.asarray(shifted[positive_order[:3_200]], dtype=np.float32)
    positive_validation = np.asarray(shifted[positive_order[3_200:]], dtype=np.float32)

    external = load_features(
        config.data_path / "features" / "validation_set_features.npy"
    )
    external_order = randomizer.permutation(len(external))
    split = len(external_order) // 2
    external_train_indices = external_order[:split]
    external_validation_indices = external_order[split:]
    arrays = {
        "positive": positive_train,
        "adversarial_negative": load_features(
            model_directory / "negative_features_train.npy"
        ),
        "background_noise": load_features(
            model_directory / "background_noise_features_train.npy"
        ),
        "general_speech": load_features(arguments.general_speech),
        "validation_speech": np.asarray(
            external[external_train_indices], dtype=np.float32
        ),
    }
    external_validation = np.asarray(
        external[external_validation_indices], dtype=np.float32
    )

    torch.manual_seed(SEED)
    device = get_device()
    model = WakeWordClassifier(config).to(device)
    model.load_state_dict(
        torch.load(arguments.initial_model, map_location=device, weights_only=True)
    )
    optimizer = torch.optim.AdamW(
        model.parameters(), lr=arguments.learning_rate, weight_decay=1e-3
    )
    arguments.output_directory.mkdir(parents=True, exist_ok=True)
    model_path = arguments.output_directory / "hey_aidoo_confirmation.pt"
    metrics_path = arguments.output_directory / "confirmation_training_metrics.json"
    metrics: list[dict[str, object]] = []
    started = time.time()

    def validate(step: int, loss: float | None) -> None:
        positive_scores = predict(model, positive_validation, device)
        negative_scores = predict(model, external_validation, device)
        entry: dict[str, object] = {
            "step": step,
            "elapsedSeconds": round(time.time() - started, 1),
            "loss": loss,
            "threshold": THRESHOLD,
            "positiveRecall": float(np.mean(positive_scores >= THRESHOLD)),
            "negativeFalsePositives": int(np.sum(negative_scores >= THRESHOLD)),
            "nPositive": len(positive_scores),
            "nNegative": len(negative_scores),
        }
        metrics.append(entry)
        metrics_path.write_text(json.dumps(metrics, indent=2) + "\n")
        print(
            f"validation step={step} recall={entry['positiveRecall']:.4f} "
            f"false_positives={entry['negativeFalsePositives']}",
            flush=True,
        )
        model.train()

    validate(0, None)
    last_loss = 0.0
    for step in range(1, arguments.steps + 1):
        batches: list[torch.Tensor] = []
        labels: list[torch.Tensor] = []
        slices: dict[str, slice] = {}
        position = 0
        for name, count in BATCH_COUNTS.items():
            array = arrays[name]
            indices = randomizer.integers(0, len(array), count)
            batches.append(
                torch.from_numpy(np.asarray(array[indices], dtype=np.float32))
            )
            target = 1.0 if name == "positive" else 0.0
            labels.append(torch.full((count,), target, dtype=torch.float32))
            slices[name] = slice(position, position + count)
            position += count

        features = torch.cat(batches).to(device)
        targets = torch.cat(labels).to(device)
        probabilities = model(features).squeeze(-1)
        per_sample = F.binary_cross_entropy(probabilities, targets, reduction="none")
        loss = sum(
            LOSS_WEIGHTS[name] * per_sample[slices[name]].mean()
            for name in BATCH_COUNTS
        ) / sum(LOSS_WEIGHTS.values())
        progress = (step - 1) / max(1, arguments.steps - 1)
        learning_rate = arguments.learning_rate * (
            0.1 + 0.9 * 0.5 * (1 + math.cos(math.pi * progress))
        )
        for group in optimizer.param_groups:
            group["lr"] = learning_rate
        optimizer.zero_grad()
        loss.backward()
        torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
        optimizer.step()
        last_loss = float(loss.detach().cpu())

        if step % 100 == 0:
            print(
                f"training step={step}/{arguments.steps} "
                f"loss={last_loss:.6f} lr={learning_rate:.2e}",
                flush=True,
            )
        if step % 250 == 0 or step == arguments.steps:
            validate(step, last_loss)
            torch.save(model.state_dict(), model_path)

    provenance = {
        "schemaVersion": 1,
        "seed": SEED,
        "threshold": THRESHOLD,
        "steps": arguments.steps,
        "learningRate": arguments.learning_rate,
        "batchCounts": BATCH_COUNTS,
        "lossWeights": LOSS_WEIGHTS,
        "initialModelSha256": sha256(arguments.initial_model),
        "generalSpeechSha256": sha256(arguments.general_speech),
        "shiftedPositiveSha256": sha256(arguments.shifted_positive),
        "positiveTrainingCount": len(positive_train),
        "positiveValidationCount": len(positive_validation),
        "externalTrainingCount": len(external_train_indices),
        "externalValidationCount": len(external_validation_indices),
        "externalTrainingIndicesSha256": hashlib.sha256(
            external_train_indices.astype("<i8").tobytes()
        ).hexdigest(),
        "externalValidationIndicesSha256": hashlib.sha256(
            external_validation_indices.astype("<i8").tobytes()
        ).hexdigest(),
        "artifact": {
            "path": model_path.name,
            "sizeBytes": model_path.stat().st_size,
            "sha256": sha256(model_path),
        },
    }
    (arguments.output_directory / "confirmation_training_provenance.json").write_text(
        json.dumps(provenance, indent=2, sort_keys=True) + "\n"
    )
    print(f"Saved {model_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
