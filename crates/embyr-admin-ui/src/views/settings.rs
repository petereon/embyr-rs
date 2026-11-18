//! SettingsView — read-only account details + Danger Zone.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! The account card shows what `GET /auth/session` reports (account, signed-in user, role); it is
//! all any role can see here. The Danger Zone follows `AppModel::can_delete_account()` (Owner only)
//! and deletes the account through `DELETE /account`.
//!
//! Single sign-on (`admin-oidc-sso`): the Owner manages the account's OIDC providers and shares the
//! `/admin/?sso=<provider id>` sign-in link; any member can link or unlink their own SSO identity
//! (re-entering password + authenticator code). Client secrets are write-only: typed into a field,
//! sent once, never shown again, and the fields are cleared when the server confirms.

#[cfg(feature = "csr")]
use crate::components::primitives::Modal;
#[cfg(feature = "csr")]
use crate::api::{
    sso_provider_from_input, sso_redirect_uri, sso_sign_in_link, ApiCall, CallKind, SignInCode, SsoIntent,
};
#[cfg(feature = "csr")]
use crate::model::{confirms_account_delete, AppModel, LinkControl, OidcProvider};
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
    let can_manage_sso = move || model.with(|m| m.can_manage_sso());
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

            <Show when=can_manage_sso fallback=|| ()>
                <SsoProvidersSection />
                <RequireSsoSection />
            </Show>

            <YourSsoSection />

            <Show when=can_delete_account fallback=|| ()>
                <DangerZoneSection />
            </Show>
        </div>
    }
}

#[cfg(feature = "csr")]
fn origin() -> String {
    web_sys::window().and_then(|w| w.location().origin().ok()).unwrap_or_default()
}

// ── SsoProvidersSection ──────────────────────────────────────────────────────

