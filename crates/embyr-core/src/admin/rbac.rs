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
pub fn check_rbac(
    action: RbacAction,
    actor_role: Role,
    _actor_id: &UserId,
    all_members: &[AccountMember],
) -> Result<(), RbacError> {
    match action {
        RbacAction::RemoveMember { target_id } => {
            if actor_role < Role::Admin {
                return Err(RbacError::InsufficientRole {
                    actor: actor_role,
                    required: Role::Admin,
                });
            }
            if let Some(target) = all_members.iter().find(|m| m.user_id == target_id) {
                if target.role == Role::Owner {
                    if actor_role < Role::Owner {
                        return Err(RbacError::AdminCannotModifyOwner);
                    }
                    let owner_count = all_members.iter().filter(|m| m.role == Role::Owner).count();
                    if owner_count <= 1 {
                        return Err(RbacError::LastOwner);
                    }
                }
            }
            Ok(())
        }
        RbacAction::ChangeMemberRole { target_id, new_role } => {
            if actor_role < Role::Admin {
                return Err(RbacError::InsufficientRole {
                    actor: actor_role,
                    required: Role::Admin,
                });
            }
            if new_role > actor_role {
                return Err(RbacError::RoleEscalation {
                    actor: actor_role,
                    requested: new_role,
                });
            }
            if let Some(target) = all_members.iter().find(|m| m.user_id == target_id) {
                if target.role == Role::Owner {
                    if actor_role < Role::Owner {
                        return Err(RbacError::AdminCannotModifyOwner);
                    }
                    if new_role < Role::Owner {
                        let owner_count =
                            all_members.iter().filter(|m| m.role == Role::Owner).count();
                        if owner_count <= 1 {
                            return Err(RbacError::LastOwner);
                        }
                    }
                }
            }
            Ok(())
        }
        RbacAction::CreateAdminKey { requested_role } => {
            if actor_role < Role::Admin {
                return Err(RbacError::InsufficientRole {
                    actor: actor_role,
                    required: Role::Admin,
                });
            }
            if requested_role > actor_role {
                return Err(RbacError::RoleEscalation {
                    actor: actor_role,
                    requested: requested_role,
                });
            }
            Ok(())
        }
        RbacAction::DeleteServiceAccount { .. } => {
            if actor_role < Role::Admin {
                return Err(RbacError::InsufficientRole {
                    actor: actor_role,
                    required: Role::Admin,
                });
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::account::AccountId;
    use uuid::Uuid;

    fn make_member(user_id: UserId, role: Role) -> AccountMember {
        AccountMember {
            id: Uuid::new_v4(),
            user_id,
            account_id: AccountId(Uuid::new_v4()),
            role,
            joined_at: None,
        }
    }

    fn owner_id() -> UserId {
        UserId(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap())
    }
    fn admin_id() -> UserId {
        UserId(Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap())
    }
    fn viewer_id() -> UserId {
        UserId(Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap())
    }
    fn second_owner_id() -> UserId {
        UserId(Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap())
    }

    // Behavior 1: Viewer cannot perform any member-management action.
    #[test]
    fn viewer_cannot_remove_any_member() {
        let members = vec![make_member(owner_id(), Role::Owner)];
        let actor = viewer_id();
        let result = check_rbac(
            RbacAction::RemoveMember { target_id: owner_id() },
            Role::Viewer,
            &actor,
            &members,
        );
        assert_eq!(
            result,
            Err(RbacError::InsufficientRole {
                actor: Role::Viewer,
                required: Role::Admin,
            })
        );
    }

    // Behavior 2: Removing the sole owner returns LastOwner.
    #[test]
    fn sole_owner_removal_returns_last_owner_error() {
        let owner = owner_id();
        let members = vec![make_member(owner.clone(), Role::Owner)];
        let result = check_rbac(
            RbacAction::RemoveMember { target_id: owner.clone() },
            Role::Owner,
            &owner,
            &members,
        );
        assert_eq!(result, Err(RbacError::LastOwner));
    }

    // Behavior 3: Admin cannot change an Owner's role.
    #[test]
    fn admin_cannot_change_owner_role() {
        let owner = owner_id();
        let admin = admin_id();
        let members = vec![
            make_member(owner.clone(), Role::Owner),
            make_member(admin.clone(), Role::Admin),
        ];
        let result = check_rbac(
            RbacAction::ChangeMemberRole { target_id: owner.clone(), new_role: Role::Admin },
            Role::Admin,
            &admin,
            &members,
        );
        assert_eq!(result, Err(RbacError::AdminCannotModifyOwner));
    }

    // Behavior 4: Admin cannot create an Owner-level key (role escalation).
    #[test]
    fn admin_cannot_create_owner_key() {
        let admin = admin_id();
        let members = vec![
            make_member(owner_id(), Role::Owner),
            make_member(admin.clone(), Role::Admin),
        ];
        let result = check_rbac(
            RbacAction::CreateAdminKey { requested_role: Role::Owner },
            Role::Admin,
            &admin,
            &members,
        );
        assert_eq!(
            result,
            Err(RbacError::RoleEscalation {
                actor: Role::Admin,
                requested: Role::Owner,
            })
        );
    }

    // Behavior 5: Owner can demote a second owner to Admin.
    #[test]
    fn owner_can_demote_second_owner() {
        let first_owner = owner_id();
        let second_owner = second_owner_id();
        let members = vec![
            make_member(first_owner.clone(), Role::Owner),
            make_member(second_owner.clone(), Role::Owner),
        ];
        let result = check_rbac(
            RbacAction::ChangeMemberRole {
                target_id: second_owner.clone(),
                new_role: Role::Admin,
            },
            Role::Owner,
            &first_owner,
            &members,
        );
        assert_eq!(result, Ok(()));
    }

    // Behavior 6: Removing a non-sole owner is allowed when another owner exists.
    #[test]
    fn second_owner_removal_allowed_when_another_owner_exists() {
        let first_owner = owner_id();
        let second_owner = second_owner_id();
        let members = vec![
            make_member(first_owner.clone(), Role::Owner),
            make_member(second_owner.clone(), Role::Owner),
        ];
        let result = check_rbac(
            RbacAction::RemoveMember { target_id: second_owner.clone() },
            Role::Owner,
            &first_owner,
            &members,
        );
        assert_eq!(result, Ok(()));
    }
}
