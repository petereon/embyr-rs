//! View modules for embyr-admin-ui.
//!
//! All items gated behind #[cfg(feature = "csr")].

#[cfg(feature = "csr")]
pub mod auth;
#[cfg(feature = "csr")]
pub mod dashboard;

#[cfg(feature = "csr")]
pub use auth::AuthView;
#[cfg(feature = "csr")]
pub use dashboard::DashboardView;
#[cfg(feature = "csr")]
pub use shell::ShellView;

/// Main application shell — sidebar + topbar + content area.
///
/// Shown when the user is authenticated.
#[cfg(feature = "csr")]
mod shell {
    use leptos::prelude::*;
    use crate::components::{Sidebar, Topbar};
    use crate::views::dashboard::DashboardView;

    #[component]
    pub fn ShellView() -> impl IntoView {
        view! {
            <div class="shell">
                <Sidebar />
                <div class="shell-content">
                    <Topbar />
                    <main class="shell-main">
                        <DashboardView />
                    </main>
                </div>
            </div>
        }
    }
}
