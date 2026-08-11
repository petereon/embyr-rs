//! ApiKeysView — account-level admin API keys table + revoke flow.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! AC-010-05: AdminKeyCreated appends the key (modal dispatches Msg::AdminKeyCreated).
//! AC-010-06: Revoke button dispatches Msg::RevokeAdminKey.

#[cfg(feature = "csr")]
use crate::components::primitives::Modal;
#[cfg(feature = "csr")]
use crate::model::{AdminKey, AppModel, KeyId, Role};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use uuid::Uuid;

// ── ApiKeysView ──────────────────────────────────────────────────────────────

/// Admin API Keys page.
///
/// Renders a table of account-level admin keys. A "Create Admin Key" button
/// opens a modal that dispatches `Msg::AdminKeyCreated`. Each row has a Revoke
/// button that dispatches `Msg::RevokeAdminKey`.
#[cfg(feature = "csr")]
#[component]
pub fn ApiKeysView() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let create_open = RwSignal::new(false);
    let key_name = RwSignal::new(String::new());

    let on_create = move |_| {
        let name = key_name.get();
        if name.trim().is_empty() {
            return;
        }
        let key = AdminKey {
            id: KeyId(Uuid::new_v4()),
            name: name.trim().to_string(),
            service_account_id: None,
            member_id: None,
            role: Role::Viewer,
            prefix: "embyr_ad".to_string(),
            created_at: None,
        };
        dispatch.run(Msg::AdminKeyCreated(key));
        create_open.set(false);
        key_name.set(String::new());
    };

    view! {
        <div class="page page-wide">
            <div class="page-head">
                <div>
                    <h1 class="page-title">"API Keys"</h1>
                    <p class="page-sub">
                        "Account-level admin keys for programmatic management"
                    </p>
                </div>
                <div class="page-head-actions">
                    <button
                        class="btn btn-primary"
                        type="button"
                        on:click=move |_| create_open.set(true)
                    >
                        "Create Admin Key"
                    </button>
                </div>
            </div>

            <div class="tbl-wrap">
                <table class="tbl">
                    <thead>
                        <tr>
                            <th>"Name"</th>
                            <th>"Key Prefix"</th>
                            <th>"Created"</th>
                            <th>"Actions"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {move || {
                            let keys = model.with(|m| m.admin_keys.clone());
                            keys.into_iter()
                                .map(|key| render_admin_key_row(key, dispatch))
                                .collect_view()
                        }}
                    </tbody>
                </table>
            </div>

            <Show when=move || create_open.get() fallback=|| ()>
                <Modal
                    title="Create admin API key"
                    on_close=Callback::new(move |_| create_open.set(false))
                >
                    <div class="modal-field">
                        <label class="modal-label">"Key name"</label>
                        <input
                            class="modal-input"
                            type="text"
                            placeholder="terraform"
                            prop:value=key_name
                            on:input=move |e| key_name.set(event_target_value(&e))
                        />
                    </div>
                    <div class="modal-footer">
                        <button
                            class="btn btn-ghost"
                            type="button"
                            on:click=move |_| create_open.set(false)
                        >
                            "Cancel"
                        </button>
                        <button
                            class="btn btn-primary"
                            type="button"
                            on:click=on_create
                        >
                            "Create key"
                        </button>
                    </div>
                </Modal>
            </Show>
        </div>
    }
}

/// Render a single admin key row.
///
/// Extracted to keep the reactive block readable.
#[cfg(feature = "csr")]
fn render_admin_key_row(key: AdminKey, dispatch: Callback<Msg>) -> impl IntoView {
    let key_id = key.id.clone();
    let created_display = key
        .created_at
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "—".to_string());

    view! {
        <tr>
            <td>{key.name.clone()}</td>
            <td><span class="tag">{key.prefix.clone()}</span></td>
            <td>{created_display}</td>
            <td>
                <button
                    class="btn btn-ghost"
                    type="button"
                    on:click=move |_| dispatch.run(Msg::RevokeAdminKey(key_id.clone()))
                >
                    "Revoke"
                </button>
            </td>
        </tr>
    }
}
