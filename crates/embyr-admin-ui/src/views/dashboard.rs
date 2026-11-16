//! DashboardView — database counts and one card per real database (`GET /projects`).
//!
//! Latency, throughput and sparklines return with `hourly-metrics-rollup` (slice 06); until then they
//! are absent rather than mocked. "New database" returns with slice 05.

#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind};
#[cfg(feature = "csr")]
use crate::components::Icon;
#[cfg(feature = "csr")]
use crate::model::{AppModel, DbStatus, Section};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
#[component]
pub fn DashboardView() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let databases = move || model.with(|m| m.databases.clone());
    let db_count = move || model.with(|m| m.databases.len());
    let active_count =
        move || model.with(|m| m.databases.iter().filter(|d| d.status == DbStatus::Active).count());
    let loading = move || model.with(|m| m.pending.contains(&CallKind::Projects));
    let account_name =
        move || model.with(|m| m.current_user.as_ref().map(|u| u.account_name.clone()).unwrap_or_default());

    view! {
        <div class="page page-wide">
            <div class="page-head">
                <div>
                    <h1 class="page-title">"Dashboard"</h1>
                    <p class="page-sub">
                        "Overview of all databases in "
                        <strong style="color:var(--text);font-weight:550">{account_name}</strong>
                    </p>
                </div>
                <div class="page-head-actions">
                    <button
                        class="btn btn-ghost btn-sm"
                        disabled=loading
                        on:click=move |_| dispatch.run(Msg::Request(ApiCall::LoadProjects))
                    >
                        <Icon name="refresh" size=14/>"Refresh"
                    </button>
                </div>
            </div>

            // KPI row
            <div class="grid" style="grid-template-columns:repeat(2,1fr);margin-bottom:28px">
                <div class="kpi">
                    <div class="kpi-label"><Icon name="database" size=14/>"Databases"</div>
                    <div class="kpi-value">{db_count}</div>
                </div>
                <div class="kpi">
                    <div class="kpi-label"><Icon name="activity" size=14/>"Active"</div>
                    <div class="kpi-value">{active_count}</div>
                </div>
            </div>

            <Show when=loading>
                <p style="color:var(--text-3)">"Loading databases…"</p>
            </Show>

            <div class="grid" style="grid-template-columns:repeat(auto-fill,minmax(300px,1fr))">
                <For
                    each=databases
                    key=|db| db.id.clone()
                    children=move |db| {
                        let db_id = db.id.clone();
                        let (status_cls, status_txt) = match db.status {
                            DbStatus::Active    => ("badge badge-green",   "Active"),
                            DbStatus::Suspended => ("badge badge-amber",   "Suspended"),
                            DbStatus::Deleted   => ("badge badge-neutral", "Deleted"),
                        };
                        let created = db
                            .created_at
                            .map(|d| d.format("%Y-%m-%d").to_string())
                            .unwrap_or_else(|| "—".to_string());
                        view! {
                            <button class="card fade-in db-card"
                                style="text-align:left;padding:16px 18px;display:flex;flex-direction:column;gap:10px;cursor:pointer"
                                on:click=move |_| dispatch.run(Msg::NavigateTo(Section::DbDetail(db_id.clone())))>
                                <div style="display:flex;align-items:center;gap:10px">
                                    <span style="width:30px;height:30px;border-radius:8px;background:var(--accent-soft);\
                                                 color:var(--accent);display:grid;place-items:center;flex-shrink:0">
                                        <Icon name="database" size=15/>
                                    </span>
                                    <div style="min-width:0;flex:1">
                                        <div style="font-weight:600;font-size:14.5px;letter-spacing:-0.01em;\
                                                    overflow:hidden;text-overflow:ellipsis;white-space:nowrap">
                                            {db.name.clone()}
                                        </div>
                                        <div class="mono" style="font-size:11px;color:var(--text-3)">{db.backend_mode.api_value()}</div>
                                    </div>
                                    <span class=status_cls><span class="dot"/>{status_txt}</span>
                                </div>
                                <div style="font-size:11.5px;color:var(--text-2)">"Created "{created}</div>
                            </button>
                        }
                    }
                />
            </div>
        </div>
    }
}
