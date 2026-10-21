# Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

class AiCatalog < Formula
  desc "Command-line tool for inspecting, validating, and packaging AI Catalog documents"
  homepage "https://github.com/Agent-Card/ai-catalog-cli"
  version "0.3.0"
  license "Apache-2.0"
  head "https://github.com/Agent-Card/ai-catalog-cli.git", branch: "main"

  on_macos do
    on_arm do
      url "https://github.com/Agent-Card/ai-catalog-cli/releases/download/v0.3.0/ai-catalog-darwin-arm64.tar.gz"
      sha256 "743d04bac674bd7afd0d233ac597dda835f8756c7d50954925983f86ea7bd796"
    end

    on_intel do
      url "https://github.com/Agent-Card/ai-catalog-cli/releases/download/v0.3.0/ai-catalog-darwin-amd64.tar.gz"
      sha256 "4c77008ba658e3702849e400f5a93c6981295a31e235a9a82f6d67c59d0102ea"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Agent-Card/ai-catalog-cli/releases/download/v0.3.0/ai-catalog-linux-arm64-gnu.tar.gz"
      sha256 "1d3b8b75deba5d41d0ce9152da987c22b0d155b3c2c793ac339043ee5d58c67d"
    end

    on_intel do
      url "https://github.com/Agent-Card/ai-catalog-cli/releases/download/v0.3.0/ai-catalog-linux-amd64-gnu.tar.gz"
      sha256 "13a7a6d864ac54ee30447667deab4438cc009f8b6b968b6da452fe0b785ea3b1"
    end
  end

  def install
    bin.install "ai-catalog"
  end

  test do
    assert_match "ai-catalog", shell_output("#{bin}/ai-catalog --help")
  end
end
