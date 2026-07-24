//! DB Detail view — tab router for a selected database.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! V1 renders the Overview tab. Connections, Keys, and Logs tabs are stubs.

#[cfg(feature = "csr")]
pub mod overview;

#[cfg(feature = "csr")]
pub use overview::DbOverview;

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use crate::model::{AppModel, DbTab, Section};
#[cfg(feature = "csr")]
use crate::msg::Msg;

/// Database detail wrapper view.
///
/// Reads `model.nav` to determine the selected database and active tab.
/// Falls back to the Databases list if no database is selected.
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

    view! {
        {move || {
            match selected_db() {
                None => view! {
                    <div class="page">
                        <p>"No database selected."</p>
                    </div>
                }.into_any(),
                Some(db) => {
                    let db_id = db.id.clone();
                    let db_id_for_tab = db_id.clone();
                    view! {
                        <div class="page page-wide">
                            <div class="page-head">
                                <button
                                    class="btn btn-ghost"
                                    type="button"
                                    on:click=move |_| dispatch.run(Msg::NavigateTo(Section::Databases))
                                >
                                    "← Databases"
                                </button>
                                <h1 class="page-title mono">{db.name.clone()}</h1>
                            </div>

                            // Tab bar
                            <div class="tabs">
                                <button
                                    class="tab"
                                    class:tab-active=move || model.with(|m| m.nav.db_tab == DbTab::Overview)
                                    type="button"
                                    on:click=move |_| dispatch.run(Msg::SetDbTab(db_id_for_tab.clone(), DbTab::Overview))
                                >
                                    "Overview"
                                </button>
                                <button class="tab" type="button">"Connections"</button>
                                <button class="tab" type="button">"API Keys"</button>
                                <button class="tab" type="button">"Query Logs"</button>
                            </div>

                            // Tab content — V1: Overview only
                            <Show
                                when=move || model.with(|m| m.nav.db_tab == DbTab::Overview)
                                fallback=|| view! { <div class="page"><p>"Coming soon."</p></div> }
                            >
                                <DbOverview db=db.clone() />
                            </Show>
                        </div>
                    }.into_any()
                }
            }
        }}
    }
}
