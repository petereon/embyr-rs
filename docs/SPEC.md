# embyr — Behavioral Specification

> **Scope**: Full system specification — all transports, data model, service operations, storage contract, authentication, configuration, and multi-tenant project management. Excludes internal implementation choices (data structures, algorithms, file layout). Derived from the Go reference implementation and its wire-contract document, with all six parity gaps resolved to target behavior and the deployment model changed from self-hosted to multi-tenant SaaS.
>
> **Version**: 2026-05-23 (derived from embyr Go source + parity plan `2026-05-23-firestore-parity.md`)
>
> **Audience**: New implementer with no prior knowledge of the original system.

---

## Overview

embyr is a **multi-tenant SaaS** that implements the Google Firestore wire protocol. It is a **stateless protocol translation layer**: it accepts Firestore-protocol requests, applies Firestore semantics (transactions, queries, live change delivery), and persists data in each tenant's **own database**. The service does not sell storage; it sells API compatibility.

embyr uses two distinct database tiers:

- **System database** (operator-managed Postgres): stores embyr's own metadata — project records, metrics, rate-limit counters. Never contains customer document data.
- **Customer database** (customer-managed): stores all document data for a project — documents, transactions, tombstones, indexes. embyr connects to it on-demand, using credentials supplied at project-creation time via one of three connectivity modes. Multiple independent tenants share a single service deployment; each tenant is represented by a **project** (the `{project}` segment of every Firestore resource name). Tenant data is fully isolated — no operation can read, write, or observe another tenant's data. Clients using the Firestore SDK or REST API point their SDK at the embyr service endpoint and use their project-scoped credentials; they observe behavior identical to Google Cloud Firestore for all supported operations.

embyr exposes the `google.firestore.v1.Firestore` gRPC service over four transports simultaneously: pure gRPC, gRPC-Web, REST (JSON over HTTP), and BrowserChannel/WebChannel (the long-poll transport the Firebase JS SDK uses in browsers). All four transports are backed by the same shared storage layer, with all queries and writes scoped to the authenticated project.

In addition to the standard Firestore service API, embyr exposes a **Project Management API** for tenant provisioning: creating and deleting projects and managing per-project authentication configuration. This API is accessed via service-level admin credentials, not project-level credentials.

The service's key guarantees:
- **Tenant isolation**: all data access is strictly bounded to the authenticated project. No cross-project reads, writes, or change events are possible.
- **Per-write atomicity and OCC** for transactions.
- **Live document change delivery** to connected `Listen` stream clients, scoped to the project.
- **Full parity** with the Firestore wire protocol for all operations listed in this document.
- **Horizontal scalability within a region**: multiple stateless embyr instances can run behind a load balancer, all backed by the same regional Postgres cluster. WebChannel sessions are instance-local; session-level stickiness (e.g., load-balancer consistent hashing on `SID`) is required for BrowserChannel clients.
- **Multi-region deployment**: each region operates an independent embyr cluster backed by a regional Postgres instance. Cross-region live-change delivery is not provided by embyr; callers connect to the nearest region. Failover is handled at the DNS/load-balancer layer.

---

## Concepts and Terminology

| Term | Definition |
|---|---|
| **Resource name** | Full document path: `projects/{p}/databases/{db}/documents/{coll}/{id}[/{coll}/{id}]*`. The segment after `documents/` must have even length. `{db}` is typically `(default)`. |
| **Project** | A tenant. Identified by `{p}` in every resource name. The unit of data isolation, authentication, and billing. All document operations are scoped to a single project. |
| **Tenant** | Synonym for project. Each project is one independent tenant sharing the service infrastructure. |
| **Project ID** | The `{p}` string from the resource name. Globally unique within the service. Immutable after project creation. |
| **Per-project auth** | Each project has its own authentication configuration (mode + credentials), stored in the projects table. Incoming requests are authenticated against the config of the project identified in the request path. |
| **Service admin** | A privileged caller (identified by a separate service-level credential) who can create, configure, and delete projects. Service admin credentials are distinct from any project's credentials. |
| **Collection** | The second-to-last path segment of a document's resource name. |
| **Parent** | Everything in a resource name up to and including the second-to-last segment. |
| **Version** | Per-document monotonic integer, incremented by one on every successful write. Used for OCC. |
| **OCC** | Optimistic Concurrency Control. A transaction records the version of every document it reads; at commit time it verifies those versions are unchanged. |
| **Transform** | A server-side field mutation applied atomically with a write: server timestamp, numeric increment, array append/remove. |
| **Upsert** | Write that creates the document if absent, updates if present. Default mode. |
| **Resume token** | Opaque bytes sent by the server to a Listen client that encodes a read timestamp. A reconnecting client presents the token to request delta delivery since that timestamp. |
| **Tombstone** | A record of a deleted document retained so that resume-token delta delivery can report deletes to reconnecting clients. |
| **gRPC-Web** | An HTTP/1.1 wrapping of gRPC that browsers can speak; auto-detected and served on the REST port. |
| **BrowserChannel / WebChannel** | The long-poll protocol the Firebase JS SDK uses for `Listen` and `Write` streams in browsers. Served on the REST port at `/channel` paths. |
| **DocChange** | An internal event emitted after every committed write, carrying project ID, path, collection, parent, kind (upsert/delete), version, and data. |
| **Registry** | An in-process fan-out hub that delivers `DocChange` events to all active `Listen` stream handlers. Each handler only receives events for its own project. |
| **System database** | The operator-managed Postgres instance that stores embyr metadata: project records, usage metrics. No customer document data is ever written here. |
| **Customer database** | A Postgres instance that the customer owns and operates. Stores all document data for one project. embyr connects to it using credentials retrieved via the project's backend connectivity mode. |
| **Backend connectivity mode** | How embyr obtains credentials to connect to a project's customer database. One of: `direct_pg` (encrypted credentials stored in embyr), `aws_secret` (credentials fetched from AWS Secrets Manager), `gcp_secret` (credentials fetched from GCP Secret Manager), `agent` (credentials never leave customer infrastructure; an embyr agent binary proxies DB operations). |
| **embyr Agent** | A statically-linked Rust binary deployed in customer infrastructure. It holds database credentials in its local environment, connects to the customer database directly, and exposes a gRPC service that the embyr SaaS calls to execute storage operations. |
| **ECIES** | Elliptic Curve Integrated Encryption Scheme: a hybrid public-key encryption scheme combining X25519 key agreement, HKDF-SHA-256 key derivation, and AES-256-GCM authenticated encryption. Used to encrypt PG credentials in `direct_pg` mode. |
| **Credential cache** | A short-lived in-process cache (TTL configurable, default 5 min) of decrypted or fetched PG DSNs, keyed by `(project_id, api_key_fingerprint)`. Avoids repeating expensive decryption or network calls on every SQL operation. |

---

## Data Model

### Project

The top-level tenant record. Created via the Project Management API (see §Tenant Management).

| Field | Type | Constraints |
|---|---|---|
| `project_id` | string | Primary key. Globally unique. Immutable after creation. Must match `^[a-z][a-z0-9-]{0,62}$` (lowercase RFC 1123 hostname label: starts with a letter, max 63 chars, lowercase letters / digits / hyphens only). |
| `status` | string | One of: `active`, `suspended`, `deleted`. Default `active`. |
| `created_at` | timestamp | Time the project was provisioned. |
| `deleted_at` | timestamp | Set when `status` transitions to `deleted`. Null for active/suspended projects. |
| `auth_mode` | string | Per-project auth mode: `none`, `key`, `google`, or `mtls`. |
| `auth_key_hash` | string | Set when `auth_mode=key`. Argon2id hash of the primary API key (see §Authentication). |
| `auth_key_hash_2` | string | Optional. Set during key rotation. Secondary Argon2id hash valid alongside the primary during the rotation window. |
| `auth_google_project_id` | string | Required when `auth_mode=google`. Validated against both id-token and access-token audiences. |
| `auth_mtls_ca` | string or bytes | Required when `auth_mode=mtls`. PEM CA certificate. |
| `backend_mode` | string | Required. One of: `direct_pg`, `aws_secret`, `gcp_secret`, `agent`. |
| `backend_pg_creds_enc` | bytes | Set when `backend_mode=direct_pg`. ECIES ciphertext of the PG DSN (see §Customer Database Connectivity). |
| `backend_pg_pubkey` | bytes | Set when `backend_mode=direct_pg`. X25519 public key derived from the project's API key; used to re-encrypt after key rotation. |
| `backend_secret_arn` | string | Set when `backend_mode=aws_secret`. Full AWS Secrets Manager ARN. |
| `backend_secret_gcp` | string | Set when `backend_mode=gcp_secret`. GCP Secret Manager resource name (`projects/*/secrets/*/versions/*`). |
| `backend_agent_endpoint` | string | Set when `backend_mode=agent`. `host:port` of the embyr agent gRPC listener in customer infrastructure. |
| `backend_agent_ca` | bytes | Set when `backend_mode=agent`. PEM CA certificate used to verify the agent's TLS certificate. |

**Invariants:**
- `project_id` is immutable after creation.
- `backend_mode=direct_pg` requires `auth_mode=key`. The API key is the sole key material for ECIES decryption; no other auth mode provides the key.
- `backend_pg_pubkey` is immutable; it is re-derived and the ciphertext is re-encrypted when the API key is rotated.
- Only `status=deleted` projects are eligible for background data purge.
- `status=suspended` projects reject all Firestore data requests with `PermissionDenied`.
- Purging a deleted project cascades to all its documents, transactions, tombstones, indexes, and metrics records.
- `auth_key_hash_2` must be cleared once key rotation is confirmed complete; only one secondary slot is provided.

### Document

> **Location**: customer database (not the system database). Each project has its own database; the schema below is migrated into it at project-creation time.

| Field | Type | Constraints |
|---|---|---|
| `project_id` | string | Foreign key → `projects.project_id`. Part of composite primary key. |
| `path` | string | Full resource name. `(project_id, path)` is the composite primary key. Immutable after creation. |
| `collection` | string | Derived from `path`. The immediate collection name. |
| `parent` | string | Derived from `path`. Everything before the collection/doc pair. |
| `data` | string | proto3-JSON encoding of `{"fields": {...}}`. Empty document stored as `"{}"`. |
| `created_at` | timestamp | Set on creation. Never updated. |
| `updated_at` | timestamp | Set on every successful write. Microsecond precision is the meaningful unit. |
| `version` | int64 | Starts at 1. Incremented by 1 on every write. Never negative. |

