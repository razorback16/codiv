mod app;
mod shell;
mod ui;
mod ipc;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && (args[1] == "--version" || args[1] == "-V") {
        println!("slate {}", VERSION);
        return;
    }

    let shutdown = Arc::new(AtomicBool::new(false));

    // SIGTERM handler
    {
        let shutdown_clone = shutdown.clone();
        let _ = unsafe {
            nix::sys::signal::signal(
                nix::sys::signal::Signal::SIGTERM,
                nix::sys::signal::SigHandler::Handler(sigterm_handler),
            )
        };
        SHUTDOWN_FLAG.set(shutdown_clone).ok();
    }

    match app::run(shutdown) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("slate: {}", e);
            std::process::exit(1);
        }
    }
}

static SHUTDOWN_FLAG: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();

extern "C" fn sigterm_handler(_: libc::c_int) {
    if let Some(flag) = SHUTDOWN_FLAG.get() {
        flag.store(true, Ordering::SeqCst);
    }
}
