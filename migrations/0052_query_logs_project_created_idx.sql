-- Per-project query-log retention (G-003): the sweeper deletes
-- `WHERE project_id = $1 AND created_at < $2` in batches, and the admin
-- log viewer filters on project_id; query_logs only had the (id, created_at)
-- primary key. Expand-only (CREATE INDEX); new daily partitions inherit it.
-- CONCURRENTLY is not supported on a partitioned parent, so this takes a
-- short write lock per partition while building (at most ~90 daily partitions).
CREATE INDEX IF NOT EXISTS query_logs_project_created_idx
    ON query_logs (project_id, created_at);
