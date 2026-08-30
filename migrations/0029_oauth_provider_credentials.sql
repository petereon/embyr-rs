-- oauth-providers (ADR-037 Decision 3): a project's registered OAuth
-- provider Client ID. Single row per (project_id, provider) -- idempotent
-- upsert is the ONLY write path (register and redefine are the same
-- action, mirrors access_rules' identical Resolution-3-style lifecycle,
-- ADR-037 Decision 1).
--
-- client_id stored plaintext -- it is public, non-confidential (Resolution
-- 1: mirrors client_identity_credentials' own no-confidentiality-property
-- public key). `provider` is a real column so a future GitHub slice adds
-- rows, not a schema migration -- but v1's only writer is the
-- /oauth_providers/google endpoint, so no non-"google" value can reach this
-- table today; no CHECK constraint (YAGNI, ADR-037 Decision 3).
CREATE TABLE oauth_provider_credentials (
    project_id  VARCHAR(63)    NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    provider    TEXT           NOT NULL,
    client_id   TEXT           NOT NULL,
    created_at  TIMESTAMPTZ(6) NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ(6) NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, provider)
);
