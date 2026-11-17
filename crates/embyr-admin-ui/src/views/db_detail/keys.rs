//! Keys tab — the project API key panel (api-key-rotation, ADR-085).
//!
//! Rotate issues a new key (shown once in the reveal modal); the previous key keeps working for the
//! server's rotation window. Revoke now is the same operation with no window and asks for a
//! confirmation first. Both are Owner/Admin only (the server enforces it; Viewers see disabled buttons).
//!
//! Gated behind #[cfg(feature = "csr")].

#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind};
#[cfg(feature = "csr")]
use crate::model::{AppModel, DbId};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
fn fmt_ts(ts: Option<chrono::DateTime<chrono::Utc>>, none: &str) -> String {
    ts.map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| none.to_string())
}

/// Project API key panel for the Keys tab.
#[cfg(feature = "csr")]
#[component]
pub fn KeysView(id: DbId) -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let info = Memo::new({
        let id = id.clone();
        move |_| model.with(|m| m.api_key_info.get(&id).cloned().unwrap_or_default())
    });
    let disabled =
        move || model.with(|m| !m.can_manage_databases() || m.pending.contains(&CallKind::ApiKey));
    let confirm_revoke = RwSignal::new(false);
    let enabled_hosted = Memo::new({
        let id = id.clone();
        move |_| model.with(|m| m.hosted_identity.contains(&id))
    });
    let hosted_disabled =
        move || model.with(|m| !m.can_manage_databases() || m.pending.contains(&CallKind::HostedIdentity));
    let hosted_id = id.clone();

    let rotate_id = id.clone();
    let revoke_id = id.clone();

    view! {
        <div class="keys-panel fade-in">
            <div class="row keys-header">
                <div class="col">
                    <span class="keys-title">"Project API key"</span>
                    <span class="sub">
                        "The key Firestore SDK clients use to authenticate to this database"
                    </span>
                </div>
            </div>
            <div class="card">
                <dl>
                    <dt>"Last rotated"</dt>
                    <dd>{move || fmt_ts(info.get().rotated_at, "Never")}</dd>
                    <dt>"Previous key accepted until"</dt>
                    <dd>{move || fmt_ts(info.get().previous_expires_at, "—")}</dd>
                </dl>
                <div class="row modal-footer">
                    <button
                        class="btn btn-primary"
                        type="button"
                        disabled=disabled
                        on:click=move |_| dispatch.run(Msg::Request(ApiCall::RotateApiKey(rotate_id.clone())))
                    >"Rotate"</button>
                    <button
                        class="btn btn-danger"
                        type="button"
                        disabled=disabled
                        on:click=move |_| confirm_revoke.set(true)
                    >"Revoke now"</button>
                </div>
                <p class="sub">
                    "Rotate issues a new key and keeps the previous one working for a grace period. \
                     Revoke now issues a new key and stops the previous one immediately."
                </p>
            </div>

            <div class="row keys-header" style="margin-top:24px">
                <div class="col">
                    <span class="keys-title">"Hosted identity"</span>
                    <span class="sub">
                        "Let your app's end users sign in through Embyr. Enabling creates a signing key \
                         that the server keeps encrypted; it is never shown. Enabling again changes nothing."
                    </span>
                </div>
            </div>
            <div class="card">
                <div class="row modal-footer">
                    <span class="sub">
                        {move || if enabled_hosted.get() {
                            "Enabled in this session. The API cannot report the current state, so a reload shows the button again."
                        } else {
                            "Not available for agent-mode databases."
                        }}
                    </span>
                    <span class="spacer"/>
                    <button
                        class="btn btn-primary"
                        type="button"
                        disabled=hosted_disabled
                        on:click=move |_| { dispatch.run(Msg::Request(ApiCall::EnableHostedIdentity(hosted_id.clone()))); }
                    >"Enable hosted identity"</button>
                </div>
            </div>

            {move || confirm_revoke.get().then(|| {
                let revoke_id = revoke_id.clone();
                view! {
                    <div class="modal-backdrop" on:click=move |_| confirm_revoke.set(false)>
                        <div class="modal" on:click=|e| e.stop_propagation()>
                            <div class="modal-head">
                                <span class="modal-title">"Revoke the current key now?"</span>
                            </div>
                            <div class="modal-body">
                                <p style="color:var(--text-2);font-size:14px;line-height:1.55">
                                    "Every client still using the current key will stop working immediately. \
                                     A new key is issued and shown once."
                                </p>
                                <div class="row modal-footer" style="justify-content:flex-end">
                                    <button
                                        class="btn btn-ghost"
                                        type="button"
                                        on:click=move |_| confirm_revoke.set(false)
                                    >"Cancel"</button>
                                    <button
                                        class="btn btn-danger"
                                        type="button"
                                        on:click=move |_| {
                                            confirm_revoke.set(false);
                                            dispatch.run(Msg::Request(ApiCall::RevokeApiKey(revoke_id.clone())));
                                        }
                                    >"Revoke now"</button>
                                </div>
                            </div>
                        </div>
                    </div>
                }
            })}
        </div>
    }
}
