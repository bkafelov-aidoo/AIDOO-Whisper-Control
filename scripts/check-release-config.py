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

    main_capabilities = json.loads(
        (ROOT / "src-tauri/capabilities/main.json").read_text()
    )
    overlay_capabilities = json.loads(
        (ROOT / "src-tauri/capabilities/overlay.json").read_text()
    )
    if main_capabilities.get("windows") != ["main"]:
        errors.append("Main capabilities must apply only to the main window")
    if overlay_capabilities.get("windows") != ["overlay"]:
        errors.append("Overlay capabilities must apply only to the overlay window")

    serialized_permissions = json.dumps(main_capabilities.get("permissions", []))
    for required_url in (
        "https://platform.openai.com/api-keys",
        "https://github.com/bkafelov-aidoo/Aidoo-Whisper/issues",
    ):
        if required_url not in serialized_permissions:
            errors.append(f"Required external URL permission is missing: {required_url}")

    expected_main_string_permissions = {
        "core:event:allow-listen",
        "core:event:allow-unlisten",
        "dialog:allow-open",
        "autostart:default",
        "allow-bootstrap",
        "allow-update-settings",
        "allow-save-api-key",
        "allow-delete-api-key",
        "allow-begin-shortcut-capture",
        "allow-cancel-shortcut-capture",
        "allow-test-microphone",
        "allow-start-recording",
        "allow-stop-and-transcribe",
        "allow-retry-failed-transcription",
        "allow-retranscribe-history-item",
        "allow-delete-failed-recording",
        "allow-copy-text",
        "allow-delete-history-item",
        "allow-open-accessibility-settings",
        "allow-refresh-accessibility-status",
        "allow-open-local-path",
        "allow-create-diagnostic-bundle",
    }
    main_string_permissions = {
        permission
        for permission in main_capabilities.get("permissions", [])
        if isinstance(permission, str)
    }
    if main_string_permissions != expected_main_string_permissions:
        errors.append("Main application-command permissions are not least-privilege")

    expected_overlay_permissions = {
        "core:event:allow-listen",
        "core:event:allow-unlisten",
        "core:window:allow-set-size",
        "allow-overlay-bootstrap",
        "allow-current-recording-snapshot",
        "allow-reposition-overlay",
    }
    overlay_permissions = set(overlay_capabilities.get("permissions", []))
    if overlay_permissions != expected_overlay_permissions:
        errors.append(
            "Overlay permissions exceed the event-listen and set-size boundary"
        )

    build_source = (ROOT / "src-tauri/build.rs").read_text()
    manifest_match = re.search(
        r"const COMMANDS:\s*&\[&str\]\s*=\s*&\[(.*?)\];",
        build_source,
        re.DOTALL,
    )
    manifest_commands = (
        set(re.findall(r'"([a-z][a-z0-9_]*)"', manifest_match.group(1)))
        if manifest_match
        else set()
    )
    rust_source = (ROOT / "src-tauri/src/lib.rs").read_text()
    handler_match = re.search(
        r"\.invoke_handler\(tauri::generate_handler!\[(.*?)\]\)",
        rust_source,
        re.DOTALL,
    )
    handler_commands = (
        set(
            re.findall(
                r"^\s*([a-z][a-z0-9_]*)\s*,?\s*$",
                handler_match.group(1),
                re.MULTILINE,
            )
        )
        if handler_match
        else set()
    )
    allowed_commands = {
        permission.removeprefix("allow-").replace("-", "_")
        for permission in main_string_permissions | overlay_permissions
        if permission.startswith("allow-")
    }
    if not manifest_commands or manifest_commands != handler_commands:
        errors.append("Tauri AppManifest commands differ from the invoke handler")
    if manifest_commands != allowed_commands:
        errors.append("Application command permissions do not cover the exact manifest")

    workflow = (ROOT.parent / ".github/workflows/release-lite-macos.yml").read_text()
    action_references = re.findall(
        r"^\s*-?\s*uses:\s*[^@\s]+@([^\s#]+)", workflow, re.MULTILINE
    )
    unpinned_actions = [
        reference
        for reference in action_references
        if not re.fullmatch(r"[0-9a-f]{40}", reference)
    ]
    if unpinned_actions:
        errors.append(
            "Release workflow actions must use immutable commit SHAs: "
            + ", ".join(unpinned_actions)
        )

    if errors:
        raise SystemExit("\n".join(errors))
    print("macOS release configuration validation passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
