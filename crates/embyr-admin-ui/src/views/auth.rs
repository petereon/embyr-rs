//! Signed-out screens (sign in, forgot password, reset password) in the embyr Console two-panel layout.

#[cfg(feature = "csr")]
use crate::components::Icon;
#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind, SignInCode, SsoIntent};
#[cfg(feature = "csr")]
use crate::model::{AppModel, AuthScreen, Enrolment, InviteStep, ProviderIds, RecoveryCodes};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
#[component]
pub fn AuthView() -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

    let email = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let code = RwSignal::new(String::new());
    let use_recovery = RwSignal::new(false);
    let busy = move || model.with(|m| m.pending.contains(&CallKind::SignIn) || m.pending.contains(&CallKind::Session));
    let error = move || model.with(|m| m.sign_in_error.clone());
    let notice = move || model.with(|m| m.auth_notice.clone());

    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let code_value = code.get().trim().to_string();
        let code = if use_recovery.get() { SignInCode::Recovery(code_value) } else { SignInCode::Totp(code_value) };
        dispatch.run(Msg::Request(ApiCall::SignIn {
            email: email.get().trim().to_string(),
            password: password.get(),
            code,
        }));
    };

    view! {
        <AuthFrame>
                    <h1 style="font-size:23px;font-weight:620;letter-spacing:-0.02em;margin:0 0 6px">"Sign in"</h1>
                    <p style="color:var(--text-2);font-size:13.5px;margin:0 0 24px">"Welcome back. Continue to your console."</p>

                    <Show when=move || notice().is_some()>
                        <p class="field-label" role="status" style="margin:0 0 14px">{move || notice().unwrap_or_default()}</p>
                    </Show>

                    <form class="col" style="gap:14px" on:submit=submit>
                        <div class="field">
                            <label class="field-label">"Email"</label>
                            <input class="input" type="email" name="email" autocomplete="username"
                                placeholder="you@example.com" bind:value=email/>
                        </div>
                        <div class="field">
                            <label class="field-label">"Password"</label>
                            <input class="input" type="password" name="password" autocomplete="current-password"
                                placeholder="••••••••" bind:value=password/>
                        </div>
                        <div class="field">
                            <label class="field-label">
                                {move || if use_recovery.get() { "Recovery code" } else { "Authenticator code" }}
                            </label>
                            <input class="input" type="text" name="code" autocomplete="one-time-code"
                                inputmode=move || if use_recovery.get() { "text" } else { "numeric" }
                                bind:value=code/>
                            <button class="btn btn-ghost" type="button" on:click=move |_| use_recovery.update(|v| *v = !*v)>
                                {move || if use_recovery.get() { "Use authenticator code" } else { "Use a recovery code" }}
                            </button>
                        </div>
                        <button class="btn btn-ghost" type="button"
                            on:click=move |_| dispatch.run(Msg::ShowAuth(AuthScreen::ForgotPassword))>
                            "Forgot password?"
                        </button>
                        <Show when=move || error().is_some()>
                            <span class="field-err" role="alert">{move || error().unwrap_or_default()}</span>
                        </Show>
                        <button class="btn btn-primary btn-block btn-lg" type="submit" disabled=busy>
                            {move || if busy() { "Signing in…" } else { "Continue" }}
                        </button>
                    </form>

                    // No provider list and no email box: nothing here tells a stranger which accounts use SSO.
                    <details style="margin-top:18px;font-size:12.5px;color:var(--text-2)">
                        <summary style="cursor:pointer">"Sign in with SSO"</summary>
                        <p style="margin:8px 0 0;line-height:1.5">
                            "Single sign-on starts from a link. Ask your account Owner for the SSO sign-in link and open it here."
                        </p>
                    </details>

                    <p style="text-align:center;font-size:12.5px;color:var(--text-3);margin-top:18px">
                        "Protected by mandatory MFA"
                    </p>
        </AuthFrame>
    }
}

