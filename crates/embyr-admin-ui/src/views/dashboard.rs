//! DashboardView — KPI row and one card per real database (`GET /projects` + `GET /metrics/summary`).
//!
//! Figures are the server's hourly-metrics rollup (ADR-084); a percentile without samples is "—"
//! (AC-AUW-30). "New database" opens the create modal (Owner/Admin).

#[cfg(feature = "csr")]
use crate::api::CallKind;
#[cfg(feature = "csr")]
use crate::components::Icon;
#[cfg(feature = "csr")]
use crate::data::fmt_num;
#[cfg(feature = "csr")]
use crate::model::{latency_label, AppModel, DbStatus, Section};
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
    let loading = move || {
        model.with(|m| {
            m.pending.contains(&CallKind::Projects) || m.pending.contains(&CallKind::MetricsSummary)
        })
    };
    let can_create = move || model.with(|m| m.can_manage_databases());
    let totals = move || model.with(|m| m.summary_totals());
    let reads = move || totals().map(|t| fmt_num(t.reads_today)).unwrap_or_else(|| "—".into());
    let writes = move || totals().map(|t| fmt_num(t.writes_today)).unwrap_or_else(|| "—".into());
    let avg_p95 = move || latency_label(totals().and_then(|t| t.avg_p95_read_ms), false);
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
                        on:click=move |_| dispatch.run(Msg::NavigateTo(Section::Dashboard))
                    >
                        <Icon name="refresh" size=14/>"Refresh"
                    </button>
                    <button
                        class="btn btn-primary btn-sm"
                        type="button"
                        disabled=move || !can_create()
                        title=move || if can_create() { "" } else { "Your role cannot create databases" }
                        on:click=move |_| dispatch.run(Msg::OpenCreateDb)
                    >
                        <Icon name="plus" size=14/>"New database"
                    </button>
                </div>
            </div>

            // KPI row
            <div class="grid" style="grid-template-columns:repeat(4,1fr);margin-bottom:28px">
                <div class="kpi">
                    <div class="kpi-label"><Icon name="database" size=14/>"Databases"</div>
                    <div class="kpi-value">{db_count}</div>
                </div>
                <div class="kpi">
                    <div class="kpi-label"><Icon name="activity" size=14/>"Active"</div>
                    <div class="kpi-value">{active_count}</div>
                </div>
                <div class="kpi">
                    <div class="kpi-label"><Icon name="zap" size=14/>"Reads / writes today"</div>
                    <div class="kpi-value">{reads}<span class="unit" style="font-size:12px;margin-left:5px">"/ "{writes}</span></div>
                </div>
                <div class="kpi">
                    <div class="kpi-label"><Icon name="gauge" size=14/>"Avg P95 read"</div>
                    <div class="kpi-value">{avg_p95}<span class="unit">"ms"</span></div>
                    <div class="kpi-foot">"last 24 h"</div>
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
                        let summary_id = db.id.clone();
                        let figures = move || {
                            model.with(|m| m.metrics_summary.get(&summary_id).cloned()).map(|row| {
                                format!(
                                    "{} reads · {} writes · P95 {} ms",
                                    fmt_num(row.reads_today),
                                    fmt_num(row.writes_today),
                                    latency_label(row.p95_read_ms, false),
                                )
                            })
                        };
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
                                <div style="font-size:12px;color:var(--text-2)">
                                    {move || figures().unwrap_or_else(|| "—".to_string())}
                                </div>
                                <div style="font-size:11.5px;color:var(--text-3)">"Created "{created}</div>
                            </button>
                        }
                    }
                />
            </div>
        </div>
    }
}
