# Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

default:
	@just --list

set windows-shell := ["powershell", "-NoLogo", "-NoProfile", "-Command"]

# renovate: datasource=github-releases depName=renovatebot/renovate versioning=semver
RENOVATE_VERSION := "44.126.1"

BIN_DIR := justfile_directory() / ".bin"
# The version is part of the path, so a bump installs fresh instead of
# reusing whatever is already there.
RENOVATE_HOME := BIN_DIR / ("renovate-" + RENOVATE_VERSION)
RENOVATE_BIN := RENOVATE_HOME / "node_modules" / ".bin" / "renovate"

demo_oci_layout_command := if os_family() == "windows" {
	"powershell -NoLogo -NoProfile -ExecutionPolicy Bypass -File ./demo/oci-layout-walkthrough.ps1"
} else {
	"./demo/oci-layout-walkthrough.sh"
}

demo_consumer_command := if os_family() == "windows" {
	"powershell -NoLogo -NoProfile -ExecutionPolicy Bypass -File ./demo/consumer-walkthrough.ps1"
} else {
	"./demo/consumer-walkthrough.sh"
}

build:
	cargo build

lint:
	cargo fmt --check
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo test

demo-oci-layout:
	{{demo_oci_layout_command}}

demo-consumer:
	{{demo_consumer_command}}

# POSIX only; there is no PowerShell port of this walkthrough yet.
demo-trust:
	./demo/trust-walkthrough.sh

# Sync dependencies with Renovate (local and analytical unless RENOVATE_PLATFORM is set)
[unix]
renovate-sync *OPTS: _renovate
	"{{ RENOVATE_BIN }}" --platform "${RENOVATE_PLATFORM:-local}" {{ OPTS }}

[unix]
_renovate:
	[ -x "{{ RENOVATE_BIN }}" ] || npm install --prefix "{{ RENOVATE_HOME }}" --no-audit --no-fund --loglevel=error renovate@{{ RENOVATE_VERSION }}

coverage:
	toolchain="$(awk -F'"' '/^channel = / {print $2}' rust-toolchain.toml)"; \
	host="$(rustc -vV | sed -n 's/^host: //p')"; \
	toolchain_root="$(dirname "$(dirname "$(rustup which rustc --toolchain "$toolchain")")")"; \
	LLVM_COV="$toolchain_root/lib/rustlib/$host/bin/llvm-cov" \
	LLVM_PROFDATA="$toolchain_root/lib/rustlib/$host/bin/llvm-profdata" \
	cargo llvm-cov --summary-only
