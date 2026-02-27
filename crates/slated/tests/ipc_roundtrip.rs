use slate_common::messages::{frame_message, parse_frame_header, ClientMessage, DaemonMessage};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

#[tokio::test]
async fn test_heartbeat_roundtrip() {
    // Use a unique socket path for this test
    let socket_path = format!("/tmp/slated-test-{}.sock", std::process::id());
    let _ = std::fs::remove_file(&socket_path);

    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

    // Simulate daemon accepting and responding to heartbeat
    let server_handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = [0u8; 4];
        stream.read_exact(&mut header).await.unwrap();
        let len = parse_frame_header(&header) as usize;
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).await.unwrap();

        let msg: ClientMessage = bincode::deserialize(&payload).unwrap();
        assert!(matches!(msg, ClientMessage::Heartbeat { .. }));

        // Send heartbeat response
        let response = DaemonMessage::Heartbeat { timestamp: 12345 };
        let frame = frame_message(&response).unwrap();
        stream.write_all(&frame).await.unwrap();
    });

    // Connect client
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut stream = UnixStream::connect(&socket_path).await.unwrap();

    // Send heartbeat
    let hb = ClientMessage::Heartbeat { timestamp: 1000 };
    let frame = frame_message(&hb).unwrap();
    stream.write_all(&frame).await.unwrap();

    // Read response
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await.unwrap();
    let len = parse_frame_header(&header) as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await.unwrap();

    let response: DaemonMessage = bincode::deserialize(&payload).unwrap();
    assert!(matches!(
        response,
        DaemonMessage::Heartbeat { timestamp: 12345 }
    ));

    server_handle.await.unwrap();
    let _ = std::fs::remove_file(&socket_path);
}
