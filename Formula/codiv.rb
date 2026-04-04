class Codiv < Formula
  desc "Terminal-native AI coding assistant built in Rust"
  homepage "https://codiv.ai"
  version "v0.1.7"
  license "GPL-3.0-only"

  on_macos do
    on_arm do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.7/codiv-v0.1.7-aarch64-apple-darwin.tar.gz"
      sha256 "7c06dcb5549bc195f33ac1e12bfc1076ad758db2c2265a975c281fa33215dbd6"
    end

    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.7/codiv-v0.1.7-x86_64-apple-darwin.tar.gz"
      sha256 "59f1b639348700f444a1e6835b47d7e50373a19d9ae25883351e3efc7c858b23"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/razorback16/codiv/releases/download/v0.1.7/codiv-v0.1.7-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "a8a5e7c4530b0c6e7456072d12eca0ef0960ee58bcd46cc77bd4a6c0f0119161"
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
