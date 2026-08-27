-- client-auth-hosted-identity (ADR-036 Decision 2, US-02/US-03): a Trailmark
-- end user's embyr-hosted email/password account. project_id carried
-- explicitly, mirroring documents' own shape (0001_documents.sql) even
-- though a single direct_pg Customer DB is conventionally one project's own
-- database -- aws_secret/gcp_secret modes can point more than one project at
-- the same physical Postgres, so this table never assumes database-level
-- single-tenancy, matching BC-2's own existing convention.
--
-- end_user_id (a UUID, not the email) is what gets minted into the sub claim
-- (ADR-036 Decision 3) -- mirrors real Firebase's own localId being distinct
-- from the email address; the email never travels inside a bearer token.
--
-- PRIMARY KEY (project_id, email) makes "email already registered"
-- (AC-18-06) a database-enforced unique-violation, not an app-level check
-- that could drift from the schema -- the same discipline ADR-025 already
-- established for client_identity_credentials' own registration.
CREATE TABLE hosted_identity_accounts (
    project_id     VARCHAR(63)    NOT NULL,
    end_user_id    UUID           NOT NULL DEFAULT gen_random_uuid(),
    email          TEXT           NOT NULL,
    password_hash  TEXT           NOT NULL,
    created_at     TIMESTAMPTZ(6) NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ(6) NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, email)
);
