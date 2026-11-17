//! LogsView — Query Logs tab for a selected database, on `GET /projects/:id/query_logs`.
//!
//! The server filters by exact operation and pages by cursor (newest first); the text box filters
//! the rows already loaded (G-33). Time range, sorting and auto-refresh have no API support and are
//! not offered (G-35): Refresh and "Load more" are.

#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind};
#[cfg(feature = "csr")]
use crate::components::Icon;
#[cfg(feature = "csr")]
use crate::model::{AppModel, DbId, LOG_OPS};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
#[component]
pub fn LogsView(id: DbId) -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");
    let logging = Memo::new({
        let id = id.clone();
        move |_| model.with(|m| m.databases.iter().find(|d| d.id == id).is_some_and(|d| d.logging_enabled))
    });
    let can_enable = move || {
        model.with(|m| m.can_manage_databases() && !m.pending.contains(&CallKind::PatchProject))
    };
    let loading = move || model.with(|m| m.pending.contains(&CallKind::QueryLogs));

    // "" = all operations.
    let op = RwSignal::new(String::new());
    let filter = RwSignal::new(String::new());

    let first_page = {
        let id = id.clone();
        move || {
            let op = op.get_untracked();
            ApiCall::LoadQueryLogs { id: id.clone(), op: (!op.is_empty()).then_some(op), after: None }
        }
    };
    // Load on open, after logging is switched on, and when the operation filter changes.
    leptos::prelude::Effect::new({
        let first_page = first_page.clone();
        move |_| {
            op.track();
            if logging.get() {
                dispatch.run(Msg::Request(first_page()));
            }
        }
    });

    let logs = Memo::new({
        let id = id.clone();
        move |_| model.with(|m| m.query_logs.get(&id).cloned().unwrap_or_default())
    });
    let rows = move || {
        let needle = filter.get().trim().to_lowercase();
        logs.get()
            .entries
            .into_iter()
            .filter(|e| {
                needle.is_empty()
                    || [Some(e.op.as_str()), e.collection_path.as_deref(), Some(e.status.as_str()), e.error_code.as_deref()]
                        .into_iter()
                        .flatten()
                        .any(|field| field.to_lowercase().contains(&needle))
            })
            .collect::<Vec<_>>()
    };

    let enable_id = id.clone();
    let more_id = id;

    view! {
        <div class="fade-in" style="display:flex;flex-direction:column;gap:16px">
            {move || (!logging.get()).then(|| {
                let db_id = enable_id.clone();
                // ── Logging off — enable panel ────────────────────────────────
                view! {
                    <div class="card" style="padding:40px 20px;display:flex;flex-direction:column;\
                                             align-items:center;gap:16px;text-align:center">
                        <div style="width:46px;height:46px;border-radius:12px;background:var(--surface-2);\
                                    display:grid;place-items:center;color:var(--text-3)">
                            <Icon name="log" size=22/>
                        </div>
                        <div>
                            <div style="font-size:15px;font-weight:600;margin-bottom:6px">"Query logging is off"</div>
                            <div style="font-size:13px;color:var(--text-2);max-width:320px;line-height:1.55">
                                "Enable logging to record every read and write operation. \
                                 Useful for debugging slow queries and auditing access patterns."
                            </div>
                        </div>
                        <button
                            class="btn btn-primary"
                            type="button"
                            disabled=move || !can_enable()
                            on:click=move |_| {
                                dispatch.run(Msg::Request(ApiCall::SetLogging {
                                    id: db_id.clone(),
                                    enabled: true,
                                    retention: None,
                                }));
                            }
                        >
                            <Icon name="log" size=15/>
                            "Enable query logging"
                        </button>
                    </div>
                }
            })}

            {move || logging.get().then(|| {
                let more_id = more_id.clone();
                let first_page = first_page.clone();
                view! {
                    // ── Filter + controls row ─────────────────────────────────────
                    <div class="row" style="gap:8px">
                        <div style="position:relative;flex:1">
                            <span style="position:absolute;left:10px;top:50%;transform:translateY(-50%);\
                                         color:var(--text-3);pointer-events:none">
                                <Icon name="log" size=14/>
                            </span>
                            <input
                                class="input"
                                style="padding-left:32px"
                                type="text"
                                placeholder="Filter the loaded entries by collection, operation or status…"
                                prop:value=move || filter.get()
                                on:input=move |e| filter.set(event_target_value(&e))
                            />
                        </div>
                        <select
                            class="input"
                            style="width:auto;padding:0 10px;font-size:12.5px"
                            disabled=loading
                            prop:value=move || op.get()
                            on:change=move |e| op.set(event_target_value(&e))
                        >
                            <option value="">"All operations"</option>
                            {LOG_OPS.iter().map(|o| view! { <option value=*o>{*o}</option> }).collect_view()}
                        </select>
                        <button
                            class="btn btn-ghost"
                            type="button"
                            disabled=loading
                            on:click=move |_| { dispatch.run(Msg::Request(first_page())); }
                        >
                            "Refresh"
                        </button>
                    </div>

                    // ── Log table ─────────────────────────────────────────────────
                    <div class="tbl-wrap">
                        <table class="tbl">
                            <thead>
                                <tr>
                                    <th>"Timestamp"</th>
                                    <th>"Operation"</th>
                                    <th>"Collection / path"</th>
                                    <th>"Duration"</th>
                                    <th>"Status"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {move || {
                                    let rows = rows();
                                    if rows.is_empty() {
                                        return view! {
                                            <tr>
                                                <td colspan="5" style="text-align:center;padding:32px;color:var(--text-3);font-size:13px">
                                                    {if loading() {
                                                        "Loading…"
                                                    } else {
                                                        "No log entries match. Operations appear here as they occur."
                                                    }}
                                                </td>
                                            </tr>
                                        }.into_any();
                                    }
                                    rows.into_iter().map(|e| {
                                        let ok = e.status == "ok" || e.status == "success";
                                        view! {
                                            <tr>
                                                <td class="mono">{e.created_at.format("%Y-%m-%d %H:%M:%S").to_string()}</td>
                                                <td>{e.op.clone()}</td>
                                                <td class="mono">{e.collection_path.clone().unwrap_or_else(|| "—".into())}</td>
                                                <td>{e.latency_ms.map(|ms| format!("{ms} ms")).unwrap_or_else(|| "—".into())}</td>
                                                <td>
                                                    <span class=if ok { "badge badge-green" } else { "badge badge-red" }>
                                                        {match &e.error_code {
                                                            Some(code) => format!("{} · {code}", e.status),
                                                            None => e.status.clone(),
                                                        }}
                                                    </span>
                                                </td>
                                            </tr>
                                        }
                                    }).collect_view().into_any()
                                }}
                            </tbody>
                        </table>
                    </div>

                    <div class="row" style="font-size:12px;color:var(--text-3)">
                        <span>
                            {move || {
                                let l = logs.get();
                                format!("{} of {} entries loaded", l.entries.len(), l.total)
                            }}
                        </span>
                        <span class="spacer"/>
                        {move || {
                            let cursor = logs.get().next_cursor().map(str::to_string);
                            cursor.map(|after| {
                                let id = more_id.clone();
                                view! {
                                    <button
                                        class="btn btn-ghost"
                                        type="button"
                                        disabled=loading
                                        on:click=move |_| {
                                            let op = op.get_untracked();
                                            dispatch.run(Msg::Request(ApiCall::LoadQueryLogs {
                                                id: id.clone(),
                                                op: (!op.is_empty()).then_some(op),
                                                after: Some(after.clone()),
                                            }));
                                        }
                                    >
                                        "Load more"
                                    </button>
                                }
                            })
                        }}
                    </div>
                }
            })}
        </div>
    }
}
