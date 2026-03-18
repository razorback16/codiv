class Codiv < Formula
  desc "Terminal-native AI coding assistant built in Rust"
  homepage "https://codiv.ai"
  version "v0.1.4"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.4/codiv-v0.1.4-aarch64-apple-darwin.tar.gz"
      sha256 "844c08424c485d327a5c597d752715d2c9877ac06a78a644e38540eae678ed74"
    end

    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.4/codiv-v0.1.4-x86_64-apple-darwin.tar.gz"
      sha256 "2bca3a3a42f4ad15468912343bcc79582f55bb4857567de482f769044572ebd6"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.4/codiv-v0.1.4-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "728dabe958cafde343893b9cda675ba82a6958c94a037308731261fb391ba118"
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
