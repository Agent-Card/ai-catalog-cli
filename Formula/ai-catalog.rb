# Copyright AI-Catalog Contributors (https://github.com/Agent-Card/ai-catalog-cli)
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

class AiCatalog < Formula
  desc "Command-line tool for inspecting, validating, and packaging AI Catalog documents"
  homepage "https://github.com/Agent-Card/ai-catalog-cli"
  version "0.4.0"
  license "Apache-2.0"
  head "https://github.com/Agent-Card/ai-catalog-cli.git", branch: "main"

  on_macos do
    on_arm do
      url "https://github.com/Agent-Card/ai-catalog-cli/releases/download/v0.4.0/ai-catalog-darwin-arm64.tar.gz"
      sha256 "0c875c1f676d67b164469bd17cacc531b795f72ace9a16dce66f2c863ff9867b"
    end

    on_intel do
      url "https://github.com/Agent-Card/ai-catalog-cli/releases/download/v0.4.0/ai-catalog-darwin-amd64.tar.gz"
      sha256 "8653be27703e0234aacba21066402a7276e7b2729dda98b8743dffb7969b710e"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Agent-Card/ai-catalog-cli/releases/download/v0.4.0/ai-catalog-linux-arm64-gnu.tar.gz"
      sha256 "69001cfe8b6b4cdaba76b347fa57d87348d94a6cbf5853f42b4d44c3be66743a"
    end

    on_intel do
      url "https://github.com/Agent-Card/ai-catalog-cli/releases/download/v0.4.0/ai-catalog-linux-amd64-gnu.tar.gz"
      sha256 "ffdf50ab9c98abea09db7634af2e1d5b0329a3684c0f76cee0ba01ef95df6bcd"
    end
  end

  def install
    bin.install "ai-catalog"
  end

  test do
    assert_match "ai-catalog", shell_output("#{bin}/ai-catalog --help")
  end
end
