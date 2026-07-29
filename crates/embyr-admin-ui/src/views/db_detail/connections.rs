//! Connections tab — backend config + active connections panel.

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use crate::data::display_stats;
#[cfg(feature = "csr")]
use crate::model::{Database, DbBackendMode, DbPatch};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use crate::components::Icon;

#[cfg(feature = "csr")]
#[component]
pub fn ConnectionsView(db: Database, idx: usize) -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let stats    = display_stats(idx);
    let is_direct = db.backend_mode == DbBackendMode::DirectPg;
    let db_id    = db.id.clone();
    let db_id2   = db.id.clone();

    let edit_mode = RwSignal::new(false);
    let draft     = RwSignal::new(String::new());

    let mode_label = if is_direct { "direct_pg" } else { "agent_mode" };
    let mode_desc  = if is_direct {
        "Embyr connects directly to Postgres via TCP. DSN stored encrypted."
    } else {
        "Embyr routes operations through your VPC agent binary. No direct DB access from embyr servers."
    };
    let field_label     = if is_direct { "Postgres DSN" } else { "Agent endpoint" };
    let field_placeholder = if is_direct { "postgres://user:pass@host:5432/db" } else { "10.0.0.5:9191" };
    let masked_value    = if is_direct { "postgres://****@db.internal:5432/embyr_prod" } else { stats.connection_detail };

    view! {
        <div class="fade-in" style="display:grid;grid-template-columns:1fr 1fr;gap:20px;align-items:start">

            // ── Backend configuration card ────────────────────────────────────
            <div class="card" style="padding:0;overflow:hidden">
                // Card header
                <div style="display:flex;align-items:center;justify-content:space-between;\
                             padding:16px 20px;border-bottom:1px solid var(--border)">
                    <div>
                        <div style="font-size:14px;font-weight:600">"Backend configuration"</div>
                        <div style="font-size:12px;color:var(--text-3);margin-top:2px">"How this database reaches Postgres"</div>
                    </div>
                    {move || if !edit_mode.get() {
                        view! {
                            <button
                                class="btn btn-ghost btn-sm"
                                type="button"
                                on:click=move |_| edit_mode.set(true)
                            >"Edit"</button>
                        }.into_any()
                    } else {
                        let d = dispatch.clone();
                        let did = db_id.clone();
                        view! {
                            <div class="row" style="gap:8px">
                                <button
                                    class="btn btn-ghost btn-sm"
                                    type="button"
                                    on:click=move |_| edit_mode.set(false)
                                >"Cancel"</button>
                                <button
                                    class="btn btn-primary btn-sm"
                                    type="button"
                                    on:click=move |_| {
                                        let patch = if is_direct {
                                            DbPatch::Dsn(draft.get_untracked())
                                        } else {
                                            DbPatch::AgentEndpoint(draft.get_untracked())
                                        };
                                        d.run(Msg::PatchDb(did.clone(), patch));
                                        edit_mode.set(false);
                                    }
                                >"Save"</button>
                            </div>
                        }.into_any()
                    }}
                </div>

                // Body
                <div style="padding:16px 20px;display:flex;flex-direction:column;gap:16px">
                    // Mode badge row
                    <div class="row" style="gap:8px">
                        <span style="width:28px;height:28px;border-radius:7px;background:var(--accent-soft);\
                                     color:var(--accent);display:grid;place-items:center;flex-shrink:0">
                            <Icon name="database" size=14/>
                        </span>
                        <div>
                            <div class="mono" style="font-size:13px;font-weight:600">{mode_label}</div>
                            <div style="font-size:11.5px;color:var(--text-3);margin-top:1px">{mode_desc}</div>
                        </div>
                    </div>

                    // Connection field
                    <div class="field">
                        <label class="field-label">{field_label}</label>
                        {move || if edit_mode.get() {
                            view! {
                                <input
                                    class="input mono"
                                    type="text"
                                    placeholder=field_placeholder
                                    prop:value=move || draft.get()
                                    on:input=move |e| draft.set(event_target_value(&e))
                                />
                            }.into_any()
                        } else {
                            view! {
                                <div class="row" style="gap:8px;margin-top:4px">
                                    <span class="mono" style="font-size:13px;color:var(--text-2)">
                                        {masked_value}
                                    </span>
                                    {if is_direct {
                                        Some(view! { <span class="tag">"credentials masked"</span> })
                                    } else { None }}
                                </div>
                            }.into_any()
                        }}
                    </div>

                    <p style="font-size:11.5px;color:var(--text-3);margin:0">
                        "Changes take effect on next connection attempt."
                    </p>

                    // Switch backend mode
                    <div style="padding-top:14px;border-top:1px solid var(--border)">
                        <div style="font-size:12.5px;color:var(--text-2);margin-bottom:10px">
                            "Switch backend mode"
                        </div>
                        <div class="row" style="gap:8px">
                            {let d = dispatch.clone();
                             let did = db_id2.clone();
                             view! {
                                <button
                                    class=move || if is_direct { "btn btn-primary btn-sm" } else { "btn btn-ghost btn-sm" }
                                    type="button"
                                    on:click=move |_| {
                                        d.run(Msg::PatchDb(did.clone(), DbPatch::BackendMode(crate::model::DbBackendMode::DirectPg)));
                                    }
                                >"Direct PG"</button>
                            }}
                            {let d = dispatch.clone();
                             let did = db.id.clone();
                             view! {
                                <button
                                    class=move || if !is_direct { "btn btn-primary btn-sm" } else { "btn btn-ghost btn-sm" }
                                    type="button"
                                    on:click=move |_| {
                                        d.run(Msg::PatchDb(did.clone(), DbPatch::BackendMode(crate::model::DbBackendMode::AgentMode)));
                                    }
                                >"Agent mode"</button>
                            }}
                        </div>
                    </div>
                </div>
            </div>

            // ── Active connections (V2 placeholder) ───────────────────────────
            <div class="card" style="padding:0;overflow:hidden">
                <div style="padding:16px 20px;border-bottom:1px solid var(--border);display:flex;align-items:center;gap:8px">
                    <Icon name="connection" size=14/>
                    <div style="font-size:14px;font-weight:600">"Active connections"</div>
                    <span style="margin-left:auto;font-size:9.5px;font-weight:600;letter-spacing:.04em;\
                                 color:var(--text-3);background:var(--surface-2);padding:2px 6px;border-radius:4px">
                        "V2"
                    </span>
                </div>
                <div style="padding:40px 20px;display:flex;flex-direction:column;align-items:center;\
                             gap:12px;text-align:center">
                    <div style="width:42px;height:42px;border-radius:11px;background:var(--surface-2);\
                                display:grid;place-items:center;color:var(--text-3)">
                        <Icon name="connection" size=20/>
                    </div>
                    <div>
                        <div style="font-size:14px;font-weight:600;margin-bottom:4px">"Live connection map"</div>
                        <div style="font-size:12.5px;color:var(--text-3);max-width:240px;line-height:1.5">
                            "Per-client connection telemetry arrives in V2. Enable the agent binary for early access."
                        </div>
                    </div>
                </div>
            </div>
        </div>
    }
}
