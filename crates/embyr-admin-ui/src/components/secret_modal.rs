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
use crate::model::{AppModel, SecretKind};
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
                // An invitation is shared as a link: the bare token is of no use to the invitee.
                let (title, hint, shown) = match revealed.kind {
                    SecretKind::ApiKey => (
                        "Copy your new API key".to_string(),
                        "This key is shown only once. Store it now: it cannot be retrieved later.".to_string(),
                        revealed.secret.clone(),
                    ),
                    SecretKind::AdminKey => (
                        "Copy your new admin key".to_string(),
                        format!(
                            "Admin key '{}' is shown only once. Store it now: it cannot be retrieved later.",
                            revealed.label
                        ),
                        revealed.secret.clone(),
                    ),
                    SecretKind::InviteToken => {
                        let origin = web_sys::window().and_then(|w| w.location().origin().ok()).unwrap_or_default();
                        (
                            "Invitation created".to_string(),
                            format!(
                                "{} is emailed this link only if the server has mail delivery configured. \
                                 It is shown once and expires in 7 days: share it directly if needed.",
                                revealed.label
                            ),
                            crate::api::invite_link(&origin, &revealed.secret),
                        )
                    }
                };
                let secret = shown.clone();
                view! {
                    <div class="modal-backdrop">
                        <div class="modal" role="dialog" aria-modal="true">
                            <div class="modal-head">
                                <span class="modal-title">{title}</span>
                            </div>
                            <div class="modal-body">
                                <p style="color:var(--text-2);font-size:13.5px;line-height:1.55">
                                    {hint}
                                </p>
                                <input
                                    class="input mono"
                                    type="text"
                                    readonly=true
                                    prop:value=shown
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
