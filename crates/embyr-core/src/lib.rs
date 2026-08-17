// Pure domain logic for embyr-rs.
// Three bounded contexts: Tenant Management (BC-1), Document Storage (BC-2),
// Real-Time Delivery (BC-3).
// NO IO crates (tokio, sqlx, tonic, axum) — enforced by deny.toml.

pub mod admin;
// security-rules (BC-4 Access Control, ADR-027/ADR-029): Condition,
// Operand, AuthContext, EvaluationOutcome, ConditionParseError, pure
// parse_condition()/evaluate(). Zero IO.
pub mod access_control;
pub mod auth;
// client-auth (BC-1 extension, ADR-024): ClientIdentityCredential,
// VerifiedEndUserIdentity, ClientIdentityVerifyError, pure
// verify_client_identity_token(). Zero IO.
pub mod client_identity;
pub mod domain;
pub mod error;
pub mod rate_limit;
pub mod realtime;
pub mod storage;

// ---------------------------------------------------------------------------
// Compile-time object-safety verification for BackendAdapter.
//
// This type alias forces the compiler to prove that `dyn BackendAdapter` is
// a valid fat pointer target. If BackendAdapter ever becomes non-object-safe,
// this line will fail to compile — catching the regression immediately.
// ---------------------------------------------------------------------------
pub use storage::backend_adapter::DynBackendAdapter;
