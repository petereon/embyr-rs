-- security-rules-operations (ADR-035): append-only history of every value
-- write_access_rules.condition_source has held, attributed to the acting
-- admin. Schema-identical to access_rule_history (migration 0025) but a
-- wholly separate, independently-stored table — mirrors write_access_rules'
-- own structural independence from access_rules (ADR-030). Never UPDATEd or
-- DELETEd (append-only invariant, locked by DISCUSS).
CREATE TABLE write_access_rule_history (
    id                BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    project_id        TEXT NOT NULL REFERENCES projects(id),
    collection_path   TEXT NOT NULL,
    condition_source  TEXT NOT NULL,
    actor_account_id  UUID NOT NULL,
    captured_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_write_access_rule_history_lookup
    ON write_access_rule_history (project_id, collection_path, id DESC);
