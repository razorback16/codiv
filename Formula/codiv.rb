class Codiv < Formula
  desc "Terminal-native AI coding assistant built in Rust"
  homepage "https://codiv.ai"
  version "v0.1.1"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.1/codiv-v0.1.1-aarch64-apple-darwin.tar.gz"
      sha256 "66d60f5c2a01777c5681314d4ae118c5a6fe0796481b8d9c57df6caf94d1d5d4"
    end

    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.1/codiv-v0.1.1-x86_64-apple-darwin.tar.gz"
      sha256 "cc3f8c978d4753b27934fcbd19b8918f575dcd1acfc5e7038627eb19207a1f0d"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.1/codiv-v0.1.1-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "634361d49cadcd6c2e4ec117866c1ddc44e46d077df436922680044b3f635a0c"
    end
  end

  def install
    bin.install "codiv"
    bin.install "codivd"
  end

  service do
    run [opt_bin/"codivd", "--foreground"]
    keep_alive true
    working_dir var
    log_path var/"log/codivd.log"
    error_log_path var/"log/codivd.log"
    environment_variables HOME: Dir.home
  end

  def post_install
    (var/"log").mkpath
  end

  def caveats
    <<~EOS
      To start codivd as a background daemon managed by launchd:
        brew services start codiv

      The codiv TUI will also auto-launch codivd if it's not running,
      but using brew services gives you auto-restart on crash and
      automatic startup on login.

      Logs: #{var}/log/codivd.log
      Config: ~/.codiv/config.toml
    EOS
  end

  test do
    assert_match "codiv", shell_output("#{bin}/codiv --help")
  end
end
