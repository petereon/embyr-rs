//! Connections tab — masked backend summary, edit DSN / agent config, switch mode (ADR-083).
//!
//! The server never returns credentials; the summary is host/port/database/TLS (or the agent
//! endpoint and certificate expiry). Saving calls `PUT /projects/:id/connection`, which probes the
//! new backend first: on failure the server's error is shown and nothing changes. New secrets live
//! only in this view's local signals and are wiped when the edit form closes; they never enter the
//! model.

#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind, ConnectionConfig};
#[cfg(feature = "csr")]
use crate::components::Icon;
#[cfg(feature = "csr")]
use crate::model::{AppModel, DbBackendMode, DbId};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
#[component]
pub fn ConnectionsView(id: DbId) -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let mode = Memo::new({
        let id = id.clone();
        move |_| model.with(|m| m.databases.iter().find(|d| d.id == id).map(|d| d.backend_mode.clone()))
    });
    let connection = Memo::new({
        let id = id.clone();
        move |_| model.with(|m| m.connections.get(&id).cloned())
    });
    let loading = move || model.with(|m| m.pending.contains(&CallKind::Connection));
    let saving = move || model.with(|m| m.pending.contains(&CallKind::PutConnection));
    let can_manage = move || model.with(|m| m.can_manage_databases());
    let saves = move || model.with(|m| m.connection_saves);

    let editing = RwSignal::new(false);
    let target_agent = RwSignal::new(false);
    let dsn = RwSignal::new(String::new());
    let endpoint = RwSignal::new(String::new());
    let ca_pem = RwSignal::new(String::new());
    let client_cert_pem = RwSignal::new(String::new());
    let client_key_pem = RwSignal::new(String::new());

    let wipe = move || {
        for field in [dsn, endpoint, ca_pem, client_cert_pem, client_key_pem] {
            field.set(String::new());
        }
        editing.set(false);
    };
    // A server-confirmed save closes the form and drops the secrets that were typed into it.
    Effect::new(move |previous: Option<u32>| {
        let now = saves();
        if previous.is_some_and(|p| p != now) {
            wipe();
        }
        now
    });

    let config = move || {
        if target_agent.get() {
            ConnectionConfig::Agent {
                endpoint: endpoint.get(),
                ca_pem: ca_pem.get(),
                client_cert_pem: client_cert_pem.get(),
                client_key_pem: client_key_pem.get(),
            }
        } else {
            ConnectionConfig::DirectPg { dsn: dsn.get() }
        }
    };
    let complete = move || config().is_complete();
    // Secret-manager modes are managed outside the console (G-25).
    let editable_mode = move || matches!(mode.get(), Some(DbBackendMode::DirectPg | DbBackendMode::AgentMode));

    let save_id = id.clone();

    view! {
        <div class="fade-in" style="max-width:720px">
            <div class="card" style="padding:0;overflow:hidden">
                <div style="display:flex;align-items:center;justify-content:space-between;\
                             padding:16px 20px;border-bottom:1px solid var(--border)">
                    <div>
                        <div style="font-size:14px;font-weight:600">"Backend configuration"</div>
                        <div style="font-size:12px;color:var(--text-3);margin-top:2px">"How this database reaches Postgres"</div>
                    </div>
                    {move || (!editing.get() && editable_mode()).then(|| view! {
                        <button
                            class="btn btn-ghost btn-sm"
                            type="button"
                            disabled=move || !can_manage()
                            on:click=move |_| {
                                target_agent.set(mode.get_untracked() == Some(DbBackendMode::AgentMode));
                                editing.set(true);
                            }
                        >"Edit"</button>
                    })}
                </div>

                <div style="padding:16px 20px;display:flex;flex-direction:column;gap:16px">
                    <div class="row" style="gap:8px">
                        <span style="width:28px;height:28px;border-radius:7px;background:var(--accent-soft);\
                                     color:var(--accent);display:grid;place-items:center;flex-shrink:0">
                            <Icon name="database" size=14/>
                        </span>
                        <div>
                            <div class="mono" style="font-size:13px;font-weight:600">
                                {move || mode.get().map(|m| m.api_value()).unwrap_or("—")}
                            </div>
                            <div style="font-size:11.5px;color:var(--text-3);margin-top:1px">
                                {move || match mode.get() {
                                    Some(DbBackendMode::DirectPg) => "Embyr connects directly to Postgres. The DSN is stored encrypted.",
                                    Some(DbBackendMode::AgentMode) => "Embyr routes operations through your VPC agent over mTLS.",
                                    Some(_) => "The credentials come from your secret manager (read-only here).",
                                    None => "",
                                }}
                            </div>
                        </div>
                    </div>

                    <div class="field">
                        <label class="field-label">"Current connection"</label>
                        <div class="row" style="gap:8px;margin-top:4px">
                            <span class="mono" style="font-size:13px;color:var(--text-2)">
                                {move || match connection.get() {
                                    Some(c) => c.summary.describe(),
                                    None if loading() => "Loading…".to_string(),
                                    None => "—".to_string(),
                                }}
                            </span>
                            <span class="tag">"credentials masked"</span>
                        </div>
                    </div>

                    {move || editing.get().then(|| view! {
                        <div style="padding-top:14px;border-top:1px solid var(--border);display:flex;flex-direction:column;gap:12px">
                            <div class="row" style="gap:8px">
                                <button
                                    class=move || if !target_agent.get() { "btn btn-primary btn-sm" } else { "btn btn-ghost btn-sm" }
                                    type="button"
                                    on:click=move |_| target_agent.set(false)
                                >"Direct PG"</button>
                                <button
                                    class=move || if target_agent.get() { "btn btn-primary btn-sm" } else { "btn btn-ghost btn-sm" }
                                    type="button"
                                    on:click=move |_| target_agent.set(true)
                                >"Agent mode"</button>
                            </div>

                            {move || if target_agent.get() {
                                view! {
                                    <div class="field">
                                        <label class="field-label">"Agent endpoint"</label>
                                        <input class="input mono" type="text" placeholder="10.0.0.5:9191"
                                            autocomplete="off" spellcheck="false"
                                            prop:value=move || endpoint.get()
                                            on:input=move |e| endpoint.set(event_target_value(&e))/>
                                    </div>
                                    <div class="field">
                                        <label class="field-label">"CA certificate (PEM)"</label>
                                        <textarea class="input mono" rows="4" spellcheck="false"
                                            prop:value=move || ca_pem.get()
                                            on:input=move |e| ca_pem.set(event_target_value(&e))/>
                                    </div>
                                    <div class="field">
                                        <label class="field-label">"Client certificate (PEM)"</label>
                                        <textarea class="input mono" rows="4" spellcheck="false"
                                            prop:value=move || client_cert_pem.get()
                                            on:input=move |e| client_cert_pem.set(event_target_value(&e))/>
                                    </div>
                                    <div class="field">
                                        <label class="field-label">"Client key (PEM)"</label>
                                        <textarea class="input mono" rows="4" spellcheck="false"
                                            prop:value=move || client_key_pem.get()
                                            on:input=move |e| client_key_pem.set(event_target_value(&e))/>
                                    </div>
                                }.into_any()
                            } else {
                                view! {
                                    <div class="field">
                                        <label class="field-label">"Postgres DSN"</label>
                                        <input class="input mono" type="password" placeholder="postgres://user:pass@host:5432/db"
                                            autocomplete="off" spellcheck="false"
                                            prop:value=move || dsn.get()
                                            on:input=move |e| dsn.set(event_target_value(&e))/>
                                    </div>
                                }.into_any()
                            }}

                            <p style="font-size:11.5px;color:var(--text-3);margin:0">
                                "Saving connects to the new backend first. If that fails, the server's error is shown \
                                 and the current connection stays in place."
                            </p>
                            <div class="row" style="gap:8px;justify-content:flex-end">
                                <button class="btn btn-ghost btn-sm" type="button" on:click=move |_| wipe()>"Cancel"</button>
                                <button
                                    class="btn btn-primary btn-sm"
                                    type="button"
                                    disabled=move || saving() || !complete() || !can_manage()
                                    on:click={
                                        let id = save_id.clone();
                                        move |_| dispatch.run(Msg::Request(ApiCall::PutConnection {
                                            id: id.clone(),
                                            config: config(),
                                        }))
                                    }
                                >{move || if saving() { "Checking…" } else { "Save" }}</button>
                            </div>
                        </div>
                    })}
                </div>
            </div>
        </div>
    }
}
