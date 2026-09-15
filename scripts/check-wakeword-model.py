#!/usr/bin/env python3
"""Validate the exact Hey, AIDOO model accepted for a macOS release."""

from __future__ import annotations

import hashlib
import json
import math
import re
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MODEL = ROOT / "src-tauri" / "resources" / "wakeword" / "hey_aidoo.onnx"
METADATA = ROOT / "docs" / "wakeword" / "hey_aidoo_model.json"
CONFIG = ROOT / "docs" / "wakeword" / "hey_aidoo.yaml"
EXPECTED_TRAINING_COMMIT = "95448a7559c453fcd87645bd67b247ffb45f85b0"
EXPECTED_SAMPLE_COUNTS = {
    "positive_train": 12_000,
    "positive_test": 2_500,
    "negative_train": 12_000,
    "negative_test": 2_500,
    "background_train": 1_500,
    "background_test": 400,
}
EXPECTED_EVALUATION_POSITIVES = 5_000
EXPECTED_EVALUATION_NEGATIVES = 35_884


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def numeric(value: object) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    result = float(value)
    return result if math.isfinite(result) else None


def main() -> int:
    errors: list[str] = []
    for path in (MODEL, METADATA, CONFIG):
        if not path.is_file():
            errors.append(f"Required wake-word release artifact is missing: {path}")
    if errors:
        raise SystemExit("\n".join(errors))

    metadata = json.loads(METADATA.read_text())
    artifact = metadata.get("artifact", {})
    runtime = metadata.get("runtime", {})
    training = metadata.get("training", {})
    evaluation = metadata.get("evaluation", {})
    if metadata.get("schemaVersion") != 1 or metadata.get("modelName") != "hey_aidoo":
        errors.append("Wake-word metadata identity or schema differs")
    if artifact.get("path") != "src-tauri/resources/wakeword/hey_aidoo.onnx":
        errors.append("Wake-word metadata points to an unexpected model path")
    if artifact.get("sha256") != sha256(MODEL):
        errors.append("Wake-word model SHA-256 differs from its release metadata")
    if artifact.get("sizeBytes") != MODEL.stat().st_size or not 1_000 < MODEL.stat().st_size < 20_000_000:
        errors.append("Wake-word model size is missing, differs or is implausible")

    cargo = tomllib.loads((ROOT / "src-tauri" / "Cargo.toml").read_text())
    runtime_version = runtime.get("version")
    if runtime != {"crate": "livekit-wakeword", "version": "0.1.3"}:
        errors.append(f"Wake-word runtime metadata differs: {runtime}")
    if cargo.get("dependencies", {}).get("livekit-wakeword") != f"={runtime_version}":
        errors.append("Cargo.toml must pin the exact evaluated wake-word runtime")
    if training.get("repository") != "https://github.com/livekit/livekit-wakeword":
        errors.append("Wake-word training repository differs")
    if training.get("commit") != EXPECTED_TRAINING_COMMIT:
        errors.append("Wake-word training commit differs")
    if training.get("configPath") != "docs/wakeword/hey_aidoo.yaml":
        errors.append("Wake-word training config path differs")
    if training.get("configSha256") != sha256(CONFIG):
        errors.append("Wake-word training config SHA-256 differs")
    if training.get("sampleCounts") != EXPECTED_SAMPLE_COUNTS:
        errors.append(f"Wake-word sample counts differ: {training.get('sampleCounts')}")

    threshold_source = (ROOT / "src-tauri" / "src" / "dictation.rs").read_text()
    threshold_match = re.search(
        r"const WAKE_WORD_THRESHOLD: f32 = ([0-9.]+);", threshold_source
    )
    threshold = numeric(metadata.get("operatingThreshold"))
    if not threshold_match or threshold is None:
        errors.append("Wake-word operating threshold is missing")
    elif abs(float(threshold_match.group(1)) - threshold) > 1e-6:
        errors.append("Runtime wake-word threshold differs from evaluated metadata")
    elif not 0.01 <= threshold <= 0.99:
        errors.append(f"Wake-word operating threshold is outside the valid range: {threshold}")

    recall = numeric(evaluation.get("optimal_recall"))
    fpph = numeric(evaluation.get("optimal_fpph"))
    validation_hours = numeric(evaluation.get("validation_hours"))
    optimal_threshold = numeric(evaluation.get("optimal_threshold"))
    if recall is None or recall < 0.90:
        errors.append(f"Wake-word held-out recall is below 90%: {recall}")
    if fpph is None or fpph > 0.125:
        errors.append(f"Wake-word held-out false positives/hour exceeds 0.125: {fpph}")
    if validation_hours is None or validation_hours < 8:
        errors.append(f"Wake-word negative validation duration is below 8 hours: {validation_hours}")
    if evaluation.get("n_positive") != EXPECTED_EVALUATION_POSITIVES:
        errors.append("Wake-word positive evaluation count differs")
    if evaluation.get("n_negative") != EXPECTED_EVALUATION_NEGATIVES:
        errors.append("Wake-word negative evaluation count differs")
    if optimal_threshold is None or threshold is None or abs(optimal_threshold - threshold) > 1e-6:
        errors.append("Runtime wake-word threshold is not the evaluated optimal threshold")

    tauri = json.loads((ROOT / "src-tauri" / "tauri.conf.json").read_text())
    resources = tauri.get("bundle", {}).get("resources", {})
    expected_resources = {
        "resources/wakeword/hey_aidoo.onnx": "wakeword/hey_aidoo.onnx",
        "resources/wakeword/LIVEKIT-WAKEWORD-LICENSE.txt": "wakeword/LIVEKIT-WAKEWORD-LICENSE.txt",
        "resources/wakeword/MODEL-NOTICES.txt": "wakeword/MODEL-NOTICES.txt",
    }
    for source, destination in expected_resources.items():
        if resources.get(source) != destination:
            errors.append(f"Wake-word bundle resource differs: {source}")

    if errors:
        raise SystemExit("\n".join(errors))
    print("Hey, AIDOO model validation passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
