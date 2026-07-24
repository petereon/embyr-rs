//! Topbar component (header bar).
//!
//! Gated behind #[cfg(feature = "csr")].
//! V1 stub — breadcrumbs and actions added in later slices.

#[cfg(feature = "csr")]
use leptos::prelude::*;

/// Top navigation bar.
#[cfg(feature = "csr")]
#[component]
pub fn Topbar() -> impl IntoView {
    view! {
        <header class="topbar">
            <div class="topbar-inner">
                <span class="topbar-title">"embyr admin"</span>
            </div>
        </header>
    }
}
