//! SecretModal — shows a one-time plaintext secret (AC-AUW-20).
//!
//! Mounted once in the shell. The secret lives only in `model.revealed_secret`; "Done" dispatches
//! `Msg::CloseSecret`, which drops it. The backdrop deliberately does not close the modal: an
//! accidental click must not lose a key that can never be shown again.

/// Best-effort clipboard write through `navigator.clipboard.writeText` (called via `js_sys`: the
/// typed `web-sys` Clipboard API needs an unstable cfg). Does nothing where the API is absent
/// (plain-HTTP origins); the secret stays selectable in the modal.
#[cfg(feature = "csr")]
pub fn copy_to_clipboard(text: &str) {
    use wasm_bindgen::JsCast;
    let Some(window) = web_sys::window() else { return };
    let Ok(clipboard) = js_sys::Reflect::get(&window.navigator(), &"clipboard".into()) else { return };
    let Ok(write) = js_sys::Reflect::get(&clipboard, &"writeText".into()) else { return };
    if let Ok(write) = write.dyn_into::<js_sys::Function>() {
        let _ = write.call1(&clipboard, &text.into());
    }
}

#[cfg(feature = "csr")]
use crate::model::AppModel;
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
#[component]
pub fn SecretModal() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    view! {
        {move || {
            model.with(|m| m.revealed_secret.clone()).map(|revealed| {
                let secret = revealed.secret.clone();
                view! {
                    <div class="modal-backdrop">
                        <div class="modal" role="dialog" aria-modal="true">
                            <div class="modal-head">
                                <span class="modal-title">"Copy your new API key"</span>
                            </div>
                            <div class="modal-body">
                                <p style="color:var(--text-2);font-size:13.5px;line-height:1.55">
                                    "This key is shown only once. Store it now: it cannot be retrieved later."
                                </p>
                                <input
                                    class="input mono"
                                    type="text"
                                    readonly=true
                                    prop:value=revealed.secret
                                />
                                <div class="row modal-footer" style="justify-content:flex-end">
                                    <button
                                        class="btn btn-ghost"
                                        type="button"
                                        on:click=move |_| copy_to_clipboard(&secret)
                                    >"Copy"</button>
                                    <button
                                        class="btn btn-primary"
                                        type="button"
                                        on:click=move |_| dispatch.run(Msg::CloseSecret)
                                    >"I have stored it"</button>
                                </div>
                            </div>
                        </div>
                    </div>
                }
            })
        }}
    }
}
