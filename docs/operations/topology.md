# Recommended Production Topology (V1)

Closes the topology half of go-live finding O4
([`go-live-assessment-2026-11-13.md`](../product/go-live-assessment-2026-11-13.md)).

## Status

A recommended *shape*, not a deployed one: there is no IaC in this repo and the owner has not
chosen a hosting platform. Statements marked **(code)** are derived from reading
`crates/embyr-server`; statements marked **(guess)** are starting points that the load test in
[`loadtest/`](../../loadtest/README.md) must confirm; **(decide)** items are collected in the
launch checklist at the end.

This document links to, and does not repeat:
[`single-host-deployment.md`](./single-host-deployment.md) (one-box recipe),
[`network-topology.md`](./network-topology.md) (DNS, :9090 firewalling),
[`resource-sizing.md`](./resource-sizing.md) (CPU/RAM estimates, Argon2 memory),
[`backup-disaster-recovery.md`](./backup-disaster-recovery.md) (backups, RPO/RTO),
[`secrets-manager-setup.md`](./secrets-manager-setup.md),
[`release-process.md`](./release-process.md), [`migration-policy.md`](./migration-policy.md),
[`runbook.md`](./runbook.md), [`slo.md`](./slo.md),
[ADR-001](../product/architecture/adr-001-process-topology.md),
[ADR-079](../product/architecture/adr-079-pool-sizing-and-limits.md).

## 1. Shape

```
 clients ──TLS──> L7 load balancer ──> N x embyr-server  (stateless apart from in-process caches)
 (SDKs, web)        :443 / :443         :8080 gRPC  :8081 gRPC-Web+REST+BrowserChannel
                                        :9090 admin (private only)
                                              │                     │
                              system Postgres (HA, PITR)    customer Postgres (their own, BYOC)
                                                            or embyr-agent / secret-manager modes
 Prometheus ──scrapes each replica :9090/metrics     blackbox_exporter ──probes /healthz
```

- **N stateless replicas, N >= 3 across 3 availability zones (guess).** Three so one replica or
  one AZ can be lost, or rolled, and capacity holds.
- Replicas hold only process-local caches and sessions: the credential cache (60 s TTL), per-
  project tenant pools, the Listen registry, BrowserChannel sessions, in-process rate-limit
  fallback buckets **(code)**. Nothing else needs to be shared between replicas.
- **Migrations run on every replica start (code, O2)**; see `migration-policy.md` for the N-1
  compatibility rule that makes rolling deploys safe.
- All state of record is in Postgres: the **system DB** (accounts, projects, rate buckets,
  hashes) and each project's **customer DB** (documents). `embyr-server` never stores documents.

## 2. Load balancer

Two public listeners behind it (`:8080` gRPC, `:8081` gRPC-Web/REST/BrowserChannel); `:9090` is
never behind the public LB (`network-topology.md` §2).

- **Health checks: `GET :8081/healthz`** (readiness: checks system Postgres, 3 s timeout, 503
  `draining` during shutdown) **(code)**. For the `:8080` target group use a health check port
  override to `8081`; `/healthz` is plain HTTP and will not satisfy a gRPC health check.
  Use `/livez` (no I/O, never fails on a Postgres outage) only for orchestrator *liveness*, never
  for LB membership. See `runbook.md`.
- **gRPC on :8080 needs HTTP/2 to the backends.** A client holds one long-lived HTTP/2
  connection, so an L4 LB pins a client to a replica for the life of the connection and balances
  badly after a deploy (all clients reconnect to the first replicas up). An L7 LB with HTTP/2
  backends balances per request. **(decide)** L7+h2 vs L4.
- **Sticky routing is required for BrowserChannel only.** Native gRPC `Listen`/`Write` and
  gRPC-Web hold their state in the connection itself; an `Listen` on replica A sees changes made
  through replica B because change events travel through Postgres NOTIFY **(code)**.
- Request limits to leave consistent with the LB: 10 MiB gRPC messages, 2 MiB on the REST router
  **(code)**; keep LB body limits at least that large and idle timeouts above the BrowserChannel
  long-poll and gRPC keepalive intervals (**guess**: >= 120 s).

