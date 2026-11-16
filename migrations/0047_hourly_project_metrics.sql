-- Migration 0047 (provisional number; renumber on merge if 0043-0046 land first):
-- hourly_project_metrics (hourly-metrics-rollup, ADR-084).
--
-- One row per (project, UTC hour, op) with request/unit/error counters and a
-- 13-bucket latency histogram. Fixed BIGINT columns (not an array) so the
-- cross-instance flush is a plain column-wise additive upsert.
-- Bucket upper bounds (ms, inclusive): 1,2,5,10,20,50,100,200,500,1000,2000,5000,+inf (b00..b12).
-- op: 0 read, 1 write, 2 delete, 3 run_query.

CREATE TABLE hourly_project_metrics (
    project_id VARCHAR(63) NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    hour       TIMESTAMPTZ NOT NULL,
    op         SMALLINT    NOT NULL CHECK (op BETWEEN 0 AND 3),
    requests   BIGINT      NOT NULL DEFAULT 0,
    units      BIGINT      NOT NULL DEFAULT 0,
    errors     BIGINT      NOT NULL DEFAULT 0,
    b00 BIGINT NOT NULL DEFAULT 0,
    b01 BIGINT NOT NULL DEFAULT 0,
    b02 BIGINT NOT NULL DEFAULT 0,
    b03 BIGINT NOT NULL DEFAULT 0,
    b04 BIGINT NOT NULL DEFAULT 0,
    b05 BIGINT NOT NULL DEFAULT 0,
    b06 BIGINT NOT NULL DEFAULT 0,
    b07 BIGINT NOT NULL DEFAULT 0,
    b08 BIGINT NOT NULL DEFAULT 0,
    b09 BIGINT NOT NULL DEFAULT 0,
    b10 BIGINT NOT NULL DEFAULT 0,
    b11 BIGINT NOT NULL DEFAULT 0,
    b12 BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, hour, op)
);

-- Retention sweep scans by hour alone.
CREATE INDEX hourly_project_metrics_hour_idx ON hourly_project_metrics (hour);
