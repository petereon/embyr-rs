//! View modules for embyr-admin-ui.
//!
//! All items gated behind #[cfg(feature = "csr")].

#[cfg(feature = "csr")]
pub mod auth;
#[cfg(feature = "csr")]
pub mod dashboard;
#[cfg(feature = "csr")]
pub mod databases;
#[cfg(feature = "csr")]
pub mod db_detail;
#[cfg(feature = "csr")]
pub mod identities;
#[cfg(feature = "csr")]
pub mod api_keys;
#[cfg(feature = "csr")]
pub mod billing;
#[cfg(feature = "csr")]
pub mod settings;

#[cfg(feature = "csr")]
pub use auth::AuthView;
#[cfg(feature = "csr")]
pub use dashboard::DashboardView;
#[cfg(feature = "csr")]
pub use databases::DatabasesView;
#[cfg(feature = "csr")]
pub use db_detail::DbDetailView;
#[cfg(feature = "csr")]
pub use identities::IdentitiesView;
#[cfg(feature = "csr")]
pub use api_keys::ApiKeysView;
#[cfg(feature = "csr")]
pub use billing::BillingView;
#[cfg(feature = "csr")]
pub use settings::SettingsView;
#[cfg(feature = "csr")]
pub use shell::ShellView;

/// Main application shell — sidebar + topbar + content area.
///
/// Shown when the user is authenticated. Routes to the correct view based on
/// `model.nav.section`.
#[cfg(feature = "csr")]
mod shell {
    use leptos::prelude::*;
    use crate::components::{Sidebar, Topbar};
    use crate::model::{AppModel, Section};
    use crate::views::billing::BillingView;
    use crate::views::dashboard::DashboardView;
    use crate::views::databases::DatabasesView;
    use crate::views::identities::IdentitiesView;
    use crate::views::api_keys::ApiKeysView;
    use crate::views::settings::SettingsView;

    #[component]
    pub fn ShellView() -> impl IntoView {
        let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

        view! {
            <div class="shell">
                <Sidebar />
                <div class="shell-content">
                    <Topbar />
                    <main class="shell-main">
                        {move || match model.with(|m| m.nav.section.clone()) {
                            Section::Databases => view! { <DatabasesView /> }.into_any(),
                            Section::Identities => view! { <IdentitiesView /> }.into_any(),
                            Section::ApiKeys => view! { <ApiKeysView /> }.into_any(),
                            Section::Billing => view! { <BillingView /> }.into_any(),
                            Section::Settings => view! { <SettingsView /> }.into_any(),
                            _ => view! { <DashboardView /> }.into_any(),
                        }}
                    </main>
                </div>
            </div>
        }
    }
}
