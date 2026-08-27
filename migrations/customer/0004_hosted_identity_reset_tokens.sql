-- client-auth-hosted-identity (ADR-036 Decision 2, US-04): single-use,
-- time-bounded password-reset tokens for hosted-identity accounts.
--
-- token_hash is BLAKE3(raw token) -- the raw token is never stored, directly
-- reusing admin/handlers/auth.rs::signin's own sessions.token_hash /
-- mfa_recovery_codes.code_hash convention.
--
-- Single-use is enforced by an atomic
--   UPDATE ... SET used_at = now() WHERE token_hash = $1 AND used_at IS NULL
-- at confirm time -- race-free, no separate SELECT-then-UPDATE window.
CREATE TABLE hosted_identity_reset_tokens (
    project_id   VARCHAR(63)    NOT NULL,
    email        TEXT           NOT NULL,
    token_hash   BYTEA          NOT NULL,
    expires_at   TIMESTAMPTZ(6) NOT NULL,
    used_at      TIMESTAMPTZ(6),
    created_at   TIMESTAMPTZ(6) NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, token_hash)
);
CREATE INDEX hosted_identity_reset_tokens_lookup
    ON hosted_identity_reset_tokens (project_id, email)
    WHERE used_at IS NULL;
