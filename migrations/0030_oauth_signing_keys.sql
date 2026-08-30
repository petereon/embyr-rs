-- oauth-providers (ADR-037 Decision 2): a NEW, disjoint, project-scoped
-- embyr-owned Ed25519 signing key used to mint tokens for every
-- "embyr independently mints" OAuth-derived identity (v1: Google sign-in
-- only) -- structurally disjoint from BOTH client_identity_credentials
-- (hard constraint) AND hosted_identity_signing_keys (DESIGN's own reasoned
-- rejection of reuse, ADR-037 Decision 2, Option A). Generated at Slice 01
-- registration time (POST .../oauth_providers/google), never lazily.
--
-- Encrypted under EMBYR_ENCRYPTION_KEY (AES-256-GCM), NOT ECIES/api_key --
-- this feature has no Customer-DB-resolution precondition at decrypt time
-- (unlike hosted_identity_signing_keys), so forcing an api_key onto it would
-- be an unjustified new requirement. Not keyed by `provider` -- one embyr-
-- owned signing key per project serves every current and future
-- "embyr independently mints" OAuth-derived identity.
CREATE TABLE oauth_signing_keys (
    project_id      TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    public_key      BYTEA NOT NULL,        -- 32 raw bytes, Ed25519 public key (not secret)
    private_key_enc BYTEA NOT NULL,        -- AES-256-GCM: 12-byte nonce || ciphertext, key = EMBYR_ENCRYPTION_KEY
    algorithm       TEXT NOT NULL DEFAULT 'EdDSA',
    created_at      TIMESTAMPTZ(6) NOT NULL DEFAULT now()
);
