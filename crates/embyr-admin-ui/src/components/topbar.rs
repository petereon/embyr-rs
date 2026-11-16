//! Topbar — breadcrumbs + notification + user badge.

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use crate::model::{AppModel, Section};

#[cfg(feature = "csr")]
#[component]
pub fn Topbar() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");

    let section_label = move || model.with(|m| match &m.nav.section {
        Section::Dashboard   => "Dashboard",
        Section::Databases   => "Databases",
        Section::Identities  => "Identities",
        Section::ApiKeys     => "API Keys",
        Section::Billing     => "Billing",
        Section::Settings    => "Account Settings",
        Section::DbDetail(_) => "Database",
        Section::Login       => "",
    });

    let user = move |f: fn(&crate::model::CurrentUser) -> String| {
        model.with(|m| m.current_user.as_ref().map(f).unwrap_or_default())
    };
    let user_name = move || user(|u| u.display_name.clone());
    let user_email = move || user(|u| u.email.clone());
    let user_initial = move || user(|u| u.display_name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default());

    view! {
        <header class="topbar">
            <div class="crumbs">
                <span class="crumb-cur">{section_label}</span>
            </div>
            <span class="topbar-spacer"/>
            <div class="row" style="gap:8px;align-items:center" title=user_email>
                <span style="width:26px;height:26px;border-radius:99px;background:linear-gradient(150deg,var(--accent-2),var(--accent-deep));color:#fff;display:grid;place-items:center;font-size:12px;font-weight:600;flex-shrink:0">
                    {user_initial}
                </span>
                <span style="font-size:12.5px;color:var(--text-2)">{user_name}</span>
            </div>
        </header>
    }
}
