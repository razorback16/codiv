class Codiv < Formula
  desc "Terminal-native AI coding assistant built in Rust"
  homepage "https://codiv.ai"
  version "v0.1.6"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.6/codiv-v0.1.6-aarch64-apple-darwin.tar.gz"
      sha256 "262e017a03f62c349f63d207b513cad939cc25e96d07fd3de152672a2410d6c3"
    end

    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.6/codiv-v0.1.6-x86_64-apple-darwin.tar.gz"
      sha256 "a4b9678ddc1c6d6e06d1cad8a57f1a925fe46e268cf9b4dbf9e8de4fa2324c97"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.6/codiv-v0.1.6-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "bebffb17e819f44fcd890aae182582ecce367e12f1857e6a1b38c13f4a5b2ac6"
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
