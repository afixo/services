//! Compiles `proto/afixo/v1/*.proto` into Rust. Needs `protoc` on PATH
//! (`brew install protobuf` / `apt install protobuf-compiler`).

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../proto");
    let files = [
        "common",
        "auth",
        "identity",
        "policy",
        "disclosure",
        "audit",
    ]
    .map(|name| root.join(format!("afixo/v1/{name}.proto")));

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .emit_rerun_if_changed(true)
        .compile_protos(&files, std::slice::from_ref(&root))?;

    Ok(())
}
