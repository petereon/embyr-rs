//! ApiKeysView — account-level admin API keys: list, create (plaintext shown once), revoke.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! Owner/Admin only for writes (Viewers see disabled controls); the role a key can carry is capped
//! at the creator's own. A key may be bound to a service account.

#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind};
#[cfg(feature = "csr")]
use crate::components::primitives::Modal;
#[cfg(feature = "csr")]
use crate::model::{AdminKey, AppModel, ServiceAccountId};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use crate::views::identities::{role_from_label, role_label, role_options};
#[cfg(feature = "csr")]
use leptos::prelude::*;

/// Admin API Keys page.
#[cfg(feature = "csr")]
#[component]
pub fn ApiKeysView() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let create_open = RwSignal::new(false);
    let key_name = RwSignal::new(String::new());
    let key_role = RwSignal::new(String::from("Viewer"));
    // Empty = an account key not bound to a service account; otherwise the service account's id.
    let key_sa = RwSignal::new(String::new());

    let can_manage = move || model.with(|m| m.can_manage_identities());
    let busy = move || model.with(|m| m.pending.contains(&CallKind::AdminKeyWrite));
    let loading = move || model.with(|m| m.pending.contains(&CallKind::AdminKeys));

    let on_create = move |_| {
        let name = key_name.get();
        if name.trim().is_empty() {
            return;
        }
        let service_account_id = uuid::Uuid::parse_str(&key_sa.get()).ok().map(ServiceAccountId);
        dispatch.run(Msg::Request(ApiCall::CreateAdminKey {
            name,
            role: role_from_label(&key_role.get()),
            service_account_id,
        }));
        create_open.set(false);
        key_name.set(String::new());
        key_role.set(String::from("Viewer"));
        key_sa.set(String::new());
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
                        class="btn btn-ghost"
                        type="button"
                        disabled=loading
                        on:click=move |_| { dispatch.run(Msg::Request(ApiCall::LoadAdminKeys)); }
                    >
                        "Refresh"
                    </button>
                    <button
                        class="btn btn-primary"
                        type="button"
                        disabled=move || !can_manage() || busy()
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
                            <th>"Role"</th>
                            <th>"Key Prefix"</th>
                            <th>"Created"</th>
                            <th>"Actions"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {move || {
                            let keys = model.with(|m| m.admin_keys.clone());
                            if keys.is_empty() {
                                return view! {
                                    <tr><td colspan="5" class="empty-state">
                                        {if loading() { "Loading…" } else { "No admin keys yet." }}
                                    </td></tr>
                                }.into_any();
                            }
                            keys.into_iter()
                                .map(|key| render_admin_key_row(key, can_manage, busy, dispatch))
                                .collect_view()
                                .into_any()
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
                    <div class="modal-field">
                        <label class="modal-label">"Role"</label>
                        <select
                            class="modal-select"
                            prop:value=key_role
                            on:change=move |e| key_role.set(event_target_value(&e))
                        >
                            {role_options(model)}
                        </select>
                    </div>
                    <div class="modal-field">
                        <label class="modal-label">"Service account (optional)"</label>
                        <select
                            class="modal-select"
                            prop:value=key_sa
                            on:change=move |e| key_sa.set(event_target_value(&e))
                        >
                            <option value="">"None"</option>
                            {move || model.with(|m| m.service_accounts.clone()).into_iter().map(|sa| {
                                view! { <option value=sa.id.0.to_string()>{sa.name}</option> }
                            }).collect_view()}
                        </select>
                    </div>
                    <div class="modal-footer">
                        <button class="btn btn-ghost" type="button" on:click=move |_| create_open.set(false)>
                            "Cancel"
                        </button>
                        <button class="btn btn-primary" type="button" on:click=on_create>
                            "Create key"
                        </button>
                    </div>
                </Modal>
            </Show>
        </div>
    }
}

/// One admin key row.
#[cfg(feature = "csr")]
fn render_admin_key_row(
    key: AdminKey,
    can_manage: impl Fn() -> bool + Copy + Send + Sync + 'static,
    busy: impl Fn() -> bool + Copy + Send + Sync + 'static,
    dispatch: Callback<Msg>,
) -> impl IntoView {
    let key_id = key.id.clone();
    let created_display = key
        .created_at
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "—".to_string());

    view! {
        <tr>
            <td>{key.name.clone()}</td>
            <td>{role_label(&key.role)}</td>
            <td><span class="tag">{key.prefix.clone()}</span></td>
            <td>{created_display}</td>
            <td>
                <button
                    class="btn btn-ghost"
                    type="button"
                    disabled=move || !can_manage() || busy()
                    on:click=move |_| { dispatch.run(Msg::Request(ApiCall::RevokeAdminKey(key_id.clone()))); }
                >
                    "Revoke"
                </button>
            </td>
        </tr>
    }
}
