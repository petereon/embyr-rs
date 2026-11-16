//! DB Detail view — header, actions and tab router for a selected database.
//!
//! Every figure comes from the admin API (`GET /projects/:id` and friends). Delete returns with
//! slice 05, which needs the server-side dual-auth `DELETE /projects/:id`.

#[cfg(feature = "csr")]
pub mod connections;
#[cfg(feature = "csr")]
pub mod keys;
#[cfg(feature = "csr")]
pub mod logs;
#[cfg(feature = "csr")]
pub mod overview;

#[cfg(feature = "csr")]
pub use connections::ConnectionsView;
#[cfg(feature = "csr")]
pub use keys::KeysView;
#[cfg(feature = "csr")]
pub use logs::LogsView;
#[cfg(feature = "csr")]
pub use overview::DbOverview;

#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind};
#[cfg(feature = "csr")]
use crate::components::{copy_to_clipboard, Icon};
#[cfg(feature = "csr")]
use crate::model::{AppModel, DbId, DbStatus, DbTab, Section};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

/// Picks the database from the navigation state. The body is keyed on the id, so its local state
/// (open tab, menu) survives unrelated model updates such as a metrics response arriving.
#[cfg(feature = "csr")]
#[component]
pub fn DbDetailView() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let selected = Memo::new(move |_| {
        model.with(|m| match &m.nav.section {
            Section::DbDetail(id) => Some(id.clone()),
            _ => None,
        })
    });

    view! {
        {move || match selected.get() {
            Some(id) => view! { <DbDetailBody id=id/> }.into_any(),
            None => view! {
                <div class="page"><p style="color:var(--text-3)">"No database selected."</p></div>
            }
            .into_any(),
        }}
    }
}

