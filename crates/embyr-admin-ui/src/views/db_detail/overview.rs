//! DB Detail Overview — KPI tiles + logging toggle.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! AC-004-01: Renders KPI tiles with "--" placeholders (V1 mock).
//! AC-004-03: Logging toggle dispatches Msg::SetDbLogging.

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use crate::model::Database;
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use crate::components::primitives::Toggle;

/// Database detail overview panel.
///
/// Shows KPI tiles (latency, reads, writes) with V1 placeholder values and
/// a logging toggle that dispatches `Msg::SetDbLogging` on change.
#[cfg(feature = "csr")]
#[component]
pub fn DbOverview(db: Database) -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let db_id = db.id.clone();
    let logging_enabled = db.logging_enabled;

    view! {
        <div class="fade-in">
            // KPI tiles
            <div class="kpi-row">
                <div class="kpi">
                    <span class="kpi-label">"P95 read latency"</span>
                    <span class="kpi-value">"--"</span>
                    <span class="kpi-unit">"ms"</span>
                </div>
                <div class="kpi">
                    <span class="kpi-label">"Reads today"</span>
                    <span class="kpi-value">"--"</span>
                </div>
                <div class="kpi">
                    <span class="kpi-label">"Writes today"</span>
                    <span class="kpi-value">"--"</span>
                </div>
            </div>

            // Logging toggle row
            <div class="card card-pad logging-row">
                <div class="logging-info">
                    <span class="logging-title">"Query logging"</span>
                    <span class="logging-sub">
                        {if logging_enabled {
                            "Enabled — operations are being recorded"
                        } else {
                            "Off — operations are not being recorded"
                        }}
                    </span>
                </div>
                <Toggle
                    checked=logging_enabled
                    on_change=Callback::new(move |enabled: bool| {
                        dispatch.run(Msg::SetDbLogging(db_id.clone(), enabled));
                    })
                />
            </div>
        </div>
    }
}
