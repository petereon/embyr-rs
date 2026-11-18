//! UI components for embyr-admin-ui.
//!
//! All items gated behind #[cfg(feature = "csr")].

#[cfg(feature = "csr")]
pub mod primitives;
#[cfg(feature = "csr")]
pub mod secret_modal;
#[cfg(feature = "csr")]
pub mod sidebar;
#[cfg(feature = "csr")]
pub mod sso_banner;
#[cfg(feature = "csr")]
pub mod suspension_banner;
#[cfg(feature = "csr")]
pub mod toast_stack;
#[cfg(feature = "csr")]
pub mod topbar;
#[cfg(feature = "csr")]
pub mod charts;
#[cfg(feature = "csr")]
pub mod icons;

#[cfg(feature = "csr")]
pub use primitives::{Badge, Button, Card, Menu};
#[cfg(feature = "csr")]
pub use secret_modal::{copy_to_clipboard, SecretModal};
#[cfg(feature = "csr")]
pub use sidebar::Sidebar;
#[cfg(feature = "csr")]
pub use sso_banner::SsoBanner;
#[cfg(feature = "csr")]
pub use suspension_banner::SuspensionBanner;
#[cfg(feature = "csr")]
pub use toast_stack::ToastStack;
#[cfg(feature = "csr")]
pub use topbar::Topbar;
#[cfg(feature = "csr")]
pub use icons::Icon;