#[cfg(feature = "csr")]
#[component]
fn DbDetailBody(id: DbId) -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let db = Memo::new({
        let id = id.clone();
        move |_| model.with(|m| m.databases.iter().find(|d| d.id == id).cloned())
    });
    let loading = move || {
        model.with(|m| {
            m.pending.contains(&CallKind::Project)
                || m.pending.contains(&CallKind::DbMetrics)
                || m.pending.contains(&CallKind::Connection)
        })
    };
    let patching = move || model.with(|m| m.pending.contains(&CallKind::PatchProject));
    let can_manage = move || model.with(|m| m.can_manage_databases());

    let active_tab = RwSignal::new(DbTab::Overview);
    let show_more = RwSignal::new(false);
    let copied_id = RwSignal::new(false);

    let refresh_id = id.clone();
    let copy_id = id.clone();
    let tab_id = id.clone();

    view! {
        <div class="page page-wide fade-in">
            // ── Page head ─────────────────────────────────────────────────
            <div class="page-head" style="margin-bottom:18px;align-items:flex-start">
                <button
                    class="btn btn-ghost btn-icon-only"
                    type="button"
                    style="margin-top:3px"
                    on:click=move |_| dispatch.run(Msg::NavigateTo(Section::Databases))
                >
                    <Icon name="arrowLeft" size=17/>
                </button>

                {move || match db.get() {
                    None => view! {
                        <div style="flex:1;min-width:0">
                            <p style="color:var(--text-3)">
                                {move || if loading() { "Loading database…" } else { "Database not found." }}
                            </p>
                        </div>
                    }
                    .into_any(),
                    Some(d) => {
                        let (status_cls, status_txt) = match d.status {
                            DbStatus::Active => ("badge badge-green", "Active"),
                            DbStatus::Suspended => ("badge badge-amber", "Suspended"),
                            DbStatus::Deleted => ("badge badge-neutral", "Deleted"),
                        };
                        let created = d
                            .created_at
                            .map(|t| t.format("%Y-%m-%d").to_string())
                            .unwrap_or_else(|| "—".to_string());
                        view! {
                            <div style="flex:1;min-width:0">
                                <div class="row" style="gap:11px;align-items:center">
                                    <h1 class="page-title mono" style="margin:0">{d.name.clone()}</h1>
                                    <span class=status_cls><span class="dot"/>{status_txt}</span>
                                </div>
                                <p class="page-sub" style="margin:4px 0 0">
                                    <span class="mono">{d.backend_mode.api_value()}</span>
                                    " · created "{created}
                                </p>
                            </div>
                        }
                        .into_any()
                    }
                }}

                <div class="page-head-actions" style="margin-top:3px">
                    <button
                        class="btn btn-ghost btn-sm"
                        type="button"
                        disabled=loading
                        on:click=move |_| dispatch.run(Msg::NavigateTo(Section::DbDetail(refresh_id.clone())))
                    >
                        <Icon name="refresh" size=14/>"Refresh"
                    </button>

                    {move || db.get().map(|d| {
                        let id = d.id.clone();
                        let active = d.status == DbStatus::Active;
                        let disabled = move || patching() || !can_manage();
                        if active {
                            view! {
                                <button
                                    class="btn btn-ghost btn-sm"
                                    type="button"
                                    disabled=disabled
                                    on:click=move |_| dispatch.run(Msg::Request(ApiCall::SetSuspended {
                                        id: id.clone(),
                                        suspended: true,
                                    }))
                                >
                                    <Icon name="suspend" size=14/>"Suspend"
                                </button>
                            }
                            .into_any()
                        } else {
                            view! {
                                <button
                                    class="btn btn-primary btn-sm"
                                    type="button"
                                    disabled=disabled
                                    on:click=move |_| dispatch.run(Msg::Request(ApiCall::SetSuspended {
                                        id: id.clone(),
                                        suspended: false,
                                    }))
                                >
                                    <Icon name="play" size=14/>"Activate"
                                </button>
                            }
                            .into_any()
                        }
                    })}

                    <div style="position:relative">
                        <button
                            class="btn btn-ghost btn-icon-only btn-sm"
                            type="button"
                            on:click=move |_| show_more.update(|v| *v = !*v)
                        >
                            <Icon name="more-h" size=16/>
                        </button>

                        {move || show_more.get().then(|| {
                            let copy_id = copy_id.clone();
                            view! {
                                <div
                                    style="position:fixed;inset:0;z-index:50"
                                    on:click=move |_| show_more.set(false)
                                />
                                <div style="position:absolute;top:36px;right:0;z-index:51;\
                                            background:var(--surface);border:1px solid var(--border);\
                                            border-radius:8px;box-shadow:0 8px 24px rgba(0,0,0,.18);\
                                            min-width:180px;padding:4px">
                                    <button
                                        class="more-menu-btn"
                                        on:click=move |_| {
                                            copy_to_clipboard(&copy_id.0);
                                            copied_id.set(true);
                                            show_more.set(false);
                                        }
                                    >
                                        {move || if copied_id.get() { "✓ Copied" } else { "Copy database ID" }}
                                    </button>
                                </div>
                            }
                        })}
                    </div>
                </div>
            </div>

            // ── Tab bar ───────────────────────────────────────────────────
            <div class="tabs" style="margin-bottom:20px">
                {[
                    (DbTab::Overview, "overview", "Overview"),
                    (DbTab::Connections, "connection", "Connections"),
                    (DbTab::Keys, "key", "API Keys"),
                    (DbTab::Logs, "log", "Query Logs"),
                ]
                .into_iter()
                .map(|(tab, icon, label)| {
                    let tab_cmp = tab.clone();
                    view! {
                        <button
                            class=move || if active_tab.get() == tab_cmp { "tab active" } else { "tab" }
                            type="button"
                            on:click=move |_| active_tab.set(tab.clone())
                        >
                            <Icon name=icon size=14/>
                            {label}
                        </button>
                    }
                })
                .collect_view()}
            </div>

            // ── Tab content ───────────────────────────────────────────────
            {move || {
                let id = tab_id.clone();
                match active_tab.get() {
                    DbTab::Overview => view! { <DbOverview id=id/> }.into_any(),
                    DbTab::Connections => view! { <ConnectionsView id=id/> }.into_any(),
                    DbTab::Keys => view! { <KeysView id=id/> }.into_any(),
                    DbTab::Logs => view! { <LogsView id=id/> }.into_any(),
                }
            }}
        </div>
    }
}
