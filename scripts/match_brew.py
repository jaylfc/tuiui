#!/usr/bin/env python3
"""Mark brew-installable catalog apps as verified in assets/recipes.json.

Fetches the Homebrew formula index and matches it against the catalog by binary
name / app name, writing a verified `brew install <formula>` recipe for each hit.
Existing verified recipes are left untouched. Run after scripts/gen_catalog.py.
"""
import json
import re
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FORMULA_API = "https://formulae.brew.sh/api/formula.json"


def main() -> int:
    with urllib.request.urlopen(FORMULA_API) as resp:
        formulae = json.load(resp)
    names = {}
    for f in formulae:
        for n in [f["name"], *f.get("aliases", [])]:
            names[n.lower()] = f["name"]

    catalog = json.loads((ROOT / "assets" / "catalog.json").read_text())
    recipes_path = ROOT / "assets" / "recipes.json"
    recipes = json.loads(recipes_path.read_text()) if recipes_path.exists() else {}

    added = 0
    for app in catalog:
        if recipes.get(app["name"], {}).get("verified"):
            continue
        cands = [app["bin"].lower(), app["name"].lower(), app["name"].lower().replace("++", "")]
        hit = next((names[c] for c in cands if c in names), None)
        if hit:
            recipes[app["name"]] = {"install": f"brew install {hit}", "method": "brew", "verified": True}
            added += 1

    recipes_path.write_text(json.dumps(recipes, ensure_ascii=False, indent=1, sort_keys=True) + "\n")
    verified = sum(1 for r in recipes.values() if r.get("verified"))
    print(f"brew-verified added: {added}; total verified: {verified}/{len(catalog)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
