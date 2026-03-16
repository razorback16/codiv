class Codiv < Formula
  desc "Terminal-native AI coding assistant built in Rust"
  homepage "https://codiv.ai"
  version "VERSION_PLACEHOLDER"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/razorback16/codiv/releases/download/VERSION_PLACEHOLDER/codiv-VERSION_PLACEHOLDER-aarch64-apple-darwin.tar.gz"
      sha256 "SHA256_ARM_PLACEHOLDER"
    end

    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/VERSION_PLACEHOLDER/codiv-VERSION_PLACEHOLDER-x86_64-apple-darwin.tar.gz"
      sha256 "SHA256_INTEL_PLACEHOLDER"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/VERSION_PLACEHOLDER/codiv-VERSION_PLACEHOLDER-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "SHA256_LINUX_PLACEHOLDER"
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
