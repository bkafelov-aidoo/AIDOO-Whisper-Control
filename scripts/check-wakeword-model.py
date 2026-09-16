#!/usr/bin/env python3
"""Validate the exact two-stage Hey, AIDOO detector accepted for release."""

from __future__ import annotations

import hashlib
import json
import math
import re
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PRIMARY = ROOT / "src-tauri" / "resources" / "wakeword" / "hey_aidoo.onnx"
CONFIRMATION = (
    ROOT / "src-tauri" / "resources" / "wakeword" / "hey_aidoo_confirmation.onnx"
)
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
EXPECTED_EVALUATION_POSITIVES = 2_500
EXPECTED_EVALUATION_NEGATIVES = 160_444


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


def check_artifact(
    errors: list[str], metadata: object, path: Path, expected_path: str, name: str
) -> None:
    if not isinstance(metadata, dict):
        errors.append(f"Wake-word {name} artifact metadata is missing")
        return
    if metadata.get("path") != expected_path:
        errors.append(f"Wake-word {name} artifact path differs")
    if metadata.get("sha256") != sha256(path):
        errors.append(f"Wake-word {name} model SHA-256 differs")
    if metadata.get("sizeBytes") != path.stat().st_size:
        errors.append(f"Wake-word {name} model size differs")
    if not 1_000 < path.stat().st_size < 20_000_000:
        errors.append(f"Wake-word {name} model size is implausible")


