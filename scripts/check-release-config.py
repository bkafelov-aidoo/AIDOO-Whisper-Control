#!/usr/bin/env python3
"""Validate immutable macOS release settings before an expensive build."""

from __future__ import annotations

import hashlib
import json
import os
import plistlib
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
EXPECTED_IDENTITY = "Developer ID Application: Aidoo Ltd. OOD (4KKVT2TUUA)"
EXPECTED_PRODUCT_ICON_SHA256 = (
    "d9cd1ed91661c76ce7a7e6d3fed82c3bc95c5335f5081f708f73182ea9d539e9"
)


def main() -> int:
    errors: list[str] = []
    package = json.loads((ROOT / "package.json").read_text())
    tauri = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())
    cargo_source = (ROOT / "src-tauri/Cargo.toml").read_text()
    cargo_version = re.search(r'^version\s*=\s*"([^"]+)"', cargo_source, re.MULTILINE)
    versions = {
        "package.json": package.get("version"),
        "tauri.conf.json": tauri.get("version"),
        "Cargo.toml": cargo_version.group(1) if cargo_version else None,
    }
    if None in versions.values() or len(set(versions.values())) != 1:
        errors.append(f"Release versions differ: {versions}")

    github_ref_type = os.environ.get("GITHUB_REF_TYPE")
    github_ref_name = os.environ.get("GITHUB_REF_NAME")
    if os.environ.get("GITHUB_ACTIONS") == "true" and github_ref_type != "tag":
        errors.append("The macOS release workflow must run from an exact release tag")
    if github_ref_type == "tag":
        expected_tag = f"lite-v{versions['package.json']}"
        if github_ref_name != expected_tag:
            errors.append(
                f"Release tag {github_ref_name!r} does not match {expected_tag!r}"
            )

    bundle = tauri.get("bundle", {})
    macos = bundle.get("macOS", {})
    expected_values = {
        "identifier": (tauri.get("identifier"), "app.aidoo.whisper-lite"),
        "minimumSystemVersion": (macos.get("minimumSystemVersion"), "13.0"),
        "signingIdentity": (macos.get("signingIdentity"), EXPECTED_IDENTITY),
        "createUpdaterArtifacts": (bundle.get("createUpdaterArtifacts"), False),
    }
    for name, (actual, expected) in expected_values.items():
        if actual != expected:
            errors.append(f"Unexpected {name}: {actual!r}; expected {expected!r}")

    if set(bundle.get("targets", [])) != {"app", "dmg"}:
        errors.append("The macOS release must produce exactly app and dmg bundles")
    if "icons/icon.icns" not in bundle.get("icon", []):
        errors.append("The original macOS icon is missing from the bundle configuration")
    icon_hash = hashlib.sha256(
        (ROOT / "src-tauri/icons/icon.png").read_bytes()
    ).hexdigest()
    if icon_hash != EXPECTED_PRODUCT_ICON_SHA256:
        errors.append("The original AIDOO product icon has been changed")

    with (ROOT / "src-tauri/Entitlements.plist").open("rb") as source:
        entitlements = plistlib.load(source)
    for entitlement in (
        "com.apple.security.device.audio-input",
        "com.apple.security.network.client",
    ):
        if entitlements.get(entitlement) is not True:
            errors.append(f"Required entitlement is missing: {entitlement}")

    capabilities = json.loads(
        (ROOT / "src-tauri/capabilities/default.json").read_text()
    )
    serialized_permissions = json.dumps(capabilities.get("permissions", []))
    for required_url in (
        "https://platform.openai.com/api-keys",
        "https://github.com/bkafelov-aidoo/Aidoo-Whisper/issues",
    ):
        if required_url not in serialized_permissions:
            errors.append(f"Required external URL permission is missing: {required_url}")

    if errors:
        raise SystemExit("\n".join(errors))
    print("macOS release configuration validation passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
