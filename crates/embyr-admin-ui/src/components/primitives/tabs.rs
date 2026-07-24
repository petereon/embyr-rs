//! Tabs primitive — horizontal tab list.
//!
//! Minimal stub for V1. Renders a list of labelled tabs.
//!
//! Gated behind #[cfg(feature = "csr")].

#[cfg(feature = "csr")]
use leptos::prelude::*;

/// A single tab descriptor.
#[cfg(feature = "csr")]
#[derive(Clone, PartialEq)]
pub struct TabItem {
    pub value: &'static str,
    pub label: &'static str,
}

/// Horizontal tab bar.
///
/// `active` identifies the currently selected tab value.
/// `on_change` is called with the new tab value when a tab is clicked.
/// `tabs` is the ordered list of tab descriptors.
#[cfg(feature = "csr")]
#[component]
pub fn Tabs(
    active: &'static str,
    on_change: Callback<&'static str>,
    tabs: Vec<TabItem>,
) -> impl IntoView {
    view! {
        <div class="tabs">
            {tabs.into_iter().map(|tab| {
                let value = tab.value;
                let is_active = value == active;
                view! {
                    <button
                        class="tab"
                        class:tab-active=move || is_active
                        type="button"
                        on:click=move |_| on_change.run(value)
                    >
                        {tab.label}
                    </button>
                }
            }).collect_view()}
        </div>
    }
}
