# Homebrew formula for Slither.
#
# Named `slither-seo` rather than `slither` because homebrew-core already ships
# `slither-analyzer` — the Solidity static analyzer, whose executable is also
# called `slither` — and a bare `slither` formula would invite confusion with
# it. The installed command is still `slither`; only the formula name differs.
#
# To publish this:
#
#   1. Create a public repo named `homebrew-tap` under your GitHub account.
#      The `homebrew-` prefix is required; the tap is then `nmang004/tap`.
#   2. Copy this file to `Formula/slither-seo.rb` in that repo.
#   3. Set `version` to the release you are publishing, and fill in the three
#      sha256 values from that release's SHA256SUMS file.
#   4. Commit. Users install with:
#
#        brew install nmang004/tap/slither-seo
#
# Bumping a release means editing `version` and the three sha256 values. That is
# worth automating once releases are regular — a workflow in this repo can open
# a PR against the tap — but it needs a token with write access to the tap repo,
# so it is deliberately not wired up here.

class SlitherSeo < Formula
  desc "Local-first SEO audit toolkit made for Scorpion, with an MCP server for Claude"
  homepage "https://github.com/nmang004/Slither"
  version "0.3.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/nmang004/Slither/releases/download/v#{version}/slither-v#{version}-macos-aarch64"
      sha256 "REPLACE_WITH_SHA256_FOR_macos-aarch64"
    else
      url "https://github.com/nmang004/Slither/releases/download/v#{version}/slither-v#{version}-macos-x86_64"
      sha256 "REPLACE_WITH_SHA256_FOR_macos-x86_64"
    end
  end

  on_linux do
    # The Linux artifact is statically linked against musl, so it runs on any
    # distribution regardless of glibc version.
    url "https://github.com/nmang004/Slither/releases/download/v#{version}/slither-v#{version}-linux-x86_64"
    sha256 "REPLACE_WITH_SHA256_FOR_linux-x86_64"
  end

  def install
    # The release publishes bare executables rather than tarballs, so the staged
    # file arrives under its download name.
    bin.install Dir["slither-v#{version}-*"].first => "slither"
  end

  test do
    # `slither version` prints the CLI version and its built-in components.
    assert_match "slither", shell_output("#{bin}/slither version").downcase
  end
end