/// Two-panel layout shared by every signed-out screen: brand panel left, `children` (the form) right.
#[cfg(feature = "csr")]
#[component]
fn AuthFrame(children: Children) -> impl IntoView {
    view! {
        <div style="display:grid;grid-template-columns:1.05fr 1fr;height:100%;background:var(--bg)">

            // ── Left: brand panel ──────────────────────────────────────────
            <div style="position:relative;overflow:hidden;border-right:1px solid var(--border);\
                        background:radial-gradient(120% 120% at 15% 10%, color-mix(in oklab, var(--accent) 16%, var(--bg)) 0%, var(--bg) 55%);\
                        display:flex;flex-direction:column;justify-content:space-between;padding:44px 48px">
                // logo
                <div class="row" style="gap:11px">
                    <svg width="30" height="30" viewBox="0 0 32 32" fill="none">
                        <rect width="32" height="32" rx="8" fill="var(--accent)"/>
                        <path d="M7 23 L16 9 L25 23" stroke="white" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/>
                        <path d="M10 19 L22 19" stroke="white" stroke-width="2" stroke-linecap="round" opacity="0.65"/>
                    </svg>
                    <span style="font-size:19px;font-weight:600;letter-spacing:-0.03em">"embyr"</span>
                </div>

                // hero copy
                <div>
                    <div style="font-size:34px;font-weight:600;letter-spacing:-0.03em;line-height:1.12;max-width:440px">
                        "The realtime database, "
                        <span style="color:var(--accent)">"without the lock-in."</span>
                    </div>
                    <p style="color:var(--text-2);font-size:15px;margin-top:16px;max-width:380px;line-height:1.55">
                        "Firestore-compatible APIs over your own Postgres. Manage databases, keys, and access from one console."
                    </p>
                    <div class="row" style="gap:20px;margin-top:28px;color:var(--text-3);font-size:12.5px">
                        <span class="row" style="gap:7px">
                            <Icon name="zap" size=14/>
                            <span style="color:var(--accent);display:contents"><Icon name="zap" size=14/></span>
                            "Sub-10ms reads"
                        </span>
                        <span class="row" style="gap:7px">
                            <Icon name="shield" size=14/>
                            "mTLS + MFA"
                        </span>
                        <span class="row" style="gap:7px">
                            <Icon name="server" size=14/>
                            "Self-hostable"
                        </span>
                    </div>
                </div>

                <span style="font-size:12px;color:var(--text-3)">"© 2026 embyr-rs · admin console"</span>
            </div>

            // ── Right: form panel ──────────────────────────────────────────
            <div style="display:grid;place-items:center;padding:32px">
                <div style="width:100%;max-width:360px" class="fade-in">
                    {children()}
                </div>
            </div>
        </div>
    }
}


/// "Forgot password": ask for a reset link. The server answers 202 whether or not the email exists.
#[cfg(feature = "csr")]
#[component]
pub fn ForgotPasswordView() -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

    let email = RwSignal::new(String::new());
    let busy = move || model.with(|m| m.pending.contains(&CallKind::ResetRequest));
    let error = move || model.with(|m| m.reset_error.clone());
    let notice = move || model.with(|m| m.auth_notice.clone());

    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        dispatch.run(Msg::Request(ApiCall::RequestPasswordReset { email: email.get().trim().to_string() }));
    };

    view! {
        <AuthFrame>
            <h1 style="font-size:23px;font-weight:620;letter-spacing:-0.02em;margin:0 0 6px">"Reset your password"</h1>
            <p style="color:var(--text-2);font-size:13.5px;margin:0 0 24px">
                "Enter your account email and we will send you a reset link. You will also need your authenticator or a recovery code."
            </p>
            <form class="col" style="gap:14px" on:submit=submit>
                <div class="field">
                    <label class="field-label">"Email"</label>
                    <input class="input" type="email" name="email" autocomplete="username"
                        placeholder="you@example.com" bind:value=email/>
                </div>
                <Show when=move || notice().is_some()>
                    <span class="field-label" role="status">{move || notice().unwrap_or_default()}</span>
                </Show>
                <Show when=move || error().is_some()>
                    <span class="field-err" role="alert">{move || error().unwrap_or_default()}</span>
                </Show>
                <button class="btn btn-primary btn-block btn-lg" type="submit" disabled=busy>
                    {move || if busy() { "Sending…" } else { "Send reset link" }}
                </button>
                <button class="btn btn-ghost" type="button"
                    on:click=move |_| dispatch.run(Msg::ShowAuth(AuthScreen::SignIn))>
                    "Back to sign in"
                </button>
            </form>
        </AuthFrame>
    }
}

