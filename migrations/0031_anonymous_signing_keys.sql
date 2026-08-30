-- anonymous-sessions (ADR-043 Decision 2): a NEW, disjoint, project-scoped
-- embyr-owned Ed25519 signing key used to mint tokens for anonymous sign-in
-- (JOB-20) -- structurally disjoint from client_identity_credentials,
-- hosted_identity_signing_keys, AND oauth_signing_keys alike (Resolution 2's
-- own hard constraint). Generated at US-01 enablement time
-- (POST .../anonymous_identity/enable), never lazily at first sign-in.
--
-- Encrypted under EMBYR_ENCRYPTION_KEY (AES-256-GCM), NOT ECIES/api_key --
-- mirrors oauth_signing_keys's own shape byte-for-byte (ADR-037 Decision 2's
-- own forward-looking guidance, confirmed by ADR-043 Decision 2): this
-- feature never resolves a Customer DB connection (stateless minting,
-- Resolution 3), so there is no natural api_key-bearing call site to derive
-- an ECIES pubkey from.
CREATE TABLE anonymous_signing_keys (
    project_id      TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    public_key      BYTEA NOT NULL,        -- 32 raw bytes, Ed25519 public key (not secret)
    private_key_enc BYTEA NOT NULL,        -- AES-256-GCM: 12-byte nonce || ciphertext, key = EMBYR_ENCRYPTION_KEY
    algorithm       TEXT NOT NULL DEFAULT 'EdDSA',
    created_at      TIMESTAMPTZ(6) NOT NULL DEFAULT now()
);
