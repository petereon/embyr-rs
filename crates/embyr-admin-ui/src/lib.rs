//! embyr-admin-ui — Leptos 0.8 CSR WASM SPA (admin console).
//!
//! Host-target (`cargo test`) compiles the pure TEA layer (model + msg + update + data)
//! without leptos or wasm-bindgen. The `csr` feature gates the browser-specific code.

pub mod model;
pub mod msg;
pub mod update;
pub mod data;
pub mod app;
