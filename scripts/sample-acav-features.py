#!/usr/bin/env python3
"""Download a deterministic, stratified subset of ACAV100M wake-word features.

The upstream training corpus is a 16 GB NumPy file. This script uses HTTP byte
ranges to sample evenly across the full corpus without downloading the entire
artifact. The resulting file keeps the exact name and layout expected by the
LiveKit wake-word trainer.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import time
import urllib.error
import urllib.request
from pathlib import Path

import numpy as np


SOURCE_COMMIT = "68a47c08bb113c684ea3a9799f3f7a257be577f1"
SOURCE_URL = (
    "https://huggingface.co/datasets/binhpham/livekit_wakeword_features/resolve/"
    f"{SOURCE_COMMIT}/openwakeword_features_ACAV100M_2000_hrs_16bit.npy"
)
SOURCE_REPOSITORY = "https://huggingface.co/datasets/binhpham/livekit_wakeword_features"
SOURCE_XET_HASH = "7e1cade4c3fda6a5081158383c8d43c4a3e1e42555150b596b373efddf9b5194"
SOURCE_SIZE = 17_280_000_128
SOURCE_HEADER_SIZE = 128
SOURCE_SHAPE = (5_625_000, 16, 96)
SOURCE_DTYPE = np.dtype("<f2")
SAMPLE_BYTES = 16 * 96 * SOURCE_DTYPE.itemsize
DEFAULT_SEED = 20_260_916
DEFAULT_CHUNKS = 64
DEFAULT_SAMPLES_PER_CHUNK = 4_096


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def fetch_range(start: int, end: int, attempts: int = 5) -> bytes:
    expected_length = end - start + 1
    for attempt in range(1, attempts + 1):
        request = urllib.request.Request(
            SOURCE_URL,
            headers={
                "Range": f"bytes={start}-{end}",
                "User-Agent": "AIDOO-Whisper-Control-wakeword-training/1.0",
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                status = response.status
                content_range = response.headers.get("Content-Range", "")
                payload = response.read()
            expected_range = f"bytes {start}-{end}/{SOURCE_SIZE}"
            if status != 206:
                raise RuntimeError(f"expected HTTP 206, received {status}")
            if content_range != expected_range:
                raise RuntimeError(
                    f"expected Content-Range {expected_range!r}, received {content_range!r}"
                )
            if len(payload) != expected_length:
                raise RuntimeError(
                    f"expected {expected_length} bytes, received {len(payload)}"
                )
            return payload
        except (OSError, RuntimeError, urllib.error.URLError) as error:
            if attempt == attempts:
                raise RuntimeError(
                    f"failed byte range {start}-{end} after {attempts} attempts"
                ) from error
            time.sleep(2**attempt)
    raise AssertionError("unreachable")


def choose_starts(
    *, total_samples: int, chunks: int, samples_per_chunk: int, seed: int
) -> list[int]:
    if chunks <= 0 or samples_per_chunk <= 0:
        raise ValueError("chunks and samples-per-chunk must be positive")
    if chunks * samples_per_chunk > total_samples:
        raise ValueError("requested subset is larger than the source corpus")

    randomizer = random.Random(seed)
    starts: list[int] = []
    for index in range(chunks):
        stratum_start = index * total_samples // chunks
        stratum_end = (index + 1) * total_samples // chunks
        latest_start = stratum_end - samples_per_chunk
        if latest_start < stratum_start:
            raise ValueError("a source stratum is smaller than one requested chunk")
        starts.append(randomizer.randint(stratum_start, latest_start))
    return starts


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--metadata", type=Path)
    parser.add_argument("--chunks", type=int, default=DEFAULT_CHUNKS)
    parser.add_argument(
        "--samples-per-chunk", type=int, default=DEFAULT_SAMPLES_PER_CHUNK
    )
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED)
    arguments = parser.parse_args()

    expected_source_size = SOURCE_HEADER_SIZE + SOURCE_SHAPE[0] * SAMPLE_BYTES
    if expected_source_size != SOURCE_SIZE:
        raise SystemExit("declared source geometry does not match its byte size")

    starts = choose_starts(
        total_samples=SOURCE_SHAPE[0],
        chunks=arguments.chunks,
        samples_per_chunk=arguments.samples_per_chunk,
        seed=arguments.seed,
    )
    sample_count = arguments.chunks * arguments.samples_per_chunk
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    destination = np.lib.format.open_memmap(
        arguments.output,
        mode="w+",
        dtype=SOURCE_DTYPE,
        shape=(sample_count, SOURCE_SHAPE[1], SOURCE_SHAPE[2]),
    )

    try:
        for chunk_index, source_start in enumerate(starts):
            source_end = source_start + arguments.samples_per_chunk - 1
            byte_start = SOURCE_HEADER_SIZE + source_start * SAMPLE_BYTES
            byte_end = SOURCE_HEADER_SIZE + (source_end + 1) * SAMPLE_BYTES - 1
            payload = fetch_range(byte_start, byte_end)
            values = np.frombuffer(payload, dtype=SOURCE_DTYPE).reshape(
                arguments.samples_per_chunk, SOURCE_SHAPE[1], SOURCE_SHAPE[2]
            )
            destination_start = chunk_index * arguments.samples_per_chunk
            destination[
                destination_start : destination_start + arguments.samples_per_chunk
            ] = values
            destination.flush()
            print(
                f"[{chunk_index + 1:02d}/{arguments.chunks:02d}] "
                f"source samples {source_start}-{source_end}"
            )
    finally:
        del destination

    sampled = np.load(arguments.output, mmap_mode="r")
    expected_shape = (sample_count, SOURCE_SHAPE[1], SOURCE_SHAPE[2])
    if sampled.shape != expected_shape or sampled.dtype != SOURCE_DTYPE:
        raise SystemExit(
            f"sampled file has unexpected geometry: {sampled.shape}, {sampled.dtype}"
        )
    if not np.isfinite(sampled).all():
        raise SystemExit("sampled file contains non-finite values")

    metadata_path = arguments.metadata or arguments.output.with_suffix(".metadata.json")
    metadata = {
        "schemaVersion": 1,
        "source": {
            "url": SOURCE_URL,
            "repository": SOURCE_REPOSITORY,
            "commit": SOURCE_COMMIT,
            "xetHash": SOURCE_XET_HASH,
            "sizeBytes": SOURCE_SIZE,
            "headerSizeBytes": SOURCE_HEADER_SIZE,
            "shape": list(SOURCE_SHAPE),
            "dtype": SOURCE_DTYPE.str,
        },
        "sampling": {
            "strategy": "one deterministic random contiguous chunk per equal-sized stratum",
            "seed": arguments.seed,
            "chunks": arguments.chunks,
            "samplesPerChunk": arguments.samples_per_chunk,
            "sampleCount": sample_count,
            "representedAudioHours": round(sample_count * 2.0 / 3600.0, 3),
            "sourceStartIndices": starts,
        },
        "artifact": {
            "path": arguments.output.name,
            "sizeBytes": arguments.output.stat().st_size,
            "sha256": sha256(arguments.output),
            "shape": list(expected_shape),
            "dtype": sampled.dtype.str,
        },
    }
    metadata_path.write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n")
    print(f"Wrote {arguments.output} ({arguments.output.stat().st_size} bytes)")
    print(f"Wrote {metadata_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
