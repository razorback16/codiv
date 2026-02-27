use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::error;

/// Execute a shell command internally for the agent's Bash tool.
/// Streams stdout/stderr through the channel. Each message is (data, is_stderr).
pub async fn execute_command(
    command: &str,
    cwd: &str,
    env: &[(String, String)],
    tx: mpsc::Sender<(Vec<u8>, bool)>,
) -> i32 {
    let mut child = match Command::new("bash")
        .args(["-c", command])
        .current_dir(cwd)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            error!("failed to spawn worker: {}", e);
            return 127;
        }
    };

    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();

    let tx_err = tx.clone();
    let stderr_handle = tokio::spawn(async move {
        let mut buf = [0u8; 8192];
        loop {
            match stderr.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if tx_err.send((buf[..n].to_vec(), true)).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut buf = [0u8; 8192];
    loop {
        match stdout.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                if tx.send((buf[..n].to_vec(), false)).await.is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let _ = stderr_handle.await;

    match child.wait().await {
        Ok(status) => status.code().unwrap_or(1),
        Err(_) => 1,
    }
}
