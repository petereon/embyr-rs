//! View modules for embyr-admin-ui.
//!
//! All items gated behind #[cfg(feature = "csr")].

#[cfg(feature = "csr")]
pub mod api_keys;
#[cfg(feature = "csr")]
pub mod auth;
#[cfg(feature = "csr")]
pub mod billing;
#[cfg(feature = "csr")]
pub mod dashboard;
#[cfg(feature = "csr")]
pub mod databases;
#[cfg(feature = "csr")]
pub mod db_detail;
#[cfg(feature = "csr")]
pub mod identities;
#[cfg(feature = "csr")]
pub mod settings;

#[cfg(feature = "csr")]
pub use api_keys::ApiKeysView;
#[cfg(feature = "csr")]
pub use auth::AuthView;
#[cfg(feature = "csr")]
pub use billing::BillingView;
#[cfg(feature = "csr")]
pub use dashboard::DashboardView;
#[cfg(feature = "csr")]
pub use databases::DatabasesView;
#[cfg(feature = "csr")]
pub use db_detail::DbDetailView;
#[cfg(feature = "csr")]
pub use identities::IdentitiesView;
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
    use crate::components::{Sidebar, SuspensionBanner, Topbar};
    use crate::model::{AppModel, Section};
    use crate::views::api_keys::ApiKeysView;
    use crate::views::billing::{BillingView, CardModal, UpgradeModal};
    use crate::views::dashboard::DashboardView;
    use crate::views::databases::DatabasesView;
    use crate::views::db_detail::DbDetailView;
    use crate::views::identities::IdentitiesView;
    use crate::views::settings::SettingsView;
    use leptos::prelude::*;

    #[component]
    pub fn ShellView() -> impl IntoView {
        let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

        // Memo: only re-runs routing when nav.section actually changes.
        // Without this, any model mutation (PatchDb, SetDbLogging, etc.) would
        // re-run the match and re-mount the active view, resetting local state.
        let section = Memo::new(move |_| model.with(|m| m.nav.section.clone()));

        view! {
            <div class="app">
                <Sidebar />
                <div class="main">
                    <Topbar />
                    // card-payments (D-6, AC-108-04): mounted above the routed
                    // content, unconditional of the active Section — visible
                    // on every page whenever `read_only()` is true.
                    <SuspensionBanner />
                    <main class="content">
                        {move || match section.get() {
                            Section::Databases   => view! { <DatabasesView /> }.into_any(),
                            Section::DbDetail(_) => view! { <DbDetailView /> }.into_any(),
                            Section::Identities  => view! { <IdentitiesView /> }.into_any(),
                            Section::ApiKeys     => view! { <ApiKeysView /> }.into_any(),
                            Section::Billing     => view! { <BillingView /> }.into_any(),
                            Section::Settings    => view! { <SettingsView /> }.into_any(),
                            _                    => view! { <DashboardView /> }.into_any(),
                        }}
                    </main>
                </div>
                // card-payments (ADR-019): global modal state — mounted here
                // (not nested inside BillingView) so a cross-cutting caller
                // (SuspensionBanner, Slice 07) can open either modal from any
                // Section via the same Msg dispatch.
                <CardModal />
                <UpgradeModal />
            </div>

        }
    }
}
