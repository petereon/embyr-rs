//! Primitive UI components — Button, Badge, Card, Menu.
//!
//! Minimal stubs for V1. Full implementations with variants/slots
//! are deferred to subsequent slices.
//!
//! All items gated behind #[cfg(feature = "csr")].

#[cfg(feature = "csr")]
use leptos::prelude::*;

/// Generic button stub.
#[cfg(feature = "csr")]
#[component]
pub fn Button(children: Children) -> impl IntoView {
    view! {
        <button class="btn">{children()}</button>
    }
}

/// Status / label badge stub.
#[cfg(feature = "csr")]
#[component]
pub fn Badge(children: Children) -> impl IntoView {
    view! {
        <span class="badge">{children()}</span>
    }
}

/// Surface card stub.
#[cfg(feature = "csr")]
#[component]
pub fn Card(children: Children) -> impl IntoView {
    view! {
        <div class="card">{children()}</div>
    }
}

/// Dropdown menu stub.
#[cfg(feature = "csr")]
#[component]
pub fn Menu(children: Children) -> impl IntoView {
    view! {
        <div class="menu">{children()}</div>
    }
}
