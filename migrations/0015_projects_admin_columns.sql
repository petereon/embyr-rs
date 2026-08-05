ALTER TABLE projects ADD COLUMN IF NOT EXISTS account_id UUID REFERENCES accounts(id);
ALTER TABLE projects ADD COLUMN IF NOT EXISTS logging_enabled BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS log_retention_days INTEGER;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS backend_pg_dsn_enc BYTEA;

CREATE INDEX projects_account_id_idx ON projects(account_id) WHERE account_id IS NOT NULL;
