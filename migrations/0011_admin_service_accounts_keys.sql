CREATE TABLE service_accounts (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  account_id UUID NOT NULL REFERENCES accounts(id),
  name TEXT NOT NULL,
  description TEXT,
  role TEXT NOT NULL CHECK (role IN ('Owner', 'Admin', 'Viewer')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE admin_api_keys (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  account_id UUID NOT NULL REFERENCES accounts(id),
  name TEXT NOT NULL,
  key_hash BYTEA NOT NULL,
  prefix TEXT NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('Owner', 'Admin', 'Viewer')),
  member_id UUID REFERENCES users(id),
  service_account_id UUID REFERENCES service_accounts(id),
  revoked_at TIMESTAMPTZ,
  last_used_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX admin_api_keys_hash_idx ON admin_api_keys(key_hash);