### BrowserChannel stickiness: the documented recipe does not work

`network-topology.md` §3 (and ADR-001) say to route on the `SID` query parameter, e.g. nginx
`hash $arg_SID consistent`. Reading the implementation shows why that cannot work with more than
one replica **(code, `rest/browser_channel.rs`)**:

- The first request (handshake) carries **no `SID`**. The replica that handles it generates a
  random UUID `SID`, stores the session in its own memory, and returns the `SID` in the body.
- Every later request carries that `SID`. A hash of the `SID` picks a replica unrelated to the
  one that created the session, so with N > 1 most follow-ups land on a replica that answers
  `400 Unknown SID`; the SDK reconnects and the cycle repeats.

Options **(decide)**, cheapest first:
1. **Route the BrowserChannel paths to a single dedicated replica.** Path-route
   `/google.firestore.v1.Firestore/Listen/channel` and `/google.firestore.v1.Firestore/Write/channel`
   to a one-replica target group. Trivial, no code change, a known SPOF and ceiling for web
   realtime; fine for a beta.
2. **LB-inserted cookie stickiness** (HAProxy `cookie ... insert`, Envoy stateful session, nginx
   `sticky`) on those two paths. Works only if the web SDK sends cookies on those requests
   (cross-origin XHR does not by default). Test with the real SDK before relying on it.
3. **Make the `SID` routable** (code change: prefix the `SID` with a per-replica tag, route on the
   prefix with `map`/Envoy `hash_policy`). The correct fix; not built.

Also note the `:9090`/native-gRPC paths are unaffected; this only concerns the browser fallback
transport (go-live finding P3, BrowserChannel, was in review when this was written).

## 3. Postgres

### System database

- **Managed, multi-AZ, with PITR** (RDS Multi-AZ / Cloud SQL HA / equivalent) **(decide)**. It
  is the single point of failure for the control plane and, via the per-request rate-limiter
  UPDATE and cache-miss lookups, for the data plane too **(code)**. `/healthz` goes 503 on every
  replica if it is unreachable. Targets, backup mechanism and the encryption-key precondition for
  restore are in `backup-disaster-recovery.md` (RPO <= 5 min and RTO <= 30 min are still
  aspirational there until a platform is chosen and a restore drill has been run).
- Size for connections first, CPU second (**guess**): small instance class with >= 8 GiB RAM, so
  the default `max_connections` is not the binding constraint.

### Pool sizing formula

Per-replica pool caps are env vars (ADR-079): `EMBYR_SYSTEM_DB_MAX_CONNECTIONS` (default 5),
`EMBYR_TENANT_DB_MAX_CONNECTIONS` (5), `EMBYR_LISTENER_DB_MAX_CONNECTIONS` (2), plus
`EMBYR_TENANT_DB_ACQUIRE_TIMEOUT_SECS` / `EMBYR_LISTENER_DB_ACQUIRE_TIMEOUT_SECS` (5 s). Pools
are lazy: the cap is a maximum, not a floor.

System DB (the embyr Postgres):

```
peak_replicas x EMBYR_SYSTEM_DB_MAX_CONNECTIONS + ops_headroom  <=  max_connections - 3
```

`peak_replicas` includes rolling-deploy surge (e.g. 3 replicas + `maxSurge` 3 = 6), and
`ops_headroom` covers migrations at start, `psql`, exporters and backup tooling (**guess**: 15).
Example: 6 x 10 + 15 = 75 <= 100 - 3. If you raise the cap, raise `max_connections` first.

Each customer Postgres (BYOC, one per project, or shared by several): for every replica that
serves that project **(code)**

```
connections_per_replica = EMBYR_TENANT_DB_MAX_CONNECTIONS            # request path
                        + EMBYR_LISTENER_DB_MAX_CONNECTIONS + 1      # only while a Listen is open:
                                                                     # fetch pool + dedicated LISTEN
customer_total = serving_replicas x connections_per_replica  (x up to 2 during pool rebuild)
```

