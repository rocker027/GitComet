#!/usr/bin/env python3
"""Collect the checked-in font and vendored component notices for packages."""

import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "BUNDLED-LICENSE-NOTICES.txt"
FONT_LICENSES = (
    "crates/gitcomet-fonts/assets/fira_code/LICENSE-OFL.txt",
    "crates/gitcomet-fonts/assets/ibm_plex_sans/LICENSE-OFL.txt",
    "crates/gitcomet-fonts/assets/lilex/LICENSE-OFL.txt",
    "crates/gitcomet-fonts/assets/noto_emoji/OFL.txt",
)


def build_notices():
    files = [ROOT / path for path in FONT_LICENSES]
    files += sorted((ROOT / "third-party-licenses/tree-sitter-wat").glob("*.txt"))
    files += sorted(
        path for path in (ROOT / "vendor").glob("*/**/*")
        if path.is_file() and path.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE"))
    )
    sections = [
        "Notices for checked-in fonts and vendored components. This file is not "
        "a complete inventory of Cargo dependencies or other bundled assets.\n"
        "Regenerate with scripts/generate-bundled-license-notices.py.\n"
    ]
    for path in files:
        sections.append(f"\n{'=' * 72}\n{path.relative_to(ROOT).as_posix()}\n{'=' * 72}\n")
        sections.append(path.read_text(encoding="utf-8").rstrip() + "\n")
    return "".join(sections)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    notices = build_notices()
    if args.check:
        if not OUTPUT.exists() or OUTPUT.read_text(encoding="utf-8") != notices:
            parser.exit(1, f"{OUTPUT.name} is stale; regenerate it\n")
        print(f"{OUTPUT.name} matches checked-in notices")
    else:
        OUTPUT.write_text(notices, encoding="utf-8")
        print(f"Wrote {OUTPUT.name}")


if __name__ == "__main__":
    main()
