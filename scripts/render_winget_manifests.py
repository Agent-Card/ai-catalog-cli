#!/usr/bin/env python3
# Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import textwrap
import tomllib
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from urllib.error import HTTPError, URLError

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "Cargo.toml"
DEFAULT_OUTPUT_DIR = ROOT / "dist" / "winget"

MANIFEST_VERSION = "1.12.0"
PACKAGE_LOCALE = "en-US"
PACKAGE_IDENTIFIER = "AgentCard.AICatalog"
PACKAGE_NAME = "ai-catalog"
PUBLISHER = "Agent-Card"
PUBLISHER_URL = "https://github.com/Agent-Card"
AUTHOR = "AI-Catalog Contributors"
BINARY_NAME = "ai-catalog"
USER_AGENT = "ai-catalog-cli-release-automation"

DEPENDENCIES = ["Microsoft.VCRedist.2015+.x64"]

INSTALLERS = {"x64": "windows-amd64", "arm64": "windows-arm64"}

TAG_PATTERN = re.compile(r"^v(?P<version>[0-9A-Za-z.+-]+)$")
REPOSITORY_PATTERN = re.compile(
    r"^https://github\.com/(?P<owner>[^/]+)/(?P<repo>[^/]+?)(?:\.git)?/?$"
)


@dataclass(frozen=True)
class Installer:
    architecture: str
    url: str
    sha256: str


@dataclass(frozen=True)
class Release:
    tag: str
    version: str
    date: str
    url: str
    installers: list[Installer]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Render WinGet manifests for a released CLI tag."
    )
    parser.add_argument("--tag", required=True, help="Release tag, e.g. v0.2.2")
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=DEFAULT_OUTPUT_DIR,
        help=f"Output root for generated manifests (default: {DEFAULT_OUTPUT_DIR}).",
    )
    parser.add_argument(
        "--github-output",
        type=Path,
        help="Optional GitHub Actions output file to append manifest metadata to.",
    )
    return parser.parse_args()


def github_headers() -> dict[str, str]:
    headers = {"Accept": "application/vnd.github+json", "User-Agent": USER_AGENT}
    for name in ("GITHUB_TOKEN", "GH_TOKEN"):
        token = os.environ.get(name)
        if token:
            headers["Authorization"] = f"Bearer {token}"
            break
    return headers


def fetch(url: str) -> urllib.request.addinfourl:
    request = urllib.request.Request(url, headers=github_headers())
    try:
        return urllib.request.urlopen(request, timeout=60)
    except URLError as error:
        reason = f"HTTP {error.code}" if isinstance(error, HTTPError) else error.reason
        raise SystemExit(f"failed to fetch {url}: {reason}") from error


def fetch_json(url: str) -> dict:
    with fetch(url) as response:
        return json.loads(response.read())


def hash_asset(url: str) -> str:
    digest = hashlib.sha256()
    with fetch(url) as response:
        while chunk := response.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest().upper()


def repository_slug(repository_url: str) -> str:
    match = REPOSITORY_PATTERN.match(repository_url)
    if not match:
        raise SystemExit(f"expected GitHub repository URL, got: {repository_url}")
    return f"{match.group('owner')}/{match.group('repo')}"


def resolve_release(tag: str, slug: str) -> Release:
    match = TAG_PATTERN.match(tag)
    if not match:
        raise SystemExit(f"expected {TAG_PATTERN.pattern} tag, got: {tag}")

    release = fetch_json(f"https://api.github.com/repos/{slug}/releases/tags/{tag}")
    assets = {
        asset["name"]: asset["browser_download_url"]
        for asset in release.get("assets", [])
    }

    installers = []
    for architecture, archive in INSTALLERS.items():
        installer_url = assets.get(f"{BINARY_NAME}-{archive}.zip")
        if installer_url is None:
            raise SystemExit(
                f"release {tag} does not contain the expected Windows asset "
                f"{BINARY_NAME}-{archive}.zip"
            )
        installers.append(
            Installer(
                architecture=architecture,
                url=installer_url,
                sha256=asset_sha256(assets, archive, installer_url),
            )
        )

    date = release.get("published_at") or release.get("created_at")
    if date is None:
        raise SystemExit(f"release {tag} does not expose a publication date")

    return Release(
        tag=tag,
        version=match.group("version"),
        date=date[:10],
        url=release["html_url"],
        installers=installers,
    )


def asset_sha256(assets: dict[str, str], archive: str, installer_url: str) -> str:
    """Prefer the digest the release published over re-hashing the archive."""
    checksum_url = assets.get(f"{BINARY_NAME}-{archive}.sha256")
    if checksum_url is None:
        return hash_asset(installer_url)
    with fetch(checksum_url) as response:
        fields = response.read().decode("utf-8").split()
    if not fields:
        raise SystemExit(f"checksum asset for {archive} is empty")
    return fields[0].upper()


