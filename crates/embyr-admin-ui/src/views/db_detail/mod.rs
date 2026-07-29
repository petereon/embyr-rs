//! DB Detail view — tab router for a selected database.

#[cfg(feature = "csr")]
pub mod overview;
#[cfg(feature = "csr")]
pub mod connections;
#[cfg(feature = "csr")]
pub mod keys;
#[cfg(feature = "csr")]
pub mod logs;

#[cfg(feature = "csr")]
pub use overview::DbOverview;
#[cfg(feature = "csr")]
pub use connections::ConnectionsView;
#[cfg(feature = "csr")]
pub use keys::KeysView;
#[cfg(feature = "csr")]
pub use logs::LogsView;

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use crate::data::display_stats;
#[cfg(feature = "csr")]
use crate::model::{AppModel, DbBackendMode, DbPatch, DbStatus, DbTab, Section};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use crate::components::Icon;

#[cfg(feature = "csr")]
#[component]
pub fn DbDetailView() -> impl IntoView {
    let model    = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let selected_db = move || {
        model.with(|m| {
            if let Section::DbDetail(ref id) = m.nav.section {
                m.databases.iter().enumerate()
                    .find(|(_, d)| d.id == *id)
                    .map(|(idx, d)| (idx, d.clone()))
            } else {
                None
            }
        })
    };

    let active_tab = RwSignal::new(DbTab::Overview);

    view! {
        {move || {
            match selected_db() {
                None => view! {
                    <div class="page">
                        <p style="color:var(--text-3)">"No database selected."</p>
                    </div>
                }.into_any(),

                Some((idx, db)) => {
                    let stats        = display_stats(idx);
                    let db_name      = db.name.clone();
                    let db_id        = db.id.clone();
                    let db_id_sus    = db.id.clone();
                    let is_active    = db.status == DbStatus::Active;
                    let sdk_key_count = model.with(|m| {
                        m.sdk_keys.get(&db.id).map(|v| v.len()).unwrap_or(0)
                    });

                    let (status_cls, status_txt) = match db.status {
                        DbStatus::Active    => ("badge badge-green",   "Active"),
                        DbStatus::Suspended => ("badge badge-amber",   "Suspended"),
                        DbStatus::Deleted   => ("badge badge-neutral", "Deleted"),
                    };

                    let backend_label = match db.backend_mode {
                        DbBackendMode::DirectPg  => "direct_pg",
                        DbBackendMode::AgentMode => "agent_mode",
                    };

                    let created = stats.created;
                    let region  = stats.region;

                    // More-menu visibility
                    let show_more  = RwSignal::new(false);
                    let show_del   = RwSignal::new(false);
                    let copied_id  = RwSignal::new(false);

                    view! {
                        <div class="page page-wide fade-in">
                            // ── Page head ─────────────────────────────────────────
                            <div class="page-head" style="margin-bottom:18px;align-items:flex-start">
                                // Back button
                                <button
                                    class="btn btn-ghost btn-icon-only"
                                    type="button"
                                    style="margin-top:3px"
                                    on:click={
                                        let d = dispatch.clone();
                                        move |_| d.run(Msg::NavigateTo(Section::Databases))
                                    }
                                >
                                    <Icon name="arrowLeft" size=17/>
                                </button>

                                // Title block
                                <div style="flex:1;min-width:0">
                                    <div class="row" style="gap:11px;align-items:center">
                                        <h1 class="page-title mono" style="margin:0">{db_name.clone()}</h1>
                                        <span class=status_cls>
                                            <span class="dot"/>
                                            {status_txt}
                                        </span>
                                    </div>
                                    <p class="page-sub" style="margin:4px 0 0">
                                        {region}
                                        " · "
                                        <span class="mono">{backend_label}</span>
                                        " · created "
                                        {created}
                                    </p>
                                </div>

                                // Head actions
                                <div class="page-head-actions" style="margin-top:3px">
                                    // Suspend / Activate
                                    {if is_active {
                                        let d = dispatch.clone();
                                        let did = db_id_sus.clone();
                                        view! {
                                            <button
                                                class="btn btn-ghost btn-sm"
                                                type="button"
                                                on:click=move |_| {
                                                    d.run(Msg::PatchDb(did.clone(), DbPatch::Suspended(true)));
                                                }
                                            >
                                                <Icon name="suspend" size=14/>
                                                "Suspend"
                                            </button>
                                        }.into_any()
                                    } else {
                                        let d = dispatch.clone();
                                        let did = db_id_sus.clone();
                                        view! {
                                            <button
                                                class="btn btn-primary btn-sm"
                                                type="button"
                                                on:click=move |_| {
                                                    d.run(Msg::PatchDb(did.clone(), DbPatch::Suspended(false)));
                                                }
                                            >
                                                <Icon name="play" size=14/>
                                                "Activate"
                                            </button>
                                        }.into_any()
                                    }}

                                    // More menu trigger
                                    <div style="position:relative">
                                        <button
                                            class="btn btn-ghost btn-icon-only btn-sm"
                                            type="button"
                                            on:click=move |_| show_more.update(|v| *v = !*v)
                                        >
                                            <Icon name="more-h" size=16/>
                                        </button>

                                        {move || show_more.get().then(|| {
                                            let db_id_copy = db_id.clone();
                                            view! {
                                                // Backdrop
                                                <div
                                                    style="position:fixed;inset:0;z-index:50"
                                                    on:click=move |_| show_more.set(false)
                                                />
                                                // Dropdown
                                                <div style="position:absolute;top:36px;right:0;z-index:51;\
                                                            background:var(--surface);border:1px solid var(--border);\
                                                            border-radius:8px;box-shadow:0 8px 24px rgba(0,0,0,.18);\
                                                            min-width:180px;padding:4px">
                                                    <button
                                                        style="display:flex;align-items:center;gap:8px;width:100%;\
                                                               padding:8px 12px;font-size:13px;background:none;border:none;\
                                                               color:var(--text);cursor:pointer;border-radius:5px;\
                                                               text-align:left"
                                                        onmouseenter="this.style.background='var(--surface-2)'"
                                                        onmouseleave="this.style.background='none'"
                                                        on:click=move |_| {
                                                            // V1: just toggle the "copied" indicator
                                                            let _ = db_id_copy.0.to_string();
                                                            copied_id.set(true);
                                                            show_more.set(false);
                                                        }
                                                    >
                                                        {move || if copied_id.get() { "✓ Copied" } else { "Copy database ID" }}
                                                    </button>
                                                    <div style="height:1px;background:var(--border);margin:4px 0"/>
                                                    <button
                                                        style="display:flex;align-items:center;gap:8px;width:100%;\
                                                               padding:8px 12px;font-size:13px;background:none;border:none;\
                                                               color:var(--red);cursor:pointer;border-radius:5px;\
                                                               text-align:left"
                                                        onmouseenter="this.style.background='color-mix(in oklab,var(--red) 10%,transparent)'"
                                                        onmouseleave="this.style.background='none'"
                                                        on:click=move |_| {
                                                            show_more.set(false);
                                                            show_del.set(true);
                                                        }
                                                    >
                                                        "Delete database"
                                                    </button>
                                                </div>
                                            }
                                        })}
                                    </div>
                                </div>
                            </div>

                            // ── Tab bar ───────────────────────────────────────────
                            <div class="tabs" style="margin-bottom:20px">
                                {[
                                    (DbTab::Overview,     "overview",    "Overview",    None),
                                    (DbTab::Connections,  "connection",  "Connections", None),
                                    (DbTab::Keys,         "key",         "API Keys",    Some(sdk_key_count)),
                                    (DbTab::Logs,         "log",         "Query Logs",  None),
                                ].into_iter().map(|(tab, icon, label, count)| {
                                    let tab_cmp = tab.clone();
                                    view! {
                                        <button
                                            class=move || {
                                                if active_tab.get() == tab_cmp {
                                                    "tab active"
                                                } else { "tab" }
                                            }
                                            type="button"
                                            on:click=move |_| active_tab.set(tab.clone())
                                        >
                                            <Icon name=icon size=14/>
                                            {label}
                                            {count.map(|c| view! {
                                                <span class="tab-count">{c}</span>
                                            })}
                                        </button>
                                    }
                                }).collect_view()}
                            </div>

                            // ── Tab content ───────────────────────────────────────
                            {move || match active_tab.get() {
                                DbTab::Overview    => view! { <DbOverview db=db.clone() idx=idx/> }.into_any(),
                                DbTab::Connections => view! { <ConnectionsView db=db.clone() idx=idx/> }.into_any(),
                                DbTab::Keys        => view! { <KeysView db=db.clone() /> }.into_any(),
                                DbTab::Logs        => view! { <LogsView db=db.clone() /> }.into_any(),
                            }}

                            // ── Delete confirmation modal ─────────────────────────
                            {move || show_del.get().then(|| {
                                view! {
                                    <div
                                        class="modal-backdrop"
                                        on:click=move |_| show_del.set(false)
                                    >
                                        <div
                                            class="modal"
                                            on:click=move |e: web_sys::MouseEvent| e.stop_propagation()
                                        >
                                            <div class="modal-head">
                                                <span class="modal-title">"Delete database?"</span>
                                                <button
                                                    class="modal-close"
                                                    type="button"
                                                    on:click=move |_| show_del.set(false)
                                                >"✕"</button>
                                            </div>
                                            <div class="modal-body">
                                                <p style="color:var(--text-2);font-size:14px;line-height:1.55">
                                                    "This database and all its SDK keys will be permanently deleted. \
                                                     Soft-delete with a 168 h grace window before data purge."
                                                </p>
                                                <div class="row modal-footer" style="justify-content:flex-end">
                                                    <button
                                                        class="btn btn-ghost"
                                                        type="button"
                                                        on:click=move |_| show_del.set(false)
                                                    >"Cancel"</button>
                                                    <button
                                                        class="btn btn-danger"
                                                        type="button"
                                                        on:click=move |_| show_del.set(false)
                                                    >"Delete"</button>
                                                </div>
                                            </div>
                                        </div>
                                    </div>
                                }
                            })}
                        </div>
                    }.into_any()
                }
            }
        }}
    }
}
