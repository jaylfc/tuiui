#!/usr/bin/env python3
"""Fill in the `os` field for recipes that don't already have one (agent-derived
`platforms` are written by merge_recipes.py and are left untouched).

Safe by construction — it never excludes a platform without a hard signal:
  - brew recipes      -> ["macos", "linux"]  (Homebrew is cross-platform)
  - distro-only method -> ["linux"]           (apt/pacman/... only exist on Linux)
  - anything else      -> unset                (shown on every OS)
"""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LINUX_METHODS = {"apt", "pacman", "dnf", "yum", "apk", "snap", "yay", "yaourt"}


def main() -> int:
    recipes_path = ROOT / "assets" / "recipes.json"
    recipes = json.loads(recipes_path.read_text())

    filled = 0
    for r in recipes.values():
        if r.get("os"):
            continue  # keep agent-derived platforms
        method = r.get("method")
        if method in LINUX_METHODS:
            r["os"] = ["linux"]
            filled += 1
        elif method == "brew":
            r["os"] = ["macos", "linux"]
        else:
            r.pop("os", None)  # unset -> shown everywhere

    recipes_path.write_text(json.dumps(recipes, ensure_ascii=False, indent=1, sort_keys=True) + "\n")
    linux = sum(1 for r in recipes.values() if r.get("os") == ["linux"])
    print(f"linux-only (distro method): {linux}; brew apps: cross-platform")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
