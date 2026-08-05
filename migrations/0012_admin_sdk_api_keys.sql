CREATE TABLE sdk_api_keys (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id TEXT NOT NULL REFERENCES projects(id),
  name TEXT NOT NULL,
  key_hash BYTEA NOT NULL,
  prefix TEXT NOT NULL,
  revoked_at TIMESTAMPTZ,
  last_used_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX sdk_api_keys_hash_idx ON sdk_api_keys(key_hash);
