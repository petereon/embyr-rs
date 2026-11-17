//! IdentitiesView — Members and Service Accounts tabs, both on the real API.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! Every action is a request: the model changes only after the server confirms (AC-AUW-19).
//! Viewers see disabled controls; an Admin cannot touch an Owner and cannot hand out a role above
//! their own (the server enforces the same rules, the controls just do not offer the dead ends).
//! A pending invitation row only offers Revoke (AC-AUW-22).

#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind};
#[cfg(feature = "csr")]
use crate::components::primitives::Modal;
#[cfg(feature = "csr")]
use crate::model::{AppModel, Member, Role, ServiceAccount};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
pub fn role_label(role: &Role) -> &'static str {
    match role {
        Role::Owner => "Owner",
        Role::Admin => "Admin",
        Role::Viewer => "Viewer",
    }
}

#[cfg(feature = "csr")]
pub fn role_from_label(label: &str) -> Role {
    match label {
        "Owner" => Role::Owner,
        "Admin" => Role::Admin,
        _ => Role::Viewer,
    }
}

/// `<option>`s for the roles the signed-in user may assign.
#[cfg(feature = "csr")]
pub fn role_options(model: RwSignal<AppModel>) -> impl IntoView {
    model
        .with(|m| m.assignable_roles())
        .into_iter()
        .map(|r| view! { <option value=role_label(&r)>{role_label(&r)}</option> })
        .collect_view()
}

// ── IdentitiesView ───────────────────────────────────────────────────────────

/// Identities page — Members | Service Accounts tabs.
#[cfg(feature = "csr")]
#[component]
pub fn IdentitiesView() -> impl IntoView {
    let active_tab = RwSignal::new("members");

    view! {
        <div class="page page-wide">
            <div class="page-head">
                <h1 class="page-title">"Identities"</h1>
                <p class="page-sub">"People and machines with access to this account"</p>
            </div>

            <div class="tabs">
                <button
                    class="tab"
                    class:tab-active=move || active_tab.get() == "members"
                    type="button"
                    on:click=move |_| active_tab.set("members")
                >
                    "Members"
                </button>
                <button
                    class="tab"
                    class:tab-active=move || active_tab.get() == "service"
                    type="button"
                    on:click=move |_| active_tab.set("service")
                >
                    "Service Accounts"
                </button>
            </div>

            {move || match active_tab.get() {
                "service" => view! { <ServiceAccountsTab /> }.into_any(),
                _ => view! { <MembersTab /> }.into_any(),
            }}
        </div>
    }
}

// ── MembersTab ───────────────────────────────────────────────────────────────

/// Members table with the Invite modal.
#[cfg(feature = "csr")]
#[component]
fn MembersTab() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let invite_open = RwSignal::new(false);
    let invite_email = RwSignal::new(String::new());
    let invite_role = RwSignal::new(String::from("Viewer"));

    let can_manage = move || model.with(|m| m.can_manage_identities());
    let inviting = move || model.with(|m| m.pending.contains(&CallKind::MemberInvite));
    let loading = move || model.with(|m| m.pending.contains(&CallKind::Members));

    let on_invite_submit = move |_| {
        let email = invite_email.get();
        if email.trim().is_empty() {
            return;
        }
        dispatch.run(Msg::Request(ApiCall::InviteMember { email, role: role_from_label(&invite_role.get()) }));
        invite_open.set(false);
        invite_email.set(String::new());
        invite_role.set(String::from("Viewer"));
    };

    view! {
        <div class="fade-in">
            <div class="page-head-actions" style="margin-bottom: 14px">
                <button
                    class="btn btn-ghost"
                    type="button"
                    disabled=loading
                    on:click=move |_| { dispatch.run(Msg::Request(ApiCall::LoadMembers)); }
                >
                    "Refresh"
                </button>
                <button
                    class="btn btn-primary"
                    type="button"
                    disabled=move || !can_manage() || inviting()
                    on:click=move |_| invite_open.set(true)
                >
                    "Invite member"
                </button>
            </div>

            <div class="tbl-wrap">
                <table class="tbl">
                    <thead>
                        <tr>
                            <th>"Email"</th>
                            <th>"Role"</th>
                            <th>"Last Login"</th>
                            <th>"Actions"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {move || {
                            let members = model.with(|m| m.members.clone());
                            if members.is_empty() {
                                return view! {
                                    <tr><td colspan="4" class="empty-state">
                                        {if loading() { "Loading…" } else { "No members yet." }}
                                    </td></tr>
                                }.into_any();
                            }
                            let owners = members.iter().filter(|m| m.role == Role::Owner && !m.pending).count();
                            members
                                .into_iter()
                                .map(|member| render_member_row(member, owners, model, dispatch))
                                .collect_view()
                                .into_any()
                        }}
                    </tbody>
                </table>
            </div>

            <Show when=move || invite_open.get() fallback=|| ()>
                <Modal
                    title="Invite member"
                    on_close=Callback::new(move |_| invite_open.set(false))
                >
                    <div class="modal-field">
                        <label class="modal-label">"Email address"</label>
                        <input
                            class="modal-input"
                            type="email"
                            placeholder="teammate@company.com"
                            prop:value=invite_email
                            on:input=move |e| invite_email.set(event_target_value(&e))
                        />
                    </div>
                    <div class="modal-field">
                        <label class="modal-label">"Role"</label>
                        <select
                            class="modal-select"
                            prop:value=invite_role
                            on:change=move |e| invite_role.set(event_target_value(&e))
                        >
                            {role_options(model)}
                        </select>
                    </div>
                    <div class="modal-footer">
                        <button class="btn btn-ghost" type="button" on:click=move |_| invite_open.set(false)>
                            "Cancel"
                        </button>
                        <button class="btn btn-primary" type="button" on:click=on_invite_submit>
                            "Create invitation"
                        </button>
                    </div>
                </Modal>
            </Show>
        </div>
    }
}

