#[allow(unused_imports, clippy::all, dead_code, mismatched_lifetime_syntaxes)]
mod ipc_generated {
    include!(concat!(env!("OUT_DIR"), "/ipc_generated.rs"));
}

pub use ipc_generated::slate::ipc::*;

use flatbuffers::FlatBufferBuilder;

pub const FRAME_HEADER_SIZE: usize = 4;
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024; // 16 MiB

/// Wrap payload in a 4-byte big-endian length-prefixed frame.
pub fn frame_message(data: &[u8]) -> Vec<u8> {
    let len = data.len() as u32;
    let mut framed = Vec::with_capacity(FRAME_HEADER_SIZE + data.len());
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(data);
    framed
}

/// Parse a 4-byte big-endian frame header into payload length.
pub fn parse_frame_header(header: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*header)
}

// ---------------------------------------------------------------------------
// Message builders (return framed bytes ready to send)
// ---------------------------------------------------------------------------

pub fn build_env_snapshot(
    session_id: &str,
    env_vars: &[(String, String)],
    path: &str,
    cwd: &str,
) -> Vec<u8> {
    let mut builder = FlatBufferBuilder::with_capacity(1024);

    let kvs: Vec<_> = env_vars
        .iter()
        .map(|(k, v)| {
            let key = builder.create_string(k);
            let val = builder.create_string(v);
            KeyValue::create(&mut builder, &KeyValueArgs {
                key: Some(key),
                value: Some(val),
            })
        })
        .collect();

    let env_vec = builder.create_vector(&kvs);
    let sid = builder.create_string(session_id);
    let p = builder.create_string(path);
    let c = builder.create_string(cwd);

    let env_msg = EnvSnapshotMsg::create(&mut builder, &EnvSnapshotMsgArgs {
        session_id: Some(sid),
        env_vars: Some(env_vec),
        path: Some(p),
        aliases: None,
        functions: None,
        cwd: Some(c),
    });

    let msg = Message::create(&mut builder, &MessageArgs {
        type_: MessageType::EnvSnapshot,
        body_type: MessageBody::EnvSnapshotMsg,
        body: Some(env_msg.as_union_value()),
    });
    builder.finish(msg, None);

    frame_message(builder.finished_data())
}

pub fn build_heartbeat(session_id: &str) -> Vec<u8> {
    let mut builder = FlatBufferBuilder::with_capacity(128);

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let sid = builder.create_string(session_id);

    let hb = HeartbeatMsg::create(&mut builder, &HeartbeatMsgArgs {
        timestamp: ts,
        session_id: Some(sid),
    });

    let msg = Message::create(&mut builder, &MessageArgs {
        type_: MessageType::Heartbeat,
        body_type: MessageBody::HeartbeatMsg,
        body: Some(hb.as_union_value()),
    });
    builder.finish(msg, None);

    frame_message(builder.finished_data())
}

// ---------------------------------------------------------------------------
// Message parser
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum DaemonMessage {
    CommandOutput {
        request_id: String,
        data: Vec<u8>,
        is_stderr: bool,
    },
    CommandComplete {
        request_id: String,
        exit_code: i32,
    },
    Heartbeat {
        timestamp: i64,
    },
    Error {
        request_id: String,
        message: String,
    },
}

pub fn parse_message(data: &[u8]) -> Option<DaemonMessage> {
    let msg = flatbuffers::root::<Message>(data).ok()?;

    match msg.type_() {
        MessageType::CommandOutput => {
            let out = msg.body_as_command_output_msg()?;
            Some(DaemonMessage::CommandOutput {
                request_id: out.request_id().unwrap_or_default().to_string(),
                data: out.data().map(|d| d.iter().map(|b| b as u8).collect()).unwrap_or_default(),
                is_stderr: out.is_stderr(),
            })
        }
        MessageType::CommandComplete => {
            let comp = msg.body_as_command_complete_msg()?;
            Some(DaemonMessage::CommandComplete {
                request_id: comp.request_id().unwrap_or_default().to_string(),
                exit_code: comp.exit_code(),
            })
        }
        MessageType::Heartbeat => {
            let hb = msg.body_as_heartbeat_msg()?;
            Some(DaemonMessage::Heartbeat {
                timestamp: hb.timestamp(),
            })
        }
        MessageType::Error => {
            let err = msg.body_as_error_msg()?;
            Some(DaemonMessage::Error {
                request_id: err.request_id().unwrap_or_default().to_string(),
                message: err.message().unwrap_or_default().to_string(),
            })
        }
        _ => None,
    }
}
