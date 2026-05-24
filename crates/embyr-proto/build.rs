use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = PathBuf::from("../../proto");

    let proto_files = &[
        "google/protobuf/timestamp.proto",
        "google/protobuf/wrappers.proto",
        "google/protobuf/empty.proto",
        "google/rpc/status.proto",
        "google/firestore/v1/common.proto",
        "google/firestore/v1/document.proto",
        "google/firestore/v1/query.proto",
        "google/firestore/v1/write.proto",
        "google/firestore/v1/firestore.proto",
        "embyr/agent/v1/storage_agent.proto",
    ];

    let full_paths: Vec<PathBuf> = proto_files
        .iter()
        .map(|f| proto_root.join(f))
        .collect();

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        // Map google.rpc to crate::rpc, which is defined manually in lib.rs.
        // This prevents generation of google.rpc.rs while allowing correct
        // absolute path references (crate::rpc::Status) in generated code.
        .extern_path(".google.rpc", "crate::rpc")
        .compile_protos(
            &full_paths,
            &[proto_root],
        )?;

    // Re-run if any proto file changes
    println!("cargo:rerun-if-changed=../../proto");

    Ok(())
}
