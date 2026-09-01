-- security-rules-cel-path-matching (ADR-063): append-only history of every
-- value an access_rule_patterns row has held, attributed to the acting
-- admin. Never UPDATEd or DELETEd (append-only invariant, mirrors
-- access_rule_history/write_access_rule_history/group_access_rule_history,
-- ADR-035 precedent extended to this 4th rule table).
CREATE TABLE access_rule_pattern_history (
    id                       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    project_id               TEXT NOT NULL REFERENCES projects(id),
    collection_path_pattern  TEXT NOT NULL,
    leaf_variable            TEXT,
    read_condition           TEXT,
    write_condition          TEXT,
    actor_account_id         UUID NOT NULL,
    captured_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Supports a future GET .../access_rules/patterns/:pattern/history's "newest
-- first" retrieval (ORDER BY id DESC, the authoritative ordering key per
-- ADR-035's own collinearity reasoning) with a single indexed lookup.
CREATE INDEX idx_access_rule_pattern_history_lookup
    ON access_rule_pattern_history (project_id, collection_path_pattern, id DESC);
