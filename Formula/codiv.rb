class Codiv < Formula
  desc "Terminal-native AI coding assistant built in Rust"
  homepage "https://codiv.ai"
  version "v0.1.2"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.2/codiv-v0.1.2-aarch64-apple-darwin.tar.gz"
      sha256 "cb1f42d48514341f3a842a29431edeab9d19fee5b05546c63a76015b4bfd78c5"
    end

    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.2/codiv-v0.1.2-x86_64-apple-darwin.tar.gz"
      sha256 "aea328ccc84c359b9685e62407814b3292206a739c824f5d7215b5b901c67ea1"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.2/codiv-v0.1.2-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "beb755f836b43a8aacf7fe4693778db2bf83fd5e7600f70df306d37123c236db"
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
