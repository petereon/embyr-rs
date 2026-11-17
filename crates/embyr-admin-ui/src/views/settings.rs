//! SettingsView — read-only account details + Danger Zone.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! The account card shows what `GET /auth/session` reports (account, signed-in user, role); it is
//! all any role can see here. The Danger Zone follows `AppModel::can_delete_account()` (Owner only)
//! and deletes the account through `DELETE /account`.
//!
//! There is no sign-in provider section: the OIDC callback is disabled server-side (audit #58), so
//! configuring providers would configure nothing. It returns with `admin-oidc-sso`.

#[cfg(feature = "csr")]
use crate::components::primitives::Modal;
#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind, SignInCode};
#[cfg(feature = "csr")]
use crate::model::{confirms_account_delete, AppModel};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

// ── SettingsView ─────────────────────────────────────────────────────────────

/// Account settings page.
#[cfg(feature = "csr")]
#[component]
pub fn SettingsView() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

    // The Danger Zone follows the signed-in user's real role (the server enforces it too).
    let can_delete_account = move || model.with(|m| m.can_delete_account());
    let me = move || model.with(|m| m.current_user.clone()).unwrap_or_default();

    view! {
        <div class="page">
            <div class="page-head">
                <h1 class="page-title">"Account Settings"</h1>
                <p class="page-sub">"Your account and sign-in"</p>
            </div>

            <div class="card" style="margin-bottom: 18px">
                <div class="card-head">
                    <div class="col" style="gap: 2px">
                        <h3>"Account"</h3>
                        <span class="sub">"As reported by the server for this session"</span>
                    </div>
                </div>
                <div class="card-pad">
                    <dl>
                        <dt>"Account"</dt>
                        <dd>{move || me().account_name}</dd>
                        <dt>"Signed in as"</dt>
                        <dd>{move || {
                            let u = me();
                            format!("{} ({})", u.display_name, u.email)
                        }}</dd>
                        <dt>"Role"</dt>
                        <dd>{move || format!("{:?}", me().role)}</dd>
                    </dl>
                </div>
            </div>

            <Show when=can_delete_account fallback=|| ()>
                <DangerZoneSection />
            </Show>
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