With defaults that is 8 per replica per project, 24 for 3 replicas. The "x 2" is a **(guess)**:
the credential cache TTL is 60 s, and a refreshed entry builds a new tenant pool while in-flight
requests may still hold the old one, so connection churn and brief overlap are expected; the
`soak` profile shows it. The customer must size their `max_connections` accordingly: this is
a **customer-facing requirement** to document, and it is why a low-limit managed DB (e.g. a
serverless or free tier) can be exhausted by a single project.

Hosted-identity REST sign-in/sign-up on a cache hit builds a new Postgres pool to the customer DB
per request (go-live finding X11, not listed as fixed); treat it as extra connection and
handshake load until verified.

### PgBouncer and other connection poolers

**Do not put a transaction-mode pooler in the path in V1 (decide if one is forced on you).**
- **LISTEN/NOTIFY needs a session.** The Listen feature holds a dedicated `LISTEN` connection per
  project per replica **(code)**; transaction pooling silently drops notifications. Session
  pooling works.
- `sqlx` uses server-side prepared statements, and its migrator holds a session-level advisory
  lock, both of which misbehave under transaction pooling **(guess: confirm for the pinned
  sqlx version)**.
- The DSN a customer supplies is used for both the request path and LISTEN, so the *whole* DSN
  must be session-capable. If a pooler is needed for connection count, use **session mode**
  and size its server connections with the formulas above.
- RDS Proxy / similar poolers pin sessions on LISTEN and prepared statements, which defeats them.

## 4. TLS

Two valid shapes **(decide)**:
1. **Terminate at the LB**, plaintext to replicas inside the private network. Simplest;
   `network-topology.md` and `single-host-deployment.md` assume it. gRPC then needs the LB to
   speak HTTP/2 (h2c) to backends, so verify your LB supports that for your platform.
2. **Re-encrypt to replicas** with `EMBYR_TLS_CERT_PATH` / `EMBYR_TLS_KEY_PATH`: one cert/key
   pair shared by all three listeners **(code)**; needed if the LB must speak h2 over TLS to
   backends or compliance wants in-VPC encryption. The cert must cover whatever name the LB
   verifies and be rotated by restarting replicas.

Customer DB TLS is enforced via `sslmode` in the DSN (go-live X10 fixed);
`EMBYR_ALLOW_PRIVATE_DSN_HOSTS` is a dev-only flag and must be **unset** in production.

## 5. Observability wiring

- **Scrape every replica individually, not through the LB**: metrics are per process. The alert
  rules assume job `embyr-server`.

```yaml
scrape_configs:
  - job_name: embyr-server
    metrics_path: /metrics
    authorization:
      type: Bearer
      credentials_file: /etc/prometheus/secrets/embyr_admin_key   # = EMBYR_ADMIN_KEY
    # one target per replica, e.g. kubernetes_sd_configs (role: pod) or dns_sd_configs, port 9090
  - job_name: embyr-healthz            # blackbox, matches the commented EmbyrReadinessFailing rule
    metrics_path: /probe
    params: { module: [http_2xx] }
    static_configs: [ { targets: ["http://<replica-or-lb>:8081/healthz"] } ]
    # + the usual blackbox relabel_configs
  - job_name: embyr-admin-probe        # SLI 4 in slo.md: :9090/healthz and :9090/admin/
```

- Rotating `EMBYR_ADMIN_KEY` breaks scraping until Prometheus has the new token; use the
  `EMBYR_ADMIN_KEY_PREVIOUS` window (`secrets-manager-setup.md`) and update the scrape secret
  inside it.
- Apply [`alert-rules.yml`](./alert-rules.yml) and import
  [`grafana-dashboard.json`](./grafana-dashboard.json); add the burn-rate rules and the two
  blackbox jobs from `slo.md`. Wire alert delivery (go-live O6, still open).
- Logs are JSON on stdout; ship them from the platform. The API key can appear in the `?key=`
  query string on some clients (go-live X13): scrub it in LB access logs.

