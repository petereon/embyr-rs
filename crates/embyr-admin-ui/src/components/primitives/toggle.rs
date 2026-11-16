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
/// `disabled` makes the switch inert (read-only roles, call in flight).
#[cfg(feature = "csr")]
#[component]
pub fn Toggle(
    checked: bool,
    on_change: Callback<bool>,
    #[prop(default = false)] disabled: bool,
) -> impl IntoView {
    let current = checked;
    view! {
        <button
            class="toggle"
            class:toggle-on=move || current
            type="button"
            disabled=disabled
            on:click=move |_| on_change.run(!current)
        >
            <span class="toggle-thumb" />
        </button>
    }
}
