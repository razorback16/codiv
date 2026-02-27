use slate_common::config;
use slate_common::messages::{self, ClientMessage, DaemonMessage};
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub type ClientId = u64;

pub struct IpcMessage {
    pub client_id: ClientId,
    pub message: ClientMessage,
}

/// The IPC server is split into parts that can be borrowed independently:
/// - `listener` for accepting new connections
/// - `msg_rx` for receiving decoded client messages
/// - `clients` / `send` / `disconnect` for managing client state
pub struct IpcServer {
    pub listener: UnixListener,
    clients: HashMap<ClientId, mpsc::Sender<Vec<u8>>>,
    next_id: ClientId,
    pub msg_rx: mpsc::Receiver<IpcMessage>,
    msg_tx: mpsc::Sender<IpcMessage>,
}

impl IpcServer {
    pub async fn new() -> std::io::Result<Self> {
        let path = config::socket_path();
        // Remove stale socket
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        info!("listening on {}", path);

        let (msg_tx, msg_rx) = mpsc::channel(256);
        Ok(Self {
            listener,
            clients: HashMap::new(),
            next_id: 1,
            msg_rx,
            msg_tx,
        })
    }

    /// Register a newly accepted stream as a client. Returns the assigned ClientId.
    pub fn register_client(&mut self, stream: UnixStream) -> ClientId {
        let client_id = self.next_id;
        self.next_id += 1;

        let (send_tx, send_rx) = mpsc::channel::<Vec<u8>>(64);
        self.clients.insert(client_id, send_tx);

        let msg_tx = self.msg_tx.clone();
        tokio::spawn(client_connection(client_id, stream, msg_tx, send_rx));

        info!("client {} connected", client_id);
        client_id
    }

    pub async fn send(&self, client_id: ClientId, msg: &DaemonMessage) {
        if let Some(tx) = self.clients.get(&client_id) {
            match messages::frame_message(msg) {
                Ok(frame) => {
                    if tx.send(frame).await.is_err() {
                        warn!("client {} send channel closed", client_id);
                    }
                }
                Err(e) => error!("failed to serialize message: {}", e),
            }
        }
    }

    pub fn disconnect(&mut self, client_id: ClientId) {
        self.clients.remove(&client_id);
        info!("client {} disconnected", client_id);
    }
}

async fn client_connection(
    client_id: ClientId,
    stream: UnixStream,
    msg_tx: mpsc::Sender<IpcMessage>,
    mut send_rx: mpsc::Receiver<Vec<u8>>,
) {
    let (mut reader, mut writer) = stream.into_split();

    // Spawn writer task
    let writer_handle = tokio::spawn(async move {
        while let Some(frame) = send_rx.recv().await {
            if writer.write_all(&frame).await.is_err() {
                break;
            }
        }
    });

    // Reader loop
    let mut header_buf = [0u8; 4];
    loop {
        if reader.read_exact(&mut header_buf).await.is_err() {
            break;
        }

        let payload_len = messages::parse_frame_header(&header_buf) as usize;
        if payload_len > config::MAX_MESSAGE_SIZE {
            error!("client {}: message too large ({})", client_id, payload_len);
            break;
        }

        let mut payload = vec![0u8; payload_len];
        if reader.read_exact(&mut payload).await.is_err() {
            break;
        }

        match bincode::deserialize::<ClientMessage>(&payload) {
            Ok(message) => {
                if msg_tx
                    .send(IpcMessage { client_id, message })
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(e) => {
                error!("client {}: failed to deserialize: {}", client_id, e);
            }
        }
    }

    writer_handle.abort();
}
