//! App root component — creates the Leptos reactive tree and mounts it to the DOM.
//!
//! Only compiled under the `csr` feature (WASM target).
//! Host-target tests import model/msg/update only.

#[cfg(feature = "csr")]
mod inner {
    use leptos::prelude::*;

    /// Root application component.
    ///
    /// Walking skeleton: renders a minimal placeholder. Subsequent slices
    /// fill in the full admin UI.
    #[component]
    pub fn App() -> impl IntoView {
        view! { <div class="embyr-admin">"embyr admin console"</div> }
    }
}

#[cfg(feature = "csr")]
pub use inner::App;
