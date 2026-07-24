//! Sidebar navigation component.
//!
//! Gated behind #[cfg(feature = "csr")].
//! V1 stub — navigation items added in slice-03.

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use crate::model::{AppModel, Section};
#[cfg(feature = "csr")]
use crate::msg::Msg;

/// Application sidebar with primary navigation links.
#[cfg(feature = "csr")]
#[component]
pub fn Sidebar() -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");
    let _model = use_context::<RwSignal<AppModel>>().expect("model context missing");

    let dispatch_nav = dispatch.clone();
    let dispatch_signout = dispatch;

    view! {
        <nav class="sidebar">
            <div class="sidebar-brand">
                <span class="sidebar-brand-name">"embyr"</span>
            </div>
            <div class="sidebar-nav">
                <button
                    class="sidebar-link sidebar-link-active"
                    on:click=move |_| dispatch_nav.run(Msg::NavigateTo(Section::Dashboard))
                >
                    "Dashboard"
                </button>
            </div>
            <div class="sidebar-footer">
                <button
                    class="sidebar-signout"
                    on:click=move |_| dispatch_signout.run(Msg::SignOut)
                >
                    "Sign out"
                </button>
            </div>
        </nav>
    }
}