## 6. Client IP and trusted proxy hops

`EMBYR_TRUSTED_PROXY_HOPS` (default 0) controls how the **admin sign-in and invitation
throttles** (:9090) identify a client **(code, `signin_rate_limit.rs`)**: 0 keys on the TCP peer
IP, N keys on the Nth entry from the right of `X-Forwarded-For`.
- Behind an LB with 0, every admin shares the LB's IP and one attacker can lock everyone out
  (go-live X8). Set it to the number of proxies that **append** to `X-Forwarded-For` in front of
  :9090 (LB = 1; LB + ingress = 2).
- Too high lets a client spoof the header; too low reintroduces the shared bucket. The data-plane
  limiter is keyed by project, not by client IP, so this setting does not affect it.
- Because :9090 is private, in practice this is the VPN/bastion/internal-LB hop count.

## 7. Graceful shutdown and LB draining

What the process does on SIGTERM/Ctrl-C **(code, `main.rs`, `lib.rs`, `grpc/healthz.rs`)**:

1. `/healthz` flips to 503 `{"status":"draining"}` immediately.
2. **At the same instant** it stops accepting new connections on all three listeners and asks
   open HTTP/1 and HTTP/2 connections to finish (graceful shutdown).
3. It waits up to `EMBYR_SHUTDOWN_GRACE_SECS` (default 25) for in-flight requests; long-lived
   gRPC `Listen` streams and BrowserChannel sessions never finish on their own and are cut at the
   deadline or at exit. Clients reconnect (SDKs handle it).
4. It flushes buffered metric rollups and exits.

Consequence: because step 2 does not wait for the LB to notice step 1, a health-check-driven
deregistration sends new requests to a closed port for roughly
`health_check_interval x unhealthy_threshold` seconds. Close that gap on the platform side, so
the LB stops routing **before** SIGTERM:

| Setting | Recommended (guess) | Why |
|---|---|---|
| Kubernetes `preStop` | `sleep 15` (the runtime image is `debian:bookworm-slim`, `sleep` exists) | endpoint removal and LB deregistration propagate before SIGTERM |
| `terminationGracePeriodSeconds` | `>= preStop + EMBYR_SHUTDOWN_GRACE_SECS + 5` = 45 | otherwise SIGKILL cuts the drain |
| `EMBYR_SHUTDOWN_GRACE_SECS` | 25 (default) | leaves the margin above inside a 45 s window |
| Readiness probe | `GET :8081/healthz`, period 5 s, failure 2 | |
| Liveness probe | `GET :8081/livez`, period 10 s, failure 3 | never use `/healthz`: a DB outage would restart every pod |
| Startup probe | `GET :8081/livez`, period 5 s, failure 30 | migrations run before listeners open; do not let liveness kill a slow migration |
| ALB/NLB deregistration delay | 30 s | in-flight requests finish; keep < the grace window used above |
| LB health check | interval 5-10 s, unhealthy threshold 2 | |
| Rolling update | `maxUnavailable: 0`, `maxSurge: 1`, one replica at a time | migrations take an advisory lock and run on each start; see `migration-policy.md` |

A code-side improvement would remove the platform workaround: delay step 2 by a configurable
`EMBYR_DRAIN_DELAY_SECS` after step 1. Not built.

## 8. Secrets and configuration

- Source `EMBYR_ADMIN_KEY` / `EMBYR_ENCRYPTION_KEY` from AWS/GCP Secrets Manager
  (`secrets-manager-setup.md`); never in the image or plain env on the platform's config page.
  The encryption key is also a **restore precondition** (`backup-disaster-recovery.md`):
  losing it makes every stored customer DSN unrecoverable.
- Other required production configuration: `DATABASE_URL`, `EMBYR_REQUIRE_SMTP=true` + SMTP URL,
  `EMBYR_PUBLIC_BASE_URL`, Stripe keys (`STRIPE_*`), `EMBYR_CORS_ALLOWED_ORIGINS` for the console
  origin; the full list is in the repo-root `README.md` / `.env.example`.
