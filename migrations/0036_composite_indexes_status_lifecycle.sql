-- composite-index-real-creation (ADR-072 Decision C): composite_indexes.status
-- widens from a bare, unconstrained default to a genuine build-lifecycle
-- value set. No backfill needed -- every existing row was already written
-- as 'ready', which remains valid under the new CHECK.
ALTER TABLE composite_indexes ALTER COLUMN status SET DEFAULT 'building';
ALTER TABLE composite_indexes ADD CONSTRAINT composite_indexes_status_check
  CHECK (status IN ('building', 'ready', 'failed'));