/// One member row. The sole Owner cannot be demoted or removed; a pending invitation can only be
/// revoked; an Admin cannot edit an Owner.
#[cfg(feature = "csr")]
fn render_member_row(
    member: Member,
    owners: usize,
    model: RwSignal<AppModel>,
    dispatch: Callback<Msg>,
) -> impl IntoView {
    let id = member.id.clone();
    let id_role = member.id.clone();
    let pending = member.pending;
    let is_sole_owner = member.role == Role::Owner && !pending && owners == 1;
    let editable = model.with(|m| m.can_edit_member(&member));
    let busy = move || model.with(|m| m.pending.contains(&CallKind::MemberChange));

    let last_login_display = member
        .last_login
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "—".to_string());

    let role_cell = if pending || !editable || is_sole_owner {
        let title = if is_sole_owner { "Cannot change the sole Owner's role" } else { "" };
        view! { <span title=title>{role_label(&member.role)}</span> }.into_any()
    } else {
        let assignable = model.with(|m| m.assignable_roles());
        view! {
            <select
                prop:value=role_label(&member.role)
                disabled=busy
                on:change=move |e| {
                    dispatch.run(Msg::Request(ApiCall::ChangeMemberRole {
                        id: id_role.clone(),
                        role: role_from_label(&event_target_value(&e)),
                    }));
                }
            >
                {assignable.iter().map(|r| view! { <option value=role_label(r)>{role_label(r)}</option> }).collect_view()}
            </select>
        }
        .into_any()
    };

    let (action_label, action_title) = if pending {
        ("Revoke", "Revoke invitation")
    } else if is_sole_owner {
        ("Remove", "Cannot remove the sole Owner")
    } else {
        ("Remove", "Remove member")
    };

    view! {
        <tr>
            <td>
                {member.email.clone()}
                {pending.then(|| view! { <span class="badge badge-neutral" style="margin-left:6px">"pending"</span> })}
            </td>
            <td>{role_cell}</td>
            <td>{last_login_display}</td>
            <td>
                <button
                    class="btn btn-ghost"
                    type="button"
                    disabled=move || !editable || is_sole_owner || busy()
                    title=action_title
                    on:click=move |_| {
                        dispatch.run(Msg::Request(if pending {
                            ApiCall::RevokeInvitation(id.clone())
                        } else {
                            ApiCall::RemoveMember(id.clone())
                        }));
                    }
                >
                    {action_label}
                </button>
            </td>
        </tr>
    }
}

// ── ServiceAccountsTab ────────────────────────────────────────────────────────

