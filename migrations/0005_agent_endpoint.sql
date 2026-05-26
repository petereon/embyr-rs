-- Add agent backend columns for credential-isolated deployments.
-- Projects with backend_mode='agent' store only the agent gRPC endpoint here;
-- backend_pg_creds_enc MUST remain NULL for these rows (security invariant).
ALTER TABLE projects ADD COLUMN IF NOT EXISTS backend_agent_endpoint TEXT;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS agent_tls_bundle_enc BYTEA;
