use std::path::PathBuf;

pub const FRAME_HEADER_SIZE: usize = 4;
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024; // 16 MiB

pub fn socket_path() -> String {
    format!("/tmp/codivd-{}.sock", unsafe { libc::getuid() })
}

fn codiv_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".codiv")
}

pub fn pid_file_path() -> PathBuf {
    codiv_dir().join("codivd.pid")
}

pub fn log_file_path() -> PathBuf {
    codiv_dir().join("codivd.log")
}

pub fn config_dir() -> PathBuf {
    codiv_dir()
}
