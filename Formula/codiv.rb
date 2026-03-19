class Codiv < Formula
  desc "Terminal-native AI coding assistant built in Rust"
  homepage "https://codiv.ai"
  version "v0.1.5"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.5/codiv-v0.1.5-aarch64-apple-darwin.tar.gz"
      sha256 "7f8bf966f39ee01be64a8f0287d135e2d5a03b32a3de3abb09d12c48f9309bb3"
    end

    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.5/codiv-v0.1.5-x86_64-apple-darwin.tar.gz"
      sha256 "9dae568d8e8049e90594b3e6467c2b86536dcc6467f21afd5546778acf889658"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.5/codiv-v0.1.5-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "1b00b0373ac8375f77f00b1f13baa9fe59faf8ab1709398f3a029691515c2e43"
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
