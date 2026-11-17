-- Migration 0050: TTL (time-to-live) policies (go-live finding P5).
--
-- One policy per (project, collection group), like Firestore: documents in any
-- collection whose id is `collection_group` are deleted by the background TTL
-- sweeper once the top-level Timestamp field `field_path` is in the past.
-- Purely additive (expand-only): old code never reads this table.
-- Policies are removed with their project (ON DELETE CASCADE).

CREATE TABLE ttl_policies (
    id               UUID          PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id       VARCHAR(63)   NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    collection_group VARCHAR(1500) NOT NULL,
    field_path       VARCHAR(1500) NOT NULL,
    enabled          BOOLEAN       NOT NULL DEFAULT true,
    created_at       TIMESTAMPTZ   NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ   NOT NULL DEFAULT now(),
    UNIQUE (project_id, collection_group)
);