**Invariants:**
- `version >= 1` for every stored document.
- `updated_at >= created_at` always.
- `data` always round-trips all Firestore `Value` types losslessly via proto3-JSON.
- All documents belong to exactly one project. No document can be accessed outside its project.

### Transaction

| Field | Type | Constraints |
|---|---|---|
| `project_id` | string | Foreign key → `projects.project_id`. Part of composite primary key. |
| `id` | string | 128-bit hex string. `(project_id, id)` is the composite primary key. |
| `started_at` | timestamp | Time the transaction was created. |
| `reads` | map(string → int64) | Path → version for every document read within this transaction. |
| `expires_at` | timestamp | `started_at + transactions.ttl`. Transaction is invalid after this time. |

### Tombstone

| Field | Type | Constraints |
|---|---|---|
| `project_id` | string | Foreign key → `projects.project_id`. |
| `path` | string | Full resource name of the deleted document. |
| `collection` | string | Derived from `path`. |
| `parent` | string | Derived from `path`. |
| `deleted_at` | timestamp | Time the deletion was committed. |

Tombstones accumulate over time. A background sweep deletes tombstones older than 24 hours.

### Indexes Table

The `indexes` table stores composite index definitions per project (`project_id`, `id`, `collection`, `fields`, `state`).

**Index enforcement**: complex queries (multi-field `orderBy`, inequality filters combined with `orderBy` on a different field, `array-contains-any`, `not-in`, `IN` with more than one equality constraint) require a matching composite index with `state=READY`. A query that would require an index but has no matching index returns `FailedPrecondition` with the Firestore standard error message format indicating which index is needed.

Single-field queries are always satisfied without an explicit index (implicit single-field auto-indexes).

**Index CRUD** follows the Firestore Admin API. `state` transitions: `CREATING` → `READY` (or `NEEDS_EXEMPTION` on error). New indexes start as `CREATING`; the background indexer builds them and transitions to `READY`. Only `READY` indexes are used at query time.

Indexes created via the admin API use `state=READY` immediately (no async build phase is required for correctness, since embyr rebuilds from the documents table on demand during index creation).

---

## Tenant Management

The Project Management API provisions and configures tenants. It is served on a **separate internal-only port** (`admin.port` in config) accessible only within the service infrastructure network — not on the public REST or gRPC ports. It requires service-level admin credentials.

Project listing for billing and operational purposes is handled by an external ETL job that reads directly from the internal admin database. There is no `GET /admin/v1/projects` list endpoint served by embyr itself.

### Create Project

**Method**: `POST /admin/v1/projects`
**Auth**: service admin credentials required
**Inputs**:
- `project_id` (required): must match `^[a-z][a-z0-9-]{0,62}$`, globally unique
- `auth_mode` (required): one of `none`, `key`, `google`, `mtls`
- `auth_key` (required when `auth_mode=key`): plaintext key supplied once; stored as Argon2id hash; also used as ECIES key material when `backend_mode=direct_pg`
- `auth_google_project_id` (required when `auth_mode=google`)
- `auth_mtls_ca` (required when `auth_mode=mtls`): PEM CA certificate
- `backend_mode` (required): one of `direct_pg`, `aws_secret`, `gcp_secret`, `agent`
- `backend_pg_dsn` (required when `backend_mode=direct_pg`): plaintext PostgreSQL DSN; encrypted with ECIES and stored; never stored nor returned in plaintext
- `backend_secret_arn` (required when `backend_mode=aws_secret`): AWS Secrets Manager ARN
- `backend_secret_gcp` (required when `backend_mode=gcp_secret`): GCP Secret Manager resource name
- `backend_agent_endpoint` (required when `backend_mode=agent`): `host:port` of the embyr agent
- `backend_agent_ca` (required when `backend_mode=agent`): PEM CA certificate for agent mTLS

**Outputs**: the created project record (without sensitive fields; raw keys, DSNs, and ciphertexts are never returned)
**Side effects**: (1) inserts a row into the projects table with `status=active`; (2) connects to the customer database and runs embyr schema migrations to create document/transaction/tombstone/index tables
**Errors**: `AlreadyExists` if `project_id` is taken; `InvalidArgument` for missing/invalid fields; `FailedPrecondition` if `backend_mode=direct_pg` is used with `auth_mode` other than `key`; `Unavailable` if the customer database cannot be reached during the initial migration

### Get Project

**Method**: `GET /admin/v1/projects/{project_id}`
**Auth**: service admin credentials required
**Outputs**: project record (without sensitive fields such as key hashes)
**Errors**: `NotFound` if project does not exist (including soft-deleted projects that have been purged)

### Update Project Auth

**Method**: `PATCH /admin/v1/projects/{project_id}`
**Auth**: service admin credentials required
**Inputs**: any subset of auth fields (mode, key, google_project_id, mtls_ca). Only provided fields are changed.
**Outputs**: updated project record
**Side effects**: all subsequent requests to the project use the new auth config

**Key rotation** (auth_mode=key): send both `auth_key` (new primary) and `auth_key_rotate_from` (old key to keep valid). The server stores the new key hash in `auth_key_hash` and the old hash in `auth_key_hash_2`. Requests matching either hash succeed during the rotation window. A subsequent `PATCH` with only `auth_key` clears `auth_key_hash_2`, completing the rotation.

When `backend_mode=direct_pg`: key rotation also re-encrypts the stored PG credentials. The server uses `auth_key_rotate_from` to decrypt the current `backend_pg_creds_enc`, then immediately re-encrypts with the new `auth_key` and stores the result. If decryption with the old key fails, the rotation request is rejected with `FailedPrecondition` and no state is changed.

**Errors**: `NotFound`, `InvalidArgument`

### Suspend Project

**Method**: `POST /admin/v1/projects/{project_id}/suspend`
**Auth**: service admin credentials required
**Outputs**: empty
**Side effects**: sets `status=suspended`. All subsequent Firestore data requests to the project return `PermissionDenied` with message `"project suspended"`. Admin API requests still succeed.
**Errors**: `NotFound`

### Resume Project

**Method**: `POST /admin/v1/projects/{project_id}/resume`
**Auth**: service admin credentials required
**Outputs**: empty
**Side effects**: sets `status=active`. Firestore requests resume normally.
**Errors**: `NotFound`, `FailedPrecondition` (if project is already deleted)

### Delete Project

**Method**: `DELETE /admin/v1/projects/{project_id}`
**Auth**: service admin credentials required
**Outputs**: empty
**Side effects**: **soft deletion**. Sets `status=deleted` and `deleted_at=now`. Firestore requests immediately return `NotFound`. Data is **not** purged synchronously. A background sweeper purges all data for deleted projects asynchronously (after a configurable retention window, default 7 days). Purge includes documents, transactions, tombstones, indexes, and metrics records.
**Errors**: `NotFound` if project does not exist or has already been deleted

---

## Configuration

Configuration covers service-level infrastructure only. Per-project authentication is managed via the Tenant Management API at runtime, not via the config file.

Loaded from an optional YAML file plus environment variables prefixed `EMBYR_`. Environment variables override file values. All keys use dot-notation → `SCREAMING_SNAKE` mapping (e.g., `admin.key` → `EMBYR_ADMIN_KEY`).

### Service-Level Parameters

| Parameter | Type | Default | Required | Description |
|---|---|---|---|---|
| `server.grpc_port` | int | 8080 | no | gRPC listener port. Range 1–65535. Must differ from `rest_port`. |
| `server.rest_port` | int | 8081 | no | REST/WebChannel listener port. Range 1–65535. |
| `server.allowed_origins` | list(string) | `[]` | no | CORS allowed origins. Empty list = allow all origins. |
| `server.tls.cert` | string | `""` | when any project uses `auth_mode=mtls` | Path to PEM server certificate. |
| `server.tls.key` | string | `""` | when any project uses `auth_mode=mtls` | Path to PEM server private key. |
| `admin.key` | string | `""` | yes | Service admin API key. Required; no admin operations are possible without it. |
| `admin.port` | int | 9090 | no | Internal-only port for the admin API. Must differ from `grpc_port` and `rest_port`. Not accessible from the public network. |
| `admin.deletion_retention` | duration | `168h` (7 days) | no | How long soft-deleted project data is retained before the background sweeper purges it. |
| `ratelimit.enabled` | bool | `true` | no | Enable per-project rate limiting. Limits are enforced **per embyr instance**, not globally across a cluster. |
| `backend.credential_cache_ttl` | duration | `5m` | no | How long decrypted/fetched customer DB credentials are cached in memory. Increase to reduce decryption/network overhead; decrease to limit exposure window. |
| `backend.aws.region` | string | `""` | when any project uses `aws_secret` | Default AWS region for Secrets Manager calls. |
| `backend.aws.role_arn` | string | `""` | no | If set, embyr assumes this IAM role before fetching secrets (cross-account access). |
| `backend.gcp.service_account_key` | string | `""` | no | Path to GCP service account JSON key file. If empty, Workload Identity / ADC is used. |
| `backend.agent.ca_cert` | string | `""` | when any project uses `agent` | Path to PEM CA certificate that signed embyr's own TLS certificate. Presented to agents for mutual verification. |
| `backend.agent.cert` | string | `""` | when any project uses `agent` | Path to PEM TLS certificate for embyr's identity when connecting to agents. |
| `backend.agent.key` | string | `""` | when any project uses `agent` | Path to PEM private key for embyr's TLS certificate. |
| `backend.type` | string | `"postgres"` | no | One of: `sqlite` (dev/test only), `postgres` (production). |
| `backend.sqlite.path` | string | `"embyr.db"` | no | SQLite database file path. Single-tenant dev/test use only. |
| `backend.postgres.dsn` | string | `""` | when `type=postgres` | PostgreSQL connection string. |
| `backend.postgres.max_conns` | int | 25 | no | Max open DB connections. 0 = unlimited. |
| `transactions.ttl` | duration | `60s` | no | Transaction lifetime for all projects. Must be positive. |
| `transactions.sweep_interval` | duration | `30s` | no | How often expired transactions are swept. Must be positive. |
| `log.level` | string | `"info"` | no | Log level. |
| `log.format` | string | `"json"` | no | `json` = structured production logs. Any other value = development text logs. |

**Validation errors** that prevent startup:
- `grpc_port`, `rest_port`, or `admin.port` out of range [1, 65535].
- Any two of `grpc_port`, `rest_port`, `admin.port` are equal.
- `transactions.ttl <= 0`.
- `transactions.sweep_interval <= 0`.
- `admin.key` is empty.
- `admin.deletion_retention <= 0`.
- Unknown `backend.type` value.

