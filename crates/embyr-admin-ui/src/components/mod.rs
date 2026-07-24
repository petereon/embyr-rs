//! UI components for embyr-admin-ui.
//!
//! All items gated behind #[cfg(feature = "csr")].

#[cfg(feature = "csr")]
pub mod primitives;
#[cfg(feature = "csr")]
pub mod sidebar;
#[cfg(feature = "csr")]
pub mod topbar;
#[cfg(feature = "csr")]
pub mod charts;

#[cfg(feature = "csr")]
pub use primitives::{Badge, Button, Card, Menu};
#[cfg(feature = "csr")]
pub use sidebar::Sidebar;
#[cfg(feature = "csr")]
pub use topbar::Topbar;
