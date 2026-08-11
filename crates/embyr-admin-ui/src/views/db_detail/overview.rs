//! DB Detail Overview — KPI tiles + latency area chart + logging row.

#[cfg(feature = "csr")]
use crate::components::{primitives::Toggle, Icon};
#[cfg(feature = "csr")]
use crate::data::{bar_chart_points, display_stats, fmt_num, sparkline_points};
#[cfg(feature = "csr")]
use crate::model::{Database, DbBackendMode, DbPatch, LogRetention};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
#[component]
pub fn DbOverview(db: Database, idx: usize) -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let stats = display_stats(idx);
    let db_id = db.id.clone();
    let db_id2 = db.id.clone();
    let logging = db.logging_enabled;
    let retention_str = match db.log_retention {
        Some(LogRetention::OneDay) => "1 day",
        Some(LogRetention::SevenDays) => "7 days",
        Some(LogRetention::ThirtyDays) => "30 days",
        None => "—",
    };

    let points = sparkline_points(&stats.spark);
    let bars = bar_chart_points(&stats.ops);
    let p95_color = if stats.p95 > 25 {
        "var(--amber)"
    } else {
        "var(--green)"
    };

    view! {
        <div class="fade-in" style="display:flex;flex-direction:column;gap:20px">

            // ── KPI row ──────────────────────────────────────────────────────
            <div class="grid" style="grid-template-columns:repeat(4,1fr)">

                // P95 latency
                <div class="kpi">
                    <div class="kpi-label"><Icon name="gauge" size=14/>"P95 read"</div>
                    <div class="kpi-value" style=format!("color:{p95_color}")>
                        {stats.p95}
                        <span class="unit">"ms"</span>
                    </div>
                    <div class="kpi-foot">
                        "P50 "{stats.p50}"ms · P99 "{stats.p99}"ms"
                    </div>
                </div>

                // Reads today
                <div class="kpi">
                    <div class="kpi-label"><Icon name="zap" size=14/>"Reads today"</div>
                    <div class="kpi-value">{fmt_num(stats.reads)}</div>
                    <div class="kpi-foot">"ops"</div>
                </div>

                // Writes + deletes
                <div class="kpi">
                    <div class="kpi-label"><Icon name="activity" size=14/>"Writes / deletes"</div>
                    <div class="kpi-value">
                        {fmt_num(stats.writes)}
                        <span class="unit" style="color:var(--text-3);font-size:12px;margin-left:5px">
                            "/ "{fmt_num(stats.deletes)}
                        </span>
                    </div>
                    <div class="kpi-foot">"today"</div>
                </div>

                // Live connections (V2 placeholder)
                <div class="kpi">
                    <div class="kpi-label">
                        <Icon name="connection" size=14/>"Live streams"
                        <span style="margin-left:auto;font-size:9.5px;font-weight:600;letter-spacing:.04em;\
                                     color:var(--text-3);background:var(--surface-2);padding:1px 5px;border-radius:4px">
                            "V2"
                        </span>
                    </div>
                    <div class="kpi-value" style="color:var(--text-3)">"—"</div>
                    <div class="kpi-foot" style="color:var(--text-3)">"coming soon"</div>
                </div>
            </div>

            // ── Charts row ────────────────────────────────────────────────────
            <div class="grid" style="grid-template-columns:1fr 1fr">

                // Latency sparkline card
                <div class="card" style="padding:20px">
                    <div style="font-size:13px;font-weight:600;margin-bottom:12px;color:var(--text)">
                        "Read latency — last 12 intervals"
                    </div>
                    <svg width="100%" height="80" viewBox="0 0 300 80" preserveAspectRatio="none"
                         style="display:block">
                        // Gradient fill
                        <defs>
                            <linearGradient id="lg-lat" x1="0" y1="0" x2="0" y2="1">
                                <stop offset="0%" stop-color="var(--accent)" stop-opacity="0.22"/>
                                <stop offset="100%" stop-color="var(--accent)" stop-opacity="0"/>
                            </linearGradient>
                        </defs>
                        // Fill area
                        <polygon
                            points=format!("0,80 {points_with_close} 300,80",
                                points_with_close = points.clone())
                            fill="url(#lg-lat)"
                        />
                        // Line
                        <polyline points=points fill="none" stroke="var(--accent)"
                                  stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
                    </svg>
                    <div class="row" style="margin-top:10px;font-size:11.5px;color:var(--text-3)">
                        <span>"P95: "
                            <span style=format!("color:{p95_color};font-weight:600")>
                                {stats.p95}"ms"
                            </span>
                        </span>
                        <span class="spacer"/>
                        <span>"P99: "{stats.p99}"ms"</span>
                    </div>
                </div>

                // Ops bar chart card
                <div class="card" style="padding:20px">
                    <div style="font-size:13px;font-weight:600;margin-bottom:12px;color:var(--text)">
                        "Operations — last 12 intervals"
                    </div>
                    <svg width="100%" height="80" viewBox="0 0 1 1" preserveAspectRatio="none"
                         style="display:block;height:80px">
                        {bars.iter().map(|&(x, w, h)| {
                            view! {
                                <rect
                                    x=format!("{:.4}", x + 0.015)
                                    y=format!("{:.4}", 1.0 - h)
                                    width=format!("{:.4}", w)
                                    height=format!("{:.4}", h)
                                    fill="var(--accent)"
                                    opacity="0.7"
                                    rx="0.02"
                                />
                            }
                        }).collect_view()}
                    </svg>
                    <div class="row" style="margin-top:10px;font-size:11.5px;color:var(--text-3)">
                        <span>"Peak: "
                            <span style="color:var(--text);font-weight:600">
                                {fmt_num(stats.ops.iter().copied().max().unwrap_or(0) as u64)}
                            </span>
                        </span>
                        <span class="spacer"/>
                        <span>"ops/interval"</span>
                    </div>
                </div>
            </div>

            // ── Backend info ──────────────────────────────────────────────────
            <div class="card" style="padding:18px 20px">
                <div class="row" style="gap:24px;flex-wrap:wrap">
                    <div>
                        <div style="font-size:11px;color:var(--text-3);margin-bottom:3px">"Backend mode"</div>
                        <span class="mono" style="font-size:13px">
                            {match db.backend_mode {
                                DbBackendMode::DirectPg  => "direct_pg",
                                DbBackendMode::AgentMode => "agent_mode",
                            }}
                        </span>
                    </div>
                    <div>
                        <div style="font-size:11px;color:var(--text-3);margin-bottom:3px">"Connection"</div>
                        <span class="mono" style="font-size:13px">{stats.connection_detail}</span>
                    </div>
                    <div>
                        <div style="font-size:11px;color:var(--text-3);margin-bottom:3px">"Region"</div>
                        <span class="mono" style="font-size:13px">{stats.region}</span>
                    </div>
                </div>
            </div>

            // ── Logging row ───────────────────────────────────────────────────
            <div class="card" style="padding:18px 20px">
                <div class="row" style="align-items:center">
                    <div style="flex:1;min-width:0">
                        <div style="font-size:14px;font-weight:600;margin-bottom:3px;display:flex;align-items:center;gap:8px">
                            <Icon name="log" size=15/>
                            "Query logging"
                        </div>
                        <div style="font-size:12.5px;color:var(--text-2)">
                            {if logging {
                                format!("Enabled · retention {}", retention_str)
                            } else {
                                "Off — operations are not recorded".to_string()
                            }}
                        </div>
                    </div>
                    <Toggle
                        checked=logging
                        on_change=Callback::new(move |enabled: bool| {
                            dispatch.run(Msg::SetDbLogging(db_id.clone(), enabled));
                        })
                    />
                </div>

                // Retention selector — shown only when logging is enabled
                {if logging {
                    let d = dispatch.clone();
                    Some(view! {
                        <div style="margin-top:14px;padding-top:14px;border-top:1px solid var(--border);display:flex;align-items:center;gap:10px">
                            <span style="font-size:12.5px;color:var(--text-2)">"Retention"</span>
                            <select
                                class="input"
                                style="width:auto;padding:5px 10px;font-size:12.5px"
                                on:change=move |e| {
                                    let val = event_target_value(&e);
                                    let ret = match val.as_str() {
                                        "1d"  => LogRetention::OneDay,
                                        "30d" => LogRetention::ThirtyDays,
                                        _     => LogRetention::SevenDays,
                                    };
                                    d.run(Msg::PatchDb(
                                        db_id2.clone(),
                                        DbPatch::LoggingEnabled(true, Some(ret)),
                                    ));
                                }
                            >
                                <option value="1d"  selected={db.log_retention == Some(LogRetention::OneDay)}>"1 day"</option>
                                <option value="7d"  selected={db.log_retention == Some(LogRetention::SevenDays) || db.log_retention.is_none()}>"7 days"</option>
                                <option value="30d" selected={db.log_retention == Some(LogRetention::ThirtyDays)}>"30 days"</option>
                            </select>
                        </div>
                    })
                } else {
                    None
                }}
            </div>
        </div>
    }
}
