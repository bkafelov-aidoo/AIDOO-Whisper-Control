#!/usr/bin/env python3
"""Keep production modules small enough to review as a whole."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE_ROOTS = (ROOT / "src", ROOT / "src-tauri" / "src", ROOT / "scripts")
CODE_SUFFIXES = {".css", ".html", ".mjs", ".py", ".rs", ".sh", ".ts", ".tsx"}
MAX_LINES = 1_000


def main() -> int:
    oversized: list[tuple[Path, int]] = []
    checked = 0
    for source_root in SOURCE_ROOTS:
        for path in source_root.rglob("*"):
            if not path.is_file() or path.suffix not in CODE_SUFFIXES:
                continue
            checked += 1
            line_count = len(path.read_text().splitlines())
            if line_count > MAX_LINES:
                oversized.append((path.relative_to(ROOT), line_count))

    if oversized:
        for path, line_count in oversized:
            print(f"{path}: {line_count} lines (maximum {MAX_LINES})")
        return 1

    print(f"Source layout validation passed ({checked} files, maximum {MAX_LINES} lines).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
