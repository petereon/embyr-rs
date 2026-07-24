//! DashboardView — shows all databases as cards.
//!
//! Gated behind #[cfg(feature = "csr")].
//! Reads `AppModel` from context; no local state.

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use crate::model::AppModel;

/// Dashboard overview.
///
/// Shows a card per database when databases are present.
/// Shows an empty-state placeholder when there are none.
#[cfg(feature = "csr")]
#[component]
pub fn DashboardView() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

    let databases = move || model.with(|m| m.databases.clone());
    let has_databases = move || model.with(|m| !m.databases.is_empty());

    view! {
        <div class="page">
            <div class="page-head">
                <h1 class="page-title">"Dashboard"</h1>
            </div>

            <Show
                when=has_databases
                fallback=|| view! {
                    <div class="dashboard-empty">
                        <p class="dashboard-empty-msg">"No databases yet."</p>
                    </div>
                }
            >
                <div class="dashboard-grid">
                    <For
                        each=databases
                        key=|db| db.id.0
                        children=move |db| view! {
                            <div class="db-card">
                                <span class="db-card-name">{db.name.clone()}</span>
                            </div>
                        }
                    />
                </div>
            </Show>
        </div>
    }
}
