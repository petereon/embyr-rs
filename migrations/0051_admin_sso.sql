-- admin-oidc-sso (ADR-086), slices 1-2. Expand-only (docs/operations/migration-policy.md):
-- one relaxed constraint, new nullable/defaulted columns, new tables, new indexes. No drops, no renames.
-- N-1 safety: no NULL password_hash exists until the new binary creates an SSO-only user, and
-- no N-1 code path can violate the new constraints (they only cover columns N-1 never writes).

-- NULL password_hash = SSO-only user. Every reader treats NULL as "no password".
ALTER TABLE users ALTER COLUMN password_hash DROP NOT NULL;

-- Anchors for the composite foreign keys that make a cross-account SSO link unrepresentable.
CREATE UNIQUE INDEX users_id_account_uidx          ON users(id, account_id);
CREATE UNIQUE INDEX oidc_providers_id_account_uidx ON oidc_providers(id, account_id);

ALTER TABLE invitations ADD COLUMN auth_method TEXT NOT NULL DEFAULT 'password'
  CHECK (auth_method IN ('password', 'sso'));
ALTER TABLE accounts ADD COLUMN sso_required BOOLEAN NOT NULL DEFAULT false;

-- O6: a session minted by SSO remembers its provider, so disabling the provider can end it and
-- deleting the provider ends it by cascade. Password sessions keep NULL.
ALTER TABLE sessions
  ADD COLUMN auth_method TEXT NOT NULL DEFAULT 'password' CHECK (auth_method IN ('password', 'sso')),
  ADD COLUMN sso_provider_id UUID REFERENCES oidc_providers(id) ON DELETE CASCADE,
  ADD CONSTRAINT sessions_sso_provider_chk CHECK ((auth_method = 'sso') = (sso_provider_id IS NOT NULL));
CREATE INDEX sessions_sso_provider_idx ON sessions(sso_provider_id) WHERE sso_provider_id IS NOT NULL;

-- The only way to sign in as a user through a provider: (provider, IdP subject) -> user.
CREATE TABLE admin_sso_identities (
  provider_id   UUID        NOT NULL,
  subject       TEXT        NOT NULL CHECK (length(subject) BETWEEN 1 AND 255),
  account_id    UUID        NOT NULL,
  user_id       UUID        NOT NULL,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_login_at TIMESTAMPTZ,
  PRIMARY KEY (provider_id, subject),
  UNIQUE (provider_id, user_id),
  FOREIGN KEY (provider_id, account_id) REFERENCES oidc_providers(id, account_id) ON DELETE CASCADE,
  FOREIGN KEY (user_id, account_id)     REFERENCES users(id, account_id)          ON DELETE CASCADE
);
CREATE INDEX admin_sso_identities_user_idx ON admin_sso_identities(user_id);

-- One row per started authorization request; consumed (deleted) on every callback outcome.
CREATE TABLE sso_auth_requests (
  state_hash        BYTEA       PRIMARY KEY,                       -- BLAKE3(state)
  provider_id       UUID        NOT NULL REFERENCES oidc_providers(id) ON DELETE CASCADE,
  intent            TEXT        NOT NULL CHECK (intent IN ('signin', 'invite', 'link')),
  invitation_id     UUID        REFERENCES invitations(id) ON DELETE CASCADE,
  user_id           UUID        REFERENCES users(id) ON DELETE CASCADE,
  nonce             TEXT        NOT NULL,
  code_verifier_enc BYTEA       NOT NULL,
  browser_binding   BYTEA       NOT NULL,                          -- BLAKE3(embyr_sso cookie)
  expires_at        TIMESTAMPTZ NOT NULL,
  CHECK ((intent = 'signin' AND invitation_id IS NULL     AND user_id IS NULL)
      OR (intent = 'invite' AND invitation_id IS NOT NULL AND user_id IS NULL)
      OR (intent = 'link'   AND user_id IS NOT NULL       AND invitation_id IS NULL))
);
CREATE INDEX sso_auth_requests_expiry_idx ON sso_auth_requests(expires_at);
CREATE INDEX sso_auth_requests_provider_idx ON sso_auth_requests(provider_id);

-- Generic admin audit trail (labels only, no tokens or claims). No foreign keys on purpose: rows
-- must outlive the user/provider/account they describe. 12-month retention (swept).
CREATE TABLE admin_audit_events (
  id            BIGSERIAL   PRIMARY KEY,
  at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  event         TEXT        NOT NULL,
  outcome       TEXT        NOT NULL CHECK (outcome IN ('ok', 'failed')),
  account_id    UUID,
  actor_user_id UUID,
  provider_id   UUID,
  reason        TEXT,
  source_ip     INET
);
CREATE INDEX admin_audit_events_account_idx ON admin_audit_events(account_id, at DESC);
CREATE INDEX admin_audit_events_at_idx      ON admin_audit_events(at);
