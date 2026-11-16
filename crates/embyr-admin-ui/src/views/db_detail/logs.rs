//! LogsView — Query Logs tab for a selected database.

#[cfg(feature = "csr")]
use crate::components::Icon;
#[cfg(feature = "csr")]
use crate::api::ApiCall;
#[cfg(feature = "csr")]
use crate::model::{AppModel, DbId};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

// Slice 09 replaces the placeholder table below with the real query-log list; here only the
// "Enable logging" action is real (PATCH), so the tab never offers a local-only change.
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
        model.with(|m| {
            m.can_manage_databases() && !m.pending.contains(&crate::api::CallKind::PatchProject)
        })
    };
    let db_id = id;

    let filter = RwSignal::new(String::new());

    view! {
        <div class="fade-in" style="display:flex;flex-direction:column;gap:16px">
            {move || (!logging.get()).then(|| {
                let db_id = db_id.clone();
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

            {move || logging.get().then(|| view! {
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
                            placeholder="Filter by query text, collection, or op type…"
                            prop:value=move || filter.get()
                            on:input=move |e| filter.set(event_target_value(&e))
                        />
                    </div>
                    <select class="input" style="width:auto;padding:0 10px;font-size:12.5px">
                        <option>"All operations"</option>
                        <option>"reads"</option>
                        <option>"writes"</option>
                        <option>"deletes"</option>
                    </select>
                    <select class="input" style="width:auto;padding:0 10px;font-size:12.5px">
                        <option>"Last 1 h"</option>
                        <option>"Last 6 h"</option>
                        <option>"Last 24 h"</option>
                        <option>"Last 7 d"</option>
                    </select>
                </div>

                // ── Log table ─────────────────────────────────────────────────
                <div class="tbl-wrap">
                    <table class="tbl">
                        <thead>
                            <tr>
                                <th>
                                    <button style="background:none;border:none;cursor:pointer;\
                                                   color:var(--text-2);font-size:12px;font-weight:600;\
                                                   display:flex;align-items:center;gap:5px;padding:0">
                                        "Timestamp"
                                        <Icon name="chevrons-ud" size=12/>
                                    </button>
                                </th>
                                <th>"Operation"</th>
                                <th>"Collection / path"</th>
                                <th>
                                    <button style="background:none;border:none;cursor:pointer;\
                                                   color:var(--text-2);font-size:12px;font-weight:600;\
                                                   display:flex;align-items:center;gap:5px;padding:0">
                                        "Duration"
                                        <Icon name="chevrons-ud" size=12/>
                                    </button>
                                </th>
                                <th>"Docs"</th>
                            </tr>
                        </thead>
                        <tbody>
                            // V1 placeholder — V2 replaces with reactive <For> over real log rows.
                            <tr>
                                <td colspan="5" style="text-align:center;padding:32px;color:var(--text-3);font-size:13px">
                                    "No log entries yet. Operations will appear here as they occur."
                                </td>
                            </tr>
                        </tbody>
                    </table>
                </div>

                <div class="row" style="font-size:12px;color:var(--text-3)">
                    <span>"0 entries"</span>
                    <span class="spacer"/>
                    <span>"Showing last 1 h · auto-refreshes every 30 s"</span>
                </div>
            })}
        </div>
    }
}
