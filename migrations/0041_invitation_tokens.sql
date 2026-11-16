-- account-bootstrap-and-invitations: redeemable invitations + pending TOTP enrolments.
ALTER TABLE invitations
  ADD COLUMN token_hash BYTEA,   -- BLAKE3 of the token; NULL only for legacy rows (unusable)
  ADD COLUMN revoked_at TIMESTAMPTZ,
  ADD COLUMN invited_by UUID REFERENCES users(id) ON DELETE SET NULL;  -- NULL for operator bootstrap

-- Legacy rows have no token and can never be redeemed. Revoke the open ones so
-- the open-email uniqueness index below cannot fail on pre-existing duplicates.
UPDATE invitations SET revoked_at = now() WHERE token_hash IS NULL AND accepted_at IS NULL;

CREATE UNIQUE INDEX invitations_token_hash_idx
  ON invitations(token_hash) WHERE token_hash IS NOT NULL;
CREATE UNIQUE INDEX invitations_open_email_idx
  ON invitations(lower(email)) WHERE accepted_at IS NULL AND revoked_at IS NULL;

CREATE TABLE pending_enrolments (
  invitation_id   UUID PRIMARY KEY REFERENCES invitations(id) ON DELETE CASCADE,
  display_name    TEXT        NOT NULL,
  password_hash   TEXT        NOT NULL,
  totp_secret_enc BYTEA       NOT NULL,
  failed_attempts INT         NOT NULL DEFAULT 0,
  expires_at      TIMESTAMPTZ NOT NULL
);
