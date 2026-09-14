-- collection-group-query-index (ADR-080 Decision A)
--
-- Plain nullable column (no default, no GENERATED clause -- metadata-only
-- catalog change, zero table-rewrite risk) + a BEFORE INSERT trigger that
-- populates it from the last path segment of collection_path. collection_path
-- is confirmed immutable after insert (ADR-080 Reading Confirmation), so a
-- BEFORE INSERT-only trigger is sufficient.
--
-- The two CREATE INDEX CONCURRENTLY statements (ADR-080 Decision D) are
-- deliberately NOT part of this migration -- CONCURRENTLY cannot run inside a
-- migration transaction (mirrors ADR-072's own established constraint). They
-- are built by PostgresBackendAdapter::ensure_collection_group_indexes(),
-- called at runtime after migrate() succeeds.

ALTER TABLE documents ADD COLUMN collection_id VARCHAR(1500);

CREATE FUNCTION documents_set_collection_id() RETURNS trigger AS $$
BEGIN
    NEW.collection_id := regexp_replace(NEW.collection_path, '^.*/', '');
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER documents_collection_id_biu
    BEFORE INSERT ON documents
    FOR EACH ROW
    EXECUTE FUNCTION documents_set_collection_id();
