-- api-key-rotation slice 2 (ADR-085 Decision 5): hosted-identity signing seed
-- moves under the server key (EMBYR_ENCRYPTION_KEY, AES-256-GCM,
-- nonce(12) || ct -- same format as anonymous_signing_keys.private_key_enc)
-- for NEW rows. Legacy rows keep the ECIES-to-API-key column until the
-- project's first rotate regenerates them (slice 4). Expand-only: the legacy
-- column is dropped in a later contract migration.
ALTER TABLE hosted_identity_signing_keys ADD COLUMN private_key_srv_enc BYTEA;
ALTER TABLE hosted_identity_signing_keys ALTER COLUMN private_key_enc DROP NOT NULL;
ALTER TABLE hosted_identity_signing_keys
    ADD CONSTRAINT hisk_one_seed CHECK (private_key_enc IS NOT NULL OR private_key_srv_enc IS NOT NULL);
