//! SuspensionBanner — cross-cutting billing-hold banner (ADR-019, D-6).
//!
//! Peer of `Sidebar`/`Topbar` (mounted once in `ShellView`, above the routed content). Reads
//! `AppModel::suspension_banner_view()`, which follows the server's subscription status, and renders
//! nothing when `None`. The CTA goes straight to Stripe: Checkout for a Free-plan limit, the billing
//! portal for a failed payment. Only the Owner may do either, so everyone else gets a pointer instead
//! of a button.

#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind};
#[cfg(feature = "csr")]
use crate::model::AppModel;
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
#[component]
pub fn SuspensionBanner() -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

    view! {
        {move || model.with(|m| m.suspension_banner_view()).map(|banner| {
            let css_class = if banner.cta_starts_checkout {
                "suspension-banner suspension-banner-amber"
            } else {
                "suspension-banner suspension-banner-red"
            };
            let is_owner = model.with(|m| m.can_manage_billing());
            let busy = move || model.with(|m| m.pending.contains(&CallKind::BillingRedirect));
            view! {
                <div class=css_class role="alert">
                    <span class="suspension-banner-message">{banner.message}</span>
                    {if is_owner {
                        view! {
                            <button
                                class="suspension-banner-cta"
                                disabled=busy
                                on:click=move |_| {
                                    dispatch.run(Msg::Request(if banner.cta_starts_checkout {
                                        ApiCall::StartCheckout
                                    } else {
                                        ApiCall::OpenPortal
                                    }));
                                }
                            >
                                {banner.cta_label}
                            </button>
                        }.into_any()
                    } else {
                        view! { <span class="suspension-banner-message">"Ask your account Owner to resolve this."</span> }.into_any()
                    }}
                </div>
            }
        })}
    }
}
