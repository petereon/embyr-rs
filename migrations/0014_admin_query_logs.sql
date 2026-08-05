CREATE TABLE query_logs (
  id UUID NOT NULL DEFAULT gen_random_uuid(),
  project_id TEXT NOT NULL,
  account_id UUID NOT NULL,
  op TEXT NOT NULL,
  collection_path TEXT,
  latency_ms INTEGER,
  status TEXT NOT NULL DEFAULT 'ok',
  error_code TEXT,
  created_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);
