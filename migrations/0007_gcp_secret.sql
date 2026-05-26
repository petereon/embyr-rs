-- Add GCP Secret Manager resource name column for projects with backend_mode='gcp_secret'.
-- The resource name is a reference, NOT a credential. ecies_encrypted_dsn MUST remain NULL
-- for gcp_secret projects (security invariant — DSN never persists in system DB).
ALTER TABLE projects ADD COLUMN IF NOT EXISTS backend_secret_gcp TEXT;
