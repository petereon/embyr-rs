//! Pure invitation / enrolment rules (account-bootstrap-and-invitations).
//! No IO: token generation uses the OS RNG only.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand_core::{OsRng, RngCore};

use super::account::Role;

pub const MIN_PASSWORD_CHARS: usize = 12;
pub const MAX_ENROLMENT_ATTEMPTS: i32 = 5;
pub const RECOVERY_CODE_COUNT: usize = 10;
const TOKEN_BYTES: usize = 32;

#[derive(Debug, PartialEq, Eq)]
pub enum InviteError {
    InvalidRole,
    InvalidEmail,
    RoleExceedsInviter,
}

pub fn parse_role(s: &str) -> Option<Role> {
    match s {
        "Owner" => Some(Role::Owner),
        "Admin" => Some(Role::Admin),
        "Viewer" => Some(Role::Viewer),
        _ => None,
    }
}

/// Minimal plausibility check for an invitee address: 1..=254 chars, exactly
/// one `@` with a non-empty local part and domain, no whitespace/control chars.
/// ponytail: not RFC 5322; real proof of ownership is the invitation email.
pub fn is_plausible_email(s: &str) -> bool {
    if s.is_empty() || s.len() > 254 || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    matches!(s.split_once('@'), Some((local, domain))
        if !local.is_empty() && !domain.is_empty() && !domain.contains('@'))
}

/// Role string, email and role cap (no escalation above the inviter's role).
/// Check order: role, email, cap.
pub fn validate_invite(email: &str, role: &str, inviter: Role) -> Result<(), InviteError> {
    let role = parse_role(role).ok_or(InviteError::InvalidRole)?;
    if !is_plausible_email(email) {
        return Err(InviteError::InvalidEmail);
    }
    if role > inviter {
        return Err(InviteError::RoleExceedsInviter);
    }
    Ok(())
}

pub fn password_acceptable(password: &str) -> bool {
    password.chars().count() >= MIN_PASSWORD_CHARS
}

/// 32 random bytes, base64url without padding (43 chars).
pub fn generate_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Cheap shape check so malformed tokens never reach the DB.
pub fn is_well_formed_token(token: &str) -> bool {
    token.len() == 43
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// BLAKE3 of the token — the only form stored at rest.
pub fn hash_token(token: &str) -> Vec<u8> {
    blake3::hash(token.as_bytes()).as_bytes().to_vec()
}

/// 10 random one-time recovery codes (14 chars each).
pub fn generate_recovery_codes() -> Vec<String> {
    (0..RECOVERY_CODE_COUNT)
        .map(|_| {
            let mut bytes = [0u8; 10];
            OsRng.fill_bytes(&mut bytes);
            URL_SAFE_NO_PAD.encode(bytes)
        })
        .collect()
}

/// RFC 4648 base32, no padding (authenticator-app secret encoding).
pub fn base32_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `otpauth://` URI matching the signin verifier (SHA1, 6 digits, 30 s).
pub fn otpauth_uri(secret: &[u8], email: &str) -> String {
    format!(
        "otpauth://totp/Embyr:{}?secret={}&issuer=Embyr&algorithm=SHA1&digits=6&period=30",
        percent_encode(email),
        base32_encode(secret)
    )
}

#[derive(Debug, PartialEq, Eq)]
pub enum WrongCodeOutcome {
    /// Keep the enrolment; the user may try again.
    Retry,
    /// Too many wrong codes: discard the enrolment, user must accept again.
    Invalidate,
}

/// Decision after a wrong TOTP code; `failed_attempts_so_far` excludes this one.
pub fn wrong_code_outcome(failed_attempts_so_far: i32) -> WrongCodeOutcome {
    if failed_attempts_so_far + 1 >= MAX_ENROLMENT_ATTEMPTS {
        WrongCodeOutcome::Invalidate
    } else {
        WrongCodeOutcome::Retry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_validation_orders_role_email_cap() {
        let ok = |e, r, who| validate_invite(e, r, who);
        assert_eq!(ok("a@b.co", "Viewer", Role::Admin), Ok(()));
        assert_eq!(ok("a@b.co", "Admin", Role::Admin), Ok(()));
        assert_eq!(ok("a@b.co", "Owner", Role::Owner), Ok(()));
        assert_eq!(ok("a@b.co", "Owner", Role::Admin), Err(InviteError::RoleExceedsInviter));
        assert_eq!(ok("a@b.co", "owner", Role::Owner), Err(InviteError::InvalidRole));
        assert_eq!(ok("a@b.co", "root", Role::Owner), Err(InviteError::InvalidRole));
        assert_eq!(ok("nope", "Viewer", Role::Owner), Err(InviteError::InvalidEmail));
        // role is checked before email, email before cap
        assert_eq!(ok("nope", "root", Role::Owner), Err(InviteError::InvalidRole));
        assert_eq!(ok("nope", "Owner", Role::Admin), Err(InviteError::InvalidEmail));
    }

    #[test]
    fn email_plausibility() {
        assert!(is_plausible_email("a@b.co"));
        for bad in ["", "ab.co", "@b.co", "a@", "a@b@c", "a b@c.co", "a@b.co\n"] {
            assert!(!is_plausible_email(bad), "{bad:?}");
        }
        assert!(!is_plausible_email(&format!("{}@b.co", "a".repeat(250))));
    }

    #[test]
    fn tokens_are_unique_well_formed_and_hash_stably() {
        let (a, b) = (generate_token(), generate_token());
        assert_ne!(a, b);
        assert!(is_well_formed_token(&a));
        assert_eq!(hash_token(&a), hash_token(&a));
        assert_ne!(hash_token(&a), hash_token(&b));
        assert_eq!(hash_token(&a).len(), 32);
        for bad in ["", "short", &"a".repeat(44), &format!("{}!", "a".repeat(42))] {
            assert!(!is_well_formed_token(bad), "{bad:?}");
        }
    }

    #[test]
    fn password_minimum_is_twelve_characters() {
        assert!(!password_acceptable("elevenchars"));
        assert!(password_acceptable("twelve chars"));
        assert!(password_acceptable(&"é".repeat(12)));
    }

    #[test]
    fn otpauth_uses_rfc4648_base32_and_escapes_the_label() {
        assert_eq!(base32_encode(b"Hello!\xde\xad\xbe\xef"), "JBSWY3DPEHPK3PXP");
        let uri = otpauth_uri(&[0u8; 20], "a+b@x.co");
        assert!(uri.starts_with("otpauth://totp/Embyr:a%2Bb%40x.co?secret="));
        assert!(uri.contains(&"A".repeat(32)));
    }

    #[test]
    fn recovery_codes_are_ten_distinct() {
        let codes = generate_recovery_codes();
        assert_eq!(codes.len(), RECOVERY_CODE_COUNT);
        let unique: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), RECOVERY_CODE_COUNT);
    }

    #[test]
    fn fifth_wrong_code_invalidates_enrolment() {
        for failed in 0..4 {
            assert_eq!(wrong_code_outcome(failed), WrongCodeOutcome::Retry);
        }
        assert_eq!(wrong_code_outcome(4), WrongCodeOutcome::Invalidate);
    }
}
