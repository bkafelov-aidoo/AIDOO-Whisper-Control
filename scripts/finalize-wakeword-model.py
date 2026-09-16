#!/usr/bin/env python3
"""Bundle the evaluated two-stage Hey, AIDOO detector and its provenance."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CONFIG = ROOT / "docs" / "wakeword" / "hey_aidoo.yaml"
PRIMARY_DESTINATION = ROOT / "src-tauri" / "resources" / "wakeword" / "hey_aidoo.onnx"
CONFIRMATION_DESTINATION = (
    ROOT / "src-tauri" / "resources" / "wakeword" / "hey_aidoo_confirmation.onnx"
)
METADATA = ROOT / "docs" / "wakeword" / "hey_aidoo_model.json"
RUNTIME_VERSION = "0.1.3"
TRAINING_REPOSITORY = "https://github.com/livekit/livekit-wakeword"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def artifact(path: Path, relative_path: str) -> dict[str, object]:
    return {
        "path": relative_path,
        "sha256": sha256(path),
        "sizeBytes": path.stat().st_size,
    }


def clip_count(directory: Path) -> int:
    return sum(
        1
        for path in directory.glob("clip_*.wav")
        if path.is_file() and re.fullmatch(r"clip_\d{6}\.wav", path.name)
    )


def training_commit(repository: Path) -> str:
    status = subprocess.run(
        ["git", "status", "--porcelain"],
        cwd=repository,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if status:
        raise RuntimeError("Training repository has uncommitted source changes")
    commit = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=repository,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise RuntimeError(f"Unexpected training repository commit: {commit!r}")
    return commit


def training_python_version(repository: Path) -> str:
    result = subprocess.run(
        [str(repository / ".venv" / "bin" / "python"), "--version"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    match = re.fullmatch(r"Python ([0-9]+\.[0-9]+\.[0-9]+)", result)
    if not match:
        raise RuntimeError(f"Unexpected training Python version: {result!r}")
    return match.group(1)


def runtime_configuration() -> dict[str, object]:
    source = (ROOT / "src-tauri" / "src" / "dictation.rs").read_text()
    primary = re.search(r"const WAKE_WORD_PRIMARY_THRESHOLD: f32 = ([0-9.]+);", source)
    confirmation = re.search(
        r"const WAKE_WORD_CONFIRMATION_THRESHOLD: f32 = ([0-9.]+);", source
    )
    wake_source = (ROOT / "src-tauri" / "src" / "wake_word.rs").read_text()
    history = re.search(r"const CONFIRMATION_HISTORY: usize = ([0-9]+);", wake_source)
    if not primary or not confirmation or not history:
        raise RuntimeError("Wake-word runtime thresholds or confirmation history are missing")
    return {
        "primaryThreshold": float(primary.group(1)),
        "confirmationThreshold": float(confirmation.group(1)),
        "confirmationHistory": int(history.group(1)),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--primary-model", type=Path, required=True)
    parser.add_argument("--confirmation-model", type=Path, required=True)
    parser.add_argument("--evaluation", type=Path, required=True)
    parser.add_argument("--training-repository", type=Path, required=True)
    parser.add_argument("--training-output", type=Path, required=True)
    parser.add_argument("--confirmation-training-provenance", type=Path, required=True)
    parser.add_argument("--general-speech-metadata", type=Path, required=True)
    parser.add_argument("--shifted-positive-metadata", type=Path, required=True)
    parser.add_argument("--confirmation-positive-metadata", type=Path, required=True)
    arguments = parser.parse_args()

    required = (
        arguments.primary_model,
        arguments.confirmation_model,
        arguments.evaluation,
        arguments.confirmation_training_provenance,
        arguments.general_speech_metadata,
        arguments.shifted_positive_metadata,
        arguments.confirmation_positive_metadata,
        CONFIG,
    )
    for path in required:
        if not path.is_file():
            raise SystemExit(f"Required wake-word artifact is missing: {path}")

    evaluation = json.loads(arguments.evaluation.read_text())
    expected_evaluation = {
        "schemaVersion": 2,
        "detector": "two_stage_temporal_confirmation",
        "primary_threshold": 0.68,
        "confirmation_threshold": 0.76,
        "confirmation_offsets": [1, 2, 3],
    }
    for key, value in expected_evaluation.items():
        if evaluation.get(key) != value:
            raise SystemExit(f"Evaluation field {key} differs: {evaluation.get(key)!r}")
    if evaluation.get("models", {}).get("primary", {}).get("sha256") != sha256(
        arguments.primary_model
    ):
        raise SystemExit("Evaluation was not produced from the primary model")
    if evaluation.get("models", {}).get("confirmation", {}).get("sha256") != sha256(
        arguments.confirmation_model
    ):
        raise SystemExit("Evaluation was not produced from the confirmation model")

    model_output = arguments.training_output / "hey_aidoo"
    samples = {
        name: clip_count(model_output / name)
        for name in (
            "positive_train",
            "positive_test",
            "negative_train",
            "negative_test",
            "background_train",
            "background_test",
        )
    }
    expected_samples = {
        "positive_train": 12_000,
        "positive_test": 2_500,
        "negative_train": 12_000,
        "negative_test": 2_500,
        "background_train": 1_500,
        "background_test": 400,
    }
    if samples != expected_samples:
        raise SystemExit(f"Wake-word sample counts are incomplete: {samples}")

    PRIMARY_DESTINATION.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(arguments.primary_model, PRIMARY_DESTINATION)
    shutil.copy2(arguments.confirmation_model, CONFIRMATION_DESTINATION)
    metadata = {
        "schemaVersion": 2,
        "modelName": "hey_aidoo",
        "artifacts": {
            "primary": artifact(
                PRIMARY_DESTINATION,
                "src-tauri/resources/wakeword/hey_aidoo.onnx",
            ),
            "confirmation": artifact(
                CONFIRMATION_DESTINATION,
                "src-tauri/resources/wakeword/hey_aidoo_confirmation.onnx",
            ),
        },
        "runtime": {
            "crate": "livekit-wakeword",
            "version": RUNTIME_VERSION,
            **runtime_configuration(),
        },
        "training": {
            "repository": TRAINING_REPOSITORY,
            "commit": training_commit(arguments.training_repository),
            "pythonVersion": training_python_version(arguments.training_repository),
            "configPath": "docs/wakeword/hey_aidoo.yaml",
            "configSha256": sha256(CONFIG),
            "sampleCounts": samples,
            "confirmationTrainingProvenance": json.loads(
                arguments.confirmation_training_provenance.read_text()
            ),
            "sourceMetadataSha256": {
                "generalSpeech": sha256(arguments.general_speech_metadata),
                "shiftedPositiveTrain": sha256(arguments.shifted_positive_metadata),
                "shiftedPositiveTest": sha256(arguments.confirmation_positive_metadata),
            },
        },
        "evaluation": evaluation,
    }
    METADATA.write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n")
    print(f"Bundled {PRIMARY_DESTINATION}")
    print(f"Bundled {CONFIRMATION_DESTINATION}")
    print(f"Wrote {METADATA}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
