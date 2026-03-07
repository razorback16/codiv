use std::path::PathBuf;

pub const VERSION: &str = "0.1.0";
pub const FRAME_HEADER_SIZE: usize = 4;
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024; // 16 MiB

pub fn socket_path() -> String {
    format!("/tmp/slated-{}.sock", unsafe { libc::getuid() })
}

fn slate_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".slate-agent")
}

pub fn pid_file_path() -> PathBuf {
    slate_dir().join("slated.pid")
}

pub fn log_file_path() -> PathBuf {
    slate_dir().join("slated.log")
}

pub fn config_dir() -> PathBuf {
    slate_dir()
}
