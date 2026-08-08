-- Migration 0018: rate_buckets table for distributed token-bucket rate limiting (ADR-015)
--
-- Provides a shared enforcement point across all embyr-server instances.
-- Each project has one row; the atomic UPDATE in check_pg() decrements tokens
-- and refills them based on elapsed wall-clock time.
--
-- FK ON DELETE CASCADE: project hard-deletion (sweeper at 168h) automatically
-- removes the rate_buckets row — no sweeper code change required.

CREATE TABLE rate_buckets (
    project_id  VARCHAR(63)      NOT NULL PRIMARY KEY
                                 REFERENCES projects(id) ON DELETE CASCADE,
    tokens      DOUBLE PRECISION NOT NULL,
    last_refill TIMESTAMPTZ      NOT NULL
);

-- Backfill rate_buckets for active/suspended projects that existed before this migration.
-- Uses 1000.0 as the default initial token count (matches the prior hardcoded default).
-- Operators running EMBYR_RATE_LIMIT_RPS != 1000 should update these rows after migration:
--   UPDATE rate_buckets SET tokens = <new_capacity> WHERE true;
INSERT INTO rate_buckets (project_id, tokens, last_refill)
SELECT id, 1000.0, now()
FROM   projects
WHERE  status IN ('active', 'suspended')
ON CONFLICT DO NOTHING;