---

## Transports and Routing

The server runs three TCP listeners simultaneously:

- **gRPC port**: pure gRPC over HTTP/2. No TLS by default. mTLS available when any project uses `auth_mode=mtls`.
- **REST port**: HTTP/1.1 (and HTTP/2) multiplexed over a single combined handler. Serves all Firestore client-facing traffic.
- **Admin port**: HTTP/1.1. Serves only `/admin/v1/projects/*` endpoints. Must be bound to a loopback or internal-network interface; infrastructure policy (firewall, network ACL) prevents external access.

### REST Port Request Routing

```
Request received on REST port
│
├─ gRPC-Web request (Content-Type grpc-web, or CORS preflight for gRPC-Web)
│    └─ gRPC-Web handler (wraps gRPC server; goes through gRPC interceptors)
│
├─ Path ends with "/channel"   ← BrowserChannel forward/back channel
│    └─ BrowserChannel handler (no write deadline)
│
└─ Everything else
     └─ HTTP TimeoutHandler (30s write deadline)
          ├─ GET /healthz             → 200 "ok"
          ├─ GET /readyz              → 200 if db reachable, 503 otherwise
          ├─ POST */documents:runQuery
          │    → custom JSON-array streamer (see §REST)
          ├─ POST */documents:batchGet
          │    → custom JSON-array streamer (see §REST)
          ├─ POST */documents:runAggregationQuery
          │    → custom JSON-array streamer (see §REST)
          └─ *  → grpc-gateway REST mux (all other Firestore REST routes)
```

### CORS Policy

```
AllowedMethods:   GET, POST, PUT, PATCH, DELETE, OPTIONS
AllowedHeaders:   * (any header)
AllowCredentials: true
AllowedOrigins:   server.allowed_origins (empty = allow any origin)
```

### Timeouts

| Scope | Timeout |
|---|---|
| Request headers read | 10 s |
| Full request body read | 30 s |
| Response write | 0 (disabled at server level) |
| REST mux responses (not streaming) | 30 s via `http.TimeoutHandler` |
| gRPC-Web, BrowserChannel | No write deadline; streams are long-lived |

### gRPC-to-HTTP Status Mapping

| gRPC code | HTTP status |
|---|---|
| `NotFound` | 404 |
| `AlreadyExists` | 409 |
| `InvalidArgument` | 400 |
| `Unauthenticated` | 401 |
| `PermissionDenied` | 403 |
| `Unimplemented` | 501 |
| anything else | 500 |

This mapping applies to the custom JSON-array streamers and BrowserChannel error responses. grpc-gateway uses its own internal mapping for routes it owns.

---

## Authentication

embyr uses a two-layer authentication model.

### Layer 1: Service Admin Auth

Requests to `/admin/v1/projects/*` require the service admin API key (`admin.key` from service config), supplied as `Authorization: Bearer <admin_key>`. All other requests do not use this credential. Failure returns `codes.Unauthenticated` / HTTP 401.

### Layer 2: Per-Project Auth

All Firestore data requests (gRPC, gRPC-Web, REST, BrowserChannel) are authenticated against the configuration of the **project identified in the request** (extracted from the resource name path). The auth mode and credentials are stored in the project record and can differ between projects.

**Request-to-project mapping:**
- gRPC/gRPC-Web: the `database` field of the request proto (e.g., `projects/{p}/databases/(default)`) identifies the project.
- REST: the `{project}` segment of the URL path identifies the project.
- BrowserChannel: the `database` query parameter identifies the project.

The server looks up the project record for the identified project. If the project does not exist or `status=deleted`, the request is rejected with `NotFound`. If `status=suspended`, the request is rejected with `PermissionDenied` (`"project suspended"`).

**Per-project auth modes:**

| Mode | Bearer token source | Validation |
|---|---|---|
| `none` | — | None. All requests to this project are accepted. |
| `key` | gRPC: `authorization` metadata. HTTP: `Authorization: Bearer <token>` header. | Argon2id verify against `auth_key_hash`. If `auth_key_hash_2` is set (rotation in progress), also verified against it. Any match = success. Comparison is constant-time at the Argon2id level. |
| `google` | gRPC: `authorization` metadata. HTTP: `Authorization: Bearer <token>` header. | Calls Google tokeninfo endpoint. Tries `?id_token=<token>` first; on 4xx retries with `?access_token=<token>`. **Both paths** require the audience to match the project's `auth_google_project_id`: ID-token: checks `audience`, `aud`, or `azp` fields. Access-token: checks the `audience` field in the tokeninfo response. Token valid for a different project is rejected with `Unauthenticated`. |
| `mtls` | gRPC: TLS peer certificate. HTTP: TLS verified client certificate chain. | Requires at least one verified certificate chain signed by the project's `auth_mtls_ca`. Subject/CN is not checked. gRPC-Web connections inherit the TLS configuration of the REST listener; when any active project uses `auth_mode=mtls`, the REST listener requires client certificates and the gRPC-Web handler validates them against the project's CA. |

**Key hashing specification** (auth_mode=key):
- Algorithm: Argon2id
- Recommended parameters: memory=65536 KiB (64 MB), iterations=3, parallelism=4, tag length=32 bytes
- Salt: 16 cryptographically random bytes, unique per key
- The raw key is never stored or logged. Only the hash is persisted.

**Auth enforcement scope:**
- Applies uniformly to gRPC, gRPC-Web, REST, and BrowserChannel transports.
- `/healthz` and `/readyz` always bypass project auth (no project context in those requests).
- `/admin/v1/*` uses service admin auth only, not project auth.

**Error**: `codes.Unauthenticated` (gRPC) / HTTP 401 for all authentication failures.

The Google tokeninfo URL is configurable at runtime for testing purposes (not exposed via the standard configuration file).

---

## Customer Database Connectivity

Every project has a `backend_mode` that governs how embyr connects to that project's customer-owned Postgres database. The connection is established on-demand and its credentials are cached for `backend.credential_cache_ttl` (default 5 minutes) in the **credential cache**, keyed by `(project_id, api_key_fingerprint)` where `api_key_fingerprint = BLAKE3(api_key)`.

embyr runs schema migrations against the customer database automatically at project creation (`Create Project` admin call). The customer database must be a PostgreSQL instance accessible from embyr at the time of provisioning.

### Mode: `direct_pg`

Credentials are stored inside embyr, encrypted with the customer's API key using ECIES.

**Constraint**: requires `auth_mode=key`.

**Encryption scheme** (ECIES with X25519):

1. At project creation, embyr receives the customer's plaintext API key (`auth_key`) and the PostgreSQL DSN (`backend_pg_dsn`).
2. embyr derives an X25519 private key: `priv = HKDF-SHA256(ikm=auth_key, salt=project_id, info="embyr-db-privkey", length=32)`.
3. embyr computes the corresponding X25519 public key: `pub = X25519(priv, basepoint)`.
4. embyr encrypts the DSN using ECIES: generates an ephemeral X25519 keypair, performs ECDH to obtain a shared secret, derives an AES-256-GCM key via HKDF-SHA256, encrypts the DSN, and produces the ciphertext bundle `{ephemeral_pub || nonce || ciphertext || auth_tag}`.
5. Stores `backend_pg_pubkey = pub` and `backend_pg_creds_enc = <ciphertext bundle>`. The plaintext DSN and the private key are discarded immediately.

**Decryption** (per request, after credential cache miss):

1. Extract the API key from the request's Bearer token.
2. Re-derive `priv = HKDF-SHA256(ikm=api_key, salt=project_id, info="embyr-db-privkey", length=32)`.
3. ECIES decrypt `backend_pg_creds_enc` using `priv` to obtain the plaintext DSN.
4. Insert into credential cache with TTL. Return DSN.

**Security property**: embyr cannot decrypt the customer's database credentials without the customer actively presenting their API key on an in-flight request. Credential material never exists at rest in plaintext.

**Key rotation** re-encrypts the DSN as described in §Tenant Management.

### Mode: `aws_secret`

Credentials are stored in AWS Secrets Manager under the customer's own AWS account. embyr fetches them using its own AWS IAM identity.

**Setup** (customer responsibility):
1. Customer stores the PostgreSQL DSN as a plaintext string secret in AWS Secrets Manager.
2. Customer attaches a resource-based policy granting `secretsmanager:GetSecretValue` to embyr's IAM principal (role ARN or account ID, as specified in service documentation).

**Fetch** (per credential cache miss):
1. embyr calls `GetSecretValue(SecretId=backend_secret_arn)` using the configured IAM credentials (optionally assuming `backend.aws.role_arn` first for cross-account access).
2. On success: parses the secret string as the PostgreSQL DSN. Inserts into credential cache with TTL.
3. On `AccessDeniedException` or `ResourceNotFoundException`: returns `PermissionDenied` to the caller with message `"cannot retrieve database credentials"`.
4. On throttling or transient AWS errors: returns `Unavailable`.

**embyr IAM identity**: provided via standard AWS credential chain (instance metadata, ECS task role, or `backend.aws.role_arn`). No static AWS credentials are stored in embyr's config.

### Mode: `gcp_secret`

Credentials are stored in GCP Secret Manager under the customer's own GCP project. embyr fetches them using its own GCP service account.

**Setup** (customer responsibility):
1. Customer stores the PostgreSQL DSN as a secret version in GCP Secret Manager.
2. Customer grants `roles/secretmanager.secretAccessor` to embyr's GCP service account on that specific secret.

**Fetch** (per credential cache miss):
1. embyr calls `projects.secrets.versions.access(name=backend_secret_gcp)` using its GCP credentials (service account key or Workload Identity ADC).
2. On success: base64-decodes the payload data as the PostgreSQL DSN. Inserts into credential cache with TTL.
3. On `PERMISSION_DENIED` or `NOT_FOUND`: returns `PermissionDenied`.
4. On transient GCP errors: returns `Unavailable`.

**embyr GCP identity**: configured via `backend.gcp.service_account_key` (path to JSON key) or Workload Identity (no key; ADC from environment). Prefer Workload Identity in GKE deployments.

### Mode: `agent`

The customer runs an **embyr agent** binary in their own infrastructure. The agent holds all database credentials locally (never transmitted to embyr SaaS) and proxies storage operations over a secure gRPC channel.

