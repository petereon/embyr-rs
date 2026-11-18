//! Pure rules for the two ways an SSO link is created (ADR-086 decision 5): an SSO invitation and
//! an authenticated self-link. Email is only ever compared against the invitation the invitee
//! already holds; it is never a lookup key for an existing user.

/// Why an invitation cannot be completed with this ID token. Static labels for the audit row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteReject {
    /// The ID token carries no `email` claim.
    EmailMissing,
    /// `email_verified` is not the JSON boolean `true`.
    EmailUnverified,
    /// The IdP email differs from the invitation email (case-insensitive).
    EmailMismatch,
}

impl InviteReject {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::EmailMissing => "invite_email_missing",
            Self::EmailUnverified => "invite_email_unverified",
            Self::EmailMismatch => "invite_email_mismatch",
        }
    }
}

/// AC-SSO-11: verified e-mail equal (case-insensitively) to the invitation e-mail.
pub fn check_invite_identity(
    invitation_email: &str,
    idp_email: Option<&str>,
    email_verified: bool,
) -> Result<(), InviteReject> {
    let idp_email = idp_email.ok_or(InviteReject::EmailMissing)?;
    if !email_verified {
        return Err(InviteReject::EmailUnverified);
    }
    if idp_email.to_lowercase() != invitation_email.to_lowercase() {
        return Err(InviteReject::EmailMismatch);
    }
    Ok(())
}

/// An SSO-only member must never be an Owner (O3, break-glass path stays password + TOTP).
pub fn sso_invite_role_allowed(role: &str) -> bool {
    role != "Owner"
}

/// SSO invitees choose no display name; use the local part of the invitation e-mail (<= 100 chars,
/// the same bound as password enrolment).
pub fn default_display_name(email: &str) -> String {
    let local = email.split('@').next().unwrap_or(email).trim();
    let name = if local.is_empty() { email } else { local };
    name.chars().take(100).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_verified_email_passes_in_any_case() {
        assert_eq!(check_invite_identity("Ann@Example.com", Some("ann@example.COM"), true), Ok(()));
    }

    #[test]
    fn every_other_case_is_rejected_with_its_own_reason() {
        assert_eq!(check_invite_identity("a@x.io", None, true), Err(InviteReject::EmailMissing));
        assert_eq!(check_invite_identity("a@x.io", Some("a@x.io"), false), Err(InviteReject::EmailUnverified));
        assert_eq!(check_invite_identity("a@x.io", Some("b@x.io"), true), Err(InviteReject::EmailMismatch));
        // prefix / suffix / whitespace are not "equal"
        for other in ["a@x.io ", " a@x.io", "aa@x.io", "a@x.io.evil.com", ""] {
            assert_eq!(check_invite_identity("a@x.io", Some(other), true), Err(InviteReject::EmailMismatch), "{other:?}");
        }
    }

    #[test]
    fn a_missing_claim_is_reported_as_missing_not_unverified() {
        assert_eq!(check_invite_identity("a@x.io", None, false), Err(InviteReject::EmailMissing));
    }

    #[test]
    fn owner_invitations_cannot_be_sso() {
        assert!(!sso_invite_role_allowed("Owner"));
        assert!(sso_invite_role_allowed("Admin") && sso_invite_role_allowed("Viewer"));
    }

    #[test]
    fn display_name_is_the_local_part_bounded() {
        assert_eq!(default_display_name("ann.lee@example.com"), "ann.lee");
        assert_eq!(default_display_name(&format!("{}@x.io", "a".repeat(300))).chars().count(), 100);
        assert_eq!(default_display_name("@x.io"), "@x.io");
    }

    #[test]
    fn reasons_are_audit_labels() {
        use crate::admin::audit::is_label;
        for r in [InviteReject::EmailMissing, InviteReject::EmailUnverified, InviteReject::EmailMismatch] {
            assert!(is_label(r.reason()));
        }
    }
}
