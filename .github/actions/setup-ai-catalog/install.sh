#!/usr/bin/env bash
# Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail

repo="Agent-Card/ai-catalog-cli"
version="${VERSION:-latest}"

if [ "$version" = "latest" ]; then
  tag="$(gh release view --repo "$repo" --json tagName --jq .tagName)"
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
gh release download "$tag" --repo "$repo" --dir "$dest" --clobber \
  --pattern "$name.tar.gz" --pattern "$name.sha256"
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
