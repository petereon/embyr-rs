//! BillingView — time-range selector + per-database usage breakdown table.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! AC-008-01: Renders a per-database breakdown table.
//! AC-008-02: All databases visible from SetDatabases appear in the table.
//!
//! V1 data source: `model.databases` (populated by `Msg::SetDatabases`).
//! V1 usage metrics (reads, writes, storage) are placeholder "—" values.
//! V2 plan: replace with `#[server]` function that returns real billing data.

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use crate::model::{AppModel, DbBackendMode};

/// Billing time range options.
///
/// Drives the time-range selector dropdown. In V1 the selected range has no
/// effect on the mock data; V2 passes it as a query parameter to the billing API.
#[cfg(feature = "csr")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum BillingRange {
    #[default]
    Last7Days,
    Last30Days,
    Last90Days,
}

#[cfg(feature = "csr")]
impl BillingRange {
    fn label(&self) -> &'static str {
        match self {
            BillingRange::Last7Days => "Last 7 days",
            BillingRange::Last30Days => "Last 30 days",
            BillingRange::Last90Days => "Last 90 days",
        }
    }
}

/// Billing overview view.
///
/// Shows:
/// - A time-range selector dropdown (`RwSignal<BillingRange>`).
/// - A per-database breakdown table with placeholder usage columns.
///
/// When no databases are present, renders an empty-state paragraph.
#[cfg(feature = "csr")]
#[component]
pub fn BillingView() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let range = RwSignal::new(BillingRange::default());

    let databases = move || model.with(|m| m.databases.clone());
    let has_databases = move || model.with(|m| !m.databases.is_empty());

    view! {
        <div class="page">
            <div class="page-head">
                <h1 class="page-title">"Billing"</h1>
                <select
                    class="select"
                    on:change=move |e| {
                        let val = event_target_value(&e);
                        range.set(match val.as_str() {
                            "30" => BillingRange::Last30Days,
                            "90" => BillingRange::Last90Days,
                            _ => BillingRange::Last7Days,
                        });
                    }
                >
                    <option value="7">"Last 7 days"</option>
                    <option value="30">"Last 30 days"</option>
                    <option value="90">"Last 90 days"</option>
                </select>
            </div>

            <p class="billing-range-label">
                {move || range.with(|r| r.label())}
            </p>

            <Show
                when=has_databases
                fallback=|| view! {
                    <p class="billing-empty">
                        "No databases — nothing to bill."
                    </p>
                }
            >
                <table class="table billing-table">
                    <thead>
                        <tr>
                            <th>"Database"</th>
                            <th>"Mode"</th>
                            <th>"Reads"</th>
                            <th>"Writes"</th>
                            <th>"Storage"</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For
                            each=databases
                            key=|db| db.id.0
                            children=move |db| {
                                let name = db.name.clone();
                                let mode = match db.backend_mode {
                                    DbBackendMode::DirectPg => "direct_pg",
                                    DbBackendMode::AgentMode => "agent_mode",
                                };
                                view! {
                                    <tr>
                                        <td class="mono">{name}</td>
                                        <td>{mode}</td>
                                        <td class="tnum">"—"</td>
                                        <td class="tnum">"—"</td>
                                        <td class="tnum">"—"</td>
                                    </tr>
                                }
                            }
                        />
                    </tbody>
                </table>
            </Show>
        </div>
    }
}
