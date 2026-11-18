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

#[cfg(test)]
mod tests {
    use super::*;

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