**Connection**:
- embyr SaaS connects to the agent at `backend_agent_endpoint` using mTLS.
- embyr presents the certificate from `backend.agent.cert`/`backend.agent.key`.
- The agent's certificate is verified against `backend_agent_ca` stored in the project record.
- The agent verifies embyr's certificate against the CA it was configured with at deployment time.

**No credential fetch needed**: the agent provides the StorageAdapter interface directly. embyr SaaS calls the agent's gRPC methods identically to how it would call a local Postgres adapter.

**Connectivity failure**: if the agent endpoint is unreachable, all storage operations for that project return `Unavailable`. No fallback. Clients should reconnect and retry.

---

## embyr Agent

The embyr agent is a statically-linked Rust binary deployed inside customer infrastructure. It bridges the embyr SaaS and the customer's PostgreSQL database without exposing credentials to the SaaS.

### Responsibilities

- Hold a PostgreSQL connection pool configured from environment variables.
- Expose a gRPC service implementing the embyr Storage RPC protocol (see §Agent gRPC Protocol).
- Authenticate incoming connections from the embyr SaaS using mTLS.
- Execute all storage operations (document CRUD, queries, transactions, subscriptions) against the local PostgreSQL database.
- Emit `DocChange` notifications back to the SaaS over the same gRPC connection.

### Configuration

All configuration is via environment variables or a YAML file:

| Variable | Required | Description |
|---|---|---|
| `EMBYR_AGENT_DB_DSN` | yes | PostgreSQL connection string |
| `EMBYR_AGENT_LISTEN_ADDR` | no (default `:9191`) | TCP address to listen on |
| `EMBYR_AGENT_CERT` | yes | Path to PEM TLS certificate (signed by a CA registered with embyr SaaS) |
| `EMBYR_AGENT_KEY` | yes | Path to PEM private key for the certificate |
| `EMBYR_AGENT_CA` | yes | Path to PEM CA certificate used to verify the embyr SaaS identity |
| `EMBYR_AGENT_MAX_CONNS` | no (default 25) | Max Postgres connections in the pool |
| `EMBYR_AGENT_LOG_LEVEL` | no (default `info`) | Log level |

### Agent gRPC Protocol

The agent exposes a private gRPC service `embyr.agent.v1.StorageAgent` over mTLS on `EMBYR_AGENT_LISTEN_ADDR`. The service mirrors the `StorageAdapter` interface:

- All document CRUD operations (Create, Get, Update, Delete, List, Query)
- Transaction lifecycle (Begin, GetForTransaction, Commit, Rollback, Sweep)
- Aggregation queries
- Collection ID listing
- Change subscription: the agent opens a server-streaming RPC `Subscribe(project_id) → stream DocChange`. embyr SaaS maintains one long-lived subscription per project; the agent pushes change events as they are committed locally.

The protocol is internal and versioned. The agent binary version must match the embyr SaaS major version.

### Security Model

- mTLS is mandatory. There is no unauthenticated mode.
- The agent's TLS certificate is issued by the customer (or by embyr's agent CA if the customer delegates certificate issuance). The CA PEM is registered with embyr when the project is created (`backend_agent_ca`).
- The embyr SaaS certificate is issued by embyr's own CA (`backend.agent.cert`/`backend.agent.key`). The customer registers embyr's CA with their agent at deployment time (`EMBYR_AGENT_CA`).
- Credential material (PG DSN) never leaves the customer's network. embyr SaaS never learns the database password.

### Lifecycle

- **Startup**: agent connects pool to Postgres, verifies WAL mode / connection, starts gRPC listener.
- **Reconnection**: embyr SaaS reconnects to the agent with exponential backoff (1 s initial, max 30 s) if the connection is lost.
- **Shutdown**: agent drains in-flight RPCs, closes pool, exits. All in-flight embyr requests receive `Unavailable`.

---

## Document Paths

### Resource Name Format

```
projects/{project}/databases/{database}/documents/{collection}/{docId}[/{collection}/{docId}]*
```

- `{project}` identifies the tenant. The authenticated project (from the auth layer) must match the `{project}` in every resource name in the request. A mismatch returns `PermissionDenied`.
- `{database}` is typically `(default)`.
- The segment count after `documents/` must be **even**.
- Trailing slashes in parent paths are normalized away before document path construction.

### Field Paths

Field paths use dot-notation to address nested map fields:

- `profile.age` addresses `document.fields["profile"].mapValue.fields["age"]`.
- Field path segments match `^[a-zA-Z_][a-zA-Z0-9_.]*$`. Invalid field paths are rejected with `InvalidArgument`.
- Both SQL query filtering and in-memory live-change filtering traverse dot-notation identically.

### Document ID Generation

When no `document_id` is provided to `CreateDocument`, the server generates a 20-character random ID drawn from the alphabet `[a-zA-Z0-9]` (62 characters) using cryptographically secure random bytes. Returns `codes.Internal` if entropy generation fails.

---

## Values

| Firestore type | JSON shape (REST/proto3-JSON) |
|---|---|
| null | `{"nullValue": null}` |
| boolean | `{"booleanValue": true}` |
| integer | `{"integerValue": "42"}` (string in proto3-JSON) |
| double | `{"doubleValue": 3.14}` |
| string | `{"stringValue": "x"}` |
| timestamp | `{"timestampValue": "2026-04-25T20:36:30.382807Z"}` |
| array | `{"arrayValue": {"values": [...]}}` |
| map | `{"mapValue": {"fields": {...}}}` |

**Cross-type numeric comparison**: `integer` and `double` fields are compared after type promotion. Integer comparisons use int64 precision; only when one side is double is floating-point used. This avoids precision loss for integers beyond 2^53.

---

## gRPC Service: `google.firestore.v1.Firestore`

### Unary RPCs

#### GetDocument

**Inputs**: `name` (required)
**Outputs**: `Document`
**Errors**: `InvalidArgument` (name empty), `NotFound` (document absent)

#### CreateDocument

**Inputs**: `parent` (required), `collection_id` (required), `document_id` (optional), `document` (optional fields)
**Outputs**: created `Document` with `create_time` and `update_time` set
**Side effects**: emits `DocChange{Upsert}`
**Errors**: `InvalidArgument` (parent or collection_id empty), `AlreadyExists` (path already exists), `Internal` (ID generation failure)

If `document_id` is omitted, the server generates a 20-character random ID.

#### UpdateDocument

**Inputs**: `document` (required, `document.name` required), `update_mask` (optional), `current_document` (optional precondition)
**Outputs**: updated `Document`

Write mode is determined by the precondition:

| Precondition | Mode |
|---|---|
| None | Upsert (create or overwrite) |
| `exists=true` | Update (must exist; `NotFound` if absent) |
| `exists=false` | InsertOnly (must be absent; `AlreadyExists` if present) |
| `update_time` | Update (must exist; `FailedPrecondition` if stored `updated_at` does not match at microsecond precision) |

When `update_mask` is set, the server reads the current document, merges only the masked fields into the existing document, and writes the result. Dot-notation paths are handled recursively; siblings outside the masked subtree are preserved.

`update_mask` alone does **not** force Update mode — `setDoc({merge: true})` sends a mask without a precondition and semantics remain Upsert.

**Errors**: `InvalidArgument`, `NotFound` (Update mode), `AlreadyExists` (InsertOnly), `FailedPrecondition` (update_time mismatch), `Internal`

#### DeleteDocument

**Inputs**: `name` (required), `current_document` (optional precondition)
**Outputs**: empty
**Side effects**: emits `DocChange{Delete}`, inserts tombstone record

Precondition `exists=true` or any `update_time` precondition → `mustExist=true` → returns `NotFound` if document is absent. Default (`exists=false` or no precondition) → idempotent (no error if absent).

**Errors**: `InvalidArgument` (name empty), `NotFound` (mustExist=true and absent)

#### ListDocuments

**Inputs**: `parent` (required), `collection_id` (optional), `page_size` (optional, default 100), `page_token` (optional)
**Outputs**: `{documents: [], next_page_token: string}`

Returns documents directly under `parent`. If `collection_id` is empty, all collections under `parent` are included. `next_page_token` is absent on the last page.

**Errors**: `InvalidArgument` (parent empty)

#### ListCollectionIds

**Inputs**: `parent` (required), `page_size` (optional), `page_token` (optional)
**Outputs**: `{collection_ids: [], next_page_token: string}`

Returns the distinct collection IDs of immediate child collections of the document at `parent`.

#### BeginTransaction

**Inputs**: `options` (optional; `read_only` sub-option sets read-only mode)
**Outputs**: `{transaction: <bytes>}` — the transaction ID encoded as bytes
**Side effects**: records a new transaction record with `expires_at = now + transactions.ttl`

#### Rollback

**Inputs**: `transaction` (required, non-empty)
**Outputs**: empty
**Side effects**: deletes the transaction record without applying any writes; no OCC checks performed

**Errors**: `InvalidArgument` (transaction empty)

#### Commit

**Inputs**: `writes` (list of writes), `transaction` (optional bytes)
**Outputs**: `{write_results: [...], commit_time}`

Always returns **exactly one `WriteResult` per input `Write`**, in the same order. This invariant holds regardless of write type, including `VerifyMutation` entries.

Two execution paths:

**Non-transaction path** (`transaction` absent): all writes run inside a single `WithTransaction` call (atomic). Uses `applyWriteBatch`.

**Transaction path** (`transaction` present): converts writes to `WriteOp` records and calls `CommitTransaction`. OCC validation runs at this point (see §Transactions). `VerifyMutation` entries do not produce a `WriteOp` but still contribute a `WriteResult` with `update_time = commit_time`.

**Errors**: any error from the write operations; `Aborted` on OCC conflict (tx path only).

#### BatchWrite

**Inputs**: `writes`
**Outputs**: `{write_results: [...], status: [null | Status, ...]}`

Each write runs in its **own** transaction — failures are isolated. A failed write does not affect its siblings. `status[i] = null` means success. Failed writes carry a non-null `status[i]` with a gRPC code and message. `write_results[i]` is always non-null (an empty `WriteResult` is used on failure to keep positions aligned).

**No top-level error is returned.** All errors are reported per-write in the `status` array.

---

### Server-Streaming RPCs

#### RunQuery

**Inputs**: `parent` (required), `structured_query` (required, must have `from`)
**Outputs**: stream of `RunQueryResponse`

Each item in the stream is one of:
- `{document: Document, read_time: Timestamp}` — for each matching document
- `{continuation_selector: {done: true}, read_time: Timestamp}` — final item, always present

