#!/usr/bin/env python3
"""Reject advisory-affected packages from the shipped macOS dependency graph."""

from __future__ import annotations

import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
MACOS_TARGET = "aarch64-apple-darwin"

# RUSTSEC-2024-0429 affects glib >=0.15,<0.20. The locked 0.18.5 package is
# pulled in only by Linux desktop support and must never enter the macOS build.
FORBIDDEN_MACOS_PACKAGES = {
    "glib v0.18.5": "RUSTSEC-2024-0429",
}


def main() -> int:
    command = [
        "cargo",
        "tree",
        "--manifest-path",
        str(ROOT / "src-tauri/Cargo.toml"),
        "--locked",
        "--target",
        MACOS_TARGET,
        "--edges",
        "normal,build",
        "--prefix",
        "none",
        "--no-dedupe",
        "--format",
        "{p}",
    ]
    result = subprocess.run(command, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()
        raise SystemExit(f"Could not inspect the macOS dependency graph:\n{detail}")

    packages = set(result.stdout.splitlines())
    violations = [
        f"{package} ({advisory})"
        for package, advisory in FORBIDDEN_MACOS_PACKAGES.items()
        if package in packages
    ]
    if violations:
        raise SystemExit(
            "Advisory-affected packages entered the macOS release graph:\n"
            + "\n".join(violations)
        )

    print("macOS dependency boundary validation passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
