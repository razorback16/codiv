use std::time::{Duration, Instant};

use crossterm::event;
use crossbeam_channel;

use super::state::PendingCommand;

pub(crate) fn process_pty_bytes(bytes: &[u8], pending: &mut PendingCommand, parser: &mut vt100::Parser) {
    parser.process(bytes);
    let text = String::from_utf8_lossy(bytes);
    pending.accumulated.push_str(&text);
    pending.last_activity = Instant::now();
}

/// Spawn a background thread that blocks on `crossterm::event::read()` and
/// forwards events through a crossbeam channel. This lets the main loop
/// `select!` on terminal input alongside PTY and daemon channels.
pub(crate) fn spawn_crossterm_reader() -> crossbeam_channel::Receiver<event::Event> {
    let (tx, rx) = crossbeam_channel::unbounded();
    std::thread::Builder::new()
        .name("crossterm-reader".into())
        .spawn(move || loop {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => match event::read() {
                    Ok(evt) => {
                        if tx.send(evt).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        })
        .expect("spawn crossterm reader");
    rx
}

/// Compute the minimum timeout until the next timer-driven action.
///
/// Returns the shortest of:
/// - Heartbeat remaining (10s interval, only when daemon connected)
/// - Activity timeout remaining (300s, only when a command is pending)
/// - Fallback ceiling of 60s (guarantees we wake up periodically)
pub(crate) fn compute_next_timeout(
    pending_command: &Option<PendingCommand>,
    daemon_connected: bool,
    last_heartbeat_sent: Instant,
    completion_engine_ready: bool,
) -> Duration {
    let mut timeout = Duration::from_secs(60); // fallback ceiling

    // While the completion engine is initializing (sourcing bash-completion),
    // poll frequently so poll_init() can drain PTY output via try_read().
    if !completion_engine_ready && pending_command.is_none() {
        timeout = timeout.min(Duration::from_millis(50));
    }

    if let Some(ref pending) = pending_command {
        // Activity timeout: 300s since last activity.
        let activity_deadline = Duration::from_secs(300);
        let activity_elapsed = pending.last_activity.elapsed();
        if activity_elapsed < activity_deadline {
            timeout = timeout.min(activity_deadline - activity_elapsed);
        } else {
            return Duration::ZERO;
        }
    }

    // Heartbeat: every 10s when connected.
    if daemon_connected {
        let hb_deadline = Duration::from_secs(10);
        let hb_elapsed = last_heartbeat_sent.elapsed();
        if hb_elapsed < hb_deadline {
            timeout = timeout.min(hb_deadline - hb_elapsed);
        } else {
            return Duration::ZERO;
        }
    }

    timeout
}
