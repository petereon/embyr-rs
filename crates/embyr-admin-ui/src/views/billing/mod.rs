//! BillingView — page header + Tabs-routed Overview/Usage/Invoices.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! AC-101-06: Overview/Usage/Invoices tabs are all navigable. Usage renders
//! `AppModel::usage_table_rows()` via `usage::UsageTab`; Invoices renders
//! `model.invoices` (or the empty-state copy) via `invoices::InvoicesTab`.
//!
//! Supersedes the old `src/views/billing.rs` (V1 placeholder table +
//! time-range selector) — see mod-level doc comment note at the bottom of
//! this file for what was ported vs. deferred.

#[cfg(feature = "csr")]
pub mod invoices;
#[cfg(feature = "csr")]
pub mod modals;
#[cfg(feature = "csr")]
pub mod overview;
#[cfg(feature = "csr")]
pub mod usage;

#[cfg(feature = "csr")]
pub use invoices::InvoicesTab;
#[cfg(feature = "csr")]
pub use modals::{CardModal, UpgradeModal};
#[cfg(feature = "csr")]
pub use overview::BillingOverviewTab;
#[cfg(feature = "csr")]
pub use usage::UsageTab;

#[cfg(feature = "csr")]
use crate::components::primitives::{TabItem, Tabs};
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
                    "usage" => view! { <UsageTab/> }.into_any(),
                    "invoices" => view! { <InvoicesTab/> }.into_any(),
                    _ => view! { <BillingOverviewTab/> }.into_any(),
                }}
            </div>
        </div>
    }
}
