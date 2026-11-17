//! BillingOverviewTab — Plan card + Billing card + Free-plan cap bars, all from the server.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! The plan, status and caps come from `GET /billing/subscription` (`model.subscription`,
//! `model.billing`); nothing is derived from usage mocks (G-48). Card data never touches this app:
//! "Upgrade to Pro" sends the Owner to Stripe-hosted Checkout and "Manage billing" to the Stripe
//! billing portal (payment method, invoices, cancel), both via a session URL the server mints.

#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind};
#[cfg(feature = "csr")]
use crate::data::{self, BarColor};
#[cfg(feature = "csr")]
use crate::model::{AppModel, CapUsage, Plan};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

/// Overview tab content — 2-column grid of billing summary cards.
#[cfg(feature = "csr")]
#[component]
pub fn BillingOverviewTab() -> impl IntoView {
    view! {
        <div class="billing-overview-grid">
            <PlanCard/>
            <PortalCard/>
            <CapUsageCard/>
        </div>
    }
}

/// Free plan: the server's cap figures per dimension. Pro has no caps.
#[cfg(feature = "csr")]
#[component]
fn CapUsageCard() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

    view! {
        <div class="card billing-card">
            <h3 class="card-title">"Usage this month"</h3>
            <div class="billing-card-body">
                {move || {
                    let (plan, caps) = model.with(|m| (m.subscription.plan.clone(), m.billing.cap_status.clone()));
                    match (plan, caps) {
                        (Plan::Pro, _) => view! {
                            <p class="card-sub">"The Pro plan has no usage caps. See the Usage tab for totals."</p>
                        }.into_any(),
                        (Plan::Free, Some(caps)) if !caps.is_empty() => {
                            caps.into_iter().map(|c| view! { <CapUsageBar cap=c/> }).collect_view().into_any()
                        }
                        (Plan::Free, _) => view! {
                            <p class="card-sub">"Usage figures are not available yet."</p>
                        }.into_any(),
                    }
                }}
            </div>
        </div>
    }
}

/// One cap bar: label, used / cap, and a colored fill (accent <80%, amber 80-99%, red >=100%,
/// via `data::bar_color()`).
#[cfg(feature = "csr")]
#[component]
fn CapUsageBar(cap: CapUsage) -> impl IntoView {
    let ratio = if cap.cap == 0 { 0.0 } else { cap.used as f64 / cap.cap as f64 };
    let color_var = match data::bar_color(ratio) {
        BarColor::Accent => "var(--accent)",
        BarColor::Amber => "var(--amber)",
        BarColor::Red => "var(--red)",
    };
    let fill_pct = (ratio * 100.0).clamp(0.0, 100.0);
    let label = {
        let mut chars = cap.dimension.chars();
        chars.next().map(|c| c.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
    };

    view! {
        <div style="margin-top:10px">
            <div class="row" style="justify-content:space-between">
                <span class="card-sub">{label}</span>
                <span class="card-sub mono">
                    {format!("{} / {} ({}%)", data::fmt_num(cap.used), data::fmt_num(cap.cap), cap.pct)}
                </span>
            </div>
            <div style="height:6px;border-radius:3px;background:var(--surface-2);overflow:hidden;margin-top:4px">
                <div style=format!(
                    "height:100%;border-radius:3px;width:{fill_pct:.1}%;background:{color_var}"
                )></div>
            </div>
        </div>
    }
}

/// Plan card: plan + server status badges, period end, and the Owner's Upgrade / Manage billing buttons.
#[cfg(feature = "csr")]
#[component]
fn PlanCard() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let plan = move || model.with(|m| m.subscription.plan.clone());
    let status = move || model.with(|m| m.billing.status);
    let period_end = move || model.with(|m| m.subscription.current_period_end);
    let is_owner = move || model.with(|m| m.can_manage_billing());
    let busy = move || model.with(|m| m.pending.contains(&CallKind::BillingRedirect));

    view! {
        <div class="card billing-card">
            <div class="row" style="justify-content:space-between;align-items:center">
                <h3 class="card-title">"Plan"</h3>
                <span class="row" style="gap:6px">
                    <span class=move || if plan() == Plan::Pro { "badge badge-accent" } else { "badge badge-neutral" }>
                        {move || if plan() == Plan::Pro { "Pro" } else { "Free" }}
                    </span>
                    {move || status().map(|s| view! { <span class="badge">{s.label()}</span> })}
                </span>
            </div>

            <div class="billing-card-body">
                {move || period_end().map(|end| view! {
                    <p class="card-sub">{format!("Current period ends {}", end.format("%Y-%m-%d"))}</p>
                })}
            </div>

            <Show
                when=is_owner
                fallback=|| view! { <p class="card-sub" style="margin-top:12px">"Only the account Owner can change the plan."</p> }
            >
                <div class="row" style="justify-content:flex-end;gap:8px;margin-top:12px">
                    <Show when=move || plan() == Plan::Free fallback=|| ()>
                        <button
                            class="btn btn-primary btn-sm"
                            type="button"
                            disabled=busy
                            on:click=move |_| dispatch.run(Msg::Request(ApiCall::StartCheckout))
                        >
                            "Upgrade to Pro"
                        </button>
                    </Show>
                    <button
                        class="btn btn-secondary btn-sm"
                        type="button"
                        disabled=busy
                        on:click=move |_| dispatch.run(Msg::Request(ApiCall::OpenPortal))
                    >
                        "Manage billing"
                    </button>
                </div>
            </Show>
        </div>
    }
}

/// Where payment method and invoices live: Stripe's portal. We only show the customer id.
#[cfg(feature = "csr")]
#[component]
fn PortalCard() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let customer = move || model.with(|m| m.subscription.stripe_customer_id.clone());

    view! {
        <div class="card billing-card">
            <h3 class="card-title">"Payment method and invoices"</h3>
            <div class="billing-card-body">
                <p class="card-sub">
                    "Cards and invoices are handled by Stripe. Use Manage billing to update your payment method, \
                     download invoices or cancel."
                </p>
                <p class="card-sub mono" style="margin-top:6px;font-size:11px">{customer}</p>
            </div>
        </div>
    }
}