/// Service accounts table with a create modal.
#[cfg(feature = "csr")]
#[component]
fn ServiceAccountsTab() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let create_open = RwSignal::new(false);
    let name = RwSignal::new(String::new());
    let description = RwSignal::new(String::new());
    let role = RwSignal::new(String::from("Viewer"));

    let can_manage = move || model.with(|m| m.can_manage_identities());
    let busy = move || model.with(|m| m.pending.contains(&CallKind::ServiceAccountWrite));
    let loading = move || model.with(|m| m.pending.contains(&CallKind::ServiceAccounts));

    let on_create = move |_| {
        if name.get().trim().is_empty() {
            return;
        }
        dispatch.run(Msg::Request(ApiCall::CreateServiceAccount {
            name: name.get(),
            description: Some(description.get()),
            role: role_from_label(&role.get()),
        }));
        create_open.set(false);
        name.set(String::new());
        description.set(String::new());
        role.set(String::from("Viewer"));
    };

    view! {
        <div class="fade-in">
            <div class="page-head-actions" style="margin-bottom: 14px">
                <button
                    class="btn btn-ghost"
                    type="button"
                    disabled=loading
                    on:click=move |_| { dispatch.run(Msg::Request(ApiCall::LoadServiceAccounts)); }
                >
                    "Refresh"
                </button>
                <button
                    class="btn btn-primary"
                    type="button"
                    disabled=move || !can_manage() || busy()
                    on:click=move |_| create_open.set(true)
                >
                    "New service account"
                </button>
            </div>

            <div class="tbl-wrap">
                <table class="tbl">
                    <thead>
                        <tr>
                            <th>"Name"</th>
                            <th>"Description"</th>
                            <th>"Role"</th>
                            <th>"Created"</th>
                            <th>"Actions"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {move || {
                            let accounts = model.with(|m| m.service_accounts.clone());
                            if accounts.is_empty() {
                                return view! {
                                    <tr><td colspan="5" class="empty-state">
                                        {if loading() { "Loading…" } else { "No service accounts yet." }}
                                    </td></tr>
                                }.into_any();
                            }
                            accounts
                                .into_iter()
                                .map(|sa| render_service_account_row(sa, can_manage, busy, dispatch))
                                .collect_view()
                                .into_any()
                        }}
                    </tbody>
                </table>
            </div>

            <Show when=move || create_open.get() fallback=|| ()>
                <Modal
                    title="New service account"
                    on_close=Callback::new(move |_| create_open.set(false))
                >
                    <div class="modal-field">
                        <label class="modal-label">"Name"</label>
                        <input
                            class="modal-input"
                            type="text"
                            placeholder="ci-deployer"
                            prop:value=name
                            on:input=move |e| name.set(event_target_value(&e))
                        />
                    </div>
                    <div class="modal-field">
                        <label class="modal-label">"Description (optional)"</label>
                        <input
                            class="modal-input"
                            type="text"
                            prop:value=description
                            on:input=move |e| description.set(event_target_value(&e))
                        />
                    </div>
                    <div class="modal-field">
                        <label class="modal-label">"Role"</label>
                        <select
                            class="modal-select"
                            prop:value=role
                            on:change=move |e| role.set(event_target_value(&e))
                        >
                            {role_options(model)}
                        </select>
                    </div>
                    <div class="modal-footer">
                        <button class="btn btn-ghost" type="button" on:click=move |_| create_open.set(false)>
                            "Cancel"
                        </button>
                        <button class="btn btn-primary" type="button" on:click=on_create>
                            "Create"
                        </button>
                    </div>
                </Modal>
            </Show>
        </div>
    }
}

/// One service account row. Deleting it revokes its admin keys on the server.
#[cfg(feature = "csr")]
fn render_service_account_row(
    sa: ServiceAccount,
    can_manage: impl Fn() -> bool + Copy + Send + Sync + 'static,
    busy: impl Fn() -> bool + Copy + Send + Sync + 'static,
    dispatch: Callback<Msg>,
) -> impl IntoView {
    let sa_id = sa.id.clone();
    let created_display = sa
        .created_at
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "—".to_string());

    view! {
        <tr>
            <td>{sa.name.clone()}</td>
            <td>{sa.description.clone().unwrap_or_else(|| "—".to_string())}</td>
            <td>{role_label(&sa.role)}</td>
            <td>{created_display}</td>
            <td>
                <button
                    class="btn btn-ghost"
                    type="button"
                    disabled=move || !can_manage() || busy()
                    title="Deleting a service account also revokes its admin keys"
                    on:click=move |_| { dispatch.run(Msg::Request(ApiCall::DeleteServiceAccount(sa_id.clone()))); }
                >
                    "Delete"
                </button>
            </td>
        </tr>
    }
}
