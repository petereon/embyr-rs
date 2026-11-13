# ADR-083: Customer Connection Secrets — Single Server-Held Encrypted Store

## Status

Accepted. **Supersedes ADR-014** in its ECIES-DSN parts (the DSN re-encryption on SDK-key rotation, steps 5 and 7b, and the
"known limitation — pre-existing projects"). **Resolves ADR-055** for converted projects (the `backend_pg_dsn_enc IS NULL` gap).
Does not touch hosted-identity signing keys, which stay ECIES-to-API-key (ADR-036).

## Context

Two stores hold the same secret in incompatible forms:

- `projects.ecies_encrypted_dsn` and `agent_tls_bundle_enc`: ECIES to a key derived from the project API key; written by `provision`,
  read by request serving only, because only a request carries the API key.
- `projects.backend_pg_dsn_enc`: AES-256-GCM under `EMBYR_ENCRYPTION_KEY` with current/previous rotation (ADR-018); written by
  `PATCH /projects/:id`, read by `TransactionSweeper` and `customer_db_connect`.

The split makes DSN editing, backend-mode switching and probe-before-save impossible, silently ignores `PATCH` DSN edits, and
leaves the sweeper unable to reach most projects (ADR-055). The admin UI now needs those operations.

## Decision

One store: connection secrets are encrypted under the server key (`adapters/encryption.rs`, with rotation) in `backend_pg_dsn_enc`
(DSN) and a new `agent_tls_bundle_srv_enc` (agent TLS bundle). Request serving, the sweeper and the admin API all read that store.
New code never writes the ECIES columns. Legacy rows convert lazily on their next authenticated request, when the API key is
available; the legacy columns are dropped in a later contract migration once an observable gauge reaches zero. Edits go through one
route that probes the target before saving. Rows holding both a legacy and a different server-store DSN keep serving the legacy one
(what traffic has actually used) and are overwritten, with a warning counter.

## Alternatives Considered

1. **Keep ECIES, make the UI re-submit.** Rejected: the server can never edit, probe on behalf of an owner without the key, or give the
   sweeper access; every edit would require the client API key to be typed into the console.
2. **Server-key store plus an eager backfill job.** Rejected: impossible, because the server cannot decrypt legacy rows without each
   API key (the ADR-014 chicken-and-egg). Only an authenticated request or owner re-submission can convert.
3. **Convert by forcing owners to re-submit DSNs (ADR-055's named follow-up).** Rejected as the primary path: breaks no-touch
   projects only when they are re-submitted, but leaves them unreachable indefinitely and burdens every customer. Kept as the
   manual path for idle projects via the new `PUT /connection`.
4. **Envelope encryption with a KMS-held key.** Rejected for now: the existing rotation mechanism is already deployed and tested;
   moving to KMS is orthogonal and can wrap the same column later.
5. **One column with a version byte for both formats.** Rejected: rewriting and distinguishing live ECIES rows in place risks
   data loss on a failed conversion; a separate column keeps the fallback intact until the contract migration.

## Consequences

Positive: one read path; the sweeper reaches every converted project; DSN edit, mode switch and probe-before-save become possible;
ADR-014's rotation no longer needs the old key; provision no longer derives ECIES keys.

Negative / accepted: a database dump together with `EMBYR_ENCRYPTION_KEY` now exposes every customer DSN and agent client key,
where before the unstored API keys were also required. Idle legacy projects stay on the legacy path (and unreachable by the sweeper)
until first request or owner re-save. A divergent legacy/server pair resolves to the legacy value, so an owner's earlier ineffective
`PATCH` is discarded and must be re-submitted. Two code paths (server-first, legacy-fallback) coexist for at least one release.

## Enforcement

Pure read-order function with exhaustive tests; test that no handler references `ecies::encrypt` for connection secrets (source scan);
gauge for unconverted rows; fault-injection acceptance tests (unreachable DSN, conversion race, rotation window).
