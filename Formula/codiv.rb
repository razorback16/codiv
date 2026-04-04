class Codiv < Formula
  desc "Terminal-native AI coding assistant built in Rust"
  homepage "https://codiv.ai"
  version "v0.1.7"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.7/codiv-v0.1.7-aarch64-apple-darwin.tar.gz"
      sha256 "14cb93a73ecee9473b044cc71cfa9718128ca16f2bf7c44e29f44618b12f6bc1"
    end

    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.7/codiv-v0.1.7-x86_64-apple-darwin.tar.gz"
      sha256 "09589533f795cf0dbebcc665df9ed02fb30ccf1536383fc57c93164ed475b128"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.7/codiv-v0.1.7-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "04937a9af2078c5f92765f03f5a33aa6f48001d4f5256d85e1fb37260b1123d7"
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
