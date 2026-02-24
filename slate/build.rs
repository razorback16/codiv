use std::process::Command;

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let schema = "../schemas/ipc.fbs";

    // Try to find flatc
    let flatc = find_flatc().unwrap_or_else(|| {
        println!("cargo:warning=flatc not found; skipping FlatBuffers codegen");
        println!("cargo:warning=Rust IPC types will not be available");
        std::process::exit(0);
    });

    let status = Command::new(&flatc)
        .args(["--rust", "-o", &out_dir, schema])
        .status()
        .expect("failed to run flatc");

    if !status.success() {
        panic!("flatc failed with status: {}", status);
    }

    println!("cargo:rerun-if-changed={}", schema);
}

fn find_flatc() -> Option<String> {
    // 1. Check slated build directory (built by cmake)
    let slated_flatc = "../slated/build/_deps/flatbuffers-build/flatc";
    if std::path::Path::new(slated_flatc).exists() {
        return Some(slated_flatc.to_string());
    }

    // 2. Check PATH
    if Command::new("flatc").arg("--version").output().is_ok() {
        return Some("flatc".to_string());
    }

    None
}
