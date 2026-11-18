//! SsoBanner — persistent notice while the account requires single sign-on (admin-oidc-sso).
//!
//! Mounted once in `ShellView`, next to `SuspensionBanner`. Reads `AppModel::sso_notice()`, which
//! follows the session probe, and renders nothing when SSO is not required. No actions: members
//! are told where they stand, Owners are reminded that the policy does not apply to them.

#[cfg(feature = "csr")]
use crate::model::{AppModel, SsoNotice};
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
#[component]
pub fn SsoBanner() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

    view! {
        {move || model.with(|m| m.sso_notice()).map(|notice| {
            let (bg, border) = if notice == SsoNotice::LinkNow {
                ("var(--amber-bg, #fef3c7)", "var(--amber, #d97706)")
            } else {
                ("var(--surface-2, #f1f5f9)", "var(--border, #cbd5e1)")
            };
            view! {
                <div role="status"
                    style=format!("padding: 8px 16px; font-size: 13px; background: {bg}; border-bottom: 1px solid {border}")>
                    {notice.message()}
                </div>
            }
        })}
    }
}
