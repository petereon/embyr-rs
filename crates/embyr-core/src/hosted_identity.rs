//! Hosted-identity domain rules (client-auth-hosted-identity, ADR-036
//! Decision 9) — the one genuinely new pure module this feature adds to
//! `embyr-core`. Password-strength validation is a hosted-identity-specific
//! domain rule that exists nowhere else in the codebase; it does not belong
//! in `client_identity` (token concerns) or `auth::argon2` (hashing
//! mechanics, not policy).
//!
//! Rule: minimum 8 characters, no composition rules (no forced
//! uppercase/digit/symbol) — NIST SP 800-63B favors length over
//! composition-rule complexity.

/// AC-18-07: the specific requirement a rejected password failed, as a
/// structural consequence of `Result`'s `Err` variant — not a string built
/// ad hoc at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasswordTooWeak {
    pub minimum_length: usize,
}

const MINIMUM_PASSWORD_LENGTH: usize = 8;

/// Validate a candidate end-user password against this feature's one
/// strength rule (Decision 9). Pure — no IO.
pub fn validate_password_strength(password: &str) -> Result<(), PasswordTooWeak> {
    if password.chars().count() < MINIMUM_PASSWORD_LENGTH {
        return Err(PasswordTooWeak {
            minimum_length: MINIMUM_PASSWORD_LENGTH,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Port-to-port: `validate_password_strength` IS its own driving port
    //! (a pure domain function) — calling it directly here IS port-to-port
    //! testing, per the domain-function convention `client_identity`'s own
    //! tests already establish.

    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_password_of_exactly_the_minimum_length_is_accepted() {
        assert_eq!(validate_password_strength("12345678"), Ok(()));
    }

    #[test]
    fn a_password_one_character_short_of_the_minimum_is_rejected_naming_the_requirement() {
        assert_eq!(
            validate_password_strength("1234567"),
            Err(PasswordTooWeak { minimum_length: 8 })
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Property: any password with fewer than 8 characters is always
        /// rejected; any password with 8 or more is always accepted —
        /// regardless of composition (digits/symbols/unicode).
        #[test]
        fn password_strength_is_governed_solely_by_length(
            password in "\\PC{0,20}",
        ) {
            let result = validate_password_strength(&password);
            if password.chars().count() >= MINIMUM_PASSWORD_LENGTH {
                prop_assert_eq!(result, Ok(()));
            } else {
                prop_assert_eq!(result, Err(PasswordTooWeak { minimum_length: MINIMUM_PASSWORD_LENGTH }));
            }
        }
    }
}
