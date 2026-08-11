//! BillingView — page header + Tabs-routed Overview/Usage/Invoices.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! AC-101-06: Overview/Usage/Invoices tabs are all navigable; Usage/
//! Invoices render structural stubs this slice (Slices 03/06 give them
//! full depth in `usage.rs`/`invoices.rs`, per feature-delta.md Component
//! Decomposition — not yet created; this step's stub content lives inline
//! below).
//!
//! Supersedes the old `src/views/billing.rs` (V1 placeholder table +
//! time-range selector) — see mod-level doc comment note at the bottom of
//! this file for what was ported vs. deferred.

#[cfg(feature = "csr")]
pub mod modals;
#[cfg(feature = "csr")]
pub mod overview;

#[cfg(feature = "csr")]
pub use modals::{CardModal, UpgradeModal};
#[cfg(feature = "csr")]
pub use overview::BillingOverviewTab;

#[cfg(feature = "csr")]
use crate::components::primitives::{TabItem, Tabs};
#[cfg(feature = "csr")]
use crate::model::AppModel;
#[cfg(feature = "csr")]
use leptos::prelude::*;

/// Billing page — "Plan & Billing" header + Overview/Usage/Invoices tabs.
///
/// Active tab is view-local state (`RwSignal<&'static str>`), same class as
/// `db_detail`'s own `active_tab` — not part of the TEA model, per
/// feature-delta.md's driving-ports note.
#[cfg(feature = "csr")]
#[component]
pub fn BillingView() -> impl IntoView {
    let active_tab = RwSignal::new("overview");

    view! {
        <div class="page">
            <div class="page-head">
                <h1 class="page-title">"Plan & Billing"</h1>
            </div>

            <Tabs
                active=active_tab.get()
                on_change=Callback::new(move |value| active_tab.set(value))
                tabs=vec![
                    TabItem { value: "overview", label: "Overview" },
                    TabItem { value: "usage", label: "Usage" },
                    TabItem { value: "invoices", label: "Invoices" },
                ]
            />

            <div style="margin-top:20px">
                {move || match active_tab.get() {
                    "usage" => view! { <UsageTabStub/> }.into_any(),
                    "invoices" => view! { <InvoicesTabStub/> }.into_any(),
                    _ => view! { <BillingOverviewTab/> }.into_any(),
                }}
            </div>
        </div>
    }
}

/// Usage tab structural stub (AC-101-06). Full per-database breakdown
/// table + KPI tiles land in Slice 02/03's `usage.rs`.
#[cfg(feature = "csr")]
#[component]
fn UsageTabStub() -> impl IntoView {
    view! {
        <p class="card-sub">"Usage breakdown — coming in a later slice."</p>
    }
}

/// Invoices tab structural stub (AC-107-03). Renders the documented
/// empty-state copy for Free-plan accounts with no invoice history; full
/// itemized table lands in Slice 06's `invoices.rs`.
#[cfg(feature = "csr")]
#[component]
fn InvoicesTabStub() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let empty_copy = move || model.with(|m| m.invoices_empty_state());

    view! {
        {move || match empty_copy() {
            Some(copy) => view! { <p class="card-sub">{copy}</p> }.into_any(),
            None => view! { <p class="card-sub">"Invoice history — coming in a later slice."</p> }.into_any(),
        }}
    }
}
