//! REST / gRPC-Web / BrowserChannel server on :8081.
//!
//! Exposes:
//! - `browser_channel` — BrowserChannel long-poll session state and handlers
//! - `grpc_web` — axum router builder that wraps FirestoreService with tonic-web

pub mod browser_channel;
pub mod grpc_web;
