//! DB Detail view — tab router for a selected database.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! Tabs: Overview | Connections | API Keys | Query Logs
//! Tab state is local (RwSignal<DbTab>) because Msg::SetDbTab is a V1 no-op in update().

#[cfg(feature = "csr")]
pub mod overview;
#[cfg(feature = "csr")]
pub mod connections;
#[cfg(feature = "csr")]
pub mod keys;

#[cfg(feature = "csr")]
pub use overview::DbOverview;
#[cfg(feature = "csr")]
pub use connections::ConnectionsView;
#[cfg(feature = "csr")]
pub use keys::KeysView;

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use crate::model::{AppModel, DbTab, Section};
#[cfg(feature = "csr")]
use crate::msg::Msg;

/// Database detail wrapper view.
///
/// Reads `model.nav` to determine the selected database and active tab.
/// Falls back to a "no database selected" message if navigation state is inconsistent.
/// Tab state is managed locally via `active_tab: RwSignal<DbTab>`.
#[cfg(feature = "csr")]
#[component]
pub fn DbDetailView() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let selected_db = move || {
        model.with(|m| {
            if let Section::DbDetail(ref id) = m.nav.section {
                m.databases.iter().find(|d| d.id == *id).cloned()
            } else {
                None
            }
        })
    };

    // Local tab selection — SetDbTab is intentionally a no-op in V1 update().
    let active_tab = RwSignal::new(DbTab::Overview);

    view! {
        {move || {
            match selected_db() {
                None => view! {
                    <div class="page">
                        <p>"No database selected."</p>
                    </div>
                }.into_any(),
                Some(db) => {
                    // Extract static values before closures capture `db`.
                    let db_name = db.name.clone();

                    view! {
                        <div class="page page-wide">
                            <div class="page-head">
                                <button
                                    class="btn btn-ghost"
                                    type="button"
                                    on:click=move |_| {
                                        dispatch.run(Msg::NavigateTo(Section::Databases))
                                    }
                                >
                                    "← Databases"
                                </button>
                                <h1 class="page-title mono">{db_name}</h1>
                            </div>

                            // Tab bar — drives local active_tab signal.
                            <div class="tabs">
                                <button
                                    class="tab"
                                    class:tab-active=move || active_tab.get() == DbTab::Overview
                                    type="button"
                                    on:click=move |_| active_tab.set(DbTab::Overview)
                                >
                                    "Overview"
                                </button>
                                <button
                                    class="tab"
                                    class:tab-active=move || active_tab.get() == DbTab::Connections
                                    type="button"
                                    on:click=move |_| active_tab.set(DbTab::Connections)
                                >
                                    "Connections"
                                </button>
                                <button
                                    class="tab"
                                    class:tab-active=move || active_tab.get() == DbTab::Keys
                                    type="button"
                                    on:click=move |_| active_tab.set(DbTab::Keys)
                                >
                                    "API Keys"
                                </button>
                                <button
                                    class="tab"
                                    class:tab-active=move || active_tab.get() == DbTab::Logs
                                    type="button"
                                    on:click=move |_| active_tab.set(DbTab::Logs)
                                >
                                    "Query Logs"
                                </button>
                            </div>

                            // Tab content — reactive on active_tab.
                            {move || match active_tab.get() {
                                DbTab::Overview => {
                                    view! { <DbOverview db=db.clone() /> }.into_any()
                                }
                                DbTab::Connections => {
                                    view! { <ConnectionsView db=db.clone() /> }.into_any()
                                }
                                DbTab::Keys => {
                                    view! { <KeysView db=db.clone() /> }.into_any()
                                }
                                DbTab::Logs => {
                                    view! {
                                        <div class="page">
                                            <p>"Query logs — available in V2."</p>
                                        </div>
                                    }.into_any()
                                }
                            }}
                        </div>
                    }.into_any()
                }
            }
        }}
    }
}
