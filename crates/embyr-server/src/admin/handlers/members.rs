// SCAFFOLD: true
//! Member handlers.
//!
//! list_members (GET /admin/v1/members):
//!   Session auth, any role. Includes pending invites (joined_at = null).
//!
//! invite_member (POST /admin/v1/members/invite):
//!   Session auth, Owner or Admin. Creates invitations row. Sends via IEmailSender.
//!   202 { invitation_id, email, role, expires_at }.
//!
//! change_member_role (PATCH /admin/v1/members/:id/role):
//!   Session auth, Owner or Admin. Check sole-owner invariant via check_rbac.
//!   Admin cannot change another Owner's role.
//!
//! remove_member (DELETE /admin/v1/members/:id):
//!   Session auth, Owner or Admin.
//!   Last Owner → 409 "Transfer ownership before removing the last Owner."
//!   Otherwise → 204. Cascade: invalidate all sessions + revoke admin_api_keys.

/// GET /admin/v1/members
///
/// # RED scaffold
pub async fn list_members() {
    panic!("Not yet implemented -- RED scaffold: list_members handler")
}

/// POST /admin/v1/members/invite
///
/// # RED scaffold
pub async fn invite_member() {
    panic!("Not yet implemented -- RED scaffold: invite_member handler")
}

/// PATCH /admin/v1/members/:id/role
///
/// # RED scaffold
pub async fn change_member_role() {
    panic!("Not yet implemented -- RED scaffold: change_member_role handler")
}

/// DELETE /admin/v1/members/:id
///
/// # RED scaffold
pub async fn remove_member() {
    panic!("Not yet implemented -- RED scaffold: remove_member handler")
}
