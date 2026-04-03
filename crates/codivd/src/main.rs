mod agent;
mod daemon;
pub mod daemon_shell;
mod handlers;
mod ipc;
mod session;
mod store;

use codiv_common::config;
use nix::libc;
use std::fs;
use std::process;
use tracing::{error, info};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let subcmd = args.get(1).map(|s| s.as_str());

    match subcmd {
        Some("--version" | "-V") => {
            println!("codivd {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some("stop") => {
            cmd_stop();
            return;
        }
        Some("start") => {
            cmd_start();
            return;
        }
        Some("status") => {
            cmd_status();
            return;
        }
        Some("--help" | "-h") => {
            print_usage();
            return;
        }
        _ => {}
    }

    let foreground = args.iter().any(|a| a == "--foreground" || a == "-f");

    // Fork BEFORE creating the tokio runtime.  After fork() the child
    // inherits only the calling thread — tokio worker threads are lost,
    // which breaks the runtime if it was already started.
    if !foreground {
        daemonize();
    }

    // Set up logging (after fork so the child writes to the log)
    let log_path = config::log_file_path();
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).ok();
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "codivd=info".into()),
        )
        .with_writer(move || {
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .unwrap_or_else(|_| fs::File::create("/dev/null").unwrap())
        })
        .init();

    // Now build the tokio runtime and run the async entry point.
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime")
        .block_on(async_main());
}

fn print_usage() {
    eprintln!("codivd {} — Codiv AI daemon", env!("CARGO_PKG_VERSION"));
    eprintln!();
    eprintln!("Usage: codivd [command]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  start        Start the daemon (daemonizes by default)");
    eprintln!("  stop         Stop the running daemon");
    eprintln!("  status       Show daemon status");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --foreground, -f   Run in foreground (don't daemonize)");
    eprintln!("  --version, -V      Print version");
    eprintln!("  --help, -h         Print this help");
}

/// Read PID from the PID file. Returns None if file missing or unparseable.
fn read_pid() -> Option<i32> {
    let contents = fs::read_to_string(config::pid_file_path()).ok()?;
    contents.trim().parse().ok()
}

/// Check if a process is alive.
fn is_pid_alive(pid: i32) -> bool {
    pid > 0 && unsafe { libc::kill(pid, 0) } == 0
}

/// Launchd labels: install.sh uses ai.codiv.daemon, Homebrew uses homebrew.mxcl.codiv
const PLIST_LABELS: &[&str] = &["ai.codiv.daemon", "homebrew.mxcl.codiv"];
const SYSTEMD_UNIT: &str = "codivd.service";

/// Find which launchd label is currently loaded, if any.
fn active_launchd_label() -> Option<&'static str> {
    let uid = unsafe { libc::getuid() };
    for label in PLIST_LABELS {
        let ok = std::process::Command::new("launchctl")
            .args(["print", &format!("gui/{}/{}", uid, label)])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Some(label);
        }
    }
    None
}

/// Check if a launchd service is loaded.
fn is_launchd_managed() -> bool {
    active_launchd_label().is_some()
}

/// Check if a systemd user service is active.
fn is_systemd_managed() -> bool {
    std::process::Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", SYSTEMD_UNIT])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Stop the launchd service (bootout unloads it so KeepAlive won't relaunch).
fn stop_launchd() {
    let label = match active_launchd_label() {
        Some(l) => l,
        None => {
            eprintln!("no active launchd service found");
            return;
        }
    };
    let uid = unsafe { libc::getuid() };
    let status = std::process::Command::new("launchctl")
        .args(["bootout", &format!("gui/{}/{}", uid, label)])
        .status();
    match status {
        Ok(s) if s.success() => println!("codivd service stopped (launchd: {})", label),
        _ => eprintln!("failed to stop launchd service ({})", label),
    }
}

/// Find the plist file on disk for bootstrapping.
fn find_launchd_plist() -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    for label in PLIST_LABELS {
        let path = format!("{}/Library/LaunchAgents/{}.plist", home, label);
        if std::path::Path::new(&path).exists() {
            return Some(path);
        }
    }
    None
}

/// Start the launchd service (bootstrap loads it and KeepAlive starts it).
fn start_launchd() {
    let plist = match find_launchd_plist() {
        Some(p) => p,
        None => {
            eprintln!("no launchd plist found in ~/Library/LaunchAgents/");
            eprintln!("Run the installer or `brew install codiv` first.");
            process::exit(1);
        }
    };
    let uid = unsafe { libc::getuid() };
    let status = std::process::Command::new("launchctl")
        .args(["bootstrap", &format!("gui/{}", uid), &plist])
        .status();
    match status {
        Ok(s) if s.success() => println!("codivd service started (launchd: {})", plist),
        _ => eprintln!("failed to start launchd service ({})", plist),
    }
}

