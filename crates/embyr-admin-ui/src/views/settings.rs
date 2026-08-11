//! SettingsView — OIDC providers + Danger Zone.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! AC-011-02: OIDC providers list with enabled Toggle per provider.
//! AC-011-02: Add OIDC provider form dispatches OidcProviderAdded + PushToast on success.
//! AC-011-04: Toggle dispatches Msg::ToggleOidc(id).
//! AC-011-05: Danger Zone section visible only to Owner role.
//!
//! Role gating:
//!   Owner  — full edit UI (Add form, Toggles, Danger Zone).
//!   Admin/Viewer — read-only OIDC list; no Add form, no Toggles, no Danger Zone.
//!
//! V1 note: There is no current_user field in AppModel. The role is inferred
//! from the presence of at least one Owner member in the list (V1 simplification).
//! A real implementation would carry current_user_id in AppModel and look up the
//! member by id.

#[cfg(feature = "csr")]
use crate::components::primitives::{Modal, Toggle};
#[cfg(feature = "csr")]
use crate::model::{AppModel, OidcId, OidcProvider, Role, Toast, ToastId, ToastLevel};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use uuid::Uuid;

// ── SettingsView ─────────────────────────────────────────────────────────────

/// Account settings page — OIDC providers and Danger Zone.
///
/// Reads `model.oidc_providers` for the OIDC list.
/// Dispatches `Msg::OidcProviderAdded`, `Msg::ToggleOidc`, `Msg::PushToast`.
/// Danger Zone gated to Owner role.
#[cfg(feature = "csr")]
#[component]
pub fn SettingsView() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

    // V1 role inference: treat as Owner when at least one Owner member exists.
    let is_owner =
        move || model.with(|m| m.members.iter().any(|member| member.role == Role::Owner));

    view! {
        <div class="page">
            <div class="page-head">
                <h1 class="page-title">"Account Settings"</h1>
                <p class="page-sub">"Manage account identity, authentication, and security"</p>
            </div>

            <OidcSection />

            <Show when=is_owner fallback=|| ()>
                <DangerZoneSection />
            </Show>
        </div>
    }
}

// ── OidcSection ──────────────────────────────────────────────────────────────

/// OIDC providers section.
///
/// Owner view: list of providers with Toggle + "Add OIDC Provider" form.
/// Admin/Viewer view: read-only list (no Toggle, no Add form).
#[cfg(feature = "csr")]
#[component]
fn OidcSection() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    // V1 role inference — same as SettingsView.
    let is_owner =
        move || model.with(|m| m.members.iter().any(|member| member.role == Role::Owner));

    // Local form state for the Add OIDC Provider form.
    let form_issuer = RwSignal::new(String::new());
    let form_client_id = RwSignal::new(String::new());
    let form_open = RwSignal::new(false);

    let on_add_submit = move |_| {
        let issuer = form_issuer.get();
        let client_id = form_client_id.get();
        if issuer.trim().is_empty() || client_id.trim().is_empty() {
            return;
        }
        let provider = OidcProvider {
            id: OidcId(Uuid::new_v4()),
            issuer: issuer.trim().to_string(),
            client_id: client_id.trim().to_string(),
            enabled: true,
        };
        dispatch.run(Msg::OidcProviderAdded(provider));
        dispatch.run(Msg::PushToast(Toast {
            id: ToastId(Uuid::new_v4()),
            message: "OIDC provider added".to_string(),
            level: ToastLevel::Info,
        }));
        form_open.set(false);
        form_issuer.set(String::new());
        form_client_id.set(String::new());
    };

    view! {
        <div class="card" style="margin-bottom: 18px">
            <div class="card-head">
                <div class="col" style="gap: 2px">
                    <h3>"OIDC Providers"</h3>
                    <span class="sub">"Single sign-on via trusted identity providers"</span>
                </div>
                <Show when=is_owner fallback=|| ()>
                    <button
                        class="btn btn-primary"
                        type="button"
                        on:click=move |_| form_open.set(true)
                    >
                        "Add OIDC Provider"
                    </button>
                </Show>
            </div>

            <div class="card-pad">
                {move || {
                    let providers = model.with(|m| m.oidc_providers.clone());
                    let owner = is_owner();
                    if providers.is_empty() {
                        view! {
                            <p class="empty-state">"No OIDC providers configured."</p>
                        }.into_any()
                    } else {
                        providers
                            .into_iter()
                            .map(|p| render_oidc_row(p, owner, dispatch))
                            .collect_view()
                            .into_any()
                    }
                }}
            </div>
        </div>

        <Show when=move || form_open.get() fallback=|| ()>
            <Modal
                title="Add OIDC Provider"
                on_close=Callback::new(move |_| form_open.set(false))
            >
                <div class="modal-field">
                    <label class="modal-label">"Issuer URL"</label>
                    <input
                        class="modal-input"
                        type="url"
                        placeholder="https://accounts.example.com"
                        prop:value=form_issuer
                        on:input=move |e| form_issuer.set(event_target_value(&e))
                    />
                </div>
                <div class="modal-field">
                    <label class="modal-label">"Client ID"</label>
                    <input
                        class="modal-input"
                        type="text"
                        placeholder="client-abc-123"
                        prop:value=form_client_id
                        on:input=move |e| form_client_id.set(event_target_value(&e))
                    />
                </div>
                <div class="modal-footer">
                    <button
                        class="btn btn-ghost"
                        type="button"
                        on:click=move |_| form_open.set(false)
                    >
                        "Cancel"
                    </button>
                    <button
                        class="btn btn-primary"
                        type="button"
                        on:click=on_add_submit
                    >
                        "Add provider"
                    </button>
                </div>
            </Modal>
        </Show>
    }
}

