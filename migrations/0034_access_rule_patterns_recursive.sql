-- security-rules-cel-recursive-wildcards (Slice 01, US-01, ADR-064 §
-- Decision — Schema): adds a discriminator column distinguishing a 4b
-- fixed-depth pattern row (`is_recursive = false`, `ancestor_segment_count`
-- always ODD) from a recursive-wildcard pattern row (`is_recursive = true`,
-- `ancestor_segment_count`/`literal_skeleton` repurposed to describe the
-- pattern's own FIXED PREFIX, always EVEN). Structural non-collision proof
-- (ADR-064): an odd-length 4b ancestor and an even-length recursive prefix
-- can never render to identical `collection_path_pattern` text — the
-- `is_recursive` column and its place in the compound PK are kept explicit
-- anyway (Earned Trust discipline: never rely solely on an implicit parity
-- argument when an explicit, DB-level one costs nothing).
ALTER TABLE access_rule_patterns ADD COLUMN is_recursive BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE access_rule_patterns
    DROP CONSTRAINT access_rule_patterns_pkey,
    ADD PRIMARY KEY (project_id, collection_path_pattern, is_recursive);

ALTER TABLE access_rule_patterns
    ADD CONSTRAINT access_rule_patterns_recursive_even_prefix
    CHECK (NOT is_recursive OR ancestor_segment_count % 2 = 0);

-- Narrows the recursive-wildcard routing/listing scan (Slice 02/03's own
-- step 3; Slice 04's own cross-checks) to ONLY recursive rows. A project
-- with zero recursive-wildcard patterns anywhere costs an empty partial
-- -index scan, not a table scan.
CREATE INDEX idx_access_rule_patterns_recursive_routing
    ON access_rule_patterns (project_id, ancestor_segment_count)
    WHERE is_recursive;
