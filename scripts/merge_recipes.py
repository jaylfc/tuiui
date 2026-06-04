#!/usr/bin/env python3
"""Merge a derive-install-recipes workflow output file into assets/recipes.json.

Usage: python3 scripts/merge_recipes.py <workflow-output.json>

Records every processed app (so it isn't re-checked), but never downgrades an
existing verified recipe.
"""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    raw = json.loads(Path(sys.argv[1]).read_text())
    results = raw.get("result", raw) if isinstance(raw, dict) else raw

    recipes_path = ROOT / "assets" / "recipes.json"
    recipes = json.loads(recipes_path.read_text())

    merged = 0
    for r in results:
        name = r.get("name")
        if not name:
            continue
        existing = recipes.get(name)
        if existing and existing.get("verified") and not r.get("verified"):
            continue  # keep the better existing recipe
        recipes[name] = {
            "install": r.get("install", ""),
            "method": r.get("method", "unknown"),
            "verified": bool(r.get("verified")),
        }
        merged += 1

    recipes_path.write_text(json.dumps(recipes, ensure_ascii=False, indent=1, sort_keys=True) + "\n")
    catalog = json.loads((ROOT / "assets" / "catalog.json").read_text())
    verified = sum(1 for x in recipes.values() if x.get("verified"))
    print(f"merged {merged}; total verified: {verified}/{len(catalog)}; recorded: {len(recipes)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
