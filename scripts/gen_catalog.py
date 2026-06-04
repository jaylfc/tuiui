#!/usr/bin/env python3
"""Generate assets/catalog.json from the awesome-tuis README.

Usage:
    curl -fsSL https://raw.githubusercontent.com/rothgar/awesome-tuis/master/README.md -o /tmp/awesome-tuis.md
    python3 scripts/gen_catalog.py /tmp/awesome-tuis.md

The catalog is the seed of the Tuiui store: every app's name, category,
description and homepage, plus a best-effort binary-name guess used to detect
already-installed apps on $PATH.
"""
import json
import re
import sys
from pathlib import Path

# "Libraries" are TUI frameworks, not runnable apps — excluded from the store.
EXCLUDE = {"Libraries"}

# Binary names that differ from the project name (improves detection).
BIN_OVERRIDES = {
    "bottom": "btm",
    "btop++": "btop",
    "neovim": "nvim",
    "helix": "hx",
    "spotify-player": "spotify_player",
    "trippy": "trip",
    "superfile": "spf",
    "tig": "tig",
}


def guess_bin(name: str) -> str:
    if name in BIN_OVERRIDES:
        return BIN_OVERRIDES[name]
    b = name.lower().replace("++", "").strip()
    return re.sub(r"[^a-z0-9._+-]", "", b)


def main() -> int:
    src = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/awesome-tuis.md")
    out = Path(__file__).resolve().parent.parent / "assets" / "catalog.json"

    category = None
    seen = set()
    apps = []
    for line in src.read_text(encoding="utf-8").splitlines():
        h = re.search(r"<summary><h2>(.*?)</h2>", line)
        if h:
            category = re.sub("<[^>]*>", "", h.group(1)).strip()
            continue
        m = re.match(r"^- \[([^\]]+)\]\(([^)]+)\)\s*(.*)$", line)
        if not m or not category or category in EXCLUDE:
            continue
        name = m.group(1).strip()
        if name.lower() in seen:
            continue
        seen.add(name.lower())
        apps.append({
            "name": name,
            "bin": guess_bin(name),
            "category": category,
            "description": m.group(3).strip(),
            "homepage": m.group(2).strip(),
        })

    apps.sort(key=lambda a: (a["category"].lower(), a["name"].lower()))
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(apps, ensure_ascii=False, indent=1), encoding="utf-8")
    print(f"wrote {len(apps)} apps across {len({a['category'] for a in apps})} categories -> {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
