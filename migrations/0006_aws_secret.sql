-- Add AWS Secrets Manager ARN column for projects with backend_mode='aws_secret'.
-- The ARN is a reference, NOT a credential. ecies_encrypted_dsn MUST remain NULL
-- for aws_secret projects (security invariant — DSN never persists in system DB).
ALTER TABLE projects ADD COLUMN IF NOT EXISTS backend_secret_arn TEXT;
