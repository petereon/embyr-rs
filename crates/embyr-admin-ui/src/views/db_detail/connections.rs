//! Connections tab — backend configuration panel.
//!
//! AC-005-01: Shows masked DSN or agent endpoint (read-only).
//! AC-005-02: Edit mode dispatches Msg::PatchDb with correct DbPatch variant.
//!
//! Gated behind #[cfg(feature = "csr")].

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use crate::model::{Database, DbBackendMode, DbPatch};
#[cfg(feature = "csr")]
use crate::msg::Msg;

/// Backend configuration panel for the Connections tab.
///
/// Displays the current backend mode and connection details.
/// "Edit" enables an input field; "Save" dispatches `Msg::PatchDb`.
/// "Cancel" resets to read-only view without dispatching.
#[cfg(feature = "csr")]
#[component]
pub fn ConnectionsView(db: Database) -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let is_direct = db.backend_mode == DbBackendMode::DirectPg;
    let db_id = db.id.clone();

    // Local edit state — does not flow through the TEA model.
    let edit_mode = RwSignal::new(false);
    // Draft holds the pending input value while editing.
    let draft = RwSignal::new(String::new());

    view! {
        <div class="connections-panel fade-in">
            <div class="card">
                <div class="card-head">
                    <div class="col">
                        <h3>"Backend configuration"</h3>
                        <span class="sub">"How this database reaches Postgres"</span>
                    </div>
                    // Header actions: Edit button (read mode) or Cancel + Save (edit mode).
                    {move || if !edit_mode.get() {
                        view! {
                            <button
                                class="btn btn-ghost"
                                type="button"
                                on:click=move |_| edit_mode.set(true)
                            >
                                "Edit"
                            </button>
                        }.into_any()
                    } else {
                        let db_id_save = db_id.clone();
                        view! {
                            <div class="row">
                                <button
                                    class="btn btn-ghost"
                                    type="button"
                                    on:click=move |_| edit_mode.set(false)
                                >
                                    "Cancel"
                                </button>
                                <button
                                    class="btn btn-primary"
                                    type="button"
                                    on:click=move |_| {
                                        let patch = if is_direct {
                                            DbPatch::Dsn(draft.get_untracked())
                                        } else {
                                            DbPatch::AgentEndpoint(draft.get_untracked())
                                        };
                                        dispatch.run(Msg::PatchDb(db_id_save.clone(), patch));
                                        edit_mode.set(false);
                                    }
                                >
                                    "Save"
                                </button>
                            </div>
                        }.into_any()
                    }}
                </div>

                <div class="card-pad">
                    // Read view: masked DSN or agent endpoint.
                    // Edit view: text input for the new value.
                    {move || if !edit_mode.get() {
                        view! {
                            <div class="connection-row">
                                <span class="mono">
                                    {if is_direct { "****@host:5432/dbname" } else { "—" }}
                                </span>
                                {if is_direct {
                                    Some(view! { <span class="tag">"credentials masked"</span> })
                                } else {
                                    None
                                }}
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <input
                                class="input mono"
                                type="text"
                                placeholder=if is_direct {
                                    "postgres://host:5432/db"
                                } else {
                                    "10.0.0.5:9191"
                                }
                                prop:value=move || draft.get()
                            />
                        }.into_any()
                    }}

                    <p class="hint">"Changes take effect on the next connection attempt."</p>
                </div>
            </div>
        </div>
    }
}
