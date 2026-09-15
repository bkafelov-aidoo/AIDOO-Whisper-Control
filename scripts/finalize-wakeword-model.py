#!/usr/bin/env python3
"""Bundle an evaluated Hey, AIDOO classifier and write reproducible metadata."""

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
DESTINATION = ROOT / "src-tauri" / "resources" / "wakeword" / "hey_aidoo.onnx"
METADATA = ROOT / "docs" / "wakeword" / "hey_aidoo_model.json"
RUNTIME_VERSION = "0.1.3"
TRAINING_REPOSITORY = "https://github.com/livekit/livekit-wakeword"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def clip_count(directory: Path) -> int:
    return sum(1 for path in directory.glob("clip_*.wav") if path.is_file())


def training_commit(repository: Path) -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=repository,
        check=True,
        capture_output=True,
        text=True,
    )
    commit = result.stdout.strip()
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise RuntimeError(f"Unexpected training repository commit: {commit!r}")
    return commit


def operating_threshold() -> float:
    source = (ROOT / "src-tauri" / "src" / "dictation.rs").read_text()
    match = re.search(r"const WAKE_WORD_THRESHOLD: f32 = ([0-9.]+);", source)
    if not match:
        raise RuntimeError("WAKE_WORD_THRESHOLD is missing from dictation.rs")
    return float(match.group(1))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--evaluation", type=Path, required=True)
    parser.add_argument("--training-repository", type=Path, required=True)
    parser.add_argument("--training-output", type=Path, required=True)
    arguments = parser.parse_args()

    for path in (arguments.model, arguments.evaluation, CONFIG):
        if not path.is_file():
            raise SystemExit(f"Required wake-word artifact is missing: {path}")
    evaluation = json.loads(arguments.evaluation.read_text())
    required_metrics = {
        "optimal_threshold",
        "optimal_recall",
        "optimal_fpph",
        "n_positive",
        "n_negative",
        "validation_hours",
    }
    missing_metrics = required_metrics - set(evaluation)
    if missing_metrics:
        raise SystemExit(
            "Evaluation is missing metrics: " + ", ".join(sorted(missing_metrics))
        )

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

    DESTINATION.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(arguments.model, DESTINATION)
    metadata = {
        "schemaVersion": 1,
        "modelName": "hey_aidoo",
        "artifact": {
            "path": "src-tauri/resources/wakeword/hey_aidoo.onnx",
            "sha256": sha256(DESTINATION),
            "sizeBytes": DESTINATION.stat().st_size,
        },
        "runtime": {
            "crate": "livekit-wakeword",
            "version": RUNTIME_VERSION,
        },
        "training": {
            "repository": TRAINING_REPOSITORY,
            "commit": training_commit(arguments.training_repository),
            "configPath": "docs/wakeword/hey_aidoo.yaml",
            "configSha256": sha256(CONFIG),
            "sampleCounts": samples,
        },
        "evaluation": evaluation,
        "operatingThreshold": operating_threshold(),
    }
    METADATA.write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n")
    print(f"Bundled {DESTINATION} ({DESTINATION.stat().st_size} bytes)")
    print(f"Wrote {METADATA}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
