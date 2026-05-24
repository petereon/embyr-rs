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
