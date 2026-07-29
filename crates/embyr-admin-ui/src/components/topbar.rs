//! Topbar — breadcrumbs + notification + user badge.

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use crate::model::{AppModel, Section};
#[cfg(feature = "csr")]
use crate::components::Icon;

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

    view! {
        <header class="topbar">
            <div class="crumbs">
                <span class="crumb-cur">{section_label}</span>
            </div>
            <span class="topbar-spacer"/>
            <div class="row" style="gap:4px">
                <button class="iconbtn" title="Docs"><Icon name="external" size=17/></button>
                <button class="iconbtn" title="Notifications" style="position:relative">
                    <Icon name="bell" size=17/>
                    <span style="position:absolute;top:7px;right:8px;width:6px;height:6px;border-radius:99px;background:var(--accent);border:1.5px solid var(--bg)"/>
                </button>
                <button class="iconbtn" style="width:auto;padding:0 6px;gap:7px">
                    <span style="width:26px;height:26px;border-radius:99px;background:linear-gradient(150deg,var(--accent-2),var(--accent-deep));color:#fff;display:grid;place-items:center;font-size:12px;font-weight:600;flex-shrink:0">
                        "Y"
                    </span>
                    <Icon name="chevrons-ud" size=14/>
                </button>
            </div>
        </header>
    }
}
