//! UsageTab — month-to-date usage per database, from `GET /billing?range=month`.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! Renders `AppModel::usage_table_rows()`. There is no storage column (G-49): the server has no
//! storage source, so showing one would show a made-up zero.

#[cfg(feature = "csr")]
use crate::model::AppModel;
#[cfg(feature = "csr")]
use leptos::prelude::*;

/// Usage tab content — per-database reads/writes/deletes for the month so far.
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
