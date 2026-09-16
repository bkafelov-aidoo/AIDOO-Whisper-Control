#!/usr/bin/env python3
"""Validate immutable macOS release settings before an expensive build."""

from __future__ import annotations

import hashlib
import json
import os
import plistlib
import re
import tomllib
import wave
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

    toolchain = tomllib.loads((ROOT / "rust-toolchain.toml").read_text()).get(
        "toolchain", {}
    )
    expected_toolchain = {
        "channel": "1.93.1",
        "profile": "minimal",
        "targets": ["aarch64-apple-darwin"],
        "components": ["clippy", "rustfmt"],
    }
    if toolchain != expected_toolchain:
        errors.append(f"Rust release toolchain differs: {toolchain}")

    github_ref_type = os.environ.get("GITHUB_REF_TYPE")
    github_ref_name = os.environ.get("GITHUB_REF_NAME")
    is_github_release = (
        os.environ.get("GITHUB_ACTIONS") == "true"
        and os.environ.get("GITHUB_WORKFLOW")
        == "Release AIDOO Whisper Lite for macOS"
    )
    if is_github_release and github_ref_type != "tag":
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

    expected_csp = (
        "default-src 'self'; script-src 'self'; connect-src 'self' ipc: "
        "http://ipc.localhost; img-src 'self' data:; style-src 'self' 'unsafe-inline'; "
        "object-src 'none'; frame-src 'none'; base-uri 'none'; form-action 'none'"
    )
    if tauri.get("app", {}).get("security", {}).get("csp") != expected_csp:
        errors.append("The WebView content security policy differs from the audited boundary")

    overlay_windows = [
        window
        for window in tauri.get("app", {}).get("windows", [])
        if window.get("label") == "overlay"
    ]
    if len(overlay_windows) != 1:
        errors.append("The release must define exactly one overlay window")
    else:
        overlay_window = overlay_windows[0]
        expected_overlay_window = {
            "alwaysOnTop": True,
            "visibleOnAllWorkspaces": True,
            "focus": False,
            "focusable": False,
            "skipTaskbar": True,
            "transparent": True,
            "visible": False,
        }
        for name, expected in expected_overlay_window.items():
            if overlay_window.get(name) != expected:
                errors.append(
                    f"Overlay {name} differs: {overlay_window.get(name)!r}; "
                    f"expected {expected!r}"
                )

    if set(bundle.get("targets", [])) != {"app", "dmg"}:
        errors.append("The macOS release must produce exactly app and dmg bundles")
    if "icons/icon.icns" not in bundle.get("icon", []):
        errors.append("The original macOS icon is missing from the bundle configuration")
    icon_hash = hashlib.sha256(
        (ROOT / "src-tauri/icons/icon.png").read_bytes()
    ).hexdigest()
    if icon_hash != EXPECTED_PRODUCT_ICON_SHA256:
        errors.append("The original AIDOO product icon has been changed")

    expected_feedback_sounds = {
        "resources/sounds/recording-start.wav": (
            "sounds/recording-start.wav",
            "69346ba258eb1e295327edc0e284a1af13cd41915bf14f81bf972ad63e362fa8",
        ),
        "resources/sounds/recording-stop.wav": (
            "sounds/recording-stop.wav",
            "50814bacfc01bbb528e4cff8d301b1475ea4cf9a284ffa71494eb37121d64785",
        ),
    }
    resources = bundle.get("resources", {})
    for source, (destination, expected_hash) in expected_feedback_sounds.items():
        path = ROOT / "src-tauri" / source
        if resources.get(source) != destination:
            errors.append(f"The feedback sound is not bundled at {destination}: {source}")
            continue
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected_hash:
            errors.append(f"The reviewed feedback sound has changed: {source}")
        with wave.open(str(path), "rb") as audio:
            if (
                audio.getnchannels() != 1
                or audio.getsampwidth() != 2
                or audio.getframerate() != 44_100
                or not 0.4 <= audio.getnframes() / audio.getframerate() <= 0.6
            ):
                errors.append(f"Unexpected feedback sound format or duration: {source}")

    with (ROOT / "src-tauri/Entitlements.plist").open("rb") as source:
        entitlements = plistlib.load(source)
    for entitlement in (
        "com.apple.security.device.audio-input",
        "com.apple.security.network.client",
        "com.apple.security.automation.apple-events",
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

    expected_external_urls = {
        "https://platform.openai.com/api-keys",
        "https://app.aidoo.bg/clinics/*/login",
        "https://aidoo-web.on.dev-craft.tech/clinics/*/login",
        "mailto:support@aidoo.bg",
    }
    structured_permissions = [
        permission
        for permission in main_capabilities.get("permissions", [])
        if isinstance(permission, dict)
    ]
    opener_permissions = [
        permission
        for permission in structured_permissions
        if permission.get("identifier") == "opener:allow-open-url"
    ]
    if len(structured_permissions) != 1:
        errors.append("Main capabilities must contain exactly one structured permission")
    if len(opener_permissions) != 1:
        errors.append("Main capabilities must contain exactly one scoped URL opener")
    else:
        opener = opener_permissions[0]
        allowed_entries = opener.get("allow", [])
        allowed_urls = {
            entry.get("url")
            for entry in allowed_entries
            if isinstance(entry, dict) and set(entry) == {"url"}
        }
        if len(allowed_urls) != len(allowed_entries) or allowed_urls != expected_external_urls:
            errors.append(
                "External URL permissions must contain exactly the reviewed OpenAI, "
                "AIDOO clinic, and support destinations"
            )
        if opener.get("deny"):
            errors.append("The scoped URL opener must not define an unexpected deny list")

    frontend_source = "\n".join(
        path.read_text() for path in (ROOT / "src").rglob("*.tsx")
    )
    expected_support_binding = 'const SUPPORT_EMAIL_URL = "mailto:support@aidoo.bg";'
    if frontend_source.count(expected_support_binding) != 1:
        errors.append("The frontend must bind support to the exact allowlisted AIDOO email")
    if "AIDOO-Whisper-Lite/issues" in frontend_source:
        errors.append("The frontend must not link users to private source-repository Issues")

    expected_transcription_models = {
        "gpt-4o-mini-transcribe": "$0.003",
        "gpt-transcribe": "$0.0045",
    }
    model_source = (ROOT / "src-tauri/src/models.rs").read_text()
    type_source = (ROOT / "src/types.ts").read_text()
    translation_source = (ROOT / "src/i18n.ts").read_text()
    model_documentation = (ROOT / "docs/MODELS.md").read_text()
    rust_models = set(
        re.findall(
            r'^pub const (?:ECONOMY|ACCURACY)_MODEL: &str = "([^"]+)";$',
            model_source,
            re.MULTILINE,
        )
    )
    frontend_models = set(
        re.findall(r'"(gpt-[a-z0-9.-]*transcribe[a-z0-9.-]*)"', frontend_source)
    )
    typed_models = set(
        re.findall(r'"(gpt-[a-z0-9.-]*transcribe[a-z0-9.-]*)"', type_source)
    )
    expected_model_ids = set(expected_transcription_models)
    for source_name, actual_models in (
        ("native model constants", rust_models),
        ("frontend model controls", frontend_models),
        ("frontend model type", typed_models),
    ):
        if actual_models != expected_model_ids:
            errors.append(
                f"{source_name} differ from the audited model set: {sorted(actual_models)}"
            )
    for model, price in expected_transcription_models.items():
        if model not in model_documentation or price not in model_documentation:
            errors.append(f"Model documentation is missing {model} at {price}/minute")
        if price not in translation_source or price not in frontend_source:
            errors.append(f"Interface price copy is missing the audited {price}/minute value")
    for official_model_page in (
        "https://developers.openai.com/api/docs/models/gpt-4o-mini-transcribe",
        "https://developers.openai.com/api/docs/models/gpt-transcribe",
        "https://developers.openai.com/api/docs/pricing",
    ):
        if official_model_page not in model_documentation:
            errors.append(
                f"Model release evidence is missing the official source: {official_model_page}"
            )

    runtime_sources = "\n".join(
        path.read_text() for path in (ROOT / "src-tauri/src").rglob("*.rs")
    )
    runtime_https_urls = set(re.findall(r'"(https://[^"\s]+)"', runtime_sources))
    expected_runtime_https_urls = {
        "https://api.openai.com/v1/models",
        "https://api.openai.com/v1/audio/transcriptions",
        "https://api.openai.com/v1/live/sessions",
        "https://app.aidoo.bg",
        "https://app.aidoo.bg/web",
        "https://aidoo-web.on.dev-craft.tech",
        "https://aidoo-platform.on.dev-craft.tech/web",
    }
    if runtime_https_urls != expected_runtime_https_urls:
        errors.append(
            f"Native runtime HTTPS destinations differ: {sorted(runtime_https_urls)}"
        )
    if runtime_sources.count(".https_only(true)") != 4:
        errors.append("All native API clients must reject non-HTTPS requests")
    # Four production clients plus the HTTP-only local contract-test client.
    if runtime_sources.count(".redirect(reqwest::redirect::Policy::none())") != 5:
        errors.append("All native API clients must reject HTTP redirects")
    javascript_dependencies = {
        **package.get("dependencies", {}),
        **package.get("devDependencies", {}),
    }
    if any("updater" in name.lower() for name in javascript_dependencies):
        errors.append("The no-updater product must not include an updater dependency")
    if "tauri-plugin-updater" in cargo_source:
        errors.append("The native application must not include the Tauri updater plugin")

    interface_styles = (ROOT / "src/styles.css").read_text()
    if 'font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text"' not in interface_styles:
        errors.append("The interface must use the macOS system font stack")
    if not re.search(r"body\s*\{[^}]*font-size:\s*13px", interface_styles):
        errors.append("The macOS body text must keep the HIG 13-point default size")
    undersized_text = [
        value
        for value in re.findall(r"font-size:\s*([0-9]+(?:\.[0-9]+)?)px", interface_styles)
        if float(value) < 10
    ]
    if undersized_text:
        errors.append(f"Interface text falls below the macOS 10-point minimum: {undersized_text}")
    for accessibility_style in (
        ":focus-visible",
        "prefers-reduced-motion: reduce",
        "prefers-reduced-transparency: reduce",
        "prefers-contrast: more",
    ):
        if accessibility_style not in interface_styles:
            errors.append(f"Interface accessibility style is missing: {accessibility_style}")

    expected_main_string_permissions = {
        "core:event:allow-listen",
        "core:event:allow-unlisten",
        "dialog:allow-open",
        "autostart:default",
        "allow-bootstrap",
        "allow-update-settings",
        "allow-save-api-key",
        "allow-delete-api-key",
        "allow-connect-aidoo",
        "allow-reconnect-aidoo",
        "allow-disconnect-aidoo",
        "allow-aidoo-search-patients",
        "allow-aidoo-select-patient",
        "allow-aidoo-next-patient",
        "allow-aidoo-begin-status",
        "allow-aidoo-start-status-visit",
        "allow-aidoo-apply-status",
        "allow-aidoo-finish-status",
        "allow-aidoo-add-procedure",
        "allow-aidoo-write-diagnosis",
        "allow-aidoo-write-official-note",
        "allow-aidoo-find-schedule-slot",
        "allow-aidoo-book-schedule-slot",
        "allow-aidoo-status-catalog",
        "allow-aidoo-diagnosis-catalog",
        "allow-aidoo-procedure-catalog",
        "allow-aidoo-active-treatments",
        "allow-aidoo-create-status-visit",
        "allow-aidoo-prepare-status-draft",
        "allow-aidoo-confirm-status-draft",
        "allow-aidoo-cancel-status-draft",
        "allow-aidoo-prepare-treatment-draft",
        "allow-aidoo-confirm-treatment-draft",
        "allow-aidoo-cancel-treatment-draft",
        "allow-begin-shortcut-capture",
        "allow-cancel-shortcut-capture",
        "allow-test-microphone",
        "allow-start-wake-word-calibration",
        "allow-stop-wake-word-calibration",
        "allow-prepare-live-session",
        "allow-create-live-session",
        "allow-end-live-session",
        "allow-record-live-backend-usage",
        "allow-set-live-phase",
        "allow-request-live-stop",
        "allow-take-assistant-request",
        "allow-start-voice-dictation",
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
        "allow-stop-and-transcribe",
        "allow-request-live-stop",
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
    rust_source = "\n".join(
        path.read_text() for path in (ROOT / "src-tauri/src").rglob("*.rs")
    )
    for rust_safety_guard in (
        "#![deny(unsafe_op_in_unsafe_fn)]",
        "#![deny(clippy::undocumented_unsafe_blocks)]",
    ):
        if rust_safety_guard not in rust_source:
            errors.append(f"Native safety lint is missing: {rust_safety_guard}")
    for lifecycle_guard in (
        'const APP_QUIT_MENU_ID: &str = "aidoo-app-quit";',
        ".menu(build_application_menu)",
        "request_app_quit(app)",
        "refresh_application_menu(app);",
        'window.label() == "overlay"',
        "overlay_accepts_pointer_input",
    ):
        if lifecycle_guard not in rust_source:
            errors.append(f"Native operation lifecycle guard is missing: {lifecycle_guard}")
    if "PredefinedMenuItem::quit" in rust_source:
        errors.append("The macOS application menu must not bypass the owned quit guard")
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

    workflow_path = ROOT / ".github/workflows/release-lite-macos.yml"
    workflow = workflow_path.read_text()
    workflow_sources = {
        path: path.read_text()
        for pattern in ("*.yml", "*.yaml")
        for path in (ROOT / ".github/workflows").glob(pattern)
    }
    action_references = [
        (path, repository, reference)
        for path, source in workflow_sources.items()
        for repository, reference in re.findall(
            r"^\s*-?\s*uses:\s*([^@\s]+)@([^\s#]+)", source, re.MULTILINE
        )
    ]
    unpinned_actions = [
        f"{path.name}:{repository}@{reference}"
        for path, repository, reference in action_references
        if not re.fullmatch(r"[0-9a-f]{40}", reference)
    ]
    if unpinned_actions:
        errors.append(
            "Release workflow actions must use immutable commit SHAs: "
            + ", ".join(unpinned_actions)
        )
    expected_action_references = {
        "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
        "actions/setup-node@820762786026740c76f36085b0efc47a31fe5020",
        "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
        "dtolnay/rust-toolchain@6bed0761d98439e5a578e2877258200ad565ba87",
        "rustsec/audit-check@69366f33c96575abad1ee0dba8212993eecbe998",
    }
    actual_action_references = {
        f"{repository}@{reference}"
        for _, repository, reference in action_references
    }
    if actual_action_references != expected_action_references:
        errors.append(
            "GitHub Action allowlist differs: "
            + ", ".join(sorted(actual_action_references))
        )
    expected_workflow_permissions = {"contents": "read", "checks": "write"}
    for path, source in workflow_sources.items():
        permissions_match = re.search(
            r"(?m)^permissions:\n((?:  [a-z-]+: (?:read|write|none)\n)+)", source
        )
        workflow_permissions = (
            dict(
                re.findall(
                    r"^  ([a-z-]+): (read|write|none)$",
                    permissions_match.group(1),
                    re.MULTILINE,
                )
            )
            if permissions_match
            else {}
        )
        if workflow_permissions != expected_workflow_permissions:
            errors.append(
                f"{path.name} workflow permissions differ: {workflow_permissions}"
            )
        if source.count("persist-credentials: false") != 1:
            errors.append(
                f"{path.name} must disable persisted checkout credentials exactly once"
            )
        if source.count("toolchain: 1.93.1") != 1:
            errors.append(f"{path.name} must use the pinned Rust release toolchain")
        if source.count(
            "cargo install cargo-audit --version 0.22.2 --locked"
        ) != 1:
            errors.append(
                f"{path.name} must install the locked Rust audit tool exactly once"
            )
    for required_workflow_guard in (
        "runs-on: macos-15",
        "group: aidoo-whisper-lite-macos-${{ github.ref }}",
        "cancel-in-progress: false",
        "timeout-minutes: 75",
        "umask 077",
        "retention-days: 14",
    ):
        if required_workflow_guard not in workflow:
            errors.append(
                f"Release workflow guard is missing: {required_workflow_guard}"
            )
    ci_workflow = workflow_sources.get(ROOT / ".github/workflows/ci.yml", "")
    if not re.search(r"(?m)^  push:\n    branches: \[main\]$", ci_workflow):
        errors.append("CI must run after every push to the dedicated main branch")
    for required_ci_guard in (
        "pull_request:",
        "runs-on: macos-15",
        "timeout-minutes: 30",
        "rustsec/audit-check@69366f33c96575abad1ee0dba8212993eecbe998",
        "token: ${{ secrets.GITHUB_TOKEN }}",
        "cargo test --locked --release --target aarch64-apple-darwin",
        "cargo clippy --locked --release --target aarch64-apple-darwin",
    ):
        if required_ci_guard not in ci_workflow:
            errors.append(f"Source CI guard is missing: {required_ci_guard}")
    release_script = (ROOT / "scripts/release-mac.sh").read_text()
    for required_release_command in (
        "cargo test --locked --release --target aarch64-apple-darwin",
        "cargo clippy --locked --release --target aarch64-apple-darwin",
        "npx tauri build --target aarch64-apple-darwin --bundles app,dmg --ci -- --locked",
    ):
        if required_release_command not in release_script:
            errors.append(f"Local release command is missing: {required_release_command}")

    expected_release_secrets = {
        "APPLE_CERTIFICATE",
        "APPLE_CERTIFICATE_PASSWORD",
        "KEYCHAIN_PASSWORD",
        "APPLE_ID",
        "APPLE_PASSWORD",
        "APPLE_TEAM_ID",
    }
    workflow_secrets = set(re.findall(r"secrets\.([A-Z0-9_]+)", workflow)) - {
        "GITHUB_TOKEN"
    }
    if workflow_secrets != expected_release_secrets:
        errors.append(
            f"Release workflow secrets differ: {sorted(workflow_secrets)}"
        )
    for secret in expected_release_secrets:
        required_preflight = ': "${' + secret + f':?Missing {secret}}}"'
        if required_preflight not in workflow:
            errors.append(f"Release workflow does not fail fast for {secret}")
    for release_identity_guard in (
        "security find-identity -v -p codesigning build.keychain",
        "Developer ID Application: Aidoo Ltd. OOD (4KKVT2TUUA)",
        '[[ "$APPLE_TEAM_ID" != "4KKVT2TUUA" ]]',
    ):
        if release_identity_guard not in workflow:
            errors.append(
                f"Release credential identity guard is missing: {release_identity_guard}"
            )
    wizard_path = ROOT / "scripts/configure-github-release-secrets.sh"
    if not wizard_path.is_file():
        errors.append("GitHub release-secret wizard is missing")
    else:
        wizard = wizard_path.read_text()
        wizard_secrets = set(
            re.findall(r"^set_secret\s+([A-Z0-9_]+)\s", wizard, re.MULTILINE)
        )
        if wizard_secrets != expected_release_secrets:
            errors.append(
                f"Release-secret wizard outputs differ: {sorted(wizard_secrets)}"
            )
        if wizard_path.stat().st_mode & 0o111 == 0:
            errors.append("GitHub release-secret wizard is not executable")
        for forbidden_release_action in ("gh workflow run", "git tag", "git push"):
            if forbidden_release_action in wizard:
                errors.append(
                    "Release-secret wizard must not publish or start releases: "
                    + forbidden_release_action
                )
        if "Refusing to accept pre-existing secret names" not in wizard:
            errors.append(
                "Release-secret wizard must reject any failed secret write"
            )
    gitignore = (ROOT / ".gitignore").read_text().splitlines()
    for private_key_pattern in ("*.p12", "*.p8"):
        if private_key_pattern not in gitignore:
            errors.append(f"Private key ignore rule is missing: {private_key_pattern}")

    if errors:
        raise SystemExit("\n".join(errors))
    print("macOS release configuration validation passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
