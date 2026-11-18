//! `accounts.sso_required` policy (ADR-086 decision 6, O3). Pure: the handlers load the facts
//! (role, flag, password presence, provider and link state) and ask these functions.
//!
//! Owners are always exempt: they keep password + TOTP, which is the break-glass path when the
//! IdP is down or misconfigured.

use crate::admin::account::Role;

/// May this member authenticate with a password (sign in, or reset one)?
/// `has_password` is false for SSO-only users (NULL `password_hash`).
pub fn may_use_password(role: Role, sso_required: bool, has_password: bool) -> bool {
    has_password && (role == Role::Owner || !sso_required)
}

/// May an invitation be created with `auth_method = 'password'` for `role` (a role label as sent
/// by the client)? Owner invitations are exempt like Owner sign-in; every other role needs SSO
/// while the account requires it.
pub fn password_invite_allowed(role: &str, sso_required: bool) -> bool {
    !sso_required || role == "Owner"
}

/// Lock-out guard for turning `sso_required` on: the account needs an enabled provider and the
/// acting Owner must already hold a link to one of the account's enabled providers, so a working
/// SSO path is known to exist before passwords stop working for everyone else.
pub fn may_enable_sso_required(has_enabled_provider: bool, caller_has_link_to_enabled_provider: bool) -> bool {
    has_enabled_provider && caller_has_link_to_enabled_provider
}

/// Why a role change is refused. `reason()` is the audit label and the 422 `error` code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleChangeReject {
    /// Demoting an Owner of a required account whose target has no link to an enabled provider:
    /// non-Owners cannot use a password there, so the target would be locked out.
    SsoLinkRequired,
    /// Promoting a member without a password: Owners are never SSO-only (O3, break-glass path).
    OwnerNeedsPassword,
}

impl RoleChangeReject {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::SsoLinkRequired => "sso_link_required",
            Self::OwnerNeedsPassword => "owner_requires_password",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::SsoLinkRequired => {
                "This account requires SSO. Link the member to an enabled SSO provider before demoting them from Owner."
            }
            Self::OwnerNeedsPassword => "Only a member with a password can become an Owner.",
        }
    }
}

/// Role-change guard. Only a change across the Owner boundary alters how a member may sign in:
/// Owner -> non-Owner loses the password path while `sso_required` is on, non-Owner -> Owner needs
/// a password to have one. Changes between Admin and Viewer, and no-op changes, never matter.
pub fn check_role_change(
    current: Role,
    new: Role,
    sso_required: bool,
    target_has_enabled_link: bool,
    target_has_password: bool,
) -> Result<(), RoleChangeReject> {
    match (current == Role::Owner, new == Role::Owner) {
        (false, true) if !target_has_password => Err(RoleChangeReject::OwnerNeedsPassword),
        (true, false) if sso_required && !target_has_enabled_link => Err(RoleChangeReject::SsoLinkRequired),
        _ => Ok(()),
    }
}

/// While the account requires SSO, disabling or deleting an enabled provider must leave another
/// enabled provider behind, otherwise nobody but the Owners can sign in. The Owner turns the
/// requirement off first.
pub fn may_remove_enabled_provider(sso_required: bool, other_enabled_providers: i64) -> bool {
    !sso_required || other_enabled_providers > 0
}

