-- security-rules-cel-path-matching (ADR-063): fixed-depth multi-segment
-- access-control PATTERN storage. Structurally independent of `access_rules`
-- (ADR-028)/`write_access_rules` (ADR-030)/`group_access_rules` (ADR-032) —
-- a wholly separate table, own primary key. One row per pattern SHAPE (not
-- per read/write operation, a departure from ADR-028/030's own disjoint
-- -table precedent, evidenced in ADR-063 § Decision — Schema: multi-segment
-- patterns have exactly one authoring path, file import, unlike
-- read/write rules which have two independent direct-define endpoints).
--
-- `collection_path_pattern` is the pattern's own ANCESTOR path only (e.g.
-- "expeditions/{expeditionId}/journal_entries") — always odd-length,
-- mirroring DocumentPath.collection_path's own shape. The pattern's LEAF
-- capture (if any), e.g. "{entryId}", is NOT part of this column — it is
-- handled entirely by the pre-existing security-rules-cel-parity (ADR-062)
-- condition-rewrite + document_id-threading mechanism, reused unchanged.
--
-- `ancestor_segment_count`/`literal_skeleton` exist purely to narrow routing
-- (ADR-063 § Decision — Routing Composition) and overlap-detection
-- (§ Decision — Overlap Detection) candidates to an indexed lookup —
-- typically 0-1 rows — never a per-project scan.
CREATE TABLE access_rule_patterns (
    project_id              TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    collection_path_pattern TEXT NOT NULL,
    ancestor_segment_count  SMALLINT NOT NULL,
    literal_skeleton        TEXT NOT NULL,
    leaf_variable           TEXT,
    read_condition          TEXT,
    write_condition         TEXT,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, collection_path_pattern)
);

CREATE INDEX idx_access_rule_patterns_routing
    ON access_rule_patterns (project_id, ancestor_segment_count, literal_skeleton);