A `from` clause with `all_descendants: true` performs a collection group query across all nested collections with the given name. A `from` clause with multiple selectors returns `Unimplemented`.

#### RunAggregationQuery

**Inputs**: `parent` (required), `structured_aggregation_query` (required)
**Outputs**: exactly one `RunAggregationQueryResponse{result: {aggregate_fields: {alias → Value}}, read_time}`

Supported aggregation operators: `COUNT` (no field), `SUM` (numeric field), `AVG` (numeric field). An alias is required; if absent, the server synthesizes `field_0`, `field_1`, etc.

**Errors**: `InvalidArgument` (missing required fields), `Unimplemented` (unsupported aggregation operator)

#### BatchGetDocuments

**Inputs**: `documents` (list of resource names), `consistency_selector` (optional; `new_transaction` creates a read-only transaction)
**Outputs**: stream of `BatchGetDocumentsResponse`

Each response is one of:
- `{transaction: bytes, read_time}` — only if `new_transaction` was requested; this is the **first** message and carries no document
- `{found: Document, read_time}` — document exists
- `{missing: path, read_time}` — document absent

Exactly one response per requested path (after the optional transaction message). Order of responses may differ from request order.

---

### Bidirectional-Streaming RPCs

#### Write Stream

Three-step protocol:

1. **Handshake**: client sends `WriteRequest` with empty `writes` and empty `stream_id`.
2. **Server opens stream**: server sends `WriteResponse{stream_id: <16-hex-encoded-unix-nanoseconds>, stream_token: <RFC3339Nano-timestamp>, commit_time}`. No `write_results` in this message.
3. **Write loop**: client sends `WriteRequest{writes: [...], stream_id, stream_token}`. Each `Write` in the `writes` array may include `update_mask` (field mask for partial updates) and `update_transforms` (server-side field transforms). Full mask and transform semantics (see §Write Semantics and §Field Transforms) apply identically on the Write bidi-stream path as on `Commit` and `BatchWrite`. The server applies all writes in a single `WithTransaction` (atomic), then replies `WriteResponse{stream_id, stream_token: <new-timestamp>, write_results: [...], commit_time}`. Step 3 repeats until the stream closes.

Stream termination:
- `io.EOF` from client → server returns nil (clean close).
- `codes.Canceled` → server returns nil.
- Any other error → server propagates the error.

#### Listen Stream

See §Listen Stream section.

---

## Write Semantics (applyWriteBatch)

All non-transaction `Commit` calls and `BatchWrite` operations route individual writes through the same write logic. For each `Write`:

### Update Write

Default mode: Upsert.

Precondition overrides:

| Precondition | Mode |
|---|---|
| `exists=true` | Update |
| `exists=false` | InsertOnly |
| `update_time` set | Update + version check at microsecond precision |

The current document is read **only when** any of these are true: `update_mask` is present, `update_transforms` is present, or `update_time` precondition is set.

When the current document is read for the transform base: a transform-only write (no mask, no explicit fields) seeds its field map from the current document so that unmentioned sibling fields survive.

Mask application: `applyMask` walks dot-notation paths recursively, deep-copying intermediate `MapValue` nodes to prevent aliasing. Sibling fields outside masked subtrees are preserved.

### Delete Write

Default: idempotent (no error if absent). `exists=true` precondition or any `update_time` precondition → `mustExist=true` → `NotFound` if absent. After successful delete, a tombstone record is inserted.

### VerifyMutation Write

Verifies document existence or version without modifying the document.

Inputs:
- `verify`: full resource name of the document to check (required; `InvalidArgument` if empty)
- `current_document`: the precondition to evaluate (optional)

Precondition evaluation:
- `exists=true` on a missing document → `FailedPrecondition`
- `exists=false` on an existing document → `FailedPrecondition`
- `update_time` mismatch (microsecond precision) → `FailedPrecondition`
- `update_time` with missing document → `FailedPrecondition`

A `WriteResult{update_time: commit_time}` is always appended to satisfy the length contract, even on a precondition failure (the error is returned and the whole batch is rolled back before the result reaches the caller).

---

## Field Transforms

Applied via `update_transforms` in a write. One `transformResults` entry is returned per transform, in input order, included in `WriteResult.transform_results`.

| Transform | Behavior | On missing field |
|---|---|---|
| `setToServerValue: REQUEST_TIME` | Sets field to current server UTC timestamp. | Creates the field. |
| `setToServerValue: <anything else>` | Returns `InvalidArgument`. Field is not modified. | — |
| `increment: <integer delta>` | Adds integer delta to existing integer value. Result is integer. | Treats missing as 0. |
| `increment: <double delta>` | Adds double delta to existing numeric value (promotes integer to double). Result is double. | Treats missing as 0.0. |
| `increment: <non-numeric delta>` | Returns `InvalidArgument`. | — |
| `appendMissingElements: <array>` | Merges incoming values into the current array, skipping values already present (proto structural equality). | Creates the field as the incoming array. |
| `removeAllFromArray: <array>` | Removes all elements matching any value in the input (proto structural equality). | No-op; does not create the field. Returns empty array as transform result. |

---

## Transactions

### Begin

Creates a transaction record. Returns a transaction ID (128-bit hex string, encoded as bytes in the response).
- `read_only` option records a read-only flag; the storage layer may optimize accordingly.
- Transaction expires at `started_at + transactions.ttl`.

### Read-Within-Transaction

`GetDocumentForTransaction(txID, path)`:
- Returns the document at `path`.
- Records `(path, document.version)` in the transaction's read set.

### Commit

`CommitTransaction(txID, ops)`:
1. Load the transaction's read set.
2. Open a SQL transaction.
3. For each `(path, recorded_version)` in the read set: re-read `documents.version`. If the stored version differs → rollback, return `codes.Aborted "version mismatch for <path>"`. If the document no longer exists → rollback, return `codes.Aborted "<path> was deleted"`.
4. Apply each `WriteOp` (update or delete).
5. Delete the transaction record.
6. Commit the SQL transaction.
7. Emit `DocChange` events for all modified documents.

### Rollback

Deletes the transaction record. No version checks. No writes applied.

### Sweep

A background goroutine sweeps expired transaction records every `transactions.sweep_interval`. Expired = `expires_at < now`.

---

## Query System

### Filter Operators

| Operator | Symbol |
|---|---|
| Equal | `==` |
| Not equal | `!=` |
| Less than | `<` |
| Less than or equal | `<=` |
| Greater than | `>` |
| Greater than or equal | `>=` |
| In | `in` |
| Not in | `not-in` |
| Array contains | `array-contains` |
| Array contains any | `array-contains-any` |

Unary filters:

| Filter | Meaning |
|---|---|
| `IS_NULL` | Field equals null (equivalent to `== null`) |
| `IS_NOT_NULL` | Field is not null (equivalent to `!= null`) |
| `IS_NAN` | Field is a double NaN value |
| `IS_NOT_NAN` | Field is a numeric value that is not NaN (integers satisfy this; missing field does not) |

A document missing the filtered field is **excluded** from all filter predicates, including `!=`, `not-in`, `IS_NOT_NULL`, and `IS_NOT_NAN`.

### Composite Filters

| Operator | Behavior |
|---|---|
| `AND` | All clauses must match. Default when op is unspecified. |
| `OR` | At least one clause must match. |

A top-level field filter with no explicit composite wrapper is treated as a single-clause AND.

### OrderBy

Each order clause: `{field: string, direction: ASC | DESC}`.

Non-scalar fields use type-aware ordering: numeric values (integer or double) sort numerically; strings and timestamps sort lexicographically/chronologically.

### Cursors

Cursors apply a boundary to a sorted result set:

| Cursor | `before` | `isEnd` | Effective boundary |
|---|---|---|---|
| `startAt(v)` | true | false | field >= v (field <= v for DESC) |
| `startAfter(v)` | false | false | field > v (field < v for DESC) |
| `endAt(v)` | false | true | field <= v (field >= v for DESC) |
| `endBefore(v)` | true | true | field < v (field > v for DESC) |

Multi-field cursors use lexicographic OR-of-AND expansion.

When a query has cursors, page tokens are ignored (offset is 0).

### Pagination

Page tokens are opaque; they encode a decimal row offset as base64. An empty or malformed token is treated as offset 0. Pagination uses look-ahead: the server fetches `pageSize + 1` rows, returns `pageSize`, and emits `next_page_token` only if more rows were available.

Default page sizes:
- `ListDocuments`: 100
- `QueryDocuments` (via Listen): 300
- All others: as specified by the caller or the query limit

### Aggregation

Supported: `COUNT` (counts matching documents), `SUM` (sums a numeric field), `AVG` (averages a numeric field). Alias required (auto-generated if absent). At least one aggregation is required per request.

### Collection Group Queries

A `from` clause with `all_descendants: true` executes a **collection group query**: the query matches documents in **any** collection named `{collection_id}` that is a descendant of `parent`, at any nesting depth, within the project.

Storage implementation must support this by querying all documents where `collection = {collection_id}` and `path` starts with `parent/`, without filtering by an exact `parent` value.

Collection group queries support all filter operators, order clauses, cursors, and pagination identically to regular collection queries. The in-memory filter for live-change delivery (Listen stream) must also support collection group matching.

A `from` clause with more than one collection selector returns `Unimplemented` (not a Firestore-standard query type).

---

## Listen Stream

The `Listen` RPC is a bidirectional stream. Each client manages a set of **targets**; each target is either a query target (collection + filter + order) or a documents target (list of specific paths).

### Client → Server Messages

**AddTarget**:
```
{
  database: "projects/p/databases/(default)",
  addTarget: {
    targetId: <int32>,        // client-chosen, unique within stream
    query: { parent, structuredQuery }   // OR
    documents: { documents: [path, ...] },
    resumeToken: <bytes>      // optional; see Delta Delivery below
  }
}
```

**RemoveTarget**:
```
{ database: ..., removeTarget: <int32> }
```

Re-adding an existing target ID is equivalent to remove + re-add: the server sends `TargetChange{REMOVE}` for the old target, then proceeds with a fresh snapshot.

### Server → Client Messages

