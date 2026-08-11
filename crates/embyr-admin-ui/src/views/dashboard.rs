//! DashboardView — KPI row + database cards matching the embyr Console design.

#[cfg(feature = "csr")]
use crate::components::Icon;
#[cfg(feature = "csr")]
use crate::data::{display_stats, fmt_num, sparkline_points};
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
    let total_reads = move || {
        model.with(|m| {
            m.databases
                .iter()
                .enumerate()
                .map(|(i, _)| display_stats(i).reads)
                .sum::<u64>()
        })
    };
    let avg_p95 = move || {
        model.with(|m| {
            let active: Vec<_> = m
                .databases
                .iter()
                .enumerate()
                .filter(|(_, d)| d.status == DbStatus::Active)
                .collect();
            if active.is_empty() {
                return 0u32;
            }
            active
                .iter()
                .map(|(i, _)| display_stats(*i).p95)
                .sum::<u32>()
                / active.len() as u32
        })
    };
    let p95_color = move || {
        if avg_p95() > 25 {
            "var(--amber)"
        } else {
            "var(--green)"
        }
    };

    view! {
        <div class="page page-wide">
            <div class="page-head">
                <div>
                    <h1 class="page-title">"Dashboard"</h1>
                    <p class="page-sub">
                        "Overview of all databases in "
                        <strong style="color:var(--text);font-weight:550">"Personal"</strong>
                        " · live as of just now"
                    </p>
                </div>
                <div class="page-head-actions">
                    <button class="btn btn-ghost btn-sm">
                        <Icon name="refresh" size=14/>"Refresh"
                    </button>
                    <button class="btn btn-primary btn-sm">
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
                    <div class="kpi-label">
                        <Icon name="activity" size=14/>"Active streams"
                        <span style="margin-left:auto;font-size:9.5px;font-weight:600;letter-spacing:.04em;\
                                     color:var(--text-3);background:var(--surface-2);padding:1px 5px;border-radius:4px">"V2"</span>
                    </div>
                    <div class="kpi-value" style="color:var(--text-3)">"—"</div>
                </div>
                <div class="kpi">
                    <div class="kpi-label"><Icon name="zap" size=14/>"Reads today"</div>
                    <div class="kpi-value">{move || fmt_num(total_reads())}</div>
                </div>
                <div class="kpi">
                    <div class="kpi-label"><Icon name="gauge" size=14/>"Avg P95 read"</div>
                    <div class="kpi-value" style=move || format!("color:{}", p95_color())>
                        {avg_p95}<span class="unit">"ms"</span>
                    </div>
                </div>
            </div>

            // Section label
            <div class="row" style="margin-bottom:14px">
                <h2 style="font-size:14px;font-weight:600;margin:0">"Databases"</h2>
                <span class="tag" style="margin-left:2px">{db_count}</span>
                <div class="spacer"/>
            </div>

            // Cards
            <div class="grid" style="grid-template-columns:repeat(auto-fill,minmax(300px,1fr))">
                <For
                    each=databases
                    key=|db| db.id.0
                    children=move |db| {
                        let dispatch = dispatch.clone();
                        let db_id = db.id.clone();
                        let idx = model.with(|m| m.databases.iter().position(|d| d.id == db.id).unwrap_or(0));
                        let stats  = display_stats(idx);
                        let points = sparkline_points(&stats.spark);
                        let spark_color = if db.status == DbStatus::Active { "var(--accent)" } else { "var(--text-3)" };
                        let (status_cls, status_txt) = match db.status {
                            DbStatus::Active    => ("badge badge-green",   "Active"),
                            DbStatus::Suspended => ("badge badge-amber",   "Suspended"),
                            DbStatus::Deleted   => ("badge badge-neutral", "Deleted"),
                        };
                        let reads_s   = fmt_num(stats.reads);
                        let writes_s  = fmt_num(stats.writes);
                        let deletes_s = fmt_num(stats.deletes);
                        let is_active = db.status == DbStatus::Active;
                        let p95 = stats.p95;

                        view! {
                            <button class="card fade-in"
                                style="text-align:left;padding:0;overflow:hidden;display:flex;flex-direction:column;\
                                       cursor:pointer;transition:border-color .14s"
                                on:click=move |_| dispatch.run(Msg::NavigateTo(Section::DbDetail(db_id.clone())))
                                onmouseenter="this.style.borderColor='var(--border-2)'"
                                onmouseleave="this.style.borderColor='var(--border)'">

                                // Header
                                <div style="padding:16px 18px 12px;display:flex;align-items:center;gap:10px">
                                    <span style="width:30px;height:30px;border-radius:8px;background:var(--accent-soft);\
                                                 color:var(--accent);display:grid;place-items:center;flex-shrink:0">
                                        <Icon name="database" size=15/>
                                    </span>
                                    <div style="min-width:0;flex:1">
                                        <div style="font-weight:600;font-size:14.5px;letter-spacing:-0.01em;\
                                                    overflow:hidden;text-overflow:ellipsis;white-space:nowrap">
                                            {db.name.clone()}
                                        </div>
                                        <div class="mono" style="font-size:11px;color:var(--text-3)">{stats.region}</div>
                                    </div>
                                    <span class=status_cls><span class="dot"/>{status_txt}</span>
                                </div>

                                // Sparkline
                                <div style="padding:0 18px 4px">
                                    <svg width="100%" height="42" viewBox="0 0 300 42" preserveAspectRatio="none">
                                        <polyline points=points fill="none" stroke=spark_color
                                                  stroke-width="1.8" stroke-linecap="round"
                                                  stroke-linejoin="round" opacity="0.85"/>
                                    </svg>
                                </div>

                                // Stats grid
                                <div style="display:grid;grid-template-columns:1fr 1fr;gap:1px;\
                                            background:var(--border);border-top:1px solid var(--border);margin-top:8px">
                                    <div style="background:var(--surface);padding:11px 18px">
                                        <div style="font-size:11px;color:var(--text-3)">"P95 read"</div>
                                        <div class="tnum" style="font-size:18px;font-weight:600;margin-top:2px">
                                            {if is_active {
                                                view! { <span>{p95}<span style="font-size:11px;color:var(--text-3);margin-left:2px;font-weight:500">"ms"</span></span> }.into_any()
                                            } else {
                                                view! { <span>"—"</span> }.into_any()
                                            }}
                                        </div>
                                    </div>
                                    <div style="background:var(--surface);padding:11px 18px">
                                        <div style="font-size:11px;color:var(--text-3)">"Live streams"</div>
                                        <div class="tnum" style="font-size:18px;font-weight:600;margin-top:2px;color:var(--text-3)">
                                            "—"
                                            <span style="font-size:9.5px;color:var(--text-3);margin-left:6px;font-weight:600;\
                                                         letter-spacing:.04em;vertical-align:middle;background:var(--surface-2);\
                                                         padding:1px 5px;border-radius:4px">"V2"</span>
                                        </div>
                                    </div>
                                </div>

                                // Footer
                                <div style="display:flex;gap:14px;padding:11px 18px;\
                                            border-top:1px solid var(--border);font-size:11.5px;color:var(--text-2)">
                                    <span><span class="tnum" style="color:var(--text);font-weight:550">{reads_s}</span>" reads"</span>
                                    <span><span class="tnum" style="color:var(--text);font-weight:550">{writes_s}</span>" writes"</span>
                                    <span><span class="tnum" style="color:var(--text);font-weight:550">{deletes_s}</span>" deletes"</span>
                                </div>
                            </button>
                        }
                    }
                />

                // Dashed "create" card
                <button class="card"
                    style="display:flex;flex-direction:column;align-items:center;justify-content:center;\
                           gap:10px;min-height:180px;cursor:pointer;color:var(--text-3);\
                           border-style:dashed;background:transparent"
                    onmouseenter="this.style.color='var(--accent)';this.style.borderColor='var(--accent-line)'"
                    onmouseleave="this.style.color='var(--text-3)';this.style.borderColor='var(--border)'">
                    <Icon name="plus" size=22/>
                    <span style="font-size:13px;font-weight:550">"Create database"</span>
                </button>
            </div>
        </div>
    }
}
