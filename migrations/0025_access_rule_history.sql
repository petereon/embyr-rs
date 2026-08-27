-- security-rules-operations (ADR-035): append-only history of every value
-- access_rules.condition_source has held, attributed to the acting admin.
-- Never UPDATEd or DELETEd (append-only invariant, locked by DISCUSS).
CREATE TABLE access_rule_history (
    id                BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    project_id        TEXT NOT NULL REFERENCES projects(id),
    collection_path   TEXT NOT NULL,
    condition_source  TEXT NOT NULL,
    actor_account_id  UUID NOT NULL,
    captured_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Supports GET .../access_rules/:collection_path/history's "newest first"
-- retrieval (ORDER BY id DESC — the authoritative ordering key, not
-- captured_at; see ADR-035 § Decision — Schema for the collinearity
-- reasoning) with a single indexed lookup, no scan.
CREATE INDEX idx_access_rule_history_lookup
    ON access_rule_history (project_id, collection_path, id DESC);