| Message | Description |
|---|---|
| `targetChange{ADD, targetIds:[id]}` | Acknowledges target registration. |
| `targetChange{REMOVE, targetIds:[id]}` | Target was removed. |
| `targetChange{CURRENT, targetIds:[id], resumeToken, readTime}` | Snapshot is complete for target. `resumeToken` encodes the snapshot read time. |
| `targetChange{NO_CHANGE, resumeToken?, readTime}` | No data change; used to advance the resume token after live changes, seal the snapshot, and keep-alive. |
| `targetChange{RESET, targetIds:[id,...]}` | Internal buffer overflowed; client must re-establish snapshot. |
| `documentChange{document, targetIds:[id,...]}` | Document was added or updated. |
| `documentDelete{document: path, removedTargetIds:[id,...], readTime}` | Document was deleted. |

### Resume Tokens

Resume tokens are `base64.RawURLEncoding(RFC3339Nano-formatted-UTC-time)`. The encoded time is the read-time of the snapshot or change event.

### Full Snapshot Delivery

On `addTarget` without a valid resume token (or for document targets), the server delivers a full snapshot:

```
1. targetChange{ADD, [targetId]}
2. for each matching document: documentChange{document, [targetId]}
3. targetChange{CURRENT, [targetId], resumeToken=<read_time>}
4. targetChange{NO_CHANGE, resumeToken=<now>}
```

Step 4 seals the snapshot; the SDK waits for it before resolving `getDoc()` / firing `onSnapshot()` for the first time.

For `documents` targets: paths that do not exist are silently skipped.
For `query` targets: pages through the query (default page size 300 or the explicit limit). Read time is captured before the first page.

### Delta Delivery (Resume-Token Reconnect)

On `addTarget` with a valid resume token **and** a query target:

1. Decode the token to obtain `sinceTime`.
2. Retrieve all documents in (parent, collectionID) with `updated_at > sinceTime`.
3. Retrieve all tombstones in (parent, collectionID) with `deleted_at > sinceTime`.
4. Send `targetChange{ADD, [targetId]}`.
5. For each updated document matching the query filter: send `documentChange`.
6. For each deleted path: send `documentDelete`.
7. Send `targetChange{CURRENT, [targetId], resumeToken=<now>}`.
8. Send `targetChange{NO_CHANGE, resumeToken=<now>}`.

If the token is malformed, or the target is a documents target: fall back to full snapshot.

### Live Changes

After snapshot delivery, every committed write reaching the registry is evaluated against all active targets:

- **Documents target**: match on exact path equality.
- **Query target**: match on `parent + collection` equality (or `collection`-only for collection group targets with `all_descendants: true`), then pass through the in-memory filter. The in-memory filter must support every filter operator defined in §Query System: comparison operators (`==`, `!=`, `<`, `<=`, `>`, `>=`), membership operators (`in`, `not-in`, `array-contains`, `array-contains-any`), unary operators (`IS_NULL`, `IS_NOT_NULL`, `IS_NAN`, `IS_NOT_NAN`), AND and OR composites, dot-notation field access, and int64-precision numeric comparison.

For `DocChange{Upsert}`: the server re-fetches the document from storage (to get the fully consistent version), then sends `documentChange`.
For `DocChange{Delete}`: sends `documentDelete` directly (no re-fetch needed).

After each delivered live change, the server sends `targetChange{NO_CHANGE, resumeToken=<now>}` to advance the client's resume token.

**Overflow recovery**: the registry's per-subscriber channel has capacity 64. If it fills, changes are dropped and an overflow flag is raised. On the next change delivery, if the overflow flag is set, the server sends `targetChange{RESET, [all target ids]}` and re-delivers full snapshots for all active targets.

### Keep-Alive

Every 30 seconds of stream idle time, the server sends `targetChange{NO_CHANGE}` (no resume token, no target IDs) to prevent proxy timeouts.

### Stream Termination

- `io.EOF` from client → server returns nil.
- Any other receive error → server propagates the error.
- The receive goroutine and main select loop both respect context cancellation so cancellation never leaks the receive goroutine.

---

## REST Endpoints

### grpc-gateway Routes

These routes are served by grpc-gateway with standard Firestore REST URL patterns. Bodies and responses use proto3-JSON (lowerCamelCase field names). Errors are returned as HTTP error responses using the gRPC-to-HTTP mapping.

| Method | URL | RPC |
|---|---|---|
| GET | `/v1/{name=projects/*/databases/*/documents/*/**}` | GetDocument |
| GET | `/v1/{parent=.../documents}/{collection_id}` | ListDocuments |
| GET | `/v1/{parent=.../documents/*/**}/{collection_id}` | ListDocuments |
| POST | `/v1/{parent=.../documents}/{collection_id}` | CreateDocument |
| POST | `/v1/{parent=.../documents/*/**}/{collection_id}` | CreateDocument |
| PATCH | `/v1/{document.name=projects/*/databases/*/documents/*/**}` | UpdateDocument |
| DELETE | `/v1/{name=projects/*/databases/*/documents/*/**}` | DeleteDocument |
| POST | `/v1/{database=.../databases/*}/documents:beginTransaction` | BeginTransaction |
| POST | `/v1/{database=.../databases/*}/documents:commit` | Commit |
| POST | `/v1/{database=.../databases/*}/documents:rollback` | Rollback |
| POST | `/v1/{database=.../databases/*}/documents:batchWrite` | BatchWrite |
| POST | `/v1/{parent=.../documents}:listCollectionIds` | ListCollectionIds |
| POST | `/v1/{parent=.../documents/*/**}:listCollectionIds` | ListCollectionIds |

### Custom JSON-Array Routes

These three routes are intercepted before grpc-gateway because the Firebase JS Lite SDK calls `JSON.parse()` on the full body and expects a JSON array — not grpc-gateway's concatenated NDJSON output.

#### POST `…/documents:runQuery` and `…/documents/{parent}:runQuery`

- **Request**: proto3-JSON `RunQueryRequest`. `parent` is taken from the URL path, overriding any value in the body.
- **Response**: streaming JSON array. Format: `[item, item, ..., item]`. `Content-Type: application/json`. Each item is a proto3-JSON `RunQueryResponse`. The array is opened with `[` immediately, items are flushed to the client as soon as each document is ready (via `http.Flusher`), and the array is closed with `]` after the final item (the `{done:true}` continuation selector). An empty result sends `[]`.
- **Error before first byte**: plain HTTP error with `grpcCodeToHTTP` status code.
- **Error after first byte**: an error element is appended as the last item: `,{"error":{"code":<HTTP_CODE>,"message":"...","status":"<GRPC_NAME>"}}]`.

#### POST `…/documents:batchGet`

- **Request**: proto3-JSON `BatchGetDocumentsRequest`. `database` is taken from the URL path.
- **Response**: fully buffered JSON array. All documents are collected before any bytes are sent. `Content-Type: application/json`. Each item is a proto3-JSON `BatchGetDocumentsResponse`. The `new_transaction` response (if requested) is the first item.
- Note: The response is buffered (not streamed) because the JS Lite SDK uses `Array.prototype.forEach` on the parsed body. This is the correct behavior for the target client environment.
- **Error**: plain HTTP error (no partial array possible since nothing is written until complete).

#### POST `…/documents:runAggregationQuery` and `…/documents/{parent}:runAggregationQuery`

- **Request**: proto3-JSON `RunAggregationQueryRequest`.
- **Response**: single JSON array: either `[]` (no result) or `[<RunAggregationQueryResponse>]`. `Content-Type: application/json`.
- **Error before writing**: plain HTTP error.

---

## BrowserChannel / WebChannel Transport

The Firebase JS SDK uses this transport for `Listen` and `Write` streams in browsers. It uses HTTP/1.1 long-polling because browsers cannot speak HTTP/2 streaming reliably.

### URL Paths

```
/google.firestore.v1.Firestore/Listen/channel
/google.firestore.v1.Firestore/Write/channel
```

Both paths accept `POST` (forward channel) and `GET` (back channel).

### Query Parameters

| Parameter | Direction | Meaning |
|---|---|---|
| `VER` | both | Protocol version (always 8). Not validated server-side. |
| `database` | both | Database path. Not validated server-side. |
| `RID` | POST | Request ID. SDK retries with the same RID; server deduplicates via a 16-entry ring buffer per session. |
| `SID` | POST, GET | 24-hex session ID. Absent on the initial new-session POST. |
| `AID` | GET | Last array-id the client has acknowledged. Log read index = `max(0, AID - 1)`. |
| `CI` | GET | Connection index (0, 1, …). Used for logging only. |
| `TYPE` | GET | `"xmlhttp"` on the back-channel. Not validated. |
| `RID=rpc` | GET | Sentinel for the back-channel request. Not validated. |

### Forward Channel (POST)

Request body: `application/x-www-form-urlencoded`:
```
count=<N>&ofs=<M>&req0___data__=<proto3-JSON>&req1___data__=<proto3-JSON>&...
```

Each `reqN___data__` is a proto3-JSON `ListenRequest` (for `Listen/channel`) or `WriteRequest` (for `Write/channel`). `ofs` is ignored server-side.

**New-session POST (no `SID`):**
- Creates a session, assigns a 24-hex session ID, starts the underlying gRPC handler goroutine and pump goroutine.
- Pushes the parsed requests to the session's receive channel.
- Returns the **connect chunk** (see §Chunk Formats) as the response body.
- Response headers: `Content-Type: text/plain; charset=utf-8`, `X-Content-Type-Options: nosniff`. Status: 200.

**Existing-session POST (with `SID`):**
- Looks up the session by ID. Returns HTTP 400 if session not found.
- If this RID was seen before (dedup): skips parsing; still returns the status body.
- If new RID: parses body, pushes requests to session's receive channel.
- Returns the forward POST status body (see §Chunk Formats). Status: 200.
- A Listen POST using a Write session's SID (or vice versa) returns HTTP 400.

### Back Channel (GET)

Long-poll streaming response. Headers: `Content-Type: text/plain; charset=utf-8`, `Transfer-Encoding: chunked`, `X-Content-Type-Options: nosniff`. Status: 200.

The handler:
1. Flushes headers immediately.
2. Reads chunks from the session's log starting at `logIdx = max(0, AID - 1)`.
3. Writes all available chunks, flushes.
4. Waits for one of: client disconnect, session shutdown, new data from pump, 25-second keep-alive tick.
5. On keep-alive: writes a noop chunk, flushes.
6. Repeats from step 2.

### Chunk Formats

Every chunk is: `<decimal-byte-length>\n<UTF-8-JSON>`. The JSON is always an array of `[seq, payload]` pairs. Multiple chunks concatenate without delimiters.

**Connect chunk** (new-session POST response and first back-channel message):
```
59
[[0,["c","<sid>","",8,8,0]],[1,["noop"]]]
```
After this, the session sequence counter is at 2.

