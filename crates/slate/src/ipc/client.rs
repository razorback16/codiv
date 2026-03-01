use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use super::messages::{DaemonMessage, FRAME_HEADER_SIZE, MAX_MESSAGE_SIZE, parse_frame_header};

pub struct SlatedClient {
    stream: UnixStream,
    rx: crossbeam_channel::Receiver<DaemonMessage>,
    connected: Arc<AtomicBool>,
    _reader_handle: Option<thread::JoinHandle<()>>,
}

impl SlatedClient {
    pub fn connect(socket_path: &str) -> Option<Self> {
        log::debug!("connecting to daemon at {}", socket_path);
        let stream = UnixStream::connect(socket_path).ok()?;
        let reader_stream = stream.try_clone().ok()?;

        let (tx, rx) = crossbeam_channel::unbounded();
        let connected = Arc::new(AtomicBool::new(true));
        let connected_clone = connected.clone();

        let handle = thread::spawn(move || {
            reader_loop(reader_stream, tx, connected_clone);
        });

        log::info!("connected to daemon");
        Some(SlatedClient {
            stream,
            rx,
            connected,
            _reader_handle: Some(handle),
        })
    }

    pub fn send(&mut self, framed_data: &[u8]) -> bool {
        if !self.connected.load(Ordering::Relaxed) {
            return false;
        }
        self.stream.write_all(framed_data).is_ok()
    }

    /// Expose the daemon message channel for use in `select!`-based event loops.
    pub fn daemon_receiver(&self) -> &crossbeam_channel::Receiver<DaemonMessage> {
        &self.rx
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
}

impl Drop for SlatedClient {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
        // Reader thread will exit when stream is shut down.
        if let Some(handle) = self._reader_handle.take() {
            let _ = handle.join();
        }
    }
}

fn reader_loop(
    mut stream: UnixStream,
    tx: crossbeam_channel::Sender<DaemonMessage>,
    connected: Arc<AtomicBool>,
) {
    let mut header_buf = [0u8; FRAME_HEADER_SIZE];

    loop {
        // Read frame header.
        if stream.read_exact(&mut header_buf).is_err() {
            break;
        }

        let payload_size = parse_frame_header(&header_buf) as usize;
        if payload_size > MAX_MESSAGE_SIZE {
            break;
        }

        // Read payload.
        let mut payload = vec![0u8; payload_size];
        if stream.read_exact(&mut payload).is_err() {
            break;
        }

        // Deserialize with bincode and send to channel.
        match bincode::deserialize::<DaemonMessage>(&payload) {
            Ok(msg) => {
                if tx.send(msg).is_err() {
                    break; // Receiver dropped.
                }
            }
            Err(e) => {
                log::warn!("failed to deserialize daemon message: {}", e);
            }
        }
    }

    log::warn!("daemon connection lost");
    connected.store(false, Ordering::Relaxed);
}
