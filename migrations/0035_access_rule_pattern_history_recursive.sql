-- security-rules-cel-recursive-wildcards (Slice 01, US-01, ADR-064):
-- symmetric audit-fidelity extension (ADR-035 precedent). History is
-- append-only, never queried for routing — no index needed.
ALTER TABLE access_rule_pattern_history ADD COLUMN is_recursive BOOLEAN NOT NULL DEFAULT false;
