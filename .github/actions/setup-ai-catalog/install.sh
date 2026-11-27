#!/usr/bin/env bash
# Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

# Download an ai-catalog release, verify its SHA-256, add it to PATH, and expose
# the binary path and release tag as step outputs.
#
# Env: VERSION (e.g. 0.4.0, v0.4.0, or "latest"), RUNNER_OS, RUNNER_ARCH, DEST,
# GITHUB_OUTPUT and GITHUB_PATH (both optional, for local runs).
set -euo pipefail

repo="Agent-Card/ai-catalog-cli"
version="${VERSION:-latest}"

if [ "$version" = "latest" ]; then
  url="$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")"
  tag="${url##*/}"
else
  tag="v${version#v}"
fi

case "${RUNNER_OS:-$(uname -s)}/${RUNNER_ARCH:-$(uname -m)}" in
  Linux/X64|Linux/x86_64) name="ai-catalog-linux-amd64-gnu" ;;
  Linux/ARM64|Linux/aarch64) name="ai-catalog-linux-arm64-gnu" ;;
  macOS/X64|Darwin/x86_64) name="ai-catalog-darwin-amd64" ;;
  macOS/ARM64|Darwin/arm64) name="ai-catalog-darwin-arm64" ;;
  *) echo "::error::Unsupported runner: ${RUNNER_OS:-?}/${RUNNER_ARCH:-?} (supported: Linux and macOS, x64 and arm64)" >&2; exit 1 ;;
esac

dest="${DEST:?DEST is required}"
mkdir -p "$dest"
base="https://github.com/$repo/releases/download/$tag"
curl -fsSL --retry 3 --max-time 120 -o "$dest/$name.tar.gz" "$base/$name.tar.gz"
curl -fsSL --retry 3 --max-time 120 -o "$dest/$name.sha256" "$base/$name.sha256"
(cd "$dest" && shasum -a 256 -c "$name.sha256" >&2)
tar -xzf "$dest/$name.tar.gz" -C "$dest"
chmod +x "$dest/ai-catalog"
echo "Installed ai-catalog $tag" >&2

if [ -n "${GITHUB_PATH:-}" ]; then
  echo "$dest" >> "$GITHUB_PATH"
fi
{
  echo "ai_catalog_path=$dest/ai-catalog"
  echo "version=$tag"
} >> "${GITHUB_OUTPUT:-/dev/null}"
