//! DB Detail Overview — KPI tiles, hourly activity, latency percentiles and the logging row.
//!
//! All figures are the server's (`GET /projects/:id/metrics`, hourly metrics rollup, ADR-084).
//! A percentile without samples is "—", never 0 ms (AC-AUW-30).

#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind};
#[cfg(feature = "csr")]
use crate::components::{primitives::Toggle, Icon};
#[cfg(feature = "csr")]
use crate::data::{bar_chart_points, fmt_num};
#[cfg(feature = "csr")]
use crate::model::{latency_label, AppModel, DbId, LatencyStats, LogRetention, MetricsWindow};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
fn latency_row(label: &'static str, stats: &LatencyStats) -> impl IntoView {
    let cell = |v| latency_label(v, stats.saturated);
    view! {
        <tr>
            <td>{label}</td>
            <td class="mono">{cell(stats.p50_ms)}</td>
            <td class="mono">{cell(stats.p95_ms)}</td>
            <td class="mono">{cell(stats.p99_ms)}</td>
            <td class="mono">{stats.samples}</td>
        </tr>
    }
}

#[cfg(feature = "csr")]
#[component]
pub fn DbOverview(id: DbId) -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let db = Memo::new({
        let id = id.clone();
        move |_| model.with(|m| m.databases.iter().find(|d| d.id == id).cloned())
    });
    let metrics = Memo::new({
        let id = id.clone();
        move |_| model.with(|m| m.db_metrics.get(&id).cloned())
    });
    let connection = Memo::new({
        let id = id.clone();
        move |_| model.with(|m| m.connections.get(&id).cloned())
    });
    let metrics_loading = move || model.with(|m| m.pending.contains(&CallKind::DbMetrics));
    let editable = move || {
        model.with(|m| m.can_manage_databases() && !m.pending.contains(&CallKind::PatchProject))
    };

    let window_button = {
        let id = id.clone();
        move |window: MetricsWindow, label: &'static str| {
            let id = id.clone();
            view! {
                <button
                    class=move || {
                        let current = metrics.get().map(|m| m.window).unwrap_or_default();
                        if current == window { "btn btn-primary btn-sm" } else { "btn btn-ghost btn-sm" }
                    }
                    type="button"
                    disabled=metrics_loading
                    on:click=move |_| dispatch.run(Msg::Request(ApiCall::LoadDbMetrics { id: id.clone(), window }))
                >{label}</button>
            }
        }
    };

    view! {
        <div class="fade-in" style="display:flex;flex-direction:column;gap:20px">
            <div class="row" style="gap:8px;align-items:center">
                <span style="font-size:12px;color:var(--text-3)">"Window"</span>
                {window_button(MetricsWindow::Day, "24 h")}
                {window_button(MetricsWindow::Week, "7 d")}
                <Show when=metrics_loading>
                    <span style="font-size:12px;color:var(--text-3)">"Loading…"</span>
                </Show>
            </div>

            {move || match metrics.get() {
                None => view! {
                    <p style="color:var(--text-3)">
                        {move || if metrics_loading() { "Loading metrics…" } else { "No metrics loaded. Use Refresh." }}
                    </p>
                }
                .into_any(),
                Some(m) => {
                    let p95 = latency_label(m.read_latency.p95_ms, m.read_latency.saturated);
                    let p95_color = match m.read_latency.p95_ms {
                        Some(v) if v > 25.0 => "var(--amber)",
                        Some(_) => "var(--green)",
                        None => "var(--text-3)",
                    };
                    let p50 = latency_label(m.read_latency.p50_ms, m.read_latency.saturated);
                    let p99 = latency_label(m.read_latency.p99_ms, m.read_latency.saturated);
                    let errors: u64 = m.series.iter().map(|p| p.errors).sum();
                    let ops: Vec<u64> = m.series.iter().map(|p| p.reads + p.writes + p.deletes).collect();
                    let bars = bar_chart_points(&ops);
                    let peak = ops.iter().copied().max().unwrap_or(0);
                    let hours = ops.len();
                    view! {
                        <div class="grid" style="grid-template-columns:repeat(4,1fr)">
                            <div class="kpi">
                                <div class="kpi-label"><Icon name="gauge" size=14/>"P95 read"</div>
                                <div class="kpi-value" style=format!("color:{p95_color}")>
                                    {p95}<span class="unit">"ms"</span>
                                </div>
                                <div class="kpi-foot">"P50 "{p50}" ms · P99 "{p99}" ms"</div>
                            </div>
                            <div class="kpi">
                                <div class="kpi-label"><Icon name="zap" size=14/>"Reads today"</div>
                                <div class="kpi-value">{fmt_num(m.reads_today)}</div>
                                <div class="kpi-foot">"ops"</div>
                            </div>
                            <div class="kpi">
                                <div class="kpi-label"><Icon name="activity" size=14/>"Writes / deletes"</div>
                                <div class="kpi-value">
                                    {fmt_num(m.writes_today)}
                                    <span class="unit" style="color:var(--text-3);font-size:12px;margin-left:5px">
                                        "/ "{fmt_num(m.deletes_today)}
                                    </span>
                                </div>
                                <div class="kpi-foot">"today"</div>
                            </div>
                            <div class="kpi">
                                <div class="kpi-label"><Icon name="shield" size=14/>"Errors"</div>
                                <div class="kpi-value">{fmt_num(errors)}</div>
                                <div class="kpi-foot">"in window"</div>
                            </div>
                        </div>

                        <div class="grid" style="grid-template-columns:1fr 1fr">
                            <div class="card" style="padding:20px">
                                <div style="font-size:13px;font-weight:600;margin-bottom:12px;color:var(--text)">
                                    "Latency (ms)"
                                </div>
                                <table class="tbl">
                                    <thead>
                                        <tr><th></th><th>"P50"</th><th>"P95"</th><th>"P99"</th><th>"Samples"</th></tr>
                                    </thead>
                                    <tbody>
                                        {latency_row("Reads", &m.read_latency)}
                                        {latency_row("Writes", &m.write_latency)}
                                    </tbody>
                                </table>
                            </div>

                            <div class="card" style="padding:20px">
                                <div style="font-size:13px;font-weight:600;margin-bottom:12px;color:var(--text)">
                                    "Operations per hour — last "{hours}" h"
                                </div>
                                <svg width="100%" height="80" viewBox="0 0 1 1" preserveAspectRatio="none"
                                     style="display:block;height:80px">
                                    {bars.iter().map(|&(x, w, h)| {
                                        view! {
                                            <rect
                                                x=format!("{:.4}", x + 0.003)
                                                y=format!("{:.4}", 1.0 - h)
                                                width=format!("{:.4}", w)
                                                height=format!("{:.4}", h)
                                                fill="var(--accent)"
                                                opacity="0.7"
                                            />
                                        }
                                    }).collect_view()}
                                </svg>
                                <div class="row" style="margin-top:10px;font-size:11.5px;color:var(--text-3)">
                                    <span>"Peak: "
                                        <span style="color:var(--text);font-weight:600">{fmt_num(peak)}</span>
                                    </span>
                                    <span class="spacer"/>
                                    <span>"ops/hour"</span>
                                </div>
                            </div>
                        </div>
                    }
                    .into_any()
                }
            }}

            // ── Backend info ──────────────────────────────────────────────
            <div class="card" style="padding:18px 20px">
                <div class="row" style="gap:24px;flex-wrap:wrap">
                    <div>
                        <div style="font-size:11px;color:var(--text-3);margin-bottom:3px">"Backend mode"</div>
                        <span class="mono" style="font-size:13px">
                            {move || db.get().map(|d| d.backend_mode.api_value()).unwrap_or("—")}
                        </span>
                    </div>
                    <div>
                        <div style="font-size:11px;color:var(--text-3);margin-bottom:3px">"Connection"</div>
                        <span class="mono" style="font-size:13px">
                            {move || connection
                                .get()
                                .map(|c| c.summary.describe())
                                .unwrap_or_else(|| "—".to_string())}
                        </span>
                    </div>
                </div>
            </div>

            // ── Logging row ───────────────────────────────────────────────
            {move || db.get().map(|d| {
                let id = d.id.clone();
                let toggle_id = d.id.clone();
                let logging = d.logging_enabled;
                let retention = d.log_retention.clone();
                let retention_text = retention.as_ref().map(LogRetention::label).unwrap_or_else(|| "—".into());
                view! {
                    <div class="card" style="padding:18px 20px">
                        <div class="row" style="align-items:center">
                            <div style="flex:1;min-width:0">
                                <div style="font-size:14px;font-weight:600;margin-bottom:3px;display:flex;align-items:center;gap:8px">
                                    <Icon name="log" size=15/>"Query logging"
                                </div>
                                <div style="font-size:12.5px;color:var(--text-2)">
                                    {if logging {
                                        format!("Enabled · retention {retention_text}")
                                    } else {
                                        "Off — operations are not recorded".to_string()
                                    }}
                                </div>
                            </div>
                            {move || view! {
                                <Toggle
                                    checked=logging
                                    disabled=!editable()
                                    on_change=Callback::new({
                                        let toggle_id = toggle_id.clone();
                                        move |enabled: bool| dispatch.run(Msg::Request(ApiCall::SetLogging {
                                            id: toggle_id.clone(),
                                            enabled,
                                            retention: None,
                                        }))
                                    })
                                />
                            }}
                        </div>

                        {logging.then(|| {
                            let selected = retention.clone().unwrap_or(LogRetention::SevenDays);
                            let other = match &selected {
                                LogRetention::Days(n) => Some(*n),
                                _ => None,
                            };
                            view! {
                                <div style="margin-top:14px;padding-top:14px;border-top:1px solid var(--border);display:flex;align-items:center;gap:10px">
                                    <span style="font-size:12.5px;color:var(--text-2)">"Retention"</span>
                                    <select
                                        class="input"
                                        style="width:auto;padding:5px 10px;font-size:12.5px"
                                        disabled=move || !editable()
                                        on:change=move |e| {
                                            let days = event_target_value(&e).parse().unwrap_or(7);
                                            dispatch.run(Msg::Request(ApiCall::SetLogging {
                                                id: id.clone(),
                                                enabled: true,
                                                retention: Some(LogRetention::from_days(days)),
                                            }));
                                        }
                                    >
                                        <option value="1" selected={selected == LogRetention::OneDay}>"1 day"</option>
                                        <option value="7" selected={selected == LogRetention::SevenDays}>"7 days"</option>
                                        <option value="30" selected={selected == LogRetention::ThirtyDays}>"30 days"</option>
                                        {other.map(|n| view! {
                                            <option value=n.to_string() selected=true>{format!("{n} days")}</option>
                                        })}
                                    </select>
                                </div>
                            }
                        })}
                    </div>
                }
            })}
        </div>
    }
}
