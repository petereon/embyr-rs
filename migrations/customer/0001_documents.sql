CREATE TABLE documents (
    project_id       VARCHAR(63)    NOT NULL,
    collection_path  VARCHAR(1500)  NOT NULL,
    document_id      VARCHAR(1500)  NOT NULL,
    fields           JSONB          NOT NULL DEFAULT '{}',
    version          BIGINT         NOT NULL DEFAULT 1,
    create_time      TIMESTAMPTZ(6) NOT NULL DEFAULT now(),
    update_time      TIMESTAMPTZ(6) NOT NULL DEFAULT now(),
    deleted          BOOLEAN        NOT NULL DEFAULT false,
    PRIMARY KEY (project_id, collection_path, document_id)
);
CREATE INDEX documents_project_collection_idx
    ON documents (project_id, collection_path)
    WHERE NOT deleted;