/// Single sign-on providers — Owner only (`SettingsView` mounts it only then).
#[cfg(feature = "csr")]
#[component]
fn SsoProvidersSection() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let issuer = RwSignal::new(String::new());
    let client_id = RwSignal::new(String::new());
    let secret = RwSignal::new(String::new());
    let rotating: RwSignal<Option<String>> = RwSignal::new(None);
    let rotate_secret = RwSignal::new(String::new());
    let confirm_delete: RwSignal<Option<String>> = RwSignal::new(None);

    let busy = move || model.with(|m| m.pending.contains(&CallKind::OidcProviderWrite));
    let ready = move || {
        !issuer.get().trim().is_empty() && !client_id.get().trim().is_empty() && !secret.get().is_empty() && !busy()
    };
    let redirect_uri = sso_redirect_uri(&origin());
    let redirect_copy = redirect_uri.clone();

    // The server confirmed a create or a rotation: the secret must not outlive it in the page.
    leptos::prelude::Effect::new(move |prev: Option<u32>| {
        let saves = model.with(|m| m.oidc_provider_saves);
        if prev.is_some_and(|p| p != saves) {
            issuer.set(String::new());
            client_id.set(String::new());
            secret.set(String::new());
            rotate_secret.set(String::new());
            rotating.set(None);
        }
        saves
    });

    let add = move |_| {
        dispatch.run(Msg::Request(ApiCall::CreateOidcProvider {
            issuer: issuer.get(),
            client_id: client_id.get(),
            client_secret: secret.get(),
        }));
    };
    let close_rotate = move || {
        rotating.set(None);
        rotate_secret.set(String::new());
    };

    view! {
        <div class="card" style="margin-bottom: 18px">
            <div class="card-head">
                <div class="col" style="gap: 2px">
                    <h3>"Single sign-on"</h3>
                    <span class="sub">"OpenID Connect providers for this account. SSO adds a way to sign in; it never creates accounts."</span>
                </div>
            </div>
            <div class="card-pad col" style="gap: 14px">
                <div class="field">
                    <label class="field-label">"Redirect URI to register at your identity provider"</label>
                    <div class="row" style="gap: 8px">
                        <input class="input mono" type="text" readonly=true prop:value=redirect_uri style="flex: 1"/>
                        <button class="btn btn-ghost" type="button"
                            on:click=move |_| crate::components::copy_to_clipboard(&redirect_copy)>"Copy"</button>
                    </div>
                </div>

                {move || {
                    let providers = model.with(|m| m.oidc_providers.clone());
                    if providers.is_empty() {
                        return view! { <p class="empty-state">"No provider configured yet."</p> }.into_any();
                    }
                    providers
                        .into_iter()
                        .map(|p| view! {
                            <ProviderRow provider=p rotating=rotating confirm_delete=confirm_delete busy=Signal::derive(busy) />
                        })
                        .collect_view()
                        .into_any()
                }}

                <div class="col" style="gap: 10px; border-top: 1px solid var(--border); padding-top: 14px">
                    <span style="font-weight: 550; font-size: 13.5px">"Add a provider"</span>
                    <div class="field">
                        <label class="field-label">"Issuer"</label>
                        <input class="input" type="url" autocomplete="off" placeholder="https://accounts.example.com"
                            prop:value=move || issuer.get()
                            on:input=move |e| issuer.set(event_target_value(&e))/>
                    </div>
                    <div class="field">
                        <label class="field-label">"Client ID"</label>
                        <input class="input" type="text" autocomplete="off"
                            prop:value=move || client_id.get()
                            on:input=move |e| client_id.set(event_target_value(&e))/>
                    </div>
                    <div class="field">
                        <label class="field-label">"Client secret"</label>
                        <input class="input" type="password" autocomplete="new-password"
                            prop:value=move || secret.get()
                            on:input=move |e| secret.set(event_target_value(&e))/>
                        <span style="font-size: 12px; color: var(--text-3)">"Stored encrypted. It is never shown again."</span>
                    </div>
                    <div class="row">
                        <button class="btn btn-primary" type="button" disabled=move || !ready() on:click=add>
                            {move || if busy() { "Saving…" } else { "Add provider" }}
                        </button>
                    </div>
                </div>
            </div>
        </div>

        <Show when=move || rotating.get().is_some() fallback=|| ()>
            <Modal title="Rotate client secret" on_close=Callback::new(move |_| close_rotate())>
                <p style="font-size: 13px; color: var(--text-2)">
                    "Enter the new client secret from your identity provider. The old one stops being used at once."
                </p>
                <div class="modal-field">
                    <label class="modal-label">"New client secret"</label>
                    <input class="modal-input" type="password" autocomplete="new-password"
                        prop:value=move || rotate_secret.get()
                        on:input=move |e| rotate_secret.set(event_target_value(&e))/>
                </div>
                <div class="modal-footer">
                    <button class="btn btn-ghost" type="button" on:click=move |_| close_rotate()>"Cancel"</button>
                    <button class="btn btn-primary" type="button"
                        disabled=move || rotate_secret.get().is_empty() || busy()
                        on:click=move |_| {
                            if let Some(id) = rotating.get_untracked() {
                                dispatch.run(Msg::Request(ApiCall::RotateOidcSecret {
                                    id,
                                    client_secret: rotate_secret.get_untracked(),
                                }));
                            }
                        }>
                        {move || if busy() { "Saving…" } else { "Save secret" }}
                    </button>
                </div>
            </Modal>
        </Show>
    }
}

