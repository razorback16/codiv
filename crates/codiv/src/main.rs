mod app;
mod cli;
mod shell;
mod ui;
mod ipc;
mod markdown;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use clap::Parser;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name = "codiv", version = VERSION, about = "AI-powered terminal agent")]
struct Cli {
    #[command(subcommand)]
    command: Option<cli::Commands>,

    /// Enable debug logging (optionally set level: error, warn, info, debug, trace)
    #[arg(long, value_name = "LEVEL", num_args = 0..=1, default_missing_value = "debug")]
    debug: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    // Handle tool subcommands immediately (no TUI setup needed)
    if let Some(cmd) = cli.command {
        cli::dispatch(cmd);
        return;
    }

    // Initialize debug logging if requested
    if let Some(ref level_str) = cli.debug {
        let level = match level_str.to_lowercase().as_str() {
            "error" => log::LevelFilter::Error,
            "warn" => log::LevelFilter::Warn,
            "info" => log::LevelFilter::Info,
            "debug" => log::LevelFilter::Debug,
            "trace" => log::LevelFilter::Trace,
            _ => {
                eprintln!("codiv: unknown log level '{}', using 'debug'", level_str);
                log::LevelFilter::Debug
            }
        };
        init_logging(level);
        log::info!("codiv {} starting", VERSION);
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
            eprintln!("codiv: {}", e);
            std::process::exit(1);
        }
    }
}

fn init_logging(level: log::LevelFilter) {
    use std::fs::OpenOptions;
    use std::io::Write;

    let target = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/codiv-debug.log")
        .expect("failed to open /tmp/codiv-debug.log");

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
