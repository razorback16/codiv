mod agent;
mod daemon;
mod ipc;
mod session;
mod store;

use codiv_common::config;
use std::fs;
use std::process;
use tracing::{error, info};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--version") {
        println!("codivd {}", env!("CARGO_PKG_VERSION"));
        return;
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
