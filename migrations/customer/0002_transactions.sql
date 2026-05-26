CREATE TABLE transactions (
    transaction_id   UUID           PRIMARY KEY,
    project_id       VARCHAR(63)    NOT NULL,
    status           VARCHAR(20)    NOT NULL DEFAULT 'active',
    started_at       TIMESTAMPTZ    NOT NULL DEFAULT now()
);