def render_version_manifest(release: Release) -> str:
    return textwrap.dedent(
        f"""\
        # yaml-language-server: $schema=https://aka.ms/winget-manifest.version.{MANIFEST_VERSION}.schema.json

        PackageIdentifier: {PACKAGE_IDENTIFIER}
        PackageVersion: {release.version}
        DefaultLocale: {PACKAGE_LOCALE}
        ManifestType: version
        ManifestVersion: {MANIFEST_VERSION}
        """
    )


def render_locale_manifest(
    release: Release,
    description: str,
    repository_url: str,
    license_id: str,
    tags: list[str],
) -> str:
    tag_block = "\n".join(["Tags:", *(f"- {tag}" for tag in tags)])
    template = textwrap.dedent(
        f"""\
        # yaml-language-server: $schema=https://aka.ms/winget-manifest.defaultLocale.{MANIFEST_VERSION}.schema.json

        PackageIdentifier: {PACKAGE_IDENTIFIER}
        PackageVersion: {release.version}
        PackageLocale: {PACKAGE_LOCALE}
        Publisher: {PUBLISHER}
        PublisherUrl: {PUBLISHER_URL}
        PublisherSupportUrl: {repository_url}/issues
        Author: {AUTHOR}
        PackageName: {PACKAGE_NAME}
        PackageUrl: {repository_url}
        License: {license_id}
        LicenseUrl: {repository_url}/blob/{release.tag}/LICENSE
        ShortDescription: {description}
        Moniker: {BINARY_NAME}
        __TAGS__
        Documentations:
        - DocumentLabel: Documentation
          DocumentUrl: {repository_url}#readme
        ReleaseNotesUrl: {repository_url}/releases/tag/{release.tag}
        ManifestType: defaultLocale
        ManifestVersion: {MANIFEST_VERSION}
        """
    )
    return template.replace("__TAGS__", tag_block)


def render_installer_manifest(release: Release) -> str:
    dependencies = "".join(
        f"  - PackageIdentifier: {name}\n" for name in DEPENDENCIES
    )
    installers = "".join(
        f"- Architecture: {installer.architecture}\n"
        f"  InstallerUrl: {installer.url}\n"
        f"  InstallerSha256: {installer.sha256}\n"
        f"  NestedInstallerFiles:\n"
        f"  - RelativeFilePath: {BINARY_NAME}.exe\n"
        f"    PortableCommandAlias: {BINARY_NAME}\n"
        for installer in release.installers
    )
    template = textwrap.dedent(
        f"""\
        # yaml-language-server: $schema=https://aka.ms/winget-manifest.installer.{MANIFEST_VERSION}.schema.json

        PackageIdentifier: {PACKAGE_IDENTIFIER}
        PackageVersion: {release.version}
        InstallerType: zip
        NestedInstallerType: portable
        Commands:
        - {BINARY_NAME}
        ReleaseDate: {release.date}
        Dependencies:
          PackageDependencies:
        __DEPENDENCIES__Installers:
        __INSTALLERS__ManifestType: installer
        ManifestVersion: {MANIFEST_VERSION}
        """
    )
    return template.replace("__DEPENDENCIES__", dependencies).replace(
        "__INSTALLERS__", installers
    )


def manifest_directory(output_dir: Path, version: str) -> Path:
    parts = PACKAGE_IDENTIFIER.split(".")
    return output_dir / "manifests" / parts[0][0].lower() / Path(*parts) / version


def main() -> int:
    args = parse_args()

    with MANIFEST.open("rb") as handle:
        package = tomllib.load(handle)["package"]

    repository_url = package["repository"].rstrip("/")
    release = resolve_release(args.tag, repository_slug(repository_url))

    output_dir = args.output_dir.resolve()
    manifests_dir = manifest_directory(output_dir, release.version)
    if manifests_dir.exists():
        shutil.rmtree(manifests_dir)
    manifests_dir.mkdir(parents=True, exist_ok=True)

    (manifests_dir / f"{PACKAGE_IDENTIFIER}.yaml").write_text(
        render_version_manifest(release), encoding="utf-8"
    )
    (manifests_dir / f"{PACKAGE_IDENTIFIER}.locale.{PACKAGE_LOCALE}.yaml").write_text(
        render_locale_manifest(
            release,
            description=package["description"],
            repository_url=repository_url,
            license_id=package["license"],
            tags=package["keywords"],
        ),
        encoding="utf-8",
    )
    (manifests_dir / f"{PACKAGE_IDENTIFIER}.installer.yaml").write_text(
        render_installer_manifest(release), encoding="utf-8"
    )

    outputs = {
        "manifest_dir": str(manifests_dir),
        "manifest_rel_dir": manifests_dir.relative_to(output_dir).as_posix(),
        "package_identifier": PACKAGE_IDENTIFIER,
        "package_version": release.version,
        "release_url": release.url,
    }
    if args.github_output is not None:
        with args.github_output.open("a", encoding="utf-8") as handle:
            for key, value in outputs.items():
                handle.write(f"{key}={value}\n")

    for key, value in outputs.items():
        print(f"{key}={value}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