/// Unlinking guard for a member who removes their own link(s): a non-Owner of a required account
/// must keep at least one link to an enabled provider (`remaining_enabled_links` after the
/// removal), otherwise they could not sign in any more. Owners keep password + TOTP.
pub fn may_unlink(role: Role, sso_required: bool, remaining_enabled_links: i64) -> bool {
    role == Role::Owner || !sso_required || remaining_enabled_links > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_change_table() {
        use Role::*;
        use RoleChangeReject::*;
        // (current, new, required, has_link, has_password, expected)
        let table = [
            // demoting an Owner: only a required account with no enabled link refuses
            (Owner, Admin, true, false, true, Err(SsoLinkRequired)),
            (Owner, Viewer, true, false, true, Err(SsoLinkRequired)),
            (Owner, Admin, true, false, false, Err(SsoLinkRequired)),
            (Owner, Admin, true, true, true, Ok(())),
            (Owner, Viewer, true, true, true, Ok(())),
            (Owner, Admin, false, false, true, Ok(())),
            (Owner, Viewer, false, false, true, Ok(())),
            (Owner, Admin, false, true, true, Ok(())),
            // Owner -> Owner is a no-op
            (Owner, Owner, true, false, true, Ok(())),
            (Owner, Owner, false, false, false, Ok(())),
            // promoting to Owner: needs a password, whatever the flag or the links
            (Admin, Owner, false, false, true, Ok(())),
            (Viewer, Owner, true, false, true, Ok(())),
            (Viewer, Owner, true, true, true, Ok(())),
            (Admin, Owner, false, true, false, Err(OwnerNeedsPassword)),
            (Viewer, Owner, true, true, false, Err(OwnerNeedsPassword)),
            (Admin, Owner, true, false, false, Err(OwnerNeedsPassword)),
            // between non-Owner roles nothing changes, even for a member already locked out
            (Admin, Viewer, true, false, true, Ok(())),
            (Viewer, Admin, true, false, false, Ok(())),
            (Admin, Admin, true, false, false, Ok(())),
            (Viewer, Admin, false, false, false, Ok(())),
        ];
        for (cur, new, req, link, pw, want) in table {
            assert_eq!(
                check_role_change(cur, new, req, link, pw),
                want,
                "{cur:?}->{new:?} required={req} link={link} pw={pw}"
            );
        }
    }

    #[test]
    fn role_change_reasons_are_audit_labels() {
        for r in [RoleChangeReject::SsoLinkRequired, RoleChangeReject::OwnerNeedsPassword] {
            assert!(crate::admin::audit::is_label(r.reason()));
            assert!(!r.message().is_empty());
        }
    }

    #[test]
    fn removing_an_enabled_provider_table() {
        // (required, other enabled providers, allowed)
        for (required, others, want) in
            [(false, 0, true), (false, 1, true), (false, 3, true), (true, 0, false), (true, 1, true), (true, 5, true)]
        {
            assert_eq!(may_remove_enabled_provider(required, others), want, "required={required} others={others}");
        }
    }

    #[test]
    fn unlink_table() {
        // (role, required, remaining enabled links, allowed)
        let table = [
            (Role::Owner, true, 0, true),
            (Role::Owner, false, 0, true),
            (Role::Admin, true, 0, false),
            (Role::Admin, true, 1, true),
            (Role::Admin, false, 0, true),
            (Role::Viewer, true, 0, false),
            (Role::Viewer, true, 2, true),
            (Role::Viewer, false, 0, true),
        ];
        for (role, required, remaining, want) in table {
            assert_eq!(may_unlink(role, required, remaining), want, "{role:?} required={required} left={remaining}");
        }
    }

    #[test]
    fn password_rule_table() {
        // (role, sso_required, has_password, allowed)
        let table = [
            (Role::Owner, false, true, true),
            (Role::Owner, false, false, false),
            (Role::Owner, true, true, true), // break-glass
            (Role::Owner, true, false, false),
            (Role::Admin, false, true, true),
            (Role::Admin, false, false, false),
            (Role::Admin, true, true, false),
            (Role::Admin, true, false, false),
            (Role::Viewer, false, true, true),
            (Role::Viewer, false, false, false),
            (Role::Viewer, true, true, false),
            (Role::Viewer, true, false, false),
        ];
        for (role, required, has_pw, want) in table {
            assert_eq!(may_use_password(role, required, has_pw), want, "{role:?} required={required} pw={has_pw}");
        }
    }

    #[test]
    fn password_invitations_table() {
        for role in ["Owner", "Admin", "Viewer", "bogus", ""] {
            assert!(password_invite_allowed(role, false), "{role}");
        }
        assert!(password_invite_allowed("Owner", true));
        for role in ["Admin", "Viewer", "bogus", "owner", ""] {
            assert!(!password_invite_allowed(role, true), "{role}");
        }
    }

    #[test]
    fn enabling_needs_provider_and_caller_link() {
        assert!(may_enable_sso_required(true, true));
        assert!(!may_enable_sso_required(true, false));
        assert!(!may_enable_sso_required(false, true));
        assert!(!may_enable_sso_required(false, false));
    }
}
