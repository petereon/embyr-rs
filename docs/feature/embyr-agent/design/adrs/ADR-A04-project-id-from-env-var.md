# ADR-A04: Project Identity — EMBYR_AGENT_PROJECT_ID Env Var

## Status
Accepted

## Context

The agent is a single-project binary. Every SQL query it executes includes `WHERE project_id = $1`. The current `storage_agent.proto` RPCs (GetDocument, CreateDocument, etc.) do not carry a `project_id` field — document resource names embed the project_id in the Firestore path format (`projects/{project_id}/databases/(default)/documents/...`), but BeginTransaction and Commit carry only a `database` field (`projects/{project_id}/databases/(default)`).

A source of truth for project_id at request-dispatch time is required. Three options were evaluated.

## Decision

**Option D1 — `EMBYR_AGENT_PROJECT_ID` environment variable** is adopted.

The agent reads `EMBYR_AGENT_PROJECT_ID` at startup alongside the existing required env vars (`EMBYR_AGENT_DB_DSN`, `EMBYR_AGENT_CERT`, `EMBYR_AGENT_KEY`, `EMBYR_AGENT_CA`). It is a required variable — missing or empty causes immediate non-zero exit with a diagnostic message (same pattern as existing `require_env` function in `config.rs`).

At startup, `AgentConfig` parses the value into a `ProjectId`. The `StorageAgentService` struct holds `project_id: ProjectId`. All RPC handlers extract the path from the proto resource name using the existing `parse_agent_doc_name` function in `AgentBackendAdapter`, but validate that the embedded project_id matches the configured `ProjectId`. Mismatched project_id in a request → `Status::permission_denied`.

For BeginTransaction/Commit/Rollback where the project_id is in the `database` field (format: `projects/{project_id}/databases/(default)`), a second parse function `parse_database_field(database: &str) -> Option<&str>` extracts the project_id by splitting on `/` and taking the second segment. This is distinct from the document resource name parser. Both parse functions live in `embyr-pg-storage` so they are shared between agent and server and cannot diverge.

## Alternatives Considered

**Option D2 — Add `project_id` field to every proto request message.**
This would require modifying 8 existing message types: `GetDocumentRequest`, `CreateDocumentRequest`, `UpdateDocumentRequest`, `DeleteDocumentRequest`, `RunQueryRequest`, `BeginTransactionRequest`, `CommitRequest`, `RollbackRequest`. Plus 5 new message types for future RPCs (Subscribe, ListDocuments, RunAggregationQuery, Ping, BatchGetDocuments). Proto changes require regenerating `embyr-proto`, rebuilding `embyr-server` and `embyr-agent`, and updating `AgentBackendAdapter` to populate the new field on every call. The agent is a single-project binary by design (multi-project is explicitly out of scope per DISCUSS). Adding `project_id` to every message adds protocol surface for a use case that will not exist in V1 or V2. Rejected: premature protocol generalisation.

**Option D3 — Derive project_id from TLS client certificate CN.**
The mTLS client certificate is issued by the embyr SaaS to authenticate itself to the agent. Using the certificate CN as a project identity signal couples security (authentication) with identity (authorisation). CN naming conventions would need to be documented, enforced, and validated by both the certificate issuance workflow and the agent runtime. If the cert is rotated for security reasons, the CN must remain stable — a constraint that is easy to violate. Rejected: implicit coupling between certificate management and runtime behaviour is fragile and not auditable.

## Consequences

**Positive:**
- Consistent with existing `AgentConfig` pattern: env vars are the sole configuration surface for the agent.
- Riley can set `EMBYR_AGENT_PROJECT_ID=finops-prod` in her Kubernetes deployment manifest alongside the other agent env vars; no additional tooling.
- The startup log can emit `project_id=finops-prod` in structured JSON, enabling Riley to grep for the project in audit logs.
- Mismatched project_id in a request is caught at the RPC boundary (permission_denied) rather than silently querying another project's data.
- SOC2 audit-friendly: the project identity is explicit in the process environment (visible in `kubectl describe pod`).

**Negative:**
- Requires a validation step in each RPC handler: parse project_id from resource name, compare against configured `ProjectId`. This is a small but non-zero amount of per-request work (~microseconds for a string comparison).
- If the resource name parsing logic in the agent diverges from the SaaS-side `parse_agent_doc_name`, the validation could produce false permission_denied errors. This risk is mitigated by extracting the parse function into `embyr-pg-storage` (shared between server and agent, same code path).

## Enforcement

`AgentConfig::from_env()` calls `require_env("EMBYR_AGENT_PROJECT_ID")` and parses it via `ProjectId::new()`. A `ProjectId` parse failure (invalid format) causes non-zero exit at startup. The `StorageAgentService` stores the `ProjectId` and validates it on each RPC. The acceptance criterion "startup fails with non-zero exit if project_id is missing" is testable in CI.
