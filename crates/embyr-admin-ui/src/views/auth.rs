//! AuthView — two-panel sign-in screen matching the embyr Console design.

#[cfg(feature = "csr")]
use crate::components::Icon;
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
#[component]
pub fn AuthView() -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");
    let d1 = dispatch.clone();
    let d2 = dispatch.clone();
    let d3 = dispatch.clone();

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
                    <h1 style="font-size:23px;font-weight:620;letter-spacing:-0.02em;margin:0 0 6px">"Sign in"</h1>
                    <p style="color:var(--text-2);font-size:13.5px;margin:0 0 24px">"Welcome back. Continue to your console."</p>

                    // SSO buttons
                    <div class="col" style="gap:9px;margin-bottom:18px">
                        <button
                            class="btn btn-block"
                            style="height:38px"
                            on:click=move |_| d1.run(Msg::SignIn)
                        >
                            <Icon name="globe" size=16/>
                            "Continue with Google"
                        </button>
                        <button
                            class="btn btn-block"
                            style="height:38px"
                            on:click=move |_| d2.run(Msg::SignIn)
                        >
                            <Icon name="github" size=16/>
                            "Continue with GitHub"
                        </button>
                    </div>

                    // OR divider
                    <div class="row" style="gap:12px;margin:4px 0 18px;color:var(--text-3);font-size:11.5px">
                        <div class="divider" style="flex:1"/>
                        "OR"
                        <div class="divider" style="flex:1"/>
                    </div>

                    // Email + password
                    <div class="col" style="gap:14px">
                        <div class="field">
                            <label class="field-label">"Email"</label>
                            <input class="input" type="email" name="email" placeholder="you@example.com"/>
                        </div>
                        <div class="field">
                            <label class="field-label">"Password"</label>
                            <input class="input" type="password" name="password" placeholder="••••••••"/>
                        </div>
                        <button
                            class="btn btn-primary btn-block btn-lg"
                            type="button"
                            on:click=move |_| d3.run(Msg::SignIn)
                        >
                            "Continue"
                        </button>
                    </div>

                    <p style="text-align:center;font-size:12.5px;color:var(--text-3);margin-top:18px">
                        "Protected by mandatory MFA"
                    </p>
                </div>
            </div>
        </div>
    }
}
