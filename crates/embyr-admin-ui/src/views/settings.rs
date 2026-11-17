//! SettingsView — OIDC providers + Danger Zone.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! AC-011-02: OIDC providers list with enabled Toggle per provider.
//! AC-011-02: Add OIDC provider form dispatches OidcProviderAdded + PushToast on success.
//! AC-011-04: Toggle dispatches Msg::ToggleOidc(id).
//! AC-011-05: Danger Zone section visible only to Owner role.
//!
//! The Danger Zone is live: it follows `AppModel::can_delete_account()` (the signed-in user's role)
//! and deletes the account through `DELETE /account`. The OIDC section is still the V1 mock
//! (slice 12) and infers its role from the members list.

#[cfg(feature = "csr")]
use crate::components::primitives::{Modal, Toggle};
#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind, SignInCode};
#[cfg(feature = "csr")]
use crate::model::{confirms_account_delete, AppModel, OidcId, OidcProvider, Role, Toast, ToastId, ToastLevel};
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

    // The Danger Zone follows the signed-in user's real role (the server enforces it too).
    let can_delete_account = move || model.with(|m| m.can_delete_account());

    view! {
        <div class="page">
            <div class="page-head">
                <h1 class="page-title">"Account Settings"</h1>
                <p class="page-sub">"Manage account identity, authentication, and security"</p>
            </div>

            <OidcSection />

            <Show when=can_delete_account fallback=|| ()>
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

/// Danger Zone — Owner only (`SettingsView` mounts it only then). Account deletion re-authenticates
/// with password + second factor and the typed account name (`DELETE /account`). Transfer ownership
/// has no endpoint: promote another member to Owner, then demote yourself in Members (G-45).
#[cfg(feature = "csr")]
#[component]
fn DangerZoneSection() -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    view! {
        <div class="card" style="border-color: color-mix(in oklab, var(--red, #e11d48) 30%, var(--border))">
            <div class="card-head">
                <div class="col" style="gap: 2px">
                    <h3 style="color: var(--red, #e11d48)">"Danger Zone"</h3>
                    <span class="sub">"Owner-only, irreversible actions"</span>
                </div>
            </div>

            <div class="row" style="padding: 15px 20px; gap: 12px; align-items: flex-start">
                <div class="col" style="gap: 2px; flex: 1">
                    <span style="font-weight: 550; font-size: 13.5px">"Delete account"</span>
                    <span style="font-size: 12px; color: var(--text-3)">
                        "Cancels the subscription and schedules the account and all its databases for permanent deletion."
                    </span>
                </div>
                <button
                    class="btn btn-ghost"
                    type="button"
                    style="color: var(--red, #e11d48)"
                    on:click=move |_| dispatch.run(Msg::OpenDeleteAccount)
                >
                    "Delete account"
                </button>
            </div>
        </div>

        <DeleteAccountModal />
    }
}

/// Open state is `model.delete_account_modal_open`; the form (and the password / code typed into it)
/// unmounts with it, so none of it outlives the modal.
#[cfg(feature = "csr")]
#[component]
fn DeleteAccountModal() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let is_open = move || model.with(|m| m.delete_account_modal_open);

    view! {
        <Show when=is_open fallback=|| ()>
            <DeleteAccountForm />
        </Show>
    }
}

#[cfg(feature = "csr")]
#[component]
fn DeleteAccountForm() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let typed = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let code = RwSignal::new(String::new());
    let use_recovery = RwSignal::new(false);

    let account_name = move || model.with(|m| m.current_user.as_ref().map(|u| u.account_name.clone()).unwrap_or_default());
    let busy = move || model.with(|m| m.pending.contains(&CallKind::DeleteAccount));
    let error = move || model.with(|m| m.account_delete_error.clone());
    let ready = move || {
        confirms_account_delete(&typed.get(), &account_name())
            && !password.get().is_empty()
            && !code.get().trim().is_empty()
            && !busy()
    };

    let submit = move |_| {
        let code_value = code.get().trim().to_string();
        let code = if use_recovery.get() { SignInCode::Recovery(code_value) } else { SignInCode::Totp(code_value) };
        dispatch.run(Msg::Request(ApiCall::DeleteAccount {
            password: password.get(),
            code,
            confirm_account_name: typed.get(),
        }));
    };

    view! {
        <Modal title="Delete account?" on_close=Callback::new(move |_| dispatch.run(Msg::CloseDeleteAccount))>
            <p style="font-size: 13px; color: var(--text-2)">
                "This cancels your subscription, signs everyone out, disables every API key and deletes every database. \
                 The data is permanently purged after a grace period."
            </p>
            <div class="modal-field">
                <label class="modal-label">{move || format!("Type \"{}\" to confirm", account_name())}</label>
                <input class="modal-input" type="text" autocomplete="off"
                    prop:value=move || typed.get()
                    on:input=move |e| typed.set(event_target_value(&e))/>
            </div>
            <div class="modal-field">
                <label class="modal-label">"Your password"</label>
                <input class="modal-input" type="password" autocomplete="current-password"
                    prop:value=move || password.get()
                    on:input=move |e| password.set(event_target_value(&e))/>
            </div>
            <div class="modal-field">
                <label class="modal-label">
                    {move || if use_recovery.get() { "Recovery code" } else { "Authenticator code" }}
                </label>
                <input class="modal-input" type="text" autocomplete="one-time-code"
                    prop:value=move || code.get()
                    on:input=move |e| code.set(event_target_value(&e))/>
                <button class="btn btn-ghost" type="button" on:click=move |_| use_recovery.update(|v| *v = !*v)>
                    {move || if use_recovery.get() { "Use authenticator code" } else { "Use a recovery code" }}
                </button>
            </div>
            <Show when=move || error().is_some()>
                <span class="field-err" role="alert">{move || error().unwrap_or_default()}</span>
            </Show>
            <div class="modal-footer">
                <button class="btn btn-ghost" type="button" on:click=move |_| dispatch.run(Msg::CloseDeleteAccount)>
                    "Cancel"
                </button>
                <button
                    class="btn btn-ghost"
                    type="button"
                    style="color: var(--red, #e11d48)"
                    disabled=move || !ready()
                    on:click=submit
                >
                    {move || if busy() { "Deleting…" } else { "Delete account" }}
                </button>
            </div>
        </Modal>
    }
}

