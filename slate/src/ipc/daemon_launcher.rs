use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

/// Get the Unix socket path for the daemon.
pub fn socket_path() -> String {
    let uid = unsafe { libc::getuid() };
    format!("/tmp/slated-{}.sock", uid)
}

/// Get the PID file path.
pub fn pid_file_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".slate-agent").join("slated.pid")
}

/// Check if the daemon is currently running.
pub fn is_daemon_running() -> bool {
    let pid_path = pid_file_path();
    if let Ok(contents) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = contents.trim().parse::<i32>() {
            // kill with signal 0 checks if process exists.
            return unsafe { libc::kill(pid, 0) } == 0;
        }
    }
    false
}

/// Find the slated binary.
pub fn find_slated_binary() -> Option<String> {
    // 1. Check next to the current executable.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("slated");
            if candidate.exists() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }

    // 2. Check relative build path (development layout).
    let dev_path = "../slated/build/slated";
    if std::path::Path::new(dev_path).exists() {
        return Some(dev_path.to_string());
    }

    // 3. Check PATH.
    if Command::new("slated").arg("--version").output().is_ok() {
        return Some("slated".to_string());
    }

    None
}

/// Ensure the daemon is running. If not, launch it and wait for the socket.
/// Returns true if the daemon is available.
pub fn ensure_daemon_running(timeout: Duration) -> bool {
    // Already running?
    let sock = socket_path();
    if std::path::Path::new(&sock).exists() && is_daemon_running() {
        log::debug!("daemon already running (socket={})", sock);
        return true;
    }

    // Find and launch the daemon.
    let binary = match find_slated_binary() {
        Some(b) => b,
        None => {
            log::warn!("slated binary not found");
            eprintln!("slate: slated binary not found");
            return false;
        }
    };

    log::info!("launching daemon: {}", binary);
    // Launch as background daemon (no --foreground flag).
    match Command::new(&binary).spawn() {
        Ok(_) => {}
        Err(e) => {
            log::error!("failed to launch slated: {}", e);
            eprintln!("slate: failed to launch slated: {}", e);
            return false;
        }
    }

    // Poll for socket availability.
    let start = Instant::now();
    while start.elapsed() < timeout {
        if std::path::Path::new(&sock).exists() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }

    log::warn!("timed out waiting for slated socket");
    eprintln!("slate: timed out waiting for slated socket");
    false
}
