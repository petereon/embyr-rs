//! DatabasesView — database list table (`GET /projects`).
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! AC-003-01: Renders table with Name, Status, Backend Mode columns.
//! Create/delete return with slice 05 (they need the server-side dual-auth routes); until then the
//! view offers no control that would only change local state.

#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind};
#[cfg(feature = "csr")]
use crate::model::{AppModel, DbBackendMode, DbStatus};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

// ── DatabasesView ────────────────────────────────────────────────────────────

/// Databases list page.
///
/// Renders a table with Name, Status, and Backend Mode columns for every
/// database in the model, with a Refresh button.
#[cfg(feature = "csr")]
#[component]
pub fn DatabasesView() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let loading = move || model.with(|m| m.pending.contains(&CallKind::Projects));
    let databases = move || model.with(|m| m.databases.clone());

    view! {
        <div class="page page-wide">
            <div class="page-head">
                <div>
                    <h1 class="page-title">"Databases"</h1>
                </div>
                <div class="page-head-actions">
                    <button
                        class="btn btn-ghost"
                        type="button"
                        disabled=loading
                        on:click=move |_| dispatch.run(Msg::Request(ApiCall::LoadProjects))
                    >
                        "Refresh"
                    </button>
                </div>
            </div>

            <div class="tbl-wrap">
                <table class="tbl">
                    <thead>
                        <tr>
                            <th>"Name"</th>
                            <th>"Status"</th>
                            <th>"Backend Mode"</th>
                            <th>"Actions"</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For
                            each=databases
                            key=|db| db.id.clone()
                            children=move |db| {
                                let db_id = db.id.clone();
                                let status_label = match db.status {
                                    DbStatus::Active => "Active",
                                    DbStatus::Suspended => "Suspended",
                                    DbStatus::Deleted => "Deleted",
                                };
                                let mode_label = match db.backend_mode {
                                    DbBackendMode::DirectPg => "Direct Postgres",
                                    DbBackendMode::AgentMode => "Agent (mTLS)",
                                    DbBackendMode::AwsSecret => "AWS Secrets Manager",
                                    DbBackendMode::GcpSecret => "GCP Secret Manager",
                                };
                                view! {
                                    <tr>
                                        <td>{db.name.clone()}</td>
                                        <td><span class="badge">{status_label}</span></td>
                                        <td><span class="badge badge-neutral">{mode_label}</span></td>
                                        <td>
                                            <button
                                                class="btn btn-ghost"
                                                type="button"
                                                on:click=move |_| {
                                                    dispatch.run(Msg::NavigateTo(
                                                        crate::model::Section::DbDetail(db_id.clone())
                                                    ));
                                                }
                                            >
                                                "Open"
                                            </button>
                                        </td>
                                    </tr>
                                }
                            }
                        />
                    </tbody>
                </table>
            </div>

            <Show when=loading>
                <p style="color:var(--text-3)">"Loading databases…"</p>
            </Show>
        </div>
    }
}