/// One provider: status, the shareable sign-in link, and the Owner's actions.
#[cfg(feature = "csr")]
#[component]
fn ProviderRow(
    provider: OidcProvider,
    rotating: RwSignal<Option<String>>,
    confirm_delete: RwSignal<Option<String>>,
    busy: Signal<bool>,
) -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");
    let link = sso_sign_in_link(&origin(), &provider.id);
    let link_copy = link.clone();
    let id = provider.id.clone();
    let enabled = provider.enabled;
    let asking = {
        let id = id.clone();
        move || confirm_delete.get().as_deref() == Some(id.as_str())
    };

    view! {
        <div class="row" style="gap: 12px; align-items: flex-start; border-top: 1px solid var(--border); padding-top: 14px">
            <div class="col" style="gap: 6px; flex: 1; min-width: 0">
                <span style="font-weight: 550; font-size: 13.5px">
                    {provider.issuer.clone()}
                    <span class="badge badge-neutral" style="margin-left: 6px">{if enabled { "enabled" } else { "disabled" }}</span>
                </span>
                <span style="font-size: 12px; color: var(--text-3)">{format!("Client ID {}", provider.client_id)}</span>
                <div class="row" style="gap: 8px">
                    <input class="input mono" type="text" readonly=true prop:value=link style="flex: 1"
                        title="Share this sign-in link with your members"/>
                    <button class="btn btn-ghost" type="button"
                        on:click=move |_| crate::components::copy_to_clipboard(&link_copy)>"Copy link"</button>
                </div>
            </div>
            <div class="row" style="gap: 8px">
                <button class="btn btn-ghost" type="button" disabled=move || busy.get()
                    on:click={
                        let id = id.clone();
                        move |_| {
                            dispatch.run(Msg::Request(ApiCall::SetOidcProviderEnabled { id: id.clone(), enabled: !enabled }));
                        }
                    }>
                    {if enabled { "Disable" } else { "Enable" }}
                </button>
                <button class="btn btn-ghost" type="button"
                    on:click={
                        let id = id.clone();
                        move |_| rotating.set(Some(id.clone()))
                    }>
                    "Rotate secret"
                </button>
                {
                    let id_ask = id.clone();
                    let id_del = id.clone();
                    move || if asking() {
                        let id_del = id_del.clone();
                        view! {
                            <button class="btn btn-ghost" type="button" style="color: var(--red, #e11d48)"
                                disabled=move || busy.get()
                                on:click=move |_| {
                                    confirm_delete.set(None);
                                    dispatch.run(Msg::Request(ApiCall::DeleteOidcProvider(id_del.clone())));
                                }>
                                "Confirm delete"
                            </button>
                            <button class="btn btn-ghost" type="button" on:click=move |_| confirm_delete.set(None)>
                                "Cancel"
                            </button>
                        }
                        .into_any()
                    } else {
                        let id_ask = id_ask.clone();
                        view! {
                            <button class="btn btn-ghost" type="button" style="color: var(--red, #e11d48)"
                                title="Deleting a provider signs out everyone who used it and removes their links"
                                on:click=move |_| confirm_delete.set(Some(id_ask.clone()))>
                                "Delete"
                            </button>
                        }
                        .into_any()
                    }
                }
            </div>
        </div>
    }
}

// ── YourSsoSection ───────────────────────────────────────────────────────────

/// Link or unlink the signed-in member's own SSO identity. Linking re-authenticates with password +
/// authenticator code, then goes through the IdP. A member cannot list providers (Owner-only), so the
/// provider is named by the sign-in link the Owner shared (or its id).
#[cfg(feature = "csr")]
#[component]
fn YourSsoSection() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let link = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let code = RwSignal::new(String::new());

    let provider = move || sso_provider_from_input(&link.get());
    let busy = move || model.with(|m| m.pending.contains(&CallKind::SsoStart));
    let unlinking = move || model.with(|m| m.pending.contains(&CallKind::SsoUnlink));
    let error = move || model.with(|m| m.sso_link_error.clone());
    let ready = move || {
        provider().is_some() && !password.get().is_empty() && !code.get().trim().is_empty() && !busy()
    };

    let submit = move |_| {
        let Some(provider_id) = provider() else { return };
        dispatch.run(Msg::Request(ApiCall::StartSso {
            provider_id,
            intent: SsoIntent::Link { password: password.get(), totp_code: code.get().trim().to_string() },
        }));
    };

    let control = move || model.with(|m| m.link_control());

    view! {
        <div class="card" style="margin-bottom: 18px">
            <div class="card-head">
                <div class="col" style="gap: 2px">
                    <h3>"Your single sign-on"</h3>
                    <span class="sub">"Link your account to your organisation's identity provider, or remove the link"</span>
                </div>
            </div>
            <div class="card-pad col" style="gap: 12px">
                {move || match control() {
                    Some(LinkControl::Link) => view! {
                        <div class="field">
                            <label class="field-label">"SSO sign-in link (or provider ID)"</label>
                            <input class="input" type="text" autocomplete="off" placeholder="https://console.example.com/admin/?sso=…"
                                prop:value=move || link.get()
                                on:input=move |e| link.set(event_target_value(&e))/>
                            <Show when=move || !link.get().trim().is_empty() && provider().is_none() fallback=|| ()>
                                <span class="field-err">"That is not an SSO sign-in link or provider ID"</span>
                            </Show>
                        </div>
                        <div class="field">
                            <label class="field-label">"Your password"</label>
                            <input class="input" type="password" autocomplete="current-password"
                                prop:value=move || password.get()
                                on:input=move |e| password.set(event_target_value(&e))/>
                        </div>
                        <div class="field">
                            <label class="field-label">"Authenticator code"</label>
                            <input class="input" type="text" autocomplete="one-time-code" inputmode="numeric"
                                prop:value=move || code.get()
                                on:input=move |e| code.set(event_target_value(&e))/>
                        </div>
                        <Show when=move || error().is_some() fallback=|| ()>
                            <span class="field-err" role="alert">{move || error().unwrap_or_default()}</span>
                        </Show>
                        <div class="row" style="gap: 8px">
                            <button class="btn btn-primary" type="button" disabled=move || !ready() on:click=submit>
                                {move || if busy() { "Opening…" } else { "Link SSO" }}
                            </button>
                        </div>
                    }.into_any(),
                    Some(LinkControl::Unlink) => view! {
                        <p style="font-size: 13px; color: var(--text-2)">
                            "Your account is linked to single sign-on. Unlinking removes the link and ends the sessions started through it."
                        </p>
                        <div class="row">
                            <button class="btn btn-ghost" type="button" disabled=unlinking
                                on:click=move |_| { dispatch.run(Msg::Request(ApiCall::UnlinkSso)); }>
                                "Unlink SSO"
                            </button>
                        </div>
                    }.into_any(),
                    Some(LinkControl::SsoSession) => view! {
                        <p style="font-size: 13px; color: var(--text-2)">
                            "You are signed in with single sign-on. Linking or unlinking needs a password and authenticator code, so it is not available in this session. If you have no password, ask an Owner."
                        </p>
                    }.into_any(),
                    Some(LinkControl::RequiredByAccount) => view! {
                        <p style="font-size: 13px; color: var(--text-2)">
                            "Your account requires single sign-on, so your link cannot be removed while that is on."
                        </p>
                    }.into_any(),
                    None => ().into_any(),
                }}
            </div>
        </div>
    }
}

