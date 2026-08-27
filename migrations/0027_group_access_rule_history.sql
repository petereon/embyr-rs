-- security-rules-operations (ADR-035): append-only history of every value
-- group_access_rules.condition_source has held, attributed to the acting
-- admin. Schema-identical to access_rule_history (migration 0025) with the
-- same 2 deliberate departures group_access_rules itself carries relative to
-- access_rules/write_access_rules (ADR-032): collection_id (never a path,
-- not collection_path) + a CHECK constraint mirroring group_access_rules'
-- own bare-identifier invariant, and ON DELETE CASCADE mirroring
-- group_access_rules.project_id's own FK policy. Never UPDATEd or DELETEd
-- (append-only invariant, locked by DISCUSS).
CREATE TABLE group_access_rule_history (
    id                BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    project_id        TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    collection_id     TEXT NOT NULL CHECK (collection_id NOT LIKE '%/%'),
    condition_source  TEXT NOT NULL,
    actor_account_id  UUID NOT NULL,
    captured_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_group_access_rule_history_lookup
    ON group_access_rule_history (project_id, collection_id, id DESC);
