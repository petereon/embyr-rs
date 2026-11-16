-- account-deletion: soft-delete marker on accounts (mirrors projects.deleted_at).
-- Provisional number: 0042 is taken by an open PR; renumber on merge if needed.
ALTER TABLE accounts ADD COLUMN deleted_at TIMESTAMPTZ;
CREATE INDEX accounts_deleted_at_idx ON accounts(deleted_at) WHERE deleted_at IS NOT NULL;
