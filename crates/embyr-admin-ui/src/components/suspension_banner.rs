//! SuspensionBanner — cross-cutting read-only banner (ADR-019, D-6).
//!
//! Peer of `Sidebar`/`Topbar` (mounted once in `ShellView`, above the routed
//! content), not nested under `views/billing/`. Reads
//! `AppModel::suspension_banner_view()` via context and renders nothing
//! when `None`. The CTA dispatches `Msg::OpenUpgradeModal` or
//! `Msg::OpenCardModal` directly — no intermediate navigation to Billing.

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use crate::model::AppModel;
#[cfg(feature = "csr")]
use crate::msg::Msg;

#[cfg(feature = "csr")]
#[component]
pub fn SuspensionBanner() -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

    view! {
        {move || model.with(|m| m.suspension_banner_view()).map(|banner| {
            let css_class = if banner.cta_opens_upgrade_modal {
                "suspension-banner suspension-banner-amber"
            } else {
                "suspension-banner suspension-banner-red"
            };
            view! {
                <div class=css_class>
                    <span class="suspension-banner-message">{banner.message}</span>
                    <button
                        class="suspension-banner-cta"
                        on:click=move |_| {
                            if banner.cta_opens_upgrade_modal {
                                dispatch.run(Msg::OpenUpgradeModal);
                            } else {
                                dispatch.run(Msg::OpenCardModal);
                            }
                        }
                    >
                        {banner.cta_label}
                    </button>
                </div>
            }
        })}
    }
}
