//! IdentitiesView — Members and Service Accounts tabs.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! AC-009-02: Invite member modal dispatches Msg::MemberInvited.
//! AC-009-04: Role selector per row dispatches Msg::SetMemberRole.
//! AC-009-05: Remove button per row dispatches Msg::RemoveMember.
//! AC-009-06: Sole-Owner guard: role selector and remove disabled when member is
//!            the sole Owner (mirrors the invariant enforced in update.rs).
//! AC-010-02: Service Accounts table with Delete dispatching Msg::DeleteServiceAccount.

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use uuid::Uuid;
#[cfg(feature = "csr")]
use crate::model::{AppModel, Member, Role, ServiceAccount, UserId};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use crate::components::primitives::Modal;

// ── IdentitiesView ───────────────────────────────────────────────────────────

/// Identities page — Members | Service Accounts tabs.
///
/// Top-level nav section for `Section::Identities`. Renders a tab bar and
/// delegates content rendering to `MembersTab` or `ServiceAccountsTab`.
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

/// Members table with Invite modal.
///
/// Renders Email | Role | Last Login | Actions for each member.
/// Sole-Owner guard disables role selector and remove button for the sole Owner.
#[cfg(feature = "csr")]
#[component]
fn MembersTab() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let invite_open = RwSignal::new(false);
    let invite_email = RwSignal::new(String::new());
    let invite_role = RwSignal::new(String::from("viewer"));

    let on_invite_submit = move |_| {
        let email = invite_email.get();
        if email.trim().is_empty() {
            return;
        }
        let role = match invite_role.get().as_str() {
            "owner" => Role::Owner,
            "admin" => Role::Admin,
            _ => Role::Viewer,
        };
        let member = Member {
            id: UserId(Uuid::new_v4()),
            email: email.trim().to_string(),
            display_name: None,
            role,
            pending: true,
            mfa_enabled: false,
            last_login: None,
        };
        dispatch.run(Msg::MemberInvited(member));
        invite_open.set(false);
        invite_email.set(String::new());
        invite_role.set(String::from("viewer"));
    };

    view! {
        <div class="fade-in">
            <div class="page-head-actions" style="margin-bottom: 14px">
                <button
                    class="btn btn-primary"
                    type="button"
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
                            let count_owners = members
                                .iter()
                                .filter(|mbr| mbr.role == Role::Owner)
                                .count();
                            members
                                .into_iter()
                                .map(|member| render_member_row(member, count_owners, dispatch))
                                .collect_view()
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
                            <option value="viewer">"Viewer"</option>
                            <option value="admin">"Admin"</option>
                            <option value="owner">"Owner"</option>
                        </select>
                    </div>
                    <div class="modal-footer">
                        <button
                            class="btn btn-ghost"
                            type="button"
                            on:click=move |_| invite_open.set(false)
                        >
                            "Cancel"
                        </button>
                        <button
                            class="btn btn-primary"
                            type="button"
                            on:click=on_invite_submit
                        >
                            "Send invitation"
                        </button>
                    </div>
                </Modal>
            </Show>
        </div>
    }
}

/// Render a single member row.
///
/// Extracted to keep the reactive block readable.
/// Sole-Owner guard: disables role selector and remove button when this member
/// is the only Owner in the account.
#[cfg(feature = "csr")]
fn render_member_row(
    member: Member,
    count_owners: usize,
    dispatch: Callback<Msg>,
) -> impl IntoView {
    let uid_role = member.id.clone();
    let uid_remove = member.id.clone();
    let is_sole_owner = member.role == Role::Owner && count_owners == 1;

    let role_str = match member.role {
        Role::Owner => "owner",
        Role::Admin => "admin",
        Role::Viewer => "viewer",
    };

    let last_login_display = member
        .last_login
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "—".to_string());

    let pending_badge = if member.pending {
        view! { <span class="badge badge-neutral" style="margin-left:6px">"pending"</span> }.into_any()
    } else {
        view! {}.into_any()
    };

    let sole_owner_tooltip = if is_sole_owner {
        "Cannot change the sole Owner's role"
    } else {
        ""
    };
    let remove_tooltip = if is_sole_owner {
        "Cannot remove the sole Owner"
    } else {
        "Remove member"
    };

    view! {
        <tr>
            <td>
                {member.email.clone()}
                {pending_badge}
            </td>
            <td>
                <select
                    prop:value=role_str
                    disabled=is_sole_owner
                    title=sole_owner_tooltip
                    on:change=move |e| {
                        let new_role = match event_target_value(&e).as_str() {
                            "owner" => Role::Owner,
                            "admin" => Role::Admin,
                            _ => Role::Viewer,
                        };
                        dispatch.run(Msg::SetMemberRole(uid_role.clone(), new_role));
                    }
                >
                    <option value="owner">"Owner"</option>
                    <option value="admin">"Admin"</option>
                    <option value="viewer">"Viewer"</option>
                </select>
            </td>
            <td>{last_login_display}</td>
            <td>
                <button
                    class="btn btn-ghost"
                    type="button"
                    disabled=is_sole_owner
                    title=remove_tooltip
                    on:click=move |_| dispatch.run(Msg::RemoveMember(uid_remove.clone()))
                >
                    "Remove"
                </button>
            </td>
        </tr>
    }
}

// ── ServiceAccountsTab ────────────────────────────────────────────────────────

/// Service Accounts table.
///
/// Renders Name | Created | Actions for each service account.
/// Delete button dispatches `Msg::DeleteServiceAccount`.
#[cfg(feature = "csr")]
#[component]
fn ServiceAccountsTab() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    view! {
        <div class="fade-in">
            <div class="page-head-actions" style="margin-bottom: 14px">
                <button class="btn btn-primary" type="button" disabled=true>
                    "New service account"
                </button>
            </div>

            <div class="tbl-wrap">
                <table class="tbl">
                    <thead>
                        <tr>
                            <th>"Name"</th>
                            <th>"Description"</th>
                            <th>"Created"</th>
                            <th>"Actions"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {move || {
                            let accounts = model.with(|m| m.service_accounts.clone());
                            accounts
                                .into_iter()
                                .map(|sa| render_service_account_row(sa, dispatch))
                                .collect_view()
                        }}
                    </tbody>
                </table>
            </div>
        </div>
    }
}

/// Render a single service account row.
#[cfg(feature = "csr")]
fn render_service_account_row(
    sa: ServiceAccount,
    dispatch: Callback<Msg>,
) -> impl IntoView {
    let sa_id = sa.id.clone();
    let created_display = sa
        .created_at
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "—".to_string());
    let description_display = sa.description.unwrap_or_else(|| "—".to_string());

    view! {
        <tr>
            <td>{sa.name.clone()}</td>
            <td>{description_display}</td>
            <td>{created_display}</td>
            <td>
                <button
                    class="btn btn-ghost"
                    type="button"
                    on:click=move |_| dispatch.run(Msg::DeleteServiceAccount(sa_id.clone()))
                >
                    "Delete"
                </button>
            </td>
        </tr>
    }
}
