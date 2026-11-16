//! ToastStack — renders `model.toasts` (G-09); mounted once at the app root so errors are
//! visible on every screen, including sign-in.

#[cfg(feature = "csr")]
use crate::model::{AppModel, ToastLevel};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
#[component]
pub fn ToastStack() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    view! {
        <div style="position:fixed;right:16px;bottom:16px;display:flex;flex-direction:column;gap:8px;z-index:1000">
            <For
                each=move || model.with(|m| m.toasts.clone())
                key=|toast| toast.id.clone()
                children=move |toast| {
                    let id = toast.id.clone();
                    let badge = match toast.level {
                        ToastLevel::Error => "badge badge-red",
                        _ => "badge",
                    };
                    view! {
                        <div class="card" role="alert" style="display:flex;gap:10px;align-items:center;padding:10px 14px">
                            <span class=badge>{toast.message.clone()}</span>
                            <button
                                class="btn btn-ghost"
                                type="button"
                                aria-label="Dismiss"
                                on:click=move |_| dispatch.run(Msg::DismissToast(id.clone()))
                            >"×"</button>
                        </div>
                    }
                }
            />
        </div>
    }
}
