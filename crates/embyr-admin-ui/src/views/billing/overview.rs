//! BillingOverviewTab — Plan card + Payment Method card.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! AC-101-01/02/03: PlanCard renders a Free badge + included volume per
//! dimension (sourced from `AppModel::plan_summary()`), or a Pro badge +
//! base price + renewal date.
//! AC-101-04/05/07: PaymentMethodCard renders brand/last4/expiry + "On
//! file" badge when a card is present, or "No card on file" + Add card CTA
//! when absent, plus the `stripeCustomerId`.
//! AC-101-06: clicking the trigger buttons dispatches `Msg::OpenUpgradeModal`
//! / `Msg::OpenCardModal` (ADR-019 global modal state).
//!
//! AC-102-01/02/03: CapUsageCard renders 4 per-dimension bars (reads/writes/
//! deletes/storage) sourced from `AppModel::cap_ratios()`, colored via
//! `data::bar_color()` (accent/amber/red thresholds).
//!
//! NextInvoiceCard / TestClockCard (Slices 03/08) are not part of this
//! slice — see feature-delta.md Component Decomposition.

#[cfg(feature = "csr")]
use crate::data::{self, BarColor};
#[cfg(feature = "csr")]
use crate::model::{AppModel, Plan};
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
            <PaymentMethodCard/>
            <CapUsageCard/>
        </div>
    }
}

/// Cap Usage card: reads/writes/deletes/storage bars as % of `FREE_CAPS`,
/// single-sourced from `AppModel::cap_ratios()` (AC-102-01/02/03).
#[cfg(feature = "csr")]
#[component]
fn CapUsageCard() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let ratios = move || model.with(|m| m.cap_ratios());

    view! {
        <div class="card billing-card">
            <h3 class="card-title">"Usage this month"</h3>
            <div class="billing-card-body">
                <CapUsageBar label="Reads" ratio=Signal::derive(move || ratios().reads)/>
                <CapUsageBar label="Writes" ratio=Signal::derive(move || ratios().writes)/>
                <CapUsageBar label="Deletes" ratio=Signal::derive(move || ratios().deletes)/>
                <CapUsageBar label="Storage" ratio=Signal::derive(move || ratios().storage_gb)/>
            </div>
        </div>
    }
}

/// A single cap-usage bar: label, percentage label, and a colored fill
/// track (AC-102-02: accent <80%, amber 80-99%, red >=100%, via
/// `data::bar_color()` — the single-sourced threshold logic).
#[cfg(feature = "csr")]
#[component]
fn CapUsageBar(label: &'static str, ratio: Signal<f64>) -> impl IntoView {
    let color_var = move || match data::bar_color(ratio.get()) {
        BarColor::Accent => "var(--accent)",
        BarColor::Amber => "var(--amber)",
        BarColor::Red => "var(--red)",
    };
    let fill_pct = move || (ratio.get() * 100.0).clamp(0.0, 100.0);
    let pct_label = move || format!("{:.0}%", ratio.get() * 100.0);

    view! {
        <div style="margin-top:10px">
            <div class="row" style="justify-content:space-between">
                <span class="card-sub">{label}</span>
                <span class="card-sub mono">{pct_label}</span>
            </div>
            <div style="height:6px;border-radius:3px;background:var(--surface-2);overflow:hidden;margin-top:4px">
                <div style=move || format!(
                    "height:100%;border-radius:3px;width:{:.1}%;background:{}",
                    fill_pct(), color_var()
                )></div>
            </div>
        </div>
    }
}

/// Plan card: Free included-volume summary, or Pro base price + renewal.
#[cfg(feature = "csr")]
#[component]
fn PlanCard() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let summary = move || model.with(|m| m.plan_summary());

    view! {
        <div class="card billing-card">
            <div class="row" style="justify-content:space-between;align-items:center">
                <h3 class="card-title">"Plan"</h3>
                {move || {
                    let is_pro = summary().plan == Plan::Pro;
                    view! {
                        <span class=if is_pro { "badge badge-accent" } else { "badge badge-neutral" }>
                            {if is_pro { "Pro" } else { "Free" }}
                        </span>
                    }
                }}
            </div>

            {move || {
                let s = summary();
                match s.included {
                    Some(included) => view! {
                        <div class="billing-card-body">
                            <p class="card-sub">"Included volume this month"</p>
                            <ul class="billing-included-list">
                                <li>"Reads: " {included.reads}</li>
                                <li>"Writes: " {included.writes}</li>
                                <li>"Deletes: " {included.deletes}</li>
                                <li>"Storage: " {format!("{:.1} GB", included.storage_gb)}</li>
                            </ul>
                        </div>
                    }.into_any(),
                    None => view! {
                        <div class="billing-card-body">
                            {s.base_price.map(|price| view! {
                                <p class="card-sub mono">{format!("${price:.2}/mo")}</p>
                            })}
                            {s.renews_at.map(|renews| view! {
                                <p class="card-sub">{format!("Renews {}", renews.format("%Y-%m-%d"))}</p>
                            })}
                        </div>
                    }.into_any(),
                }
            }}

            <div class="row" style="justify-content:flex-end;margin-top:12px">
                <button
                    class="btn btn-secondary btn-sm"
                    type="button"
                    on:click=move |_| dispatch.run(Msg::OpenUpgradeModal)
                >
                    {move || if summary().plan == Plan::Pro { "Change plan" } else { "Upgrade to Pro" }}
                </button>
            </div>
        </div>
    }
}

/// Payment Method card: card summary + "On file" badge, or "Add card" CTA.
#[cfg(feature = "csr")]
#[component]
fn PaymentMethodCard() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let summary = move || model.with(|m| m.payment_method_summary());

    view! {
        <div class="card billing-card">
            <div class="row" style="justify-content:space-between;align-items:center">
                <h3 class="card-title">"Payment method"</h3>
                {move || summary().on_file.then(|| view! {
                    <span class="badge badge-green">"On file"</span>
                })}
            </div>

            <div class="billing-card-body">
                {move || match summary().card {
                    Some(card) => view! {
                        <p class="mono">
                            {format!("{:?} •••• {} — {:02}/{}", card.brand, card.last4, card.exp_month, card.exp_year)}
                        </p>
                    }.into_any(),
                    None => view! {
                        <p class="card-sub">"No card on file"</p>
                    }.into_any(),
                }}
                <p class="card-sub mono" style="margin-top:6px;font-size:11px">
                    {move || summary().stripe_customer_id}
                </p>
            </div>

            <div class="row" style="justify-content:flex-end;margin-top:12px">
                <button
                    class="btn btn-secondary btn-sm"
                    type="button"
                    on:click=move |_| dispatch.run(Msg::OpenCardModal)
                >
                    {move || if summary().on_file { "Update" } else { "Add card" }}
                </button>
            </div>
        </div>
    }
}
