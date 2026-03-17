class Codiv < Formula
  desc "Terminal-native AI coding assistant built in Rust"
  homepage "https://codiv.ai"
  version "v0.1.3"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.3/codiv-v0.1.3-aarch64-apple-darwin.tar.gz"
      sha256 "d592af3e0503dfaeeeb81106727235fef677664a4c0071421842249c01624c65"
    end

    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.3/codiv-v0.1.3-x86_64-apple-darwin.tar.gz"
      sha256 "a0dbb578267bee95cea2099eaaab89c30e4efcf40afadbb85a846b2d8500b26b"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.3/codiv-v0.1.3-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "9018bd9539478d0ec10a0431c59f5583dedb266e88509c6af6353ff6aa843a8b"
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
