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

    if let Some(level) = parse_debug_flag(&args) {
        init_logging(level);
        log::info!("slate {} starting (args: {:?})", VERSION, args);
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
            log::error!("fatal: {}", e);
            eprintln!("slate: {}", e);
            std::process::exit(1);
        }
    }
}

/// Parse `--debug` or `--debug=<level>` from args.
/// Returns `Some(level)` if present, `None` otherwise.
/// Supported levels: error, warn, info, debug, trace.
/// `--debug` without a value defaults to `debug`.
fn parse_debug_flag(args: &[String]) -> Option<log::LevelFilter> {
    for arg in args {
        if arg == "--debug" {
            return Some(log::LevelFilter::Debug);
        }
        if let Some(level_str) = arg.strip_prefix("--debug=") {
            return Some(match level_str.to_lowercase().as_str() {
                "error" => log::LevelFilter::Error,
                "warn" => log::LevelFilter::Warn,
                "info" => log::LevelFilter::Info,
                "debug" => log::LevelFilter::Debug,
                "trace" => log::LevelFilter::Trace,
                _ => {
                    eprintln!("slate: unknown log level '{}', using 'debug'", level_str);
                    log::LevelFilter::Debug
                }
            });
        }
    }
    None
}

/// Initialize logging to `/tmp/slate-debug.log` at the given level.
fn init_logging(level: log::LevelFilter) {
    use std::fs::OpenOptions;
    use std::io::Write;

    let target = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/slate-debug.log")
        .expect("failed to open /tmp/slate-debug.log");

    env_logger::Builder::new()
        .filter_level(level)
        .format(|buf, record| {
            writeln!(
                buf,
                "[{} {:>5} {}] {}",
                buf.timestamp_millis(),
                record.level(),
                record.target(),
                record.args()
            )
        })
        .target(env_logger::Target::Pipe(Box::new(target)))
        .init();
}

static SHUTDOWN_FLAG: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();

extern "C" fn sigterm_handler(_: libc::c_int) {
    if let Some(flag) = SHUTDOWN_FLAG.get() {
        flag.store(true, Ordering::SeqCst);
    }
}