**Data chunk**:
```
N
[[<seq>,[<proto3-JSON-message>]]]
```
The message is wrapped in an extra array (`[<json>]`). The sequence number is claimed from a monotonic atomic counter.

**Noop chunk**:
```
N
[[<seq>,["noop"]]]
```
Sequence number from the same counter as data chunks (monotonic, no gaps, no duplicates within a session).

**Forward POST status body** (response to existing-session POST):
```
<decimal-byte-length>\n[<lastSeqSent>,0,0]
```
The SDK uses the same chunk parser on forward POST responses as on the back channel.

### RID Deduplication

A ring buffer of the last 16 RIDs is maintained per session. If the incoming RID is in the buffer, the request body is silently ignored. The response is still written to advance the SDK's state machine.

### Session Lifecycle

Session bridge contexts are rooted in `context.Background()` so the underlying gRPC handler survives HTTP request boundaries. On server shutdown, all active sessions are explicitly cancelled so their goroutines exit cleanly.

---

## Storage Backend Contract

embyr uses two distinct adapter roles:

**SystemAdapter** — connects to the operator-managed system database (`backend.type` + `backend.postgres.dsn` from service config). Stores only project records and metrics. Operations: `Ping`, `Migrate` (system schema), `CreateProject`, `GetProject`, `UpdateProjectAuth`, `DeleteProject`, `RecordMetrics`, `SweepDeletedProjects`.

**ProjectAdapter** — connects to a specific project's customer database. Obtained by resolving credentials for the project (credential cache → backend mode → fetch/decrypt) and establishing a Postgres connection pool. Operations: all document, transaction, query, subscription, and tombstone operations. Obtained per-request via `AdapterForProject(project_id, api_key)`.

Both adapter types implement the same underlying Postgres adapter code; the distinction is which database DSN they connect to and which schema they operate against. `backend.type=sqlite` is only valid for the system adapter in single-node development configurations.

### Operations

**Ping**: verifies the database is reachable. Used by `/readyz`.

**Migrate**: applies all pending schema migrations in order. Called once at startup.

**AdapterForProject(project_id, api_key) → ProjectAdapter**: resolves the customer database connection for a project. Steps: (1) check credential cache for `(project_id, BLAKE3(api_key))`; on hit, return cached DSN. (2) Load project record from system DB. (3) Resolve credentials per backend mode (ECIES decrypt for `direct_pg`; AWS/GCP fetch for secret manager modes; no-op for `agent` — connection is already managed separately). (4) Insert into credential cache. (5) Return or create a Postgres connection pool for the resolved DSN. Returns `Unavailable` if credentials cannot be resolved. Returns `Unauthenticated` if ECIES decryption fails (wrong key).

All document operations below are **scoped to a `project_id`**. The caller (server layer) always passes the authenticated project ID; the adapter never allows cross-project access.

**CreateProject / GetProject / UpdateProjectAuth / DeleteProject**: tenant lifecycle operations. `DeleteProject` cascades all data deletion.

**CreateDocument**: inserts a new document within a project. Returns `AlreadyExists` if `(project_id, path)` is taken.

**GetDocument**: fetches a document by `(project_id, path)`. Returns `NotFound` if absent.

**UpdateDocument**: writes a document according to the specified mode (Upsert / Update / InsertOnly). The `data` field must contain the complete final state; the caller is responsible for mask merging before calling.

**DeleteDocument**: removes a document. Also inserts a tombstone record. If `mustExist=true` and the document is absent, returns `NotFound`.

**ListDocuments**: returns a paginated list of documents within a project under a parent, optionally filtered by collection ID. Default page size: 100.

**QueryDocuments**: returns paginated documents within a project matching a `Query` (filter, order, cursors, limit, page token).

**WithTransaction**: executes a function inside a SQL transaction. Rolls back on error or panic. Panic rollback is critical for SQLite (single-connection) to prevent deadlock.

**BeginTransaction / GetDocumentForTransaction / CommitTransaction / RollbackTransaction**: Firestore transaction lifecycle, all scoped to a project. See §Transactions.

**SweepExpiredTransactions**: deletes transaction records across all projects with `expires_at < now`. Returns count deleted.

**RunAggregationQuery**: executes COUNT, SUM, or AVG over a base query within a project.

**ListCollectionIds**: returns distinct collection IDs of immediate child collections within a project.

**Subscribe**: returns `(<-chan DocChange, cancelFn)`. The channel receives every committed write **for the specified project only**. Multiple concurrent subscribers (from concurrent Listen streams for the same project) are supported. The channel is buffered (capacity 64). Slow subscribers drop changes rather than blocking the notification path; the overflow is signaled via the `Subscription.Overflowed()` flag. Subscribers for different projects are fully isolated.

**GetDocumentsSince**: returns all documents in `(project_id, parent, collectionID)` with `updated_at > since`, ordered by `updated_at` ascending. Used for resume-token delta delivery.

**GetDeletedSince**: returns paths of documents in `(project_id, parent, collectionID)` deleted after `since` (from the tombstones table). Used for resume-token delta delivery.

**SweepTombstones**: deletes tombstone records across all projects with `deleted_at < before`. Returns count deleted. Called by a background goroutine every sweep interval with `before = now - 24h`.

**RecordMetrics(project_id, ingress_bytes, egress_bytes, cpu_ms)**: atomically increments the `daily_project_metrics` row for `(project_id, today_UTC)` by the given deltas using an upsert (`INSERT ... ON CONFLICT DO UPDATE SET ingress_bytes = ingress_bytes + $delta, ...`). Called by the request middleware after each completed Firestore data request. Does not block the request path; recording failures are logged and silently dropped.

**SweepDeletedProjects**: finds all projects with `status=deleted` and `deleted_at < now - admin.deletion_retention`. For each: (1) connects to the customer database (using stored credentials); drops all customer schema tables (`documents`, `transactions`, `deleted_documents`, `indexes`) or deletes all rows; (2) deletes the project's rows from `daily_project_metrics` in the system DB; (3) removes the project record from the system DB. If the customer database is unreachable (credentials invalid, server down), logs a warning and skips that project (retried on the next sweep cycle). Returns count purged. Called by a background goroutine on a configurable sweep interval.

### Schema

#### System Database Schema

Lives in the operator-managed Postgres instance. Contains no customer document data.

```sql
-- Tenants
projects(
  project_id             TEXT    PRIMARY KEY,
  status                 TEXT    NOT NULL DEFAULT 'active',  -- 'active' | 'suspended' | 'deleted'
  created_at             TIMESTAMPTZ NOT NULL,
  deleted_at             TIMESTAMPTZ,
  auth_mode              TEXT    NOT NULL DEFAULT 'none',
  auth_key_hash          TEXT,   -- argon2id hash; auth_mode='key'
  auth_key_hash_2        TEXT,   -- argon2id hash; key rotation in progress
  auth_google_project_id TEXT,   -- auth_mode='google'
  auth_mtls_ca           TEXT,   -- PEM CA; auth_mode='mtls'
  backend_mode           TEXT    NOT NULL DEFAULT 'direct_pg',
  backend_pg_creds_enc   BYTEA,  -- ECIES ciphertext of PG DSN; backend_mode='direct_pg'
  backend_pg_pubkey      BYTEA,  -- X25519 public key; backend_mode='direct_pg'
  backend_secret_arn     TEXT,   -- AWS secret ARN; backend_mode='aws_secret'
  backend_secret_gcp     TEXT,   -- GCP secret resource name; backend_mode='gcp_secret'
  backend_agent_endpoint TEXT,   -- host:port; backend_mode='agent'
  backend_agent_ca       TEXT    -- PEM CA cert for agent mTLS; backend_mode='agent'
);

-- Daily usage metrics
daily_project_metrics(
  project_id    TEXT  NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
  date          DATE  NOT NULL,
  ingress_bytes BIGINT NOT NULL DEFAULT 0,
  egress_bytes  BIGINT NOT NULL DEFAULT 0,
  cpu_ms        BIGINT NOT NULL DEFAULT 0,
  PRIMARY KEY (project_id, date)
);
```

System DB secondary indexes: `projects.(status, deleted_at)`. `daily_project_metrics.(date)`.

#### Customer Database Schema

Migrated into each project's customer-owned Postgres at project-creation time. The `project_id` column is a fixed literal (the project's own ID) — it is retained for compatibility with shared-DB deployments and for query scoping, but in a per-project database it is always a single value.

```sql
-- Documents
documents(
  project_id  TEXT   NOT NULL,
  path        TEXT   NOT NULL,
  collection  TEXT   NOT NULL,
  parent      TEXT,
  data        JSONB,          -- proto3-JSON {"fields":{...}}
  created_at  TIMESTAMPTZ NOT NULL,
  updated_at  TIMESTAMPTZ NOT NULL,
  version     BIGINT NOT NULL DEFAULT 1,
  PRIMARY KEY (project_id, path)
);

-- Transactions
transactions(
  project_id  TEXT   NOT NULL,
  id          TEXT   NOT NULL,  -- 128-bit hex
  started_at  TIMESTAMPTZ NOT NULL,
  reads       JSONB,            -- {"path": version, ...}
  expires_at  TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (project_id, id)
);

-- Tombstones (resume-token delta delivery)
deleted_documents(
  project_id  TEXT   NOT NULL,
  path        TEXT   NOT NULL,
  collection  TEXT   NOT NULL,
  parent      TEXT,
  deleted_at  TIMESTAMPTZ NOT NULL
);

-- Composite indexes
indexes(
  project_id  TEXT   NOT NULL,
  id          TEXT   NOT NULL,
  collection  TEXT   NOT NULL,
  fields      JSONB  NOT NULL,
  state       TEXT   NOT NULL DEFAULT 'READY',
  PRIMARY KEY (project_id, id)
);
```

Customer DB secondary indexes: `documents.(project_id, collection)`, `documents.(project_id, parent)`, `documents.(project_id, updated_at)`, `documents.data` (GIN). `transactions.(project_id, expires_at)`. `deleted_documents.(project_id, deleted_at)`, `deleted_documents.(project_id, collection, parent)`.

### OCC Contract

The version column is the OCC token. `CommitTransaction` verifies the version of every read document matches the recorded value at the moment writes are applied (inside a SQL transaction). Any mismatch aborts the Firestore transaction with `codes.Aborted`.

### Change Notification

