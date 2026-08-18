-- security-rules-write-path (ADR-030): per-collection WRITE access-control
-- rule. Structurally independent of `access_rules` (the READ rule table,
-- ADR-028) — a wholly separate table, own primary key, own rows. Redefining
-- a write rule is a statement against this table ONLY (no `access_rules` in
-- its FROM/INTO clause), and vice versa, satisfying AC-17-43 structurally,
-- not conventionally (ADR-030 § Decision — Storage Shape, Option B).
--
-- Same idempotent-upsert-only, no-history-machinery shape as `access_rules`
-- (ADR-028 § Decision Drivers 5): no active/previous/version columns.
CREATE TABLE write_access_rules (
    project_id       TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    collection_path  TEXT NOT NULL,
    condition_source TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, collection_path)
);
