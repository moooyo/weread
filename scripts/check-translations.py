#!/usr/bin/env python3
"""Check interface string references on the designated verification worker."""

from __future__ import annotations

import json
from pathlib import Path
import re


def unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate translation key: {key}")
        result[key] = value
    return result


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    labels = json.loads(
        (root / "assets/i18n/zh-CN.json").read_text(encoding="utf-8-sig"),
        object_pairs_hook=unique_object,
    )
    missing: set[tuple[str, str]] = set()
    for path in (root / "ui").rglob("*.slint"):
        for key in re.findall(r"\bStrings\.([a-z][a-z0-9-]*)", path.read_text(encoding="utf-8")):
            key = key.replace("-", "_")
            if key not in labels:
                missing.add((str(path.relative_to(root)), key))
    pattern = r'\b(?:tr|i18n::text|i18n::format)\(\s*"([a-z][a-z0-9_]*)"'
    for path in (root / "src").rglob("*.rs"):
        for key in re.findall(pattern, path.read_text(encoding="utf-8")):
            if key not in labels:
                missing.add((str(path.relative_to(root)), key))
    for path, key in sorted(missing):
        print(f"Missing interface translation: {path}: {key}")
    if not missing:
        print(f"Interface translations are complete: {len(labels)} keys")
    return int(bool(missing))


if __name__ == "__main__":
    raise SystemExit(main())
