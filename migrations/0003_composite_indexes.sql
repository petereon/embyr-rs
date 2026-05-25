-- Firestore composite index definitions per project
CREATE TABLE composite_indexes (
    id               UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id       VARCHAR(63)  NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    collection_path  VARCHAR(1500) NOT NULL,
    fields           JSONB        NOT NULL,
    status           VARCHAR(20)  NOT NULL DEFAULT 'ready',
    created_at       TIMESTAMPTZ  NOT NULL DEFAULT now(),
    UNIQUE (project_id, collection_path, fields)
);