- Same image for every replica; tag by immutable version/SHA (`release-process.md`; go-live O1
  covers the missing first git tag).

## 9. Sizing starting point (all guess, validate with the load test)

Per replica: **2 vCPU / 2 GiB** (`resource-sizing.md` "moderate" tier, limit side), 3 replicas.

| Item | Starting value | Reasoning |
|---|---|---|
| Replicas | 3, one per AZ | lose one and keep 2/3 capacity |
| `EMBYR_SYSTEM_DB_MAX_CONNECTIONS` | 10 | every authenticated request does one rate-limiter UPDATE here; default 5 is likely the first ceiling |
| `EMBYR_TENANT_DB_MAX_CONNECTIONS` | 10 | |
| `EMBYR_LISTENER_DB_MAX_CONNECTIONS` | 2 (default) | |
| `EMBYR_RATE_LIMIT_RPS` | per product tier | per **project**, default 1000 |
| System DB | managed Multi-AZ, >= 2 vCPU / 8 GiB, `max_connections` >= 100 | |
| `EMBYR_SHUTDOWN_GRACE_SECS` | 25 | see section 7 |

**CPU is dominated by Argon2, and it scales with active API keys, not request rate** (code:
credential cache TTL 60 s, `adapters/credential_cache.rs`): every key that is in use costs one
verification per minute per replica, plus every cold start/deploy/cache eviction.

```
argon2_cpu_cores_per_replica ~= active_keys_seen_by_replica / 60  x  seconds_of_cpu_per_verify
peak argon2 memory           =  cores x 64 MiB            (gate = one permit per core)
```

`seconds_of_cpu_per_verify` for `m=64 MiB, t=3, p=4` is not measured here. Measure it on the
target instance type (the bad-key scenario's CPU and the `ramp` profile give it), then size
vCPU. Example with a **hypothetical** 0.1 s: 1 000 active keys over 3 replicas ~ 333 each ~
5.5 verifies/s ~ 0.55 core per replica, before any request work.

## 10. Launch checklist: what the owner must decide or do

Decisions:
1. Hosting platform and region(s); AZ count. (Everything above is platform-neutral.)
2. Managed Postgres product, HA tier, instance class, PITR retention; ratify RPO/RTO in
   `backup-disaster-recovery.md`.
3. LB product; gRPC mode (L7+h2 vs L4); where TLS terminates; certificate issuance and renewal;
   public DNS names (`network-topology.md` §1).
4. **BrowserChannel at launch:** support web SDK realtime fallback or not; if yes, which of the
   three stickiness options in section 2 (option 3 needs a code change).
5. Whether to run a connection pooler (default: no).
6. Secrets store (AWS vs GCP) and who can read it; admin-key rotation owner.
7. SMTP provider; `EMBYR_PUBLIC_BASE_URL`; Stripe live keys and tax code (go-live "needs the owner").
8. Image registry visibility and the first git tag (O1).
9. Alert delivery target and on-call (O6); ratify `slo.md` numbers after the load test.
10. Per-tier `EMBYR_RATE_LIMIT_RPS` and the maximum projects per customer DB.
11. `EMBYR_TRUSTED_PROXY_HOPS` value for the admin path (section 6).
12. What you tell BYOC customers about their DB: `max_connections` headroom (section 3), TLS,
    network allow-list for the embyr egress IPs.

Do before go-live:
- [ ] Run `loadtest/` `ramp` and `soak` against this topology; record results in `slo.md`/this file.
- [ ] Run the bad-key scenario; confirm the Argon2 gate holds and `/healthz` stays 200.
- [ ] Roll a deploy under load; confirm no errors with the `preStop` + grace settings.
- [ ] Kill one replica and one AZ's worth of replicas under load.
- [ ] Restore the system DB from PITR into a scratch instance (`backup-disaster-recovery.md` drill).
- [ ] `EMBYR_ALLOW_PRIVATE_DSN_HOSTS` unset, `:9090` unreachable from the internet (test it).
- [ ] Alerts firing to a real channel (test `EmbyrTargetDown` by stopping a replica).
