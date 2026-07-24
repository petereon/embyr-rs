//! DatabasesView — database list table + CreateDatabaseModal.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! AC-003-01: Renders table with Name, Status, Backend Mode columns.
//! AC-003-02: CreateDatabaseModal dispatches Msg::DatabaseCreated on submit.

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use uuid::Uuid;
#[cfg(feature = "csr")]
use crate::model::{AppModel, Database, DbBackendMode, DbId, DbStatus};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use crate::components::primitives::{Modal, Button};

// ── CreateDatabaseModal ──────────────────────────────────────────────────────

/// Modal form for creating a new database.
///
/// On submit dispatches `Msg::DatabaseCreated` with a newly constructed
/// `Database` value. Backend mode defaults to `DirectPg`.
#[cfg(feature = "csr")]
#[component]
fn CreateDatabaseModal(on_close: Callback<()>) -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let name = RwSignal::new(String::new());
    let backend_mode = RwSignal::new(DbBackendMode::DirectPg);

    let on_submit = move |_| {
        let db_name = name.get();
        if db_name.trim().is_empty() {
            return;
        }
        let new_db = Database {
            id: DbId(Uuid::new_v4()),
            name: db_name.trim().to_string(),
            status: DbStatus::Active,
            backend_mode: backend_mode.get(),
            logging_enabled: false,
            log_retention: None,
            created_at: None,
        };
        dispatch.run(Msg::DatabaseCreated(new_db));
        on_close.run(());
    };

    view! {
        <Modal title="Create database" on_close=on_close>
            <div class="modal-field">
                <label class="modal-label">"Name"</label>
                <input
                    class="modal-input"
                    type="text"
                    placeholder="my-database"
                    prop:value=name
                    on:input=move |e| name.set(event_target_value(&e))
                />
            </div>
            <div class="modal-field">
                <label class="modal-label">"Backend mode"</label>
                <select
                    class="modal-select"
                    on:change=move |e| {
                        let value = event_target_value(&e);
                        backend_mode.set(if value == "agent_mode" {
                            DbBackendMode::AgentMode
                        } else {
                            DbBackendMode::DirectPg
                        });
                    }
                >
                    <option value="direct_pg">"Direct Postgres"</option>
                    <option value="agent_mode">"Agent (mTLS)"</option>
                </select>
            </div>
            <div class="modal-footer">
                <Button on:click=move |_| on_close.run(())>"Cancel"</Button>
                <Button on:click=on_submit>"Create database"</Button>
            </div>
        </Modal>
    }
}

// ── DatabasesView ────────────────────────────────────────────────────────────

/// Databases list page.
///
/// Renders a table with Name, Status, and Backend Mode columns for every
/// database in the model. A "New database" button opens `CreateDatabaseModal`.
#[cfg(feature = "csr")]
#[component]
pub fn DatabasesView() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let modal_open = RwSignal::new(false);

    let databases = move || model.with(|m| m.databases.clone());

    view! {
        <div class="page page-wide">
            <div class="page-head">
                <div>
                    <h1 class="page-title">"Databases"</h1>
                </div>
                <div class="page-head-actions">
                    <button
                        class="btn btn-primary"
                        type="button"
                        on:click=move |_| modal_open.set(true)
                    >
                        "New database"
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
                            key=|db| db.id.0
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

            <Show when=move || modal_open.get() fallback=|| ()>
                <CreateDatabaseModal
                    on_close=Callback::new(move |_| modal_open.set(false))
                />
            </Show>
        </div>
    }
}