fn stop_systemd() {
    let status = std::process::Command::new("systemctl")
        .args(["--user", "stop", SYSTEMD_UNIT])
        .status();
    match status {
        Ok(s) if s.success() => println!("codivd service stopped (systemd)"),
        _ => eprintln!("failed to stop systemd service"),
    }
}

fn start_systemd() {
    let status = std::process::Command::new("systemctl")
        .args(["--user", "start", SYSTEMD_UNIT])
        .status();
    match status {
        Ok(s) if s.success() => println!("codivd service started (systemd)"),
        _ => eprintln!("failed to start systemd service"),
    }
}

fn cmd_status() {
    let managed = if is_launchd_managed() {
        Some("launchd")
    } else if is_systemd_managed() {
        Some("systemd")
    } else {
        None
    };

    match read_pid() {
        Some(pid) if is_pid_alive(pid) => {
            let sock = config::socket_path();
            let socket_ok = std::path::Path::new(&sock).exists();
            println!("codivd is running (pid {})", pid);
            if let Some(mgr) = managed {
                println!("  Managed by: {}", mgr);
            }
            println!("  PID file: {}", config::pid_file_path().display());
            println!("  Socket:   {} {}", sock, if socket_ok { "(ok)" } else { "(missing)" });
            println!("  Log:      {}", config::log_file_path().display());
        }
        Some(pid) => {
            println!("codivd is not running (stale pid file references pid {})", pid);
        }
        None => {
            println!("codivd is not running");
        }
    }
}

fn cmd_stop() {
    // If managed by a service manager, use that to stop (prevents relaunch)
    if is_launchd_managed() {
        stop_launchd();
        return;
    }
    if is_systemd_managed() {
        stop_systemd();
        return;
    }

    // Otherwise kill the process directly
    match read_pid() {
        Some(pid) if is_pid_alive(pid) => {
            unsafe { libc::kill(pid, libc::SIGTERM); }
            for _ in 0..30 {
                if !is_pid_alive(pid) {
                    println!("codivd stopped (pid {})", pid);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            eprintln!("codivd (pid {}) did not stop within 3s, sending SIGKILL", pid);
            unsafe { libc::kill(pid, libc::SIGKILL); }
            println!("codivd killed (pid {})", pid);
        }
        _ => {
            println!("codivd is not running");
        }
    }
}

fn cmd_start() {
    // Check if already running
    if let Some(pid) = read_pid() {
        if is_pid_alive(pid) {
            println!("codivd is already running (pid {})", pid);
            return;
        }
    }

    // If a service plist/unit exists, use the service manager
    if find_launchd_plist().is_some() {
        start_launchd();
        return;
    }

    let unit = format!(
        "{}/.config/systemd/user/{}",
        std::env::var("HOME").unwrap_or_default(),
        SYSTEMD_UNIT
    );
    if std::path::Path::new(&unit).exists() {
        start_systemd();
        return;
    }

    // Fallback: self-daemonize
    let exe = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("could not determine executable path: {}", e);
        process::exit(1);
    });

    match std::process::Command::new(&exe).spawn() {
        Ok(_) => {
            for _ in 0..30 {
                if let Some(pid) = read_pid() {
                    if is_pid_alive(pid) {
                        println!("codivd started (pid {})", pid);
                        return;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            println!("codivd started");
        }
        Err(e) => {
            eprintln!("failed to start codivd: {}", e);
            process::exit(1);
        }
    }
}

async fn async_main() {
    // Check for existing daemon
    let pid_path = config::pid_file_path();
    if let Ok(contents) = fs::read_to_string(&pid_path) {
        if let Ok(pid) = contents.trim().parse::<i32>() {
            if pid > 0 && nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok() {
                error!("daemon already running (pid {})", pid);
                process::exit(1);
            }
        }
        fs::remove_file(&pid_path).ok();
    }

    // Write PID file
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&pid_path, format!("{}", process::id())).expect("cannot write PID file");

    info!("codivd started (pid {})", process::id());

    // Run daemon
    match daemon::Daemon::new().await {
        Ok(daemon) => {
            daemon.run().await;
        }
        Err(e) => {
            error!("failed to start daemon: {}", e);
            process::exit(1);
        }
    }

    // Cleanup
    fs::remove_file(&pid_path).ok();
    info!("codivd shutdown complete");
}

fn daemonize() {
    use nix::unistd::{fork, setsid, ForkResult};

    match unsafe { fork() } {
        Ok(ForkResult::Parent { .. }) => process::exit(0),
        Ok(ForkResult::Child) => {
            setsid().expect("setsid failed");
        }
        Err(e) => {
            eprintln!("fork failed: {}", e);
            process::exit(1);
        }
    }
}