/// `?reset=<token>` landing: new password + second factor. The token lives only in this component's
/// props (never in the model or the URL bar) and is sent once, inside `ApiCall::ConfirmPasswordReset`.
#[cfg(feature = "csr")]
#[component]
pub fn ResetPasswordView(token: RwSignal<Option<String>>) -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

    let password = RwSignal::new(String::new());
    let confirm = RwSignal::new(String::new());
    let code = RwSignal::new(String::new());
    let use_recovery = RwSignal::new(false);
    let mismatch = RwSignal::new(false);
    let busy = move || model.with(|m| m.pending.contains(&CallKind::ResetConfirm));
    let error = move || model.with(|m| m.reset_error.clone());

    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let Some(token) = token.get_untracked() else { return };
        if password.get() != confirm.get() {
            mismatch.set(true);
            return;
        }
        mismatch.set(false);
        let code_value = code.get().trim().to_string();
        let code = if use_recovery.get() { SignInCode::Recovery(code_value) } else { SignInCode::Totp(code_value) };
        dispatch.run(Msg::Request(ApiCall::ConfirmPasswordReset { token, new_password: password.get(), code }));
    };

    view! {
        <AuthFrame>
            <h1 style="font-size:23px;font-weight:620;letter-spacing:-0.02em;margin:0 0 6px">"Choose a new password"</h1>
            <p style="color:var(--text-2);font-size:13.5px;margin:0 0 24px">
                "Confirm it is you with your authenticator or a recovery code. All your sessions will be signed out."
            </p>
            <form class="col" style="gap:14px" on:submit=submit>
                <div class="field">
                    <label class="field-label">"New password"</label>
                    <input class="input" type="password" name="new-password" autocomplete="new-password"
                        minlength="12" placeholder="At least 12 characters" bind:value=password/>
                </div>
                <div class="field">
                    <label class="field-label">"Confirm new password"</label>
                    <input class="input" type="password" name="confirm-password" autocomplete="new-password"
                        bind:value=confirm/>
                </div>
                <div class="field">
                    <label class="field-label">
                        {move || if use_recovery.get() { "Recovery code" } else { "Authenticator code" }}
                    </label>
                    <input class="input" type="text" name="code" autocomplete="one-time-code"
                        inputmode=move || if use_recovery.get() { "text" } else { "numeric" }
                        bind:value=code/>
                    <button class="btn btn-ghost" type="button" on:click=move |_| use_recovery.update(|v| *v = !*v)>
                        {move || if use_recovery.get() { "Use authenticator code" } else { "Use a recovery code" }}
                    </button>
                </div>
                <Show when=move || mismatch.get()>
                    <span class="field-err" role="alert">"Passwords do not match"</span>
                </Show>
                <Show when=move || error().is_some()>
                    <span class="field-err" role="alert">{move || error().unwrap_or_default()}</span>
                </Show>
                <button class="btn btn-primary btn-block btn-lg" type="submit" disabled=busy>
                    {move || if busy() { "Updating…" } else { "Update password" }}
                </button>
                <button class="btn btn-ghost" type="button"
                    on:click=move |_| dispatch.run(Msg::ShowAuth(AuthScreen::ForgotPassword))>
                    "Request a new link"
                </button>
            </form>
        </AuthFrame>
    }
}

/// `?invite=<token>` landing: redeem an emailed invitation in three steps (details, authenticator
/// enrolment, recovery codes). The token lives only in this component's props (never in the model or
/// the URL bar); the authenticator secret and recovery codes sit in `model.invite_step`, which `update`
/// drops when the screen is left.
#[cfg(feature = "csr")]
#[component]
pub fn AcceptInviteView(token: RwSignal<Option<String>>) -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let step = move || model.with(|m| m.invite_step.clone());

    view! {
        <AuthFrame>
            {move || match step() {
                InviteStep::Checking => view! {
                    <p style="color:var(--text-3)">"Checking your invitation…"</p>
                }
                .into_any(),
                InviteStep::Sso(ids) => view! { <InviteSsoStep token=token ids=ids /> }.into_any(),
                InviteStep::Details => view! { <InviteDetailsForm token=token /> }.into_any(),
                InviteStep::Enrol(enrolment) => view! { <InviteEnrolForm token=token enrolment=enrolment /> }.into_any(),
                InviteStep::Codes(codes) => view! { <InviteCodes codes=codes /> }.into_any(),
            }}
        </AuthFrame>
    }
}

