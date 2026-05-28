# Shared Artifacts Registry — embyr-agent

> Feature: embyr-agent full StorageAgent implementation
> Wave: DISCUSS
> Updated: 2026-05-27

---

## Registry

| Artifact | Source of Truth | Consumers | Owner | Integration Risk |
|----------|----------------|-----------|-------|-----------------|
| `EMBYR_AGENT_DB_DSN` | Customer K8s Secret (env var) | Agent Postgres pool (startup); NEVER log output; NEVER agent wire | Customer (Riley) | HIGH — must never appear in any log line at any level (SPEC.md Invariant 13). Negative test mandatory. |
| `EMBYR_AGENT_CERT` | PEM file at configured path | Agent TLS listener identity; embyr SaaS verifies against backend_agent_ca | Customer PKI or delegated embyr CA | HIGH — wrong cert → silent TLS failure; named error at startup if file unreadable |
| `EMBYR_AGENT_CA` | PEM file at configured path | Agent verifies embyr SaaS client cert during mTLS handshake | embyr SaaS PKI | HIGH — mismatch = all SaaS→agent connections rejected |
| `backend_agent_ca` | System DB projects row (PEM text) | embyr SaaS verifies agent cert during mTLS handshake | Customer (provided at project creation) | HIGH — stored in system DB as PEM text; must not be confused with EMBYR_AGENT_CA |
| `backend_agent_endpoint` | System DB projects row (host:port) | embyr SaaS connection pool to agent | Admin API (project creation) | MEDIUM — wrong port = Unavailable on all project operations |
| `AGENT_LISTEN_ADDR` | EMBYR_AGENT_LISTEN_ADDR env var (default :9191) | Agent gRPC listener; Step 4 registration endpoint; Step 6 mTLS probe | Customer (Riley) | MEDIUM — must match backend_agent_endpoint in system DB |
| `PROJECT_ID` | Admin API create-project request | All StorageAgent RPCs (scoped by project_id); DocChange fan-out filter; audit query | Admin operator | HIGH — all RPCs must be scoped to this project; cross-project access = PermissionDenied |
| `DOCUMENT_PATH` | SDK doc() reference | CreateDocument parent+collection_id; GetDocument name; all write/read RPCs; DocChange path | SDK Developer (Alex) | MEDIUM — must be consistent across write, read, and change notification |
| `DOCUMENT_VERSION` | Postgres `documents.version` column | CommitTransaction OCC re-validation; GetDocument response; DocChange.version; write result | Postgres adapter | HIGH — monotonically increasing; never decrements (SPEC.md Invariant 3) |
| `SUBSCRIBE_STREAM` | embyr SaaS long-lived gRPC connection to agent | All Listen targets for the project; DocChange fan-out Registry | embyr SaaS | HIGH — one stream per project; reconnect on disconnect; overflow → RESET |
| `RESUME_TOKEN` | base64.RawURLEncoding(read_time RFC3339Nano) | SDK Listen reconnect delta delivery; targetChange{CURRENT}; targetChange{NO_CHANGE} | embyr SaaS | MEDIUM — malformed token falls back to full snapshot (safe degradation) |
| `AUDIT_EVIDENCE_SET` | System DB row + agent log export + TLS probe | External security auditor (SOC2 evidence package) | Riley (compiled and presented) | HIGH — DSN in any log invalidates the evidence; NULL backend_pg_creds_enc is the primary database artifact |

---

## Integration Validation Rules

1. **DSN isolation**: `EMBYR_AGENT_DB_DSN` appears in exactly ONE place (agent process env). It MUST NOT appear in:
   - Any log line (any level: trace, debug, info, warn, error)
   - Any gRPC response proto
   - System DB `projects` row
   - Any agent wire protocol message
   - Any error message returned to callers
   
2. **mTLS cert cross-reference**: `EMBYR_AGENT_CA` (agent's CA for verifying SaaS) and `backend_agent_ca` (SaaS's CA for verifying agent) are from different PKI contexts. They must NOT be the same cert unless a shared CA is used for both sides (which requires explicit design decision).

3. **PROJECT_ID consistency**: All StorageAgent RPCs dispatched by embyr SaaS for a given project MUST carry the same `project_id`. The agent does not validate `project_id` (it trusts the mTLS connection); however, the SaaS must not mix project_ids on the same agent connection.

4. **DOCUMENT_VERSION monotonicity**: version MUST monotonically increase. If any test observes a version decrease or repeat, it is a correctness failure (SPEC.md Invariant 3).

5. **SUBSCRIBE_STREAM singleton per project**: embyr SaaS maintains exactly one Subscribe stream per project. Multiple Listen clients for the same project share one stream on the SaaS side. If two Subscribe streams exist for the same project simultaneously, DocChange events may be duplicated to the Registry.
