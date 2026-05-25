-- Daily project usage metrics
CREATE TABLE daily_project_metrics (
    project_id          VARCHAR(63)  NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    date                DATE         NOT NULL,
    read_ops            BIGINT       NOT NULL DEFAULT 0,
    write_ops           BIGINT       NOT NULL DEFAULT 0,
    delete_ops          BIGINT       NOT NULL DEFAULT 0,
    listen_connections  BIGINT       NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, date)
);