// ── RequireSsoSection ────────────────────────────────────────────────────────

/// "Require SSO for members" — Owner only (`SettingsView` mounts it only then). Enabling asks for
/// confirmation, because non-Owner members lose password sign-in at once; disabling is immediate.
#[cfg(feature = "csr")]
#[component]
fn RequireSsoSection() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let confirm_open = RwSignal::new(false);
    let required = move || model.with(|m| m.sso_required());
    let busy = move || model.with(|m| m.pending.contains(&CallKind::SsoRequired));

    view! {
        <div class="card" style="margin-bottom: 18px">
            <div class="card-head">
                <div class="col" style="gap: 2px">
                    <h3>"Require SSO for members"</h3>
                    <span class="sub">"Admins and Viewers must sign in through single sign-on"</span>
                </div>
            </div>
            <div class="card-pad col" style="gap: 12px">
                <p style="font-size: 13px; color: var(--text-2)">
                    "While this is on, non-Owner members can no longer sign in with a password, and new Admin and Viewer invitations must use SSO. Owners are exempt: they always keep password and two-factor sign-in as a break-glass way in. Link your own SSO identity first, under \"Your single sign-on\"."
                </p>
                <div class="row" style="gap: 10px; align-items: center">
                    <span style="font-weight: 550; font-size: 13.5px">
                        {move || if required() { "SSO is required" } else { "SSO is not required" }}
                    </span>
                    <button class="btn btn-ghost" type="button" disabled=busy
                        on:click=move |_| {
                            if required() {
                                dispatch.run(Msg::Request(ApiCall::SetSsoRequired(false)));
                            } else {
                                confirm_open.set(true);
                            }
                        }>
                        {move || if required() { "Stop requiring SSO" } else { "Require SSO" }}
                    </button>
                </div>
            </div>
        </div>

        <Show when=move || confirm_open.get() fallback=|| ()>
            <Modal title="Require SSO for members?" on_close=Callback::new(move |_| confirm_open.set(false))>
                <p style="font-size: 13px; color: var(--text-2)">
                    "Admins and Viewers will not be able to sign in with a password any more. They must already be linked to SSO, or you must invite them again with SSO. Owners keep password and two-factor sign-in."
                </p>
                <div class="modal-footer">
                    <button class="btn btn-ghost" type="button" on:click=move |_| confirm_open.set(false)>"Cancel"</button>
                    <button class="btn btn-primary" type="button" disabled=busy
                        on:click=move |_| {
                            confirm_open.set(false);
                            dispatch.run(Msg::Request(ApiCall::SetSsoRequired(true)));
                        }>
                        "Require SSO"
                    </button>
                </div>
            </Modal>
        </Show>
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

