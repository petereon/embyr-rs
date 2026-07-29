//! Sidebar navigation — matches embyr Console design.

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use crate::model::{AppModel, Section};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use crate::components::Icon;

#[cfg(feature = "csr")]
#[component]
pub fn Sidebar() -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let db_count = move || model.with(|m| m.databases.len());

    struct NavItem {
        section: Section,
        label: &'static str,
        icon: &'static str,
    }
    let items = vec![
        NavItem { section: Section::Dashboard,  label: "Dashboard",  icon: "dashboard" },
        NavItem { section: Section::Databases,  label: "Databases",  icon: "database" },
        NavItem { section: Section::Billing,    label: "Billing",    icon: "billing" },
        NavItem { section: Section::Identities, label: "Identities", icon: "users" },
        NavItem { section: Section::ApiKeys,    label: "API Keys",   icon: "key" },
    ];

    let dispatch_signout = dispatch.clone();

    view! {
        <aside class="sidebar">
            // Brand
            <div class="brand">
                <svg class="brand-mark" width="26" height="26" viewBox="0 0 32 32" fill="none">
                    <rect width="32" height="32" rx="8" fill="var(--accent)"/>
                    <path d="M7 23 L16 9 L25 23" stroke="white" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/>
                    <path d="M10 19 L22 19" stroke="white" stroke-width="2" stroke-linecap="round" opacity="0.65"/>
                </svg>
                <span class="brand-word">"embyr"</span>
            </div>

            // Account badge (simplified — no dropdown in V1)
            <div class="acct-switch" style="cursor:default">
                <span class="acct-badge">"P"</span>
                <span class="acct-meta">
                    <span class="acct-name">"Personal"</span>
                    <span class="acct-plan">"Pro plan"</span>
                </span>
                <Icon name="chevrons-ud" size=15/>
            </div>

            // Main nav
            <nav class="nav">
                {items.into_iter().map(|item| {
                    let dispatch = dispatch.clone();
                    let section = item.section.clone();
                    let section_cmp = item.section.clone();
                    let is_databases = matches!(item.section, Section::Databases);
                    view! {
                        <button
                            class=move || {
                                let cur = model.with(|m| m.nav.section.clone());
                                if cur == section_cmp { "nav-item active" } else { "nav-item" }
                            }
                            on:click=move |_| dispatch.run(Msg::NavigateTo(section.clone()))
                        >
                            <Icon name=item.icon size=17/>
                            {item.label}
                            {if is_databases {
                                view! { <span class="nav-count">{db_count}</span> }.into_any()
                            } else {
                                view! { <span/> }.into_any()
                            }}
                        </button>
                    }
                }).collect_view()}
            </nav>

            // Footer: settings + sign out
            <div class="sidebar-foot">
                <button
                    class=move || {
                        let cur = model.with(|m| m.nav.section.clone());
                        if cur == Section::Settings { "nav-item active" } else { "nav-item" }
                    }
                    on:click={
                        let d = dispatch.clone();
                        move |_| d.run(Msg::NavigateTo(Section::Settings))
                    }
                >
                    <Icon name="settings" size=17/>
                    "Account Settings"
                </button>
                <div style="height:1px;background:var(--border);margin:6px 0"/>
                <button
                    class="nav-item"
                    on:click=move |_| dispatch_signout.run(Msg::SignOut)
                >
                    <Icon name="logout" size=17/>
                    "Sign out"
                </button>
            </div>
        </aside>
    }
}
