use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use crate::ipc::client::SlatedClient;
use crate::ipc::daemon_launcher;
use crate::ipc::messages;
use crate::shell::BashCoprocess;
use crate::shell::CommandIndex;
use crate::ui;

pub fn run(shutdown: Arc<AtomicBool>) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Spawn bash co-process
    let (_cols, rows) = crossterm::terminal::size().unwrap_or((500, 24));
    // Use a wide PTY so the sentinel command (~82 chars) never wraps.
    // ratatui handles display-width rendering independently.
    log::debug!("spawning bash coprocess (rows={})", rows);
    let mut bash = BashCoprocess::spawn(500, rows)?;
    log::debug!("bash coprocess spawned successfully");

    // 2. Build command index
    log::debug!("building command index");
    let mut command_index = CommandIndex::new();
    command_index.scan_path_directories();
    command_index.add_builtins();

    // 3. Get initial cwd
    let cwd = bash.capture_cwd();
    log::debug!("initial cwd: {}", cwd);

    // 4. Connect to daemon
    log::debug!("connecting to daemon");
    let client = connect_to_daemon(&mut bash, &cwd);
    log::debug!("daemon connected: {}", client.is_some());

    // 5. Run the terminal UI (blocks until exit)
    log::debug!("entering terminal UI");
    ui::terminal::run(&mut bash, &command_index, shutdown, cwd, client)
}

fn connect_to_daemon(bash: &mut BashCoprocess, cwd: &str) -> Option<SlatedClient> {
    // Try to ensure daemon is running (3s timeout).
    if !daemon_launcher::ensure_daemon_running(Duration::from_secs(3)) {
        log::warn!("daemon not available");
        return None;
    }

    let socket = daemon_launcher::socket_path();
    let mut client = SlatedClient::connect(&socket)?;

    // Send initial environment snapshot.
    let env_pairs: Vec<(String, String)> = bash.capture_env();

    if let Some(snapshot) = messages::build_env_snapshot(&env_pairs, "", cwd) {
        client.send(&snapshot);
    }

    Some(client)
}
