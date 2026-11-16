-- admin-password-reset: at most one live reset token per user (a new request
-- replaces the old one). Only the BLAKE3 hash of the token is stored.
-- CASCADE: the soft-delete purge sweeper deletes users and must not trip on this table.
CREATE TABLE password_reset_tokens (
  user_id    UUID        PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  token_hash BYTEA       NOT NULL,
  expires_at TIMESTAMPTZ NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX password_reset_tokens_hash_idx ON password_reset_tokens(token_hash);
