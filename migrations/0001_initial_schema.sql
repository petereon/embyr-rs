-- System DB initial schema: project registry
CREATE TABLE projects (
    id                    VARCHAR(63)  PRIMARY KEY,
    status                VARCHAR(20)  NOT NULL DEFAULT 'active',
    backend_mode          VARCHAR(20)  NOT NULL,
    api_key_hash_current  TEXT         NOT NULL,
    api_key_hash_previous TEXT,
    ecies_encrypted_dsn   BYTEA,
    created_at            TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ  NOT NULL DEFAULT now()
);
