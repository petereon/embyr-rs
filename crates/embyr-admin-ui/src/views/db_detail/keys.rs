//! Keys tab — SDK API key management panel.
//!
//! AC-006-02: Create modal dispatches Msg::SdkKeyCreated (server-side key generation).
//! AC-006-04: Revoke confirmation modal dispatches Msg::RevokeSdkKey.
//!
//! Gated behind #[cfg(feature = "csr")].

#[cfg(feature = "csr")]
use crate::model::{AppModel, Database, KeyId};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

/// SDK API key management panel for the Keys tab.
///
/// Displays a table of existing SDK keys for the given database.
/// "New SDK key" opens a create modal (V1 placeholder — key creation is
/// a server round-trip outside the SPA scope).
/// "Revoke" opens a confirmation modal before dispatching `Msg::RevokeSdkKey`.
#[cfg(feature = "csr")]
#[component]
pub fn KeysView(db: Database) -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let db_id = db.id.clone();
    let db_id_for_keys = db_id.clone();

    // Local modal visibility signals.
    let show_create = RwSignal::new(false);
    let revoke_target: RwSignal<Option<KeyId>> = RwSignal::new(None);

    // Derive the key list reactively from the model.
    let sdk_keys =
        move || model.with(|m| m.sdk_keys.get(&db_id_for_keys).cloned().unwrap_or_default());

    view! {
        <div class="keys-panel fade-in">
            // Section header + "New SDK key" action.
            <div class="row keys-header">
                <div class="col">
                    <span class="keys-title">"SDK API keys"</span>
                    <span class="sub">
                        "Bearer tokens authorising Firestore SDK clients to this database"
                    </span>
                </div>
                <button
                    class="btn btn-primary"
                    type="button"
                    on:click=move |_| show_create.set(true)
                >
                    "+ New SDK key"
                </button>
            </div>

            // Keys table or empty state.
            {move || {
                let keys = sdk_keys();
                if keys.is_empty() {
                    view! {
                        <div class="card">
                            <div class="empty">
                                <p>"No SDK keys yet. Create one to let SDK clients connect."</p>
                            </div>
                        </div>
                    }.into_any()
                } else {
                    let rows = keys.into_iter().map(|k| {
                        let key_id = k.id.clone();
                        view! {
                            <tr>
                                <td class="td-name">{k.name}</td>
                                <td><span class="tag">{k.prefix}</span></td>
                                <td class="td-dim">"—"</td>
                                <td class="td-actions">
                                    <button
                                        class="btn btn-ghost"
                                        type="button"
                                        on:click=move |_| revoke_target.set(Some(key_id.clone()))
                                    >
                                        "Revoke"
                                    </button>
                                </td>
                            </tr>
                        }
                    }).collect_view();
                    view! {
                        <div class="tbl-wrap">
                            <table class="tbl">
                                <thead>
                                    <tr>
                                        <th>"Name"</th>
                                        <th>"Prefix"</th>
                                        <th>"Created"</th>
                                        <th></th>
                                    </tr>
                                </thead>
                                <tbody>{rows}</tbody>
                            </table>
                        </div>
                    }.into_any()
                }
            }}

            // Create key modal (V1: placeholder — real key is generated server-side).
            {move || show_create.get().then(|| view! {
                <div class="modal-backdrop" on:click=move |_| show_create.set(false)>
                    <div class="modal" on:click=move |e: web_sys::MouseEvent| e.stop_propagation()>
                        <div class="modal-head">
                            <span class="modal-title">"Create SDK API key"</span>
                            <button
                                class="modal-close"
                                type="button"
                                on:click=move |_| show_create.set(false)
                            >
                                "✕"
                            </button>
                        </div>
                        <div class="modal-body">
                            <p>
                                "Key generation is handled server-side. \
                                 Use the API or CLI to create a key — it will appear here after creation."
                            </p>
                            <div class="row modal-footer">
                                <button
                                    class="btn btn-ghost"
                                    type="button"
                                    on:click=move |_| show_create.set(false)
                                >
                                    "Close"
                                </button>
                            </div>
                        </div>
                    </div>
                </div>
            })}

            // Confirm revoke modal — shown when revoke_target is Some(key_id).
            {move || {
                let target = revoke_target.get();
                if let Some(key_id) = target {
                    let db_id_revoke = db_id.clone();
                    Some(view! {
                        <div class="modal-backdrop" on:click=move |_| revoke_target.set(None)>
                            <div
                                class="modal"
                                on:click=move |e: web_sys::MouseEvent| e.stop_propagation()
                            >
                                <div class="modal-head">
                                    <span class="modal-title">"Revoke SDK key?"</span>
                                    <button
                                        class="modal-close"
                                        type="button"
                                        on:click=move |_| revoke_target.set(None)
                                    >
                                        "✕"
                                    </button>
                                </div>
                                <div class="modal-body">
                                    <p>
                                        "This key will no longer authenticate SDK clients. \
                                         In-flight requests are rejected on next validation. \
                                         This cannot be undone."
                                    </p>
                                    <div class="row modal-footer">
                                        <button
                                            class="btn btn-ghost"
                                            type="button"
                                            on:click=move |_| revoke_target.set(None)
                                        >
                                            "Cancel"
                                        </button>
                                        <button
                                            class="btn btn-danger"
                                            type="button"
                                            on:click=move |_| {
                                                dispatch.run(Msg::RevokeSdkKey(
                                                    db_id_revoke.clone(),
                                                    key_id.clone(),
                                                ));
                                                revoke_target.set(None);
                                            }
                                        >
                                            "Revoke key"
                                        </button>
                                    </div>
                                </div>
                            </div>
                        </div>
                    })
                } else {
                    None
                }
            }}
        </div>
    }
}
