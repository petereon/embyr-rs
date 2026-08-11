//! CardModal / UpgradeModal — global-state modal shells (ADR-019).
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! Mounted at the `ShellView` level (see `views/mod.rs`), not nested inside
//! `BillingView` — their visibility is driven by `AppModel.card_modal_open`
//! / `AppModel.upgrade_modal_open`, global state so a cross-cutting caller
//! (SuspensionBanner, Slice 07) can open either modal from any Section.
//!
//! AC-101-06: this slice wires the open/close plumbing only. Full form
//! content (Stripe-Elements-styled capture, compare/confirm-downgrade
//! steps) lands in Slices 04/05.

#[cfg(feature = "csr")]
use crate::components::primitives::Modal;
#[cfg(feature = "csr")]
use crate::model::AppModel;
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

/// Card capture modal shell. Renders only when `card_modal_open` is true.
#[cfg(feature = "csr")]
#[component]
pub fn CardModal() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let is_open = move || model.with(|m| m.card_modal_open);
    let on_close = Callback::new(move |_: ()| dispatch.run(Msg::CloseCardModal));

    view! {
        {move || is_open().then(|| view! {
            <Modal title="Add payment method" on_close=on_close>
                <p class="card-sub">
                    "Card details are tokenized by Stripe Elements — embyr never sees the raw number (PCI SAQ-A)."
                </p>
            </Modal>
        })}
    }
}

/// Plan-change modal shell. Renders only when `upgrade_modal_open` is true.
#[cfg(feature = "csr")]
#[component]
pub fn UpgradeModal() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let is_open = move || model.with(|m| m.upgrade_modal_open);
    let on_close = Callback::new(move |_: ()| dispatch.run(Msg::CloseUpgradeModal));

    view! {
        {move || is_open().then(|| view! {
            <Modal title="Change plan" on_close=on_close>
                <p class="card-sub">"Compare Free and Pro plans."</p>
            </Modal>
        })}
    }
}
