-- security-rules-collection-group-rules (ADR-032): per-collection-id
-- COLLECTION-GROUP access-control rule. Structurally independent of
-- `access_rules` (the exact-path READ rule table, ADR-028) and
-- `write_access_rules` (the exact-path WRITE rule table, ADR-030) — a
-- wholly separate table, own primary key, own rows, keyed by
-- `(project_id, collection_id)` ALONE (no parent-path component — a
-- collection-group id is, by construction, a bare identifier, never a
-- path — ADR-032 § Decision — Schema).
--
-- Same idempotent-upsert-only, no-history-machinery shape as `access_rules`/
-- `write_access_rules`: no active/previous/version columns.
--
-- `CHECK (collection_id NOT LIKE '%/%')` — a genuinely new departure from
-- ADR-028/030's own precedent: a DB-level defense-in-depth layer for this
-- table's own bare-id invariant, alongside the admin handler's own
-- friendly-400 validation (ADR-032 § Decision — Schema, "two-layer
-- defense").
CREATE TABLE group_access_rules (
    project_id       TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    collection_id    TEXT NOT NULL CHECK (collection_id NOT LIKE '%/%'),
    condition_source TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, collection_id)
);
