-- ADR-083: single server-held store for connection secrets.
-- AES-256-GCM under EMBYR_ENCRYPTION_KEY: nonce(12) || ciphertext, same format as backend_pg_dsn_enc.
-- Legacy ECIES columns (ecies_encrypted_dsn, agent_tls_bundle_enc) stay as read-only fallbacks
-- until a later contract migration drops them.
ALTER TABLE projects ADD COLUMN IF NOT EXISTS agent_tls_bundle_srv_enc BYTEA;
