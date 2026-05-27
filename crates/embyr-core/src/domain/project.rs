use crate::error::CoreError;

/// Validated project identifier: `^[a-z][a-z0-9-]{0,62}$`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectId(pub String);

impl ProjectId {
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let s = value.into();
        if is_valid_project_id(&s) {
            Ok(Self(s))
        } else {
            Err(CoreError::InvalidArgument(format!(
                "project id must match ^[a-z][a-z0-9-]{{0,62}}$, got: {s}"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- ProjectId::as_str ---
    // Kills: replace as_str -> &str with "" and "xyzzy"
    #[test]
    fn project_id_as_str_returns_original_value() {
        let id = ProjectId::new("my-project").unwrap();
        assert_eq!(id.as_str(), "my-project");
    }

    // --- is_valid_project_id ---
    // Kills: replace || with && (line 25), replace > with ==/</>= (line 25),
    //        match guard mutations, replace || with && (line 33), replace == with != (line 33)
    #[test]
    fn empty_string_invalid() {
        assert!(ProjectId::new("").is_err());
    }

    #[test]
    fn string_64_chars_invalid() {
        // > 63 chars
        let s = "a".repeat(64);
        assert!(ProjectId::new(s).is_err());
    }

    #[test]
    fn string_63_chars_valid() {
        // exactly 63 chars: 'a' + 62 more lowercase
        let s = "a".repeat(63);
        assert!(ProjectId::new(s).is_ok());
    }

    #[test]
    fn starts_with_digit_invalid() {
        // kills: match guard c.is_ascii_lowercase() -> true
        assert!(ProjectId::new("1abc").is_err());
    }

    #[test]
    fn starts_with_uppercase_invalid() {
        assert!(ProjectId::new("Abc").is_err());
    }

    #[test]
    fn contains_uppercase_invalid() {
        // kills: c.is_ascii_lowercase() || ... -> c.is_ascii_lowercase() && ...
        assert!(ProjectId::new("abcDef").is_err());
    }

    #[test]
    fn contains_underscore_invalid() {
        // kills: c == '-' part
        assert!(ProjectId::new("abc_def").is_err());
    }

    #[test]
    fn single_lowercase_char_valid() {
        assert!(ProjectId::new("a").is_ok());
    }

    #[test]
    fn lowercase_digits_dashes_valid() {
        assert!(ProjectId::new("my-project-123").is_ok());
    }

    #[test]
    fn dash_only_after_start_valid() {
        assert!(ProjectId::new("a-b-c").is_ok());
    }

    #[test]
    fn starts_with_dash_invalid() {
        assert!(ProjectId::new("-abc").is_err());
    }
}

fn is_valid_project_id(s: &str) -> bool {
    if s.is_empty() || s.len() > 63 {
        return false;
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectStatus {
    Active,
    Suspended,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendMode {
    DirectPg,
    AwsSecret,
    GcpSecret,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMode {
    Key,
}

/// Opaque backend configuration — details resolved in infra layer.
#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub mode: BackendMode,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub id: ProjectId,
    pub status: ProjectStatus,
    pub backend_mode: BackendMode,
    pub auth_mode: AuthMode,
}

/// Argon2id output stored as 32-byte hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyHash(pub [u8; 32]);

/// Cache key for authenticated credential lookups.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CredentialCacheKey {
    pub project_id: ProjectId,
    pub api_key_blake3: [u8; 32],
}
