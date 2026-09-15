#!/usr/bin/env python3
"""Validate the small, static website bundle without network access."""

from __future__ import annotations

from html.parser import HTMLParser
import json
from pathlib import Path
import sys
from typing import Optional
from urllib.parse import unquote, urlsplit


ROOT = Path(__file__).resolve().parent.parent
WEBSITE = ROOT / "website"
REQUIRED_PAGES = {
    "privacy.html": ("openai", "keychain", "диагност"),
    "support.html": ("support@aidoo.bg", "api ключ", "accessibility"),
    "release-notes.html": ("първо публично издание",),
}
FORBIDDEN_ELEMENTS = {"script", "iframe", "form", "object", "embed"}


class PageParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.html_language = ""
        self.has_viewport = False
        self.in_title = False
        self.title_parts: list[str] = []
        self.references: list[tuple[str, str, str]] = []
        self.forbidden: set[str] = set()

    def handle_starttag(self, tag: str, attrs: list[tuple[str, Optional[str]]]) -> None:
        attributes = dict(attrs)
        if tag == "html":
            self.html_language = attributes.get("lang") or ""
        if tag == "meta" and (attributes.get("name") or "").lower() == "viewport":
            self.has_viewport = bool(attributes.get("content"))
        if tag == "title":
            self.in_title = True
        if tag in FORBIDDEN_ELEMENTS:
            self.forbidden.add(tag)
        for attribute in ("href", "src"):
            value = attributes.get(attribute)
            if value:
                self.references.append((tag, attribute, value))

    def handle_endtag(self, tag: str) -> None:
        if tag == "title":
            self.in_title = False

    def handle_data(self, data: str) -> None:
        if self.in_title:
            self.title_parts.append(data)

    @property
    def title(self) -> str:
        return " ".join(self.title_parts).strip()


def resolve_local_reference(
    page: Path, tag: str, attribute: str, reference: str
) -> Optional[Path]:
    parsed = urlsplit(reference)
    if parsed.scheme:
        if (
            tag == "a"
            and attribute == "href"
            and parsed.scheme == "mailto"
            and reference == "mailto:support@aidoo.bg"
        ):
            return None
        if parsed.scheme != "https":
            raise ValueError(f"{page.name}: non-HTTPS external reference: {reference}")
        if tag != "a" or attribute != "href":
            raise ValueError(f"{page.name}: remote embedded resource is not allowed: {reference}")
        return None
    if parsed.netloc:
        raise ValueError(f"{page.name}: protocol-relative reference is not allowed: {reference}")
    if not parsed.path:
        return None
    target = (page.parent / unquote(parsed.path)).resolve()
    if WEBSITE.resolve() not in target.parents and target != WEBSITE.resolve():
        raise ValueError(f"{page.name}: reference escapes website/: {reference}")
    return target


def validate_page(page: Path, required_phrases: tuple[str, ...]) -> list[str]:
    errors: list[str] = []
    source = page.read_text(encoding="utf-8")
    parser = PageParser()
    try:
        parser.feed(source)
        parser.close()
    except Exception as error:
        return [f"{page.name}: invalid HTML: {error}"]

    if parser.html_language != "bg":
        errors.append(f"{page.name}: <html lang=\"bg\"> is required")
    if not parser.title:
        errors.append(f"{page.name}: a non-empty <title> is required")
    if not parser.has_viewport:
        errors.append(f"{page.name}: a viewport meta tag is required")
    if parser.forbidden:
        errors.append(
            f"{page.name}: forbidden active elements: {', '.join(sorted(parser.forbidden))}"
        )
    lowered = source.lower()
    for phrase in required_phrases:
        if phrase not in lowered:
            errors.append(f"{page.name}: required content is missing: {phrase}")
    for tag, attribute, reference in parser.references:
        try:
            target = resolve_local_reference(page, tag, attribute, reference)
        except ValueError as error:
            errors.append(str(error))
            continue
        if target is not None and not target.is_file():
            errors.append(f"{page.name}: broken {attribute}: {reference}")
    return errors


def main() -> int:
    errors: list[str] = []
    actual_pages = {path.name for path in WEBSITE.glob("*.html")}
    if actual_pages != set(REQUIRED_PAGES):
        errors.append("website/ must contain exactly privacy, support and release-notes HTML pages")
    for name, phrases in REQUIRED_PAGES.items():
        page = WEBSITE / name
        if not page.is_file():
            errors.append(f"Missing website page: {name}")
            continue
        errors.extend(validate_page(page, phrases))

    version = json.loads((ROOT / "package.json").read_text())["version"]
    release_notes_path = WEBSITE / "release-notes.html"
    if release_notes_path.is_file():
        release_notes = release_notes_path.read_text(encoding="utf-8")
        if f">{version} —" not in release_notes:
            errors.append("release-notes.html does not contain the package version")
    if not (WEBSITE / "site.css").is_file():
        errors.append("website/site.css is missing")
    website_icon = WEBSITE / "app-icon.png"
    product_icon = ROOT / "public/app-icon.png"
    if not website_icon.is_file():
        errors.append("website/app-icon.png is missing")
    elif not product_icon.is_file() or website_icon.read_bytes() != product_icon.read_bytes():
        errors.append("The website and product icons differ")

    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(f"Website validation passed ({len(REQUIRED_PAGES)} pages).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
