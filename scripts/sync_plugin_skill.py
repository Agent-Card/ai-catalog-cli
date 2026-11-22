"""Generate the packaged skill from the canonical root SKILL.md."""

import argparse
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Fail if the copy is stale.")
    args = parser.parse_args()

    root = Path(__file__).resolve().parent.parent
    source = root / "SKILL.md"
    target = root / "plugins" / "ai-catalog-cli" / "skills" / "ai-catalog-cli" / "SKILL.md"
    content = source.read_text(encoding="utf-8")
    if args.check:
        if not target.is_file() or target.read_text(encoding="utf-8") != content:
            parser.exit(1, "Plugin skill is stale. Run: just sync-plugin-skill\n")
    else:
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main()
