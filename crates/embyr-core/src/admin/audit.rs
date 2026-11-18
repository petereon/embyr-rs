//! Admin audit events (ADR-086 decision 9). Generic on purpose: SSO writes the
//! first rows, key rotation and member changes can reuse the table.
//!
//! The store takes labels only, never free text: `event` and `reason` must be
//! short `[a-z0-9_.:-]` strings, so a token, claim, e-mail address or secret
//! cannot be written by accident (AC-SSO-18).

use std::net::IpAddr;
use uuid::Uuid;

/// Event names for the SSO feature (ADR-086 decision 9).
pub mod events {
    pub const SSO_SIGNIN: &str = "sso_signin";
    pub const SSO_LINK_CREATED: &str = "sso_link_created";
    pub const SSO_LINK_REMOVED: &str = "sso_link_removed";
    pub const SSO_INVITE_ACCEPTED: &str = "sso_invite_accepted";
    pub const SSO_PROVIDER_CREATED: &str = "sso_provider_created";
    pub const SSO_PROVIDER_ENABLED: &str = "sso_provider_enabled";
    pub const SSO_PROVIDER_DISABLED: &str = "sso_provider_disabled";
    pub const SSO_PROVIDER_SECRET_ROTATED: &str = "sso_provider_secret_rotated";
    pub const SSO_PROVIDER_DELETED: &str = "sso_provider_deleted";
    pub const SSO_REQUIRED_TOGGLED: &str = "sso_required_toggled";
    pub const MEMBER_ROLE_CHANGED: &str = "member_role_changed";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOutcome {
    Ok,
    Failed,
}

impl AuditOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
        }
    }
}

/// One row of `admin_audit_events`. All ids are optional because failures can
/// happen before any is known (e.g. an unknown `state`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent<'a> {
    pub event: &'a str,
    pub outcome: AuditOutcome,
    pub account_id: Option<Uuid>,
    pub actor_user_id: Option<Uuid>,
    pub provider_id: Option<Uuid>,
    /// Granular failure reason (e.g. `SsoReject::reason()`); a label, never free text.
    pub reason: Option<&'a str>,
    pub source_ip: Option<IpAddr>,
}

impl<'a> AuditEvent<'a> {
    pub fn new(event: &'a str, outcome: AuditOutcome) -> Self {
        Self {
            event,
            outcome,
            account_id: None,
            actor_user_id: None,
            provider_id: None,
            reason: None,
            source_ip: None,
        }
    }

    /// True when `event` and `reason` are labels the table accepts.
    pub fn is_valid(&self) -> bool {
        is_label(self.event) && self.reason.is_none_or(is_label)
    }
}

/// 1-64 characters of `[a-z0-9_.:-]`.
pub fn is_label(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b':' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_accept_identifiers_and_refuse_free_text() {
        for ok in ["sso_signin", "token_expired", "a", "sso.link:created-1", &"a".repeat(64)] {
            assert!(is_label(ok), "{ok}");
        }
        for bad in [
            "", " ", "Sso", "has space", "a@example.com", "eyJhbGciOi.payload.sig", "bearer abc",
            "line\nbreak", "caf\u{e9}", &"a".repeat(65),
        ] {
            assert!(!is_label(bad), "{bad:?}");
        }
    }

    #[test]
    fn every_defined_event_name_is_a_label() {
        use events::*;
        for e in [
            SSO_SIGNIN, SSO_LINK_CREATED, SSO_LINK_REMOVED, SSO_INVITE_ACCEPTED, SSO_PROVIDER_CREATED, SSO_PROVIDER_ENABLED,
            SSO_PROVIDER_DISABLED, SSO_PROVIDER_SECRET_ROTATED, SSO_PROVIDER_DELETED, SSO_REQUIRED_TOGGLED,
            MEMBER_ROLE_CHANGED,
        ] {
            assert!(is_label(e), "{e}");
        }
    }

    #[test]
    fn event_validity_checks_event_and_reason() {
        let ok = AuditEvent::new(events::SSO_SIGNIN, AuditOutcome::Ok);
        assert!(ok.is_valid());
        let mut failed = AuditEvent::new(events::SSO_SIGNIN, AuditOutcome::Failed);
        failed.reason = Some("token_expired");
        assert!(failed.is_valid());
        failed.reason = Some("user a@example.com");
        assert!(!failed.is_valid());
        assert!(!AuditEvent::new("Not A Label", AuditOutcome::Ok).is_valid());
    }

    #[test]
    fn outcome_strings_match_the_table_check() {
        assert_eq!(AuditOutcome::Ok.as_str(), "ok");
        assert_eq!(AuditOutcome::Failed.as_str(), "failed");
    }
}
