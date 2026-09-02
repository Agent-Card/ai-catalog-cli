# Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

class AiCatalog < Formula
  desc "Command-line tool for inspecting, validating, and packaging AI Catalog documents"
  homepage "https://github.com/Agent-Card/ai-catalog-cli"
  version "0.2.2"
  license "Apache-2.0"
  head "https://github.com/Agent-Card/ai-catalog-cli.git", branch: "main"

  on_macos do
    on_arm do
      url "https://github.com/Agent-Card/ai-catalog-cli/releases/download/v0.2.2/ai-catalog-darwin-arm64.tar.gz"
      sha256 "4a3a095073ad9f835f363b2d246892cd6de34103cb45c6b2cfc1feb167776fbe"
    end

    on_intel do
      url "https://github.com/Agent-Card/ai-catalog-cli/releases/download/v0.2.2/ai-catalog-darwin-amd64.tar.gz"
      sha256 "65d3656d921ce52ca473223b82e0f02466ab8762ec108a7d10f19f4f02bd8704"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Agent-Card/ai-catalog-cli/releases/download/v0.2.2/ai-catalog-linux-arm64-gnu.tar.gz"
      sha256 "edadd1e13f39905e19b30faf844f12d276827a2d38418748337b70bc4da681f6"
    end

    on_intel do
      url "https://github.com/Agent-Card/ai-catalog-cli/releases/download/v0.2.2/ai-catalog-linux-amd64-gnu.tar.gz"
      sha256 "9945114a2e288f5ad422f6fef46d1da24b41158dd3e343bea9e104dd6d701a39"
    end
  end

  def install
    bin.install "ai-catalog"
  end

  test do
    assert_match "ai-catalog", shell_output("#{bin}/ai-catalog --help")
  end
end
