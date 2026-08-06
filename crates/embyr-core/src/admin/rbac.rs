// SCAFFOLD: true
//! RBAC: pure domain function `check_rbac` + supporting types.
//!
//! No IO. Testable without Axum or a database connection.
//! Enforces sole-owner invariant, self-demotion guard, key role cap.

use crate::admin::account::{AccountMember, Role, UserId};

/// Actions that require RBAC authorization.
#[derive(Debug, Clone)]
pub enum RbacAction {
    /// Remove a member from the account.
    RemoveMember { target_id: UserId },
    /// Change a member's role.
    ChangeMemberRole {
        target_id: UserId,
        new_role: Role,
    },
    /// Create an admin key with a given role.
    CreateAdminKey { requested_role: Role },
    /// Delete a service account.
    DeleteServiceAccount { service_account_id: uuid::Uuid },
}

/// Reasons an RBAC check can fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RbacError {
    /// The actor does not have sufficient role for this action.
    InsufficientRole { actor: Role, required: Role },
    /// The actor is the last Owner and cannot remove or demote themselves.
    LastOwner,
    /// An Admin cannot modify an Owner's membership.
    AdminCannotModifyOwner,
    /// An Actor tried to escalate above their own role.
    RoleEscalation { actor: Role, requested: Role },
}

impl std::fmt::Display for RbacError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RbacError::InsufficientRole { actor, required } => {
                write!(f, "role {:?} is insufficient; required {:?}", actor, required)
            }
            RbacError::LastOwner => {
                write!(f, "Transfer ownership before removing the last Owner.")
            }
            RbacError::AdminCannotModifyOwner => {
                write!(f, "Admin cannot modify an Owner's role or membership.")
            }
            RbacError::RoleEscalation { actor, requested } => {
                write!(
                    f,
                    "Cannot create key with role {:?}; actor role is {:?}",
                    requested, actor
                )
            }
        }
    }
}

/// Pure RBAC authorization check.
///
/// # Invariants enforced
/// - Sole owner cannot be removed or demoted (LastOwner).
/// - Admin cannot modify an Owner's membership (AdminCannotModifyOwner).
/// - Actor cannot create keys with higher role than their own (RoleEscalation).
///
/// # RED scaffold
/// Returns `Ok(())` unconditionally until B-05 implementation wires real logic.
pub fn check_rbac(
    action: RbacAction,
    actor_role: Role,
    actor_id: &UserId,
    all_members: &[AccountMember],
) -> Result<(), RbacError> {
    panic!("Not yet implemented -- RED scaffold: check_rbac")
}
