#!/usr/bin/env python3
# Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import argparse
import hashlib
import re
import textwrap
import tomllib
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "Cargo.toml"
DEFAULT_OUTPUT = ROOT / "Formula" / "ai-catalog.rb"

TAG_PATTERN = re.compile(r"^v(?P<version>[0-9A-Za-z.+-]+)$")
BINARY_NAME = "ai-catalog"
CLASS_NAME = "AiCatalog"
USER_AGENT = "ai-catalog-cli-release-automation"

# Homebrew on_<os>/on_<cpu> block to release archive name. The gnu Linux
# archives are used rather than musl: Homebrew targets glibc distributions.
ARCHIVES = {
    "macos": {"arm": "darwin-arm64", "intel": "darwin-amd64"},
    "linux": {"arm": "linux-arm64-gnu", "intel": "linux-amd64-gnu"},
}

HEADER = textwrap.dedent(
    """\
    # Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
    # Copyright AGNTCY Contributors (https://github.com/agntcy)
    # SPDX-License-Identifier: Apache-2.0

    """
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Render the Homebrew formula for a released CLI tag."
    )
    parser.add_argument("--tag", required=True, help="Release tag, e.g. v0.2.2")
    parser.add_argument(
        "--output",
        type=Path,
        default=DEFAULT_OUTPUT,
        help=f"Formula output path (default: {DEFAULT_OUTPUT}).",
    )
    return parser.parse_args()


def fetch(url: str) -> urllib.request.addinfourl:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    return urllib.request.urlopen(request, timeout=60)


def hash_archive(url: str) -> str:
    digest = hashlib.sha256()
    with fetch(url) as response:
        while chunk := response.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def archive_sha256(base_url: str, archive: str) -> str:
    """Prefer the digest the release published over re-hashing the archive."""
    try:
        with fetch(f"{base_url}/{BINARY_NAME}-{archive}.sha256") as response:
            fields = response.read().decode("utf-8").split()
        if fields:
            return fields[0]
    except OSError:
        pass
    return hash_archive(f"{base_url}/{BINARY_NAME}-{archive}.tar.gz")


def ruby_string(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


def arch_blocks(base_url: str, targets: dict[str, str]) -> str:
    blocks = []
    for cpu, archive in targets.items():
        url = f"{base_url}/{BINARY_NAME}-{archive}.tar.gz"
        blocks.append(
            f"    on_{cpu} do\n"
            f'      url "{ruby_string(url)}"\n'
            f'      sha256 "{archive_sha256(base_url, archive)}"\n'
            f"    end"
        )
    return "\n\n".join(blocks)


def render_formula(tag: str) -> str:
    match = TAG_PATTERN.match(tag)
    if not match:
        raise SystemExit(f"expected {TAG_PATTERN.pattern} tag, got: {tag}")
    version = match.group("version")

    with MANIFEST.open("rb") as handle:
        package = tomllib.load(handle)["package"]

    homepage = package["repository"].rstrip("/")
    base_url = f"{homepage}/releases/download/{tag}"
    git_url = homepage if homepage.endswith(".git") else f"{homepage}.git"

    body = textwrap.dedent(
        f"""\
        class {CLASS_NAME} < Formula
          desc "{ruby_string(package["description"])}"
          homepage "{ruby_string(homepage)}"
          version "{version}"
          license "{ruby_string(package["license"])}"
          head "{ruby_string(git_url)}", branch: "main"

        __MACOS__

        __LINUX__

          def install
            bin.install "{BINARY_NAME}"
          end

          test do
            assert_match "{BINARY_NAME}", shell_output("#{{bin}}/{BINARY_NAME} --help")
          end
        end
        """
    )

    for placeholder, os_name in (("__MACOS__", "macos"), ("__LINUX__", "linux")):
        blocks = arch_blocks(base_url, ARCHIVES[os_name])
        body = body.replace(placeholder, f"  on_{os_name} do\n{blocks}\n  end")

    return f"{HEADER}{body}"


def main() -> int:
    args = parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(render_formula(args.tag), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
