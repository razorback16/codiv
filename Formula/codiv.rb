class Codiv < Formula
  desc "Terminal-native AI coding assistant built in Rust"
  homepage "https://codiv.ai"
  version "v0.1.0"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.0/codiv-v0.1.0-aarch64-apple-darwin.tar.gz"
      sha256 "16945792132b7a20d85f7385ab5634d60f5b995498f68d6e862ebeca3d6bdce8"
    end

    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.0/codiv-v0.1.0-x86_64-apple-darwin.tar.gz"
      sha256 "b8dba63b7122d9dba3a4eca0fb38873ebcdbf44d7d415897b8f56201c5fd2f5b"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.0/codiv-v0.1.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "f2de581417c100e73bc2ee5bf86ef1e3f0a06da4947e6f04701abcefe73145f9"
    end
  end

  def install
    bin.install "codiv"
    bin.install "codivd"
  end

  test do
    assert_match "codiv", shell_output("#{bin}/codiv --help")
  end
end