def main() -> int:
    errors: list[str] = []
    for path in (PRIMARY, CONFIRMATION, METADATA, CONFIG):
        if not path.is_file():
            errors.append(f"Required wake-word release artifact is missing: {path}")
    if errors:
        raise SystemExit("\n".join(errors))

    metadata = json.loads(METADATA.read_text())
    artifacts = metadata.get("artifacts", {})
    runtime = metadata.get("runtime", {})
    training = metadata.get("training", {})
    evaluation = metadata.get("evaluation", {})
    if metadata.get("schemaVersion") != 2 or metadata.get("modelName") != "hey_aidoo":
        errors.append("Wake-word metadata identity or schema differs")
    check_artifact(
        errors,
        artifacts.get("primary"),
        PRIMARY,
        "src-tauri/resources/wakeword/hey_aidoo.onnx",
        "primary",
    )
    check_artifact(
        errors,
        artifacts.get("confirmation"),
        CONFIRMATION,
        "src-tauri/resources/wakeword/hey_aidoo_confirmation.onnx",
        "confirmation",
    )

    cargo = tomllib.loads((ROOT / "src-tauri" / "Cargo.toml").read_text())
    if runtime.get("crate") != "livekit-wakeword" or runtime.get("version") != "0.1.3":
        errors.append(f"Wake-word runtime metadata differs: {runtime}")
    if cargo.get("dependencies", {}).get("livekit-wakeword") != "=0.1.3":
        errors.append("Cargo.toml must pin livekit-wakeword 0.1.3")
    if training.get("repository") != "https://github.com/livekit/livekit-wakeword":
        errors.append("Wake-word training repository differs")
    if training.get("commit") != EXPECTED_TRAINING_COMMIT:
        errors.append("Wake-word training commit differs")
    if training.get("pythonVersion") != "3.11.16":
        errors.append("Wake-word training Python version differs")
    if training.get("configPath") != "docs/wakeword/hey_aidoo.yaml":
        errors.append("Wake-word training config path differs")
    if training.get("configSha256") != sha256(CONFIG):
        errors.append("Wake-word training config SHA-256 differs")
    if training.get("sampleCounts") != EXPECTED_SAMPLE_COUNTS:
        errors.append(f"Wake-word sample counts differ: {training.get('sampleCounts')}")

    provenance = training.get("confirmationTrainingProvenance", {})
    if provenance.get("schemaVersion") != 1 or provenance.get("seed") != 20_260_916:
        errors.append("Confirmation training provenance differs")
    if provenance.get("steps") != 2_000 or provenance.get("threshold") != 0.76:
        errors.append("Confirmation training parameters differ")
    source_hashes = training.get("sourceMetadataSha256", {})
    if set(source_hashes) != {
        "generalSpeech",
        "shiftedPositiveTrain",
        "shiftedPositiveTest",
    } or any(not re.fullmatch(r"[0-9a-f]{64}", value or "") for value in source_hashes.values()):
        errors.append("Wake-word source metadata hashes differ")

    dictation_source = (ROOT / "src-tauri" / "src" / "dictation.rs").read_text()
    primary_match = re.search(
        r"const WAKE_WORD_PRIMARY_THRESHOLD: f32 = ([0-9.]+);", dictation_source
    )
    confirmation_match = re.search(
        r"const WAKE_WORD_CONFIRMATION_THRESHOLD: f32 = ([0-9.]+);",
        dictation_source,
    )
    wake_source = (ROOT / "src-tauri" / "src" / "wake_word.rs").read_text()
    history_match = re.search(
        r"const CONFIRMATION_HISTORY: usize = ([0-9]+);", wake_source
    )
    if not primary_match or float(primary_match.group(1)) != 0.68:
        errors.append("Runtime primary threshold differs")
    if not confirmation_match or float(confirmation_match.group(1)) != 0.76:
        errors.append("Runtime confirmation threshold differs")
    if not history_match or int(history_match.group(1)) != 3:
        errors.append("Runtime confirmation history differs")
    if runtime.get("primaryThreshold") != 0.68:
        errors.append("Primary threshold metadata differs")
    if runtime.get("confirmationThreshold") != 0.76:
        errors.append("Confirmation threshold metadata differs")
    if runtime.get("confirmationHistory") != 3:
        errors.append("Confirmation history metadata differs")

    if evaluation.get("schemaVersion") != 2:
        errors.append("Wake-word evaluation schema differs")
    if evaluation.get("detector") != "two_stage_temporal_confirmation":
        errors.append("Wake-word evaluation detector differs")
    if evaluation.get("confirmation_offsets") != [1, 2, 3]:
        errors.append("Wake-word evaluation confirmation offsets differ")
    if evaluation.get("primary_threshold") != 0.68:
        errors.append("Wake-word evaluated primary threshold differs")
    if evaluation.get("confirmation_threshold") != 0.76:
        errors.append("Wake-word evaluated confirmation threshold differs")
    models = evaluation.get("models", {})
    if models.get("primary", {}).get("sha256") != sha256(PRIMARY):
        errors.append("Evaluation primary model hash differs")
    if models.get("confirmation", {}).get("sha256") != sha256(CONFIRMATION):
        errors.append("Evaluation confirmation model hash differs")

    recall = numeric(evaluation.get("optimal_recall"))
    fpph = numeric(evaluation.get("optimal_fpph"))
    validation_hours = numeric(evaluation.get("validation_hours"))
    if recall is None or recall < 0.90:
        errors.append(f"Wake-word held-out recall is below 90%: {recall}")
    if fpph is None or fpph > 0.125:
        errors.append(f"Wake-word false positives/hour exceeds 0.125: {fpph}")
    if validation_hours is None or validation_hours < 8:
        errors.append(
            f"Wake-word continuous validation duration is below 8 hours: {validation_hours}"
        )
    if evaluation.get("n_positive") != EXPECTED_EVALUATION_POSITIVES:
        errors.append("Wake-word positive evaluation count differs")
    if evaluation.get("n_negative") != EXPECTED_EVALUATION_NEGATIVES:
        errors.append("Wake-word continuous evaluation frame count differs")
    if evaluation.get("false_positives") != 1:
        errors.append("Wake-word continuous false-positive count differs")

    tauri = json.loads((ROOT / "src-tauri" / "tauri.conf.json").read_text())
    resources = tauri.get("bundle", {}).get("resources", {})
    expected_resources = {
        "resources/wakeword/hey_aidoo.onnx": "wakeword/hey_aidoo.onnx",
        "resources/wakeword/hey_aidoo_confirmation.onnx": "wakeword/hey_aidoo_confirmation.onnx",
        "resources/wakeword/LIVEKIT-WAKEWORD-LICENSE.txt": "wakeword/LIVEKIT-WAKEWORD-LICENSE.txt",
        "resources/wakeword/MODEL-NOTICES.txt": "wakeword/MODEL-NOTICES.txt",
    }
    for source, destination in expected_resources.items():
        if resources.get(source) != destination:
            errors.append(f"Wake-word bundle resource differs: {source}")

    if errors:
        raise SystemExit("\n".join(errors))
    print("Hey, AIDOO two-stage model validation passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
