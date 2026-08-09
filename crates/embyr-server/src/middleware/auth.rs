//! Auth context types for gRPC middleware.
//!
//! Authentication logic lives in `grpc::handler::FirestoreService::authenticate`.
//! This module holds shared auth context types used across middleware layers.

/// Resolved authentication context attached to a request after successful
/// credential verification.
pub struct AuthContext {
    pub project_id: String,
    pub project_status: String,
}