/// An SSO invitation: no password and no authenticator steps. The IdP proves who the invitee is; the
/// server matches the verified email to the invitation. One button per enabled provider of the account
/// (usually one); the token stays in this component's props and is sent only inside the start call.
#[cfg(feature = "csr")]
#[component]
fn InviteSsoStep(token: RwSignal<Option<String>>, ids: ProviderIds) -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

    let busy = move || model.with(|m| m.pending.contains(&CallKind::SsoStart));
    let error = move || model.with(|m| m.reset_error.clone());
    let many = ids.0.len() > 1;
    let ids_empty = ids.0.is_empty();
    let buttons = ids
        .0
        .into_iter()
        .enumerate()
        .map(|(i, provider_id)| {
            let label = if many { format!("Continue with SSO ({})", i + 1) } else { "Continue with SSO".to_string() };
            view! {
                <button class="btn btn-primary btn-block btn-lg" type="button" disabled=busy
                    on:click=move |_| {
                        let Some(token) = token.get_untracked() else { return };
                        dispatch.run(Msg::Request(ApiCall::StartSso {
                            provider_id: provider_id.clone(),
                            intent: SsoIntent::Invite { token },
                        }));
                    }>
                    {label}
                </button>
            }
        })
        .collect_view();
    let empty = ids_empty;

    view! {
        <h1 style="font-size:23px;font-weight:620;letter-spacing:-0.02em;margin:0 0 6px">"Accept your invitation"</h1>
        <p style="color:var(--text-2);font-size:13.5px;margin:0 0 24px">
            "Your account uses single sign-on. Continue with your organisation's identity provider: there is no password or authenticator to set up here."
        </p>
        <div class="col" style="gap:14px">
            {buttons}
            {empty.then(|| view! {
                <span class="field-err" role="alert">
                    "No sign-in provider is enabled for this invitation. Ask an admin to check the SSO settings."
                </span>
            })}
            <Show when=move || error().is_some()>
                <span class="field-err" role="alert">{move || error().unwrap_or_default()}</span>
            </Show>
            <button class="btn btn-ghost" type="button"
                on:click=move |_| dispatch.run(Msg::ShowAuth(AuthScreen::SignIn))>
                "Back to sign in"
            </button>
        </div>
    }
}

#[cfg(feature = "csr")]
#[component]
fn InviteDetailsForm(token: RwSignal<Option<String>>) -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

    let display_name = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let confirm = RwSignal::new(String::new());
    let mismatch = RwSignal::new(false);
    let busy = move || model.with(|m| m.pending.contains(&CallKind::InviteAccept));
    let error = move || model.with(|m| m.reset_error.clone());

    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let Some(token) = token.get_untracked() else { return };
        if password.get() != confirm.get() {
            mismatch.set(true);
            return;
        }
        mismatch.set(false);
        dispatch.run(Msg::Request(ApiCall::AcceptInvitation {
            token,
            display_name: display_name.get().trim().to_string(),
            password: password.get(),
        }));
    };

    view! {
        <h1 style="font-size:23px;font-weight:620;letter-spacing:-0.02em;margin:0 0 6px">"Accept your invitation"</h1>
        <p style="color:var(--text-2);font-size:13.5px;margin:0 0 24px">
            "Choose how you appear and set a password. Next you will enrol an authenticator app."
        </p>
        <form class="col" style="gap:14px" on:submit=submit>
            <div class="field">
                <label class="field-label">"Display name"</label>
                <input class="input" type="text" name="display-name" autocomplete="name" maxlength="100"
                    bind:value=display_name/>
            </div>
            <div class="field">
                <label class="field-label">"Password"</label>
                <input class="input" type="password" name="new-password" autocomplete="new-password"
                    minlength="12" placeholder="At least 12 characters" bind:value=password/>
            </div>
            <div class="field">
                <label class="field-label">"Confirm password"</label>
                <input class="input" type="password" name="confirm-password" autocomplete="new-password"
                    bind:value=confirm/>
            </div>
            <Show when=move || mismatch.get()>
                <span class="field-err" role="alert">"Passwords do not match"</span>
            </Show>
            <Show when=move || error().is_some()>
                <span class="field-err" role="alert">{move || error().unwrap_or_default()}</span>
            </Show>
            <button class="btn btn-primary btn-block btn-lg" type="submit" disabled=busy>
                {move || if busy() { "Continuing…" } else { "Continue" }}
            </button>
            <button class="btn btn-ghost" type="button"
                on:click=move |_| dispatch.run(Msg::ShowAuth(AuthScreen::SignIn))>
                "Back to sign in"
            </button>
        </form>
    }
}

