//! UsageTab — per-database usage breakdown table.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! AC-104-01/02/03: renders `AppModel::usage_table_rows()` as a table with
//! Database/Reads/Writes/Deletes/Storage columns, single-sourced from the
//! model — no per-view recomputation of usage figures.

#[cfg(feature = "csr")]
use crate::model::AppModel;
#[cfg(feature = "csr")]
use leptos::prelude::*;

/// Usage tab content — per-database reads/writes/deletes/storage table.
#[cfg(feature = "csr")]
#[component]
pub fn UsageTab() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let rows = move || model.with(|m| m.usage_table_rows());

    view! {
        <div class="tbl-wrap">
            <table class="tbl">
                <thead>
                    <tr>
                        <th>"Database"</th>
                        <th>"Reads"</th>
                        <th>"Writes"</th>
                        <th>"Deletes"</th>
                        <th>"Storage"</th>
                    </tr>
                </thead>
                <tbody>
                    {move || {
                        rows()
                            .into_iter()
                            .map(|row| {
                                view! {
                                    <tr>
                                        <td class="td-name">{row.name}</td>
                                        <td class="mono">{row.usage.reads}</td>
                                        <td class="mono">{row.usage.writes}</td>
                                        <td class="mono">{row.usage.deletes}</td>
                                        <td class="mono">{format!("{:.1} GB", row.usage.storage_gb)}</td>
                                    </tr>
                                }
                            })
                            .collect_view()
                    }}
                </tbody>
            </table>
        </div>
    }
}
