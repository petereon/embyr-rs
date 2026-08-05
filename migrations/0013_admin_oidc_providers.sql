CREATE TABLE oidc_providers (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  account_id UUID NOT NULL REFERENCES accounts(id),
  issuer TEXT NOT NULL,
  client_id TEXT NOT NULL,
  client_secret_enc BYTEA NOT NULL,
  enabled BOOLEAN NOT NULL DEFAULT true,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(account_id, issuer)
);
