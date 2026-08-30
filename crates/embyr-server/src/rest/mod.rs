//! REST / gRPC-Web / BrowserChannel server on :8081.
//!
//! Exposes:
//! - `browser_channel` — BrowserChannel long-poll session state and handlers
//! - `grpc_web` — axum router builder that wraps FirestoreService with tonic-web

pub mod browser_channel;
pub mod grpc_web;
// client-auth (US-02, ADR-026): signInWithCustomToken() bridge endpoint.
pub mod sign_in;
// client-auth-hosted-identity (US-03, ADR-036): accounts:signInWithPassword() endpoint.
pub mod sign_in_with_password;
// client-auth-hosted-identity (US-02, ADR-036): accounts:signUp() endpoint.
pub mod sign_up;
// client-auth-hosted-identity (US-04, ADR-036): accounts:sendOobCode() /
// accounts:resetPassword() endpoints.
pub mod reset_password;
