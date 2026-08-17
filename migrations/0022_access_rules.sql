-- security-rules (ADR-028): per-collection access-control rule. Single row
-- per (project_id, collection_path) — idempotent upsert is the ONLY write
-- path (Resolution 3: define and redefine are the same action). No
-- active/previous/version columns — deliberately (see ADR-028 § Decision
-- Drivers 1, Considered Options A/B rejected).
CREATE TABLE access_rules (
    project_id       TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    collection_path  TEXT NOT NULL,
    condition_source TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, collection_path)
);
