//! LogsView — Query Logs tab for a selected database.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! AC-007-01: When `db.logging_enabled == false`, renders the "Logging is off"
//!            empty state.
//! AC-007-02: When `db.logging_enabled == true`, renders filter controls and
//!            the log-row table (V1: mock placeholder rows).

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use crate::model::Database;

/// Query Logs tab component.
///
/// The `logging_enabled` flag gates between the empty-state panel and the log
/// table. No local signal is needed — the flag is a plain bool captured from
/// the `db` prop at mount time.
///
/// In V1 the table body is a placeholder row ("No log entries."). V2 replaces
/// this with a reactive `<For>` over real log rows fetched via `#[server]`.
#[cfg(feature = "csr")]
#[component]
pub fn LogsView(db: Database) -> impl IntoView {
    let logging_enabled = db.logging_enabled;

    view! {
        <div class="page fade-in">
            <Show
                when=move || logging_enabled
                fallback=|| view! {
                    <div class="logs-empty">
                        <p class="logs-empty-msg">
                            "Logging is off — enable it on the Overview tab."
                        </p>
                    </div>
                }
            >
                // Filter controls
                <div class="logs-controls">
                    <input
                        class="input"
                        type="text"
                        placeholder="Filter by query text…"
                    />
                </div>

                // Log table
                <table class="table logs-table">
                    <thead>
                        <tr>
                            <th>"Timestamp"</th>
                            <th>"Query"</th>
                            <th>"Duration"</th>
                            <th>"Rows"</th>
                        </tr>
                    </thead>
                    <tbody>
                        // V1 placeholder — replaced with real rows in V2.
                        <tr>
                            <td colspan="4" class="logs-table-empty">
                                "No log entries."
                            </td>
                        </tr>
                    </tbody>
                </table>
            </Show>
        </div>
    }
}
