//! Toggle primitive — boolean switch.
//!
//! Minimal stub for V1. Renders a checkbox-style toggle.
//!
//! Gated behind #[cfg(feature = "csr")].

#[cfg(feature = "csr")]
use leptos::prelude::*;

/// Boolean toggle switch.
///
/// `checked` controls the display state.
/// `on_change` is called with the new boolean value on click.
#[cfg(feature = "csr")]
#[component]
pub fn Toggle(checked: bool, on_change: Callback<bool>) -> impl IntoView {
    let current = checked;
    view! {
        <button
            class="toggle"
            class:toggle-on=move || current
            type="button"
            on:click=move |_| on_change.run(!current)
        >
            <span class="toggle-thumb" />
        </button>
    }
}
