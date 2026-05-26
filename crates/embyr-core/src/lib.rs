// Pure domain logic for embyr-rs.
// Three bounded contexts: Tenant Management (BC-1), Document Storage (BC-2),
// Real-Time Delivery (BC-3).
// NO IO crates (tokio, sqlx, tonic, axum) — enforced by deny.toml.

pub mod auth;
pub mod domain;
pub mod error;
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
