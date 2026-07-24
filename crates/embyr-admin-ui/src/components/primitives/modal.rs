//! Modal primitive — overlay dialog container.
//!
//! Minimal stub for V1. Wraps children in a modal overlay with a title.
//!
//! Gated behind #[cfg(feature = "csr")].

#[cfg(feature = "csr")]
use leptos::prelude::*;

/// Generic modal overlay.
///
/// Renders children inside a centred dialog.
/// `on_close` is invoked when the user clicks the backdrop or close button.
#[cfg(feature = "csr")]
#[component]
pub fn Modal(
    title: &'static str,
    on_close: Callback<()>,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="modal-backdrop" on:click=move |_| on_close.run(())>
            <div class="modal" on:click=|e| e.stop_propagation()>
                <div class="modal-head">
                    <span class="modal-title">{title}</span>
                    <button class="modal-close" type="button" on:click=move |_| on_close.run(())>
                        "✕"
                    </button>
                </div>
                <div class="modal-body">
                    {children()}
                </div>
            </div>
        </div>
    }
}
