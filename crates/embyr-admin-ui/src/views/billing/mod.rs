//! BillingView — page header + checkout-return notice + Tabs-routed Overview/Usage.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! Everything shown comes from the API: plan, status and caps from `GET /billing/subscription`,
//! month-to-date usage from `GET /billing?range=month`. There is no invoices tab (G-47): invoices
//! and the payment method live in Stripe's billing portal, reached with "Manage billing".

#[cfg(feature = "csr")]
pub mod modals;
#[cfg(feature = "csr")]
pub mod overview;
#[cfg(feature = "csr")]
pub mod usage;

#[cfg(feature = "csr")]
pub use modals::{CardModal, UpgradeModal};
#[cfg(feature = "csr")]
pub use overview::BillingOverviewTab;
#[cfg(feature = "csr")]
pub use usage::UsageTab;

#[cfg(feature = "csr")]
use crate::api::ApiCall;
#[cfg(feature = "csr")]
use crate::components::primitives::{TabItem, Tabs};
#[cfg(feature = "csr")]
use crate::model::{AppModel, CheckoutReturn};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

/// Billing page — "Plan & Billing" header + Overview/Usage tabs.
///
/// Active tab is view-local state (`RwSignal<&'static str>`), same class as
/// `db_detail`'s own `active_tab` — not part of the TEA model.
#[cfg(feature = "csr")]
#[component]
pub fn BillingView() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let active_tab = RwSignal::new("overview");
    let unavailable = move || model.with(|m| m.billing.unavailable);

    view! {
        <div class="page">
            <div class="page-head">
                <h1 class="page-title">"Plan & Billing"</h1>
            </div>

            <CheckoutNotice/>

            <Show
                when=move || !unavailable()
                fallback=|| view! {
                    <p class="empty-state" role="status">
                        "Billing is not configured on this server, or Stripe cannot be reached right now."
                    </p>
                }
            >
                <Tabs
                    active=active_tab.get()
                    on_change=Callback::new(move |value| active_tab.set(value))
                    tabs=vec![
                        TabItem { value: "overview", label: "Overview" },
                        TabItem { value: "usage", label: "Usage" },
                    ]
                />

                <div style="margin-top:20px">
                    {move || match active_tab.get() {
                        "usage" => view! { <UsageTab/> }.into_any(),
                        _ => view! { <BillingOverviewTab/> }.into_any(),
                    }}
                </div>
            </Show>
        </div>
    }
}

/// Outcome of the Stripe Checkout the browser just came back from (`?checkout=`). The plan itself
/// arrives from Stripe by webhook, so success offers a refresh instead of claiming the upgrade.
#[cfg(feature = "csr")]
#[component]
fn CheckoutNotice() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    view! {
        {move || model.with(|m| m.checkout_return).map(|outcome| view! {
            <div class="row" role="status" style="gap:12px;align-items:center;margin-bottom:16px">
                <span class="card-sub">
                    {match outcome {
                        CheckoutReturn::Success => "Checkout complete. Your plan updates as soon as Stripe confirms the payment, usually within a minute.",
                        CheckoutReturn::Cancelled => "Checkout cancelled. You have not been charged.",
                    }}
                </span>
                {(outcome == CheckoutReturn::Success).then(|| view! {
                    <button class="btn btn-secondary btn-sm" type="button"
                        on:click=move |_| dispatch.run(Msg::Request(ApiCall::LoadSubscription))>
                        "Refresh"
                    </button>
                })}
                <button class="btn btn-ghost btn-sm" type="button"
                    on:click=move |_| dispatch.run(Msg::DismissCheckoutNotice)>
                    "Dismiss"
                </button>
            </div>
        })}
    }
}