#[cfg(feature = "csr")]
#[component]
fn InviteEnrolForm(token: RwSignal<Option<String>>, enrolment: Enrolment) -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

    let code = RwSignal::new(String::new());
    let busy = move || model.with(|m| m.pending.contains(&CallKind::InviteConfirm));
    let error = move || model.with(|m| m.reset_error.clone());

    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let Some(token) = token.get_untracked() else { return };
        dispatch.run(Msg::Request(ApiCall::ConfirmInvitation { token, totp_code: code.get().trim().to_string() }));
    };
    let secret = enrolment.totp_secret.clone();

    view! {
        <h1 style="font-size:23px;font-weight:620;letter-spacing:-0.02em;margin:0 0 6px">"Set up your authenticator"</h1>
        <p style="color:var(--text-2);font-size:13.5px;margin:0 0 18px">
            "Add this account to an authenticator app, then enter the 6-digit code it shows. "
            "Open the link on your phone, or type the key by hand."
        </p>
        <div class="field" style="margin-bottom:14px">
            <label class="field-label">"Key"</label>
            <input class="input mono" type="text" readonly=true prop:value=enrolment.totp_secret/>
            <div class="row" style="gap:8px">
                <button class="btn btn-ghost" type="button"
                    on:click=move |_| crate::components::copy_to_clipboard(&secret)>"Copy key"</button>
                <a class="btn btn-ghost" href=enrolment.otpauth_uri>"Open in authenticator app"</a>
            </div>
        </div>
        <form class="col" style="gap:14px" on:submit=submit>
            <div class="field">
                <label class="field-label">"Authenticator code"</label>
                <input class="input" type="text" name="code" autocomplete="one-time-code" inputmode="numeric"
                    bind:value=code/>
            </div>
            <Show when=move || error().is_some()>
                <span class="field-err" role="alert">{move || error().unwrap_or_default()}</span>
            </Show>
            <button class="btn btn-primary btn-block btn-lg" type="submit" disabled=busy>
                {move || if busy() { "Verifying…" } else { "Verify and create account" }}
            </button>
            <button class="btn btn-ghost" type="button"
                on:click=move |_| dispatch.run(Msg::ShowAuth(AuthScreen::AcceptInvite))>
                "Back"
            </button>
        </form>
    }
}

#[cfg(feature = "csr")]
#[component]
fn InviteCodes(codes: RecoveryCodes) -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");
    let joined = codes.0.join("\n");

    view! {
        <h1 style="font-size:23px;font-weight:620;letter-spacing:-0.02em;margin:0 0 6px">"Save your recovery codes"</h1>
        <p style="color:var(--text-2);font-size:13.5px;margin:0 0 18px">
            "Each code works once if you lose your authenticator. They are shown only now: store them somewhere safe."
        </p>
        <pre class="input mono" style="white-space:pre;margin:0 0 14px">{codes.0.join("\n")}</pre>
        <div class="row" style="gap:8px">
            <button class="btn btn-ghost" type="button"
                on:click=move |_| crate::components::copy_to_clipboard(&joined)>"Copy codes"</button>
            <button class="btn btn-primary" type="button"
                on:click=move |_| dispatch.run(Msg::InviteFinished)>"I have stored them"</button>
        </div>
    }
}
