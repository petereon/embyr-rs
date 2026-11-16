//! Signed-out screens (sign in, forgot password, reset password) in the embyr Console two-panel layout.

#[cfg(feature = "csr")]
use crate::components::Icon;
#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind, SignInCode};
#[cfg(feature = "csr")]
use crate::model::{AppModel, AuthScreen};
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

                    // SSO sign-in is hidden until `admin-oidc-sso` lands (G-13, AC-AUW-28).
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
