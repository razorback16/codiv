use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use crate::ipc::client::CodivdClient;
use crate::ipc::daemon_launcher;
use crate::ipc::messages;
use crate::shell::BashCoprocess;
use crate::ui;

pub fn run(shutdown: Arc<AtomicBool>) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Spawn bash co-process
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    log::debug!("spawning bash coprocess (cols={}, rows={})", cols, rows);
    let mut bash = BashCoprocess::spawn(cols, rows)?;
    log::debug!("bash coprocess spawned successfully");

    // 2. Get initial cwd
    let cwd = bash.capture_cwd();
    log::debug!("initial cwd: {}", cwd);

    // 3. Query terminal colors (before raw mode)
    log::debug!("querying terminal colors");
    let terminal_colors = ui::terminal::io::query_terminal_colors();
    log::debug!("terminal colors: fg={}, bg={}", terminal_colors.fg, terminal_colors.bg);

    // 4. Connect to daemon
    log::debug!("connecting to daemon");
    let client = connect_to_daemon(&mut bash, &cwd);
    log::debug!("daemon connected: {}", client.is_some());

    // 5. Run the terminal UI (blocks until exit)
    log::debug!("entering terminal UI");
    ui::terminal::run(&mut bash, shutdown, cwd, client, terminal_colors)
}

fn connect_to_daemon(bash: &mut BashCoprocess, cwd: &str) -> Option<CodivdClient> {
    // Try to ensure daemon is running (3s timeout).
    if !daemon_launcher::ensure_daemon_running(Duration::from_secs(3)) {
        log::warn!("daemon not available");
        return None;
    }

    let socket = daemon_launcher::socket_path();
    let mut client = CodivdClient::connect(&socket)?;

    // Send initial environment snapshot.
    let env_pairs: Vec<(String, String)> = bash.capture_env();

    if let Some(snapshot) = messages::build_env_snapshot(&env_pairs, "", cwd) {
        client.send(&snapshot);
    }

    Some(client)
}
