//! Pure admin password-reset rules (admin-password-reset). No IO.
//! Token generation/hashing/shape and password strength are shared with the
//! invitation flow (`super::invitation`); only reset-specific rules live here.

use super::email::EmailMessage;
use super::invitation::is_plausible_email;

/// Trimmed, lower-cased email if plausible; `None` for anything that cannot be
/// an address (the caller answers identically either way — no oracle).
pub fn normalize_email(raw: &str) -> Option<String> {
    let email = raw.trim().to_lowercase();
    is_plausible_email(&email).then_some(email)
}

/// Reset email. `base_url` is the trimmed `EMBYR_PUBLIC_BASE_URL`; without it
/// the link is host-relative, like the invitation email.
pub fn reset_email(to: &str, base_url: Option<&str>, token: &str) -> EmailMessage {
    let link = format!("{}/admin/?reset={token}", base_url.unwrap_or(""));
    EmailMessage {
        to: to.to_string(),
        subject: "Reset your password".to_string(),
        body_text: format!(
            "A password reset was requested for this address. Open {link} \
             and enter your authenticator (or a recovery) code to choose a new \
             password. The link expires in one hour. If you did not ask for \
             this, ignore this email — your password is unchanged."
        ),
        body_html: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_is_normalised_or_rejected() {
        assert_eq!(normalize_email("  Bob@Example.COM ").as_deref(), Some("bob@example.com"));
        for bad in ["", "   ", "nope", "a b@c.co", &format!("{}@b.co", "a".repeat(250))] {
            assert_eq!(normalize_email(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn link_is_absolute_with_base_and_relative_without() {
        let with = reset_email("a@b.co", Some("https://c.example.com"), "TOK");
        assert!(with.body_text.contains("https://c.example.com/admin/?reset=TOK"));
        assert_eq!(with.to, "a@b.co");
        let without = reset_email("a@b.co", None, "TOK");
        assert!(without.body_text.contains("Open /admin/?reset=TOK "));
    }
}
