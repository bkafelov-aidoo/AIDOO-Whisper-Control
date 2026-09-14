#!/usr/bin/env python3
"""Create and verify one self-contained website release directory."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import uuid


ROOT = Path(__file__).resolve().parent.parent
VERSION = json.loads((ROOT / "package.json").read_text())["version"]
DMG_NAME = f"AIDOO Whisper Lite_{VERSION}_aarch64.dmg"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def public_website_files(root: Path) -> list[Path]:
    return sorted(
        path
        for path in root.rglob("*")
        if path.is_file()
        and not any(part.startswith(".") for part in path.relative_to(root).parts)
    )


def atomic_copy(source: Path, target: Path) -> None:
    temporary = target.with_name(f".{target.name}.tmp-{uuid.uuid4().hex}")
    try:
        shutil.copy2(source, temporary)
        with temporary.open("rb") as copied:
            os.fsync(copied.fileno())
        os.replace(temporary, target)
        sync_directory(target.parent)
    finally:
        temporary.unlink(missing_ok=True)


def atomic_write_text(target: Path, contents: str) -> None:
    temporary = target.with_name(f".{target.name}.tmp-{uuid.uuid4().hex}")
    descriptor: int | None = None
    try:
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            descriptor = None
            output.write(contents)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, target)
        sync_directory(target.parent)
    finally:
        if descriptor is not None:
            os.close(descriptor)
        temporary.unlink(missing_ok=True)


def sync_directory(directory: Path) -> None:
    if not hasattr(os, "O_DIRECTORY"):
        return
    descriptor = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def replace_website_directory(source: Path, target: Path) -> None:
    temporary = target.with_name(f".{target.name}.tmp-{uuid.uuid4().hex}")
    previous = target.with_name(f".{target.name}.old-{uuid.uuid4().hex}")
    try:
        shutil.copytree(
            source,
            temporary,
            ignore=shutil.ignore_patterns(".*"),
        )
        if target.exists():
            os.replace(target, previous)
        os.replace(temporary, target)
        sync_directory(target.parent)
        if previous.exists():
            shutil.rmtree(previous)
    except Exception:
        if previous.exists() and not target.exists():
            os.replace(previous, target)
        raise
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)
        if previous.exists():
            shutil.rmtree(previous)


def expected_manifest(release_dir: Path) -> dict[str, object]:
    dmg = release_dir / DMG_NAME
    checksum = release_dir / f"{DMG_NAME}.sha256"
    website_root = release_dir / "website"
    files = [dmg, checksum, *public_website_files(website_root)]
    return {
        "schemaVersion": 1,
        "product": "AIDOO Whisper Lite",
        "version": VERSION,
        "platform": "macOS",
        "architecture": "arm64",
        "minimumSystemVersion": "13.0",
        "bundleIdentifier": "app.aidoo.whisper-lite",
        "files": [
            {
                "path": path.relative_to(release_dir).as_posix(),
                "bytes": path.stat().st_size,
                "sha256": sha256(path),
            }
            for path in files
        ],
    }


def verify(release_dir: Path) -> None:
    dmg = release_dir / DMG_NAME
    checksum = release_dir / f"{DMG_NAME}.sha256"
    manifest_path = release_dir / "release-manifest.json"
    website_source = ROOT / "website"
    website_release = release_dir / "website"
    for path in (dmg, checksum, manifest_path, website_release):
        if not path.exists():
            raise ValueError(f"Missing website release asset: {path}")
    actual_entries = {
        path.name for path in release_dir.iterdir() if not path.name.startswith(".")
    }
    expected_entries = {
        DMG_NAME,
        f"{DMG_NAME}.sha256",
        "release-manifest.json",
        "website",
    }
    if actual_entries != expected_entries:
        raise ValueError("The website release directory contains missing or unexpected assets")

    expected_checksum = f"{sha256(dmg)}  {DMG_NAME}\n"
    if checksum.read_text() != expected_checksum:
        raise ValueError("The DMG checksum file does not match the staged DMG")

    source_files = public_website_files(website_source)
    release_files = public_website_files(website_release)
    source_names = [path.relative_to(website_source) for path in source_files]
    release_names = [path.relative_to(website_release) for path in release_files]
    if source_names != release_names:
        raise ValueError("The staged website file set differs from website/")
    for relative in source_names:
        if sha256(website_source / relative) != sha256(website_release / relative):
            raise ValueError(f"The staged website file differs: {relative}")

    actual_manifest = json.loads(manifest_path.read_text())
    if actual_manifest != expected_manifest(release_dir):
        raise ValueError("release-manifest.json does not match the staged assets")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dmg", type=Path)
    parser.add_argument("--output", type=Path, default=ROOT / "release" / VERSION)
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    release_dir = args.output.expanduser().resolve()

    if not args.verify:
        source_dmg = (args.dmg or release_dir / DMG_NAME).expanduser().resolve()
        if not source_dmg.is_file():
            raise ValueError(f"DMG not found: {source_dmg}")
        release_dir.mkdir(parents=True, exist_ok=True)
        target_dmg = release_dir / DMG_NAME
        if source_dmg != target_dmg:
            atomic_copy(source_dmg, target_dmg)
        checksum = release_dir / f"{DMG_NAME}.sha256"
        atomic_write_text(checksum, f"{sha256(target_dmg)}  {DMG_NAME}\n")
        replace_website_directory(ROOT / "website", release_dir / "website")
        manifest = expected_manifest(release_dir)
        manifest_path = release_dir / "release-manifest.json"
        atomic_write_text(manifest_path, json.dumps(manifest, indent=2) + "\n")

    verify(release_dir)
    print(f"Website release package verified: {release_dir}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(error, file=sys.stderr)
        raise SystemExit(1)