/// Render a single OIDC provider row.
///
/// Owner: shows Toggle that dispatches `Msg::ToggleOidc(id)`.
/// Admin/Viewer: shows read-only enabled/disabled badge.
///
/// `is_owner` is a plain bool resolved once at render time. Since the
/// members list rarely changes, a full re-render of the OIDC list handles
/// any role change correctly.
#[cfg(feature = "csr")]
fn render_oidc_row(
    provider: OidcProvider,
    is_owner: bool,
    dispatch: Callback<Msg>,
) -> impl IntoView {
    let enabled = provider.enabled;
    let issuer = provider.issuer.clone();
    let client_id = provider.client_id.clone();
    let enabled_label = if enabled { "enabled" } else { "disabled" };

    // Resolve the toggle/badge widget outside the reactive closure to avoid
    // moving `provider.id` into a `move ||` that would become `FnOnce`.
    let action_widget = if is_owner {
        let toggle_id = provider.id.clone();
        view! {
            <Toggle
                checked=enabled
                on_change=Callback::new(move |_| {
                    dispatch.run(Msg::ToggleOidc(toggle_id.clone()))
                })
            />
        }
        .into_any()
    } else {
        view! {
            <span class="badge">{enabled_label}</span>
        }
        .into_any()
    };

    view! {
        <div class="row" style="padding: 11px 0; border-top: 1px solid var(--border); gap: 12px; align-items: center">
            <div class="col" style="gap: 1px; flex: 1">
                <span class="mono" style="font-size: 13px; font-weight: 530">
                    {client_id}
                </span>
                <span class="mono" style="font-size: 11px; color: var(--text-3)">
                    {issuer}
                </span>
            </div>
            {action_widget}
        </div>
    }
}

// ── DangerZoneSection ────────────────────────────────────────────────────────

