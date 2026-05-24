// Generated gRPC stubs for Firestore-compatible API.
// This crate contains protobuf-generated types only.
// IO crates (tokio, sqlx, axum) are intentionally absent.

/// Firestore v1 protobuf types and gRPC service stubs.
pub mod firestore {
    tonic::include_proto!("google.firestore.v1");
}

/// embyr agent v1 StorageAgent service stubs.
pub mod agent {
    tonic::include_proto!("embyr.agent.v1");

    // Re-export server/client types at the agent module level for ergonomic access.
    pub use storage_agent_server::StorageAgentServer;
    pub use storage_agent_client::StorageAgentClient;
}

/// google.rpc types.
/// Defined manually because tonic-build does not generate a separate file for
/// this package when `extern_path` is used to reference it from generated code.
pub mod rpc {
    /// The Status type from google.rpc, representing a logical error model.
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Status {
        /// The status code.
        #[prost(int32, tag = "1")]
        pub code: i32,
        /// A developer-facing error message.
        #[prost(string, tag = "2")]
        pub message: ::prost::alloc::string::String,
        /// Error details as opaque bytes.
        #[prost(bytes = "vec", repeated, tag = "3")]
        pub details: ::prost::alloc::vec::Vec<::prost::alloc::vec::Vec<u8>>,
    }
}
