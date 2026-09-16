#!/usr/bin/env python3
"""Generate the license notices bundled with AIDOO Whisper Control."""

from __future__ import annotations

import json
import hashlib
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "resources" / "THIRD_PARTY_NOTICES.txt"
LICENSE_NAMES = ("license", "licence", "copying", "notice", "unlicense")


def stable_text_key(value: object) -> tuple[str, str]:
    text = str(value)
    return (text.lower(), text)


def license_files(directory: Path) -> list[Path]:
    return sorted(
        path
        for path in directory.iterdir()
        if path.is_file()
        and path.name.lower().startswith(LICENSE_NAMES)
        and path.stat().st_size <= 250_000
    )


def cargo_packages() -> list[tuple[str, str, str, str, list[Path]]]:
    result = subprocess.run(
        [
            "cargo",
            "metadata",
            "--format-version=1",
            "--locked",
            "--manifest-path",
            str(ROOT / "src-tauri" / "Cargo.toml"),
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    metadata = json.loads(result.stdout)
    own_id = next(
        package["id"] for package in metadata["packages"] if package["name"] == "aidoo-whisper-control"
    )
    resolved = {node["id"] for node in metadata["resolve"]["nodes"]}
    packages = []
    for package in metadata["packages"]:
        if package["id"] == own_id or package["id"] not in resolved:
            continue
        directory = Path(package["manifest_path"]).parent
        files = license_files(directory)
        explicit = package.get("license_file")
        if explicit:
            candidate = directory / explicit
            if candidate.is_file() and candidate not in files:
                files.append(candidate)
        packages.append(
            (
                package["name"],
                package["version"],
                package.get("license") or "See included notice",
                package.get("repository") or package.get("homepage") or "",
                sorted(files),
            )
        )
    return sorted(packages, key=lambda value: (value[0].lower(), value[1]))


def node_packages() -> list[tuple[str, str, str, str, list[Path]]]:
    root_manifest = json.loads((ROOT / "package.json").read_text())
    pending = list(root_manifest.get("dependencies", {}))
    seen: set[str] = set()
    packages = []
    while pending:
        name = pending.pop()
        if name in seen:
            continue
        seen.add(name)
        directory = ROOT / "node_modules" / Path(name)
        manifest_path = directory / "package.json"
        if not manifest_path.is_file():
            raise RuntimeError(f"Missing installed production dependency: {name}")
        manifest = json.loads(manifest_path.read_text())
        pending.extend(manifest.get("dependencies", {}))
        repository = manifest.get("repository", "")
        if isinstance(repository, dict):
            repository = repository.get("url", "")
        packages.append(
            (
                manifest.get("name", name),
                manifest.get("version", "unknown"),
                manifest.get("license", "See included notice"),
                repository or manifest.get("homepage", ""),
                license_files(directory),
            )
        )
    return sorted(packages, key=lambda value: (value[0].lower(), value[1]))


def render() -> str:
    ecosystems = (("Rust", cargo_packages()), ("JavaScript", node_packages()))
    sections = [
        "AIDOO Whisper Control — Third-Party Notices",
        "==========================================",
        "",
        "AIDOO Whisper Control includes the open-source packages listed below. The application code",
        "created by Aidoo Ltd. OOD remains subject to its own distribution terms. Each dependency",
        "retains its original copyright and license terms.",
        "",
    ]
    notices: dict[str, dict[str, object]] = {}
    missing_notices: list[str] = []
    for ecosystem, packages in ecosystems:
        heading = ecosystem + " dependency inventory"
        sections.extend([heading, "-" * len(heading), ""])
        for name, version, license_name, source, files in packages:
            identity = f"{name} {version}"
            sections.append(f"{identity} — {license_name}" + (f" — {source}" if source else ""))
            if files:
                for path in files:
                    content = "\n".join(
                        line.rstrip()
                        for line in path.read_text(errors="replace").splitlines()
                    ).strip()
                    digest = hashlib.sha256(content.encode()).hexdigest()
                    notice = notices.setdefault(
                        digest,
                        {"content": content, "files": set(), "packages": set()},
                    )
                    notice["files"].add(path.name)  # type: ignore[union-attr]
                    notice["packages"].add(identity)  # type: ignore[union-attr]
            else:
                missing_notices.append(identity)
        sections.append("")

    if missing_notices:
        sections.extend([
            "Packages without a separate license file",
            "----------------------------------------",
            "",
            "Their declared license expression is included in the inventory above:",
            ", ".join(sorted(missing_notices, key=stable_text_key)),
            "",
        ])

    sections.extend(["Unique license and notice texts", "-------------------------------", ""])
    ordered = sorted(
        notices.items(),
        key=lambda item: sorted(item[1]["packages"], key=stable_text_key)[0],  # type: ignore[arg-type]
    )
    for index, (_, notice) in enumerate(ordered, start=1):
        packages = sorted(notice["packages"], key=stable_text_key)  # type: ignore[arg-type]
        files = sorted(notice["files"], key=stable_text_key)  # type: ignore[arg-type]
        sections.extend([
            f"Notice {index:03d} ({', '.join(files)})",
            f"Used by: {', '.join(packages)}",
            "",
            str(notice["content"]),
            "",
            "=" * 78,
            "",
        ])
    return "\n".join(sections).rstrip() + "\n"


def main() -> None:
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(render())
    print(f"Wrote {OUTPUT} ({OUTPUT.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
