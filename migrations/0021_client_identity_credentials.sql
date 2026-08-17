-- client-auth (ADR-025): project-scoped client-identity verification credential.
-- 1:1 with projects (PK is project_id itself — exactly one verification
-- credential per project by design, unlike the 1:many sdk_api_keys shape).
-- Public key material only — never hashed, never encrypted (a public key has
-- no confidentiality property to protect; see ADR-025 § Alternatives 2/3).
CREATE TABLE client_identity_credentials (
    project_id          TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    public_key_current  BYTEA NOT NULL,      -- 32 raw bytes, Ed25519 public key
    public_key_previous BYTEA,                -- NULL when no rotation window is open
    algorithm           TEXT NOT NULL DEFAULT 'EdDSA',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    rotated_at          TIMESTAMPTZ
);
