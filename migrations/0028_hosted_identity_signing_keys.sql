-- client-auth-hosted-identity (ADR-036 Decision 2): embyr-owned, project-scoped
-- signing key used to mint hosted-identity tokens (US-01). Structurally
-- disjoint from client_identity_credentials (migration 0021) -- a different
-- table, never a shared row or custody boundary (Resolution 3, locked).
--
-- Deliberately in SYSTEM DB, not Customer DB: this is embyr's own
-- control-plane secret, not project data -- see ADR-036 Decision 2 for the
-- full security rationale (a customer's own DBA administers their own
-- Customer DB in backend_mode=direct_pg and must never be handed embyr's own
-- signing private key).
--
-- No _previous/rotation columns in v1 (YAGNI) -- no story in this feature's
-- locked scope rotates the embyr-owned key. ADR-025's dual-generation shape
-- is the documented upgrade path if a future feature needs it.
CREATE TABLE hosted_identity_signing_keys (
    project_id      TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    public_key      BYTEA NOT NULL,      -- 32 raw bytes, Ed25519 public key (not secret)
    private_key_enc BYTEA NOT NULL,      -- ECIES-encrypted 32-byte Ed25519 seed;
                                          -- key derived from the project's own api_key
                                          -- (embyr_core::auth::ecies, mirrors
                                          -- ecies_encrypted_dsn's identical pattern)
    algorithm       TEXT NOT NULL DEFAULT 'EdDSA',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