/// Danger Zone — Owner only.
///
/// Renders "Transfer Ownership" and "Delete Account" actions with confirmation
/// modals. V1: both show placeholder modals that dispatch PushToast.
#[cfg(feature = "csr")]
#[component]
fn DangerZoneSection() -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let transfer_open = RwSignal::new(false);
    let delete_open = RwSignal::new(false);
    let delete_confirm_text = RwSignal::new(String::new());

    let on_transfer = move |_| {
        transfer_open.set(false);
        dispatch.run(Msg::PushToast(Toast {
            id: ToastId(Uuid::new_v4()),
            message: "Ownership transfer initiated (V1 mock)".to_string(),
            level: ToastLevel::Warning,
        }));
    };

    let on_delete = move |_| {
        delete_open.set(false);
        dispatch.run(Msg::PushToast(Toast {
            id: ToastId(Uuid::new_v4()),
            message: "Account deletion is disabled in this demo".to_string(),
            level: ToastLevel::Error,
        }));
    };

    view! {
        <div class="card" style="border-color: color-mix(in oklab, var(--red, #e11d48) 30%, var(--border))">
            <div class="card-head">
                <div class="col" style="gap: 2px">
                    <h3 style="color: var(--red, #e11d48)">"Danger Zone"</h3>
                    <span class="sub">"Owner-only, irreversible actions"</span>
                </div>
            </div>

            <div style="display: flex; flex-direction: column">
                <div class="row" style="padding: 15px 20px; border-bottom: 1px solid var(--border); gap: 12px; align-items: flex-start">
                    <div class="col" style="gap: 2px; flex: 1">
                        <span style="font-weight: 550; font-size: 13.5px">"Transfer ownership"</span>
                        <span style="font-size: 12px; color: var(--text-3)">"Hand the Owner role to another member."</span>
                    </div>
                    <button
                        class="btn btn-ghost"
                        type="button"
                        on:click=move |_| transfer_open.set(true)
                    >
                        "Transfer"
                    </button>
                </div>

                <div class="row" style="padding: 15px 20px; gap: 12px; align-items: flex-start">
                    <div class="col" style="gap: 2px; flex: 1">
                        <span style="font-weight: 550; font-size: 13.5px">"Delete account"</span>
                        <span style="font-size: 12px; color: var(--text-3)">"Permanently delete this account and all databases."</span>
                    </div>
                    <button
                        class="btn btn-ghost"
                        type="button"
                        style="color: var(--red, #e11d48)"
                        on:click=move |_| {
                            delete_open.set(true);
                            delete_confirm_text.set(String::new());
                        }
                    >
                        "Delete account"
                    </button>
                </div>
            </div>
        </div>

        <Show when=move || transfer_open.get() fallback=|| ()>
            <Modal
                title="Transfer Ownership"
                on_close=Callback::new(move |_| transfer_open.set(false))
            >
                <p style="font-size: 13px; color: var(--text-2)">"Select a member to become the new Owner. This action cannot be undone."</p>
                <div class="modal-footer">
                    <button
                        class="btn btn-ghost"
                        type="button"
                        on:click=move |_| transfer_open.set(false)
                    >
                        "Cancel"
                    </button>
                    <button
                        class="btn btn-primary"
                        type="button"
                        on:click=on_transfer
                    >
                        "Transfer (V1 mock)"
                    </button>
                </div>
            </Modal>
        </Show>

        <Show when=move || delete_open.get() fallback=|| ()>
            <Modal
                title="Delete Account?"
                on_close=Callback::new(move |_| delete_open.set(false))
            >
                <p style="font-size: 13px; color: var(--text-2)">"This permanently deletes the account, every database, and all keys. There is no recovery."</p>
                <div class="modal-field">
                    <label class="modal-label">"Type the account name to confirm"</label>
                    <input
                        class="modal-input"
                        type="text"
                        placeholder="account name"
                        prop:value=delete_confirm_text
                        on:input=move |e| delete_confirm_text.set(event_target_value(&e))
                    />
                </div>
                <div class="modal-footer">
                    <button
                        class="btn btn-ghost"
                        type="button"
                        on:click=move |_| delete_open.set(false)
                    >
                        "Cancel"
                    </button>
                    <button
                        class="btn btn-ghost"
                        type="button"
                        style="color: var(--red, #e11d48)"
                        on:click=on_delete
                    >
                        "Delete forever (V1 mock)"
                    </button>
                </div>
            </Modal>
        </Show>
    }
}
