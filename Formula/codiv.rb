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

  # launchd service: runs codivd in foreground mode so launchd manages the
  # lifecycle (auto-start on login, restart on crash).
  # Usage:
  #   brew services start codiv   — start daemon now and on login
  #   brew services stop codiv    — stop daemon
  #   brew services restart codiv — restart daemon
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