**Direct Postgres** (`direct_pg`, `aws_secret`, `gcp_secret`): change events are driven via `LISTEN doc_changes` on a dedicated connection per customer database. A trigger fires `pg_notify('doc_changes', payload)` on every `documents` row change, including the `project_id` in the payload. The listener auto-reconnects on failure with exponential backoff (1 s initial, up to 30 s).

**Agent mode**: the embyr SaaS maintains one long-lived gRPC `Subscribe` stream per project against the agent. The agent pushes `DocChange` events over this stream as writes are committed locally. On stream disconnection, embyr reconnects with backoff; active Listen clients for the project receive `RESET` and re-snapshot on reconnection.

`DocChange` carries a `project_id` field. The Registry fans out only to subscribers registered for the matching project. Subscribers for different projects never receive each other's change events.

`DocChange.Data` carries the full document JSON for upserts. The Listen handler re-fetches via `GetDocument` rather than using the payload directly (because the Postgres trigger payload has an 8 KB size cap and may be truncated; agent payloads have no such limit but re-fetch is kept for consistency).

### SQLite Specifics

Used only for the system database in single-node development. Must not be used for customer databases or in multi-instance deployments.

- WAL journal mode is required and verified at startup.
- `busy_timeout` = 5000 ms.
- `MaxOpenConns = 1` (single writer; all reads and writes serialize through one connection).

### Postgres Specifics

- `MaxOpenConns` = `backend.postgres.max_conns` (default 25).
- DSN is held in a closure to prevent exposure via reflection or logging.

---

## Health Endpoints

| Endpoint | Response |
|---|---|
| `GET /healthz` | Always `200 OK` with body `ok`, as long as the process is running. |
| `GET /readyz` | `200 OK` with body `ok` if `db.Ping` succeeds within 2 seconds. `503 Service Unavailable` with body `db unreachable: <error>` otherwise. |

No authentication, no CORS handling, no timeout applies to health endpoints.

---

## Error Model

All errors are gRPC status errors (`codes.Code` + message string). Over REST, errors are mapped to HTTP status codes via the gRPC-to-HTTP table in §Transports.

| Category | Code | Meaning |
|---|---|---|
| Bad input | `InvalidArgument` | Missing required field, invalid format, unsupported value |
| Not found | `NotFound` | Document absent when required |
| Conflict | `AlreadyExists` | Document present when absence required |
| Precondition | `FailedPrecondition` | `update_time` mismatch, `exists` precondition violated, VerifyMutation failure |
| OCC conflict | `Aborted` | Transaction version mismatch at commit |
| Auth | `Unauthenticated` | Missing or invalid credentials |
| Auth | `PermissionDenied` | Valid credentials but access denied |
| Not supported | `Unimplemented` | Feature not implemented (multi-selector `from` clauses, unsupported aggregation ops) |
| Server error | `Internal` | Encoding failures, entropy failures, unexpected internal states |

Errors are not recoverable vs. fatal at the RPC level; every error terminates the current call. For streaming RPCs (`Listen`, `Write`, `RunQuery`), an error on the stream terminates the stream. The client is expected to reconnect.

`BatchWrite` is the single exception: per-write errors are reported in the `status` array without terminating the RPC.

---

## Invariants

1. The length of `write_results` in any `CommitResponse` or `BatchWriteResponse` exactly equals the length of the input `writes` array, regardless of write type.
2. Every `DocChange` event emitted after a successful write has `Version >= 1` for upserts, and `Kind == Delete` for deletes.
3. A document's `version` monotonically increases across successive writes. It never decreases.
4. A tombstone record exists for every document that has been deleted since the current tombstone sweep window (24 hours).
5. `updated_at >= created_at` for every stored document.
6. Field paths interpolated into SQL are always validated against `^[a-zA-Z_][a-zA-Z0-9_.]*$` before use.
7. No sequence number within a BrowserChannel session is ever reused. Data chunks and noop chunks share the same monotonic counter.
8. Within a transaction, every document read is recorded in the read set. At commit, every recorded read is re-validated.
9. Authentication is enforced on all requests (gRPC, gRPC-Web, REST, WebChannel) except `/healthz` and `/readyz`.
10. **Tenant isolation**: every database query, every storage operation, and every DocChange fan-out is predicated on a `project_id`. No code path can access or emit events for a project other than the one identified by the authenticated request.
11. The `{project}` in every resource name in a request must match the authenticated project. Mismatches are rejected with `PermissionDenied` before any storage is accessed.
12. Soft-deleting a project immediately blocks all Firestore requests for that project. Background purge runs after `admin.deletion_retention` and removes all documents, transactions, tombstones, indexes, and metrics rows. No orphaned rows remain after purge completes.
13. **Customer database credentials never exist in plaintext in the system database.** `direct_pg` stores only an ECIES ciphertext that cannot be decrypted without the customer's API key. `aws_secret`/`gcp_secret` store only a resource identifier, never the DSN. `agent` mode stores no credentials at all.
14. `backend_mode=direct_pg` is only permitted when `auth_mode=key`. The invariant is enforced at project creation and at auth mode changes.
15. The credential cache entry for a project is invalidated whenever the project's `auth_key_hash` or `backend_pg_creds_enc` is updated (key rotation or credential update). Stale cached DSNs must not be used after rotation completes.

---

## Limits and Constraints

| Constraint | Value |
|---|---|
| Max document write batch (transactional atomicity) | Unbounded (limited by SQL tx capacity) |
| Max resume token retention window | 24 hours (tombstone sweep retention) |
| Max concurrent BrowserChannel sessions | Unbounded (limited by memory) |
| BrowserChannel RID dedup window | 16 most-recent RIDs |
| Listen registry per-subscriber buffer | 64 `DocChange` events |
| Transaction TTL | Configurable; default 60 s |
| Default query page size (Listen) | 300 documents |
| Default query page size (ListDocuments) | 100 documents |
| Look-ahead pagination fetch | `pageSize + 1` rows |
| gRPC-to-HTTP timeout (REST mux, non-streaming) | 30 s |
| Keep-alive interval (Listen stream) | 30 s |
| Keep-alive interval (BrowserChannel back-channel) | 25 s |
| readyz Ping timeout | 2 s |
| Field path character set | `[a-zA-Z_][a-zA-Z0-9_.]*` |
| Document ID length | 20 characters from `[a-zA-Z0-9]` |
| Transaction ID length | 32 hex characters (128 bits) |

**Ordering guarantees**: `GetDocumentsSince` returns results ordered by `updated_at` ascending. `QueryDocuments` order is determined by the `OrderBy` clauses in the query; unordered queries have no guaranteed order.

**Idempotency**: `DeleteDocument` with `mustExist=false` is idempotent. All other writes are not idempotent (each call increments `version`).

---

## Rate Limiting

Per-project rate limits are enforced at the transport layer (before any storage access). Limits are configured per-project via the admin API and have service-wide defaults.

### Default Limits

| Resource | Default | Error on exceed |
|---|---|---|
| Concurrent `Listen` streams per project | 100 | `ResourceExhausted` |
| Concurrent `Write` bidi-stream sessions per project | 50 | `ResourceExhausted` |
| Write requests per second per project (across Commit, BatchWrite, Write stream) | 500 | `ResourceExhausted` |
| Read requests per second per project (GetDocument, RunQuery, BatchGet, etc.) | 1000 | `ResourceExhausted` |
| BrowserChannel sessions per project | 200 | `ResourceExhausted` (HTTP 429) |

Limits use a **token bucket** algorithm with a burst capacity equal to 2× the per-second rate. The bucket state is in-memory, per embyr instance. In a multi-instance deployment, limits are per-instance (not globally enforced across the cluster); operators must size limits accordingly.

### Per-Project Limit Configuration

The admin API allows overriding any limit for a specific project:

**Method**: `PATCH /admin/v1/projects/{project_id}/limits`
**Auth**: service admin credentials required
**Inputs**: any subset of limit fields (concurrent_listen, concurrent_write_streams, writes_per_second, reads_per_second, browser_channel_sessions)
**Outputs**: updated limits record
**Side effects**: limits take effect immediately for the next request window

**Error response** when a rate limit is exceeded:
- gRPC: `codes.ResourceExhausted` with message `"rate limit exceeded: <resource_name>"`
- HTTP: 429 Too Many Requests with body `{"error":{"code":429,"message":"rate limit exceeded: <resource_name>","status":"RESOURCE_EXHAUSTED"}}`

---

## Billing and Usage Metering

embyr records per-project daily usage metrics for billing purposes. The service does not sell storage; it sells protocol translation (API access). Metrics are stored in `daily_project_metrics` and consumed by an external billing system via direct DB access or ETL.

### Measured Dimensions

| Dimension | Granularity | Notes |
|---|---|---|
| Network ingress bytes | Per project, per day | Total bytes received across all transports for Firestore data requests |
| Network egress bytes | Per project, per day | Total bytes sent across all transports for Firestore data responses |
| CPU time (milliseconds) | Per project, per day | Wall-time of request processing attributable to the project; recorded on best-effort basis |

### Recording

Metrics are recorded atomically via upsert (`INSERT ... ON CONFLICT (project_id, date) DO UPDATE SET ingress_bytes = ingress_bytes + $delta, ...`) after each request completes. Dates use UTC.

Metrics for suspended or soft-deleted projects continue to accrue until the deletion retention window expires. The billing system uses `deleted_at` to apply final billing adjustments.

### Access

No embyr API endpoint exposes metrics data. All access is direct-to-database by authorized internal systems (ETL jobs, billing service). embyr does not serve a billing API.

---

## Multi-Region Architecture

embyr is a stateless protocol translation layer. Multi-region deployment follows the pattern:

- Each region runs one or more embyr instances backed by a **regional Postgres cluster**.
- embyr instances within a region are stateless across requests (except in-process BrowserChannel session tables and Listen registries). Horizontal scaling requires **sticky routing** for clients with active BrowserChannel sessions (by `SID`) or active Listen streams (by TCP connection), handled by the load balancer.
- The Postgres `NOTIFY/LISTEN` change channel is regional: a write committed by an embyr instance in region A notifies all Listen subscribers connected to region A's embyr instances only. Clients in region B do not receive live updates for region A writes.
- **Failover**: handled at DNS/load-balancer level. Client SDKs reconnect to the surviving region; on reconnect, they re-deliver the last resume token, triggering delta delivery for changes since the last snapshot.
- embyr owns no cross-region replication. Postgres replication (streaming replication, logical replication, or Citus global tables) is the operator's concern and outside this specification.
- **SQLite backend** is single-node only; it must not be used in multi-region or multi-instance deployments.
