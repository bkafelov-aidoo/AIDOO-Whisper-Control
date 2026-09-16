# macOS release runbook

## Certificate and notarization

Direct website distribution uses **Developer ID Application: Aidoo Ltd. OOD (4KKVT2TUUA)** with Hardened Runtime. It does not use Apple Distribution, Mac Installer Distribution or a development certificate. The existing `AIDOO_VIEWER_NOTARY` notarytool Keychain profile authenticates Apple notarization.

The bundle identifier is `app.aidoo.whisper-lite`, the minimum system version is macOS 13, and the first release targets Apple Silicon only. The app requests microphone access, Accessibility and optional Apple Events access to Google Chrome so AIDOO Control can reuse the clinic tab and show confirmed changes immediately. If the user denies Chrome control, the app opens the canonical patient URL without tab reuse. It is not App Sandbox constrained because it is distributed outside the Mac App Store.

## Version and local release

Keep the same semantic version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`. The verified release toolchain is pinned in `rust-toolchain.toml` and both workflows; update it deliberately only after the full source checks pass on the replacement compiler. Update `website/release-notes.html`, regenerate `resources/THIRD_PARTY_NOTICES.txt`, commit, and create the exact `lite-vX.Y.Z` tag. The final release command refuses uncommitted Lite changes, an untagged HEAD or notices that differ after regeneration. Its preflight also proves that every registered Rust command appears in the application manifest and exactly one window capability boundary. CI rejects a branch run or a tag whose version differs from the application before it imports the signing certificate.

`npm run check:website` validates the three static public pages, their local links, required privacy/support/release content, matching product icon and release version. It also rejects active web elements and every external reference except the exact `mailto:support@aidoo.bg` support link.

Run `npm run release:mac`. It writes the final website assets to `release/<version>/`:

- notarized and stapled DMG;
- SHA-256 checksum for the DMG;
- privacy, support and release-notes pages with their shared style and product icon;
- `release-manifest.json` with the size and SHA-256 of every staged file.

Upload the DMG and checksum to the AIDOO website. Users install a new version by downloading the newer notarized DMG from the website; the application does not perform background or in-app update checks.

The release command automatically runs `npm run audit:mac` on the exact copied DMG. This independent check mounts the distribution image and revalidates its checksum, architecture, deployment target, identity, entitlements, notarization and Gatekeeper status without launching the application. Run `npm run audit:mac` again after transferring the files to another location or before upload.

Publish the DMG and checksum links on the product website. Publish the contents of `release/<version>/website/` alongside them. `npm run verify:website` confirms that every staged page matches `website/`, the checksum matches the DMG and the manifest matches all staged files.

## GitHub secrets for CI

Run `./scripts/configure-github-release-secrets.sh` from a terminal while signed in with GitHub CLI. The four-stage wizard verifies access to the dedicated private repository, validates the exact Developer ID Application `.p12`, stores the six encrypted repository secrets and confirms only their names. It never creates a release tag or starts a workflow. Exported `.p12` and App Store Connect `.p8` files are ignored by Git as a second guard against accidental commits.

The included workflow expects `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID`. Export the Developer ID Application identity with its private key as a password-protected `.p12`; store only its base64 value and password as repository secrets. Use an Apple app-specific password for notarization. The workflow fails before build when a secret is empty, verifies the imported Developer ID identity immediately and rejects a Team ID other than `4KKVT2TUUA`. The wizard also fails if any secret write failed, so an older secret with the same name cannot be mistaken for the newly supplied value. Each secret is scoped to its required workflow step, the temporary certificate uses owner-only permissions, notarization credentials are removed from the environment before npm, Cargo and Tauri child processes run, and the temporary signing Keychain is deleted even after a failed job. Every external GitHub Action in both workflows is pinned to an immutable commit SHA, which the release preflight enforces. Per-tag concurrency prevents duplicate simultaneous release jobs, the job has a 75-minute ceiling, and the private artifact is retained for 14 days. Release and source checks use the Apple Silicon `macos-15` runner; the configuration moved off `macos-14` before its scheduled November 2026 retirement. The source workflow audits the locked Rust and production JavaScript dependencies, runs the source checks, builds the production frontend, and runs native unit tests and Clippy without release secrets. The release workflow repeats both dependency audits, then runs the same build, app-first stapling, DMG rebuild, notarization and `audit:mac` sequence used for a local release.

The dedicated repository requires full commit-SHA pins for every GitHub Action, keeps the workflow token read-only by default and deletes merged branches automatically. The source and release workflows add only `checks: write` so the pinned RustSec action can publish its result; source access remains read-only, and checkout credentials are removed before project scripts run. The source workflow runs for pull requests and every push to `main`, including subtree exports from the development repository. Cargo test, lint and bundle commands use the committed lockfile and refuse dependency resolution changes. The local notarized release remains the reference path until the release secrets are configured and a CI artifact passes the acceptance checklist.

## Mac App Store later

An App Store build requires a separate Apple Distribution signing path, App Sandbox entitlements and replacement of the private macOS overlay APIs. Treat it as a separate distribution target rather than reusing this direct-download bundle unchanged.
