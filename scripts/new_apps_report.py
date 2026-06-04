#!/usr/bin/env python3
"""Compare the freshly regenerated assets/catalog.json against the committed one
and write a Markdown report of apps that are new in awesome-tuis.

Usage: python3 scripts/new_apps_report.py <out.md>
Prints the number of new apps to stdout. Used by the catalog-check GitHub Action
to decide whether to open/update an alert issue.
"""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    committed = subprocess.run(
        ["git", "show", "HEAD:assets/catalog.json"],
        capture_output=True, text=True, cwd=ROOT,
    )
    old_names = {a["name"] for a in json.loads(committed.stdout)} if committed.returncode == 0 else set()

    new_catalog = json.loads((ROOT / "assets" / "catalog.json").read_text())
    new_apps = [a for a in new_catalog if a["name"] not in old_names]

    lines = [
        f"The scheduled check found **{len(new_apps)} new TUI(s)** in "
        "[awesome-tuis](https://github.com/rothgar/awesome-tuis) "
        "that are not yet in the tuiui catalog:\n",
    ]
    for a in new_apps:
        lines.append(f"- [{a['name']}]({a['homepage']}) — _{a['category']}_")
    lines.append(
        "\n---\nTo add them, run locally and commit:\n"
        "```\n"
        "curl -fsSL https://raw.githubusercontent.com/rothgar/awesome-tuis/master/README.md -o /tmp/awesome-tuis.md\n"
        "python3 scripts/gen_catalog.py /tmp/awesome-tuis.md\n"
        "python3 scripts/match_brew.py\n"
        "# then derive install recipes for the rest, run scripts/derive_os.py, and commit\n"
        "```"
    )

    Path(sys.argv[1]).write_text("\n".join(lines))
    print(len(new_apps))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
