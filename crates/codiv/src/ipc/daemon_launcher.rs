pub use codiv_common::config::socket_path;

/// Suggest how to start the daemon based on how codiv was installed.
pub fn daemon_start_hint() -> &'static str {
    let home = std::env::var("HOME").unwrap_or_default();

    // Check for Homebrew plist
    let brew_plist = format!("{}/Library/LaunchAgents/homebrew.mxcl.codiv.plist", home);
    if std::path::Path::new(&brew_plist).exists() {
        return "Run: brew services start codiv";
    }

    // Default: codivd start works for both install.sh and manual installs
    "Run: codivd start"
}
