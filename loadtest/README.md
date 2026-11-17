# embyr-server load test (k6)

One k6 script, [`embyr.js`](./embyr.js), drives the Firestore data plane on **:8081 over
gRPC-Web** with no SDK. It exists to answer go-live finding O5 ("no load test, pool/resource
sizing unvalidated"). Thresholds come from [`docs/operations/slo.md`](../docs/operations/slo.md);
sizing assumptions to confirm are in [`docs/operations/topology.md`](../docs/operations/topology.md).

**This was written without being run** (no k6, Docker or server available when it was authored).
The protobuf bytes were checked by hand and the script parses under Node, but expect to fix
small things on the first real run.

## What it does and does not cover

:8081 has no JSON/REST documents API. It serves Firestore as gRPC-Web (binary protobuf,
`content-type: application/grpc-web+proto`, `authorization: Bearer <api key>`), so the script
hand-encodes the handful of messages it needs. Mapping from the usual REST verbs:

| Scenario | RPC | Share of `TARGET_RPS` |
|---|---|---|
| `get` | `GetDocument` | 60 % |
| `commit` | `Commit` (one upsert; keyspace `WRITE_KEYSPACE`) | 20 % |
| `update` | `UpdateDocument` with an update mask (the REST `PATCH` equivalent) | 10 % |
| `query` | `RunQuery`, single-field `orderBy n` ascending + `limit` (no filter, so no composite index) | 10 % |
| `badkeys` (opt-in) | `GetDocument` with many distinct wrong keys, see below | `BAD_KEY_RPS` |

Not covered: `Listen`/`Write` streams (Listen delivery latency, SLI 3, must be measured with a
real client), BrowserChannel, the hosted-identity REST endpoints (`accounts:signIn*`), the admin
API, native gRPC on :8080 (same handlers; use `ghz` if you want that transport), `backend_mode`
other than `direct_pg`, and multi-project fan-out (see "Limits of this test").

## Prerequisites

- [k6](https://grafana.com/docs/k6/latest/set-up/install-k6/) (`brew install k6`), or
  `docker run --rm -i grafana/k6 run - <loadtest/embyr.js` with `-e VAR=...` flags
  (use `host.docker.internal` instead of `localhost` on macOS).
- A project to hit: either give `API_KEY` for an existing one, or let `setup()` provision it
  (needs `ADMIN_KEY` and `CUSTOMER_DSN`; it calls `POST :9090/admin/v1/projects`, the same
  endpoint the admin console uses). Then it seeds `SEED_DOCS` documents (`d-0` ... `d-N`,
  integer field `n`) with batched Commits.

## Run against docker compose

```bash
# 1. stack, with the load-test overrides (lifts the per-project limiter; see below)
docker compose -f docker-compose.yml -f loadtest/docker-compose.loadtest.yml up -d --build

# 2. a database for the tenant (customer) side; compose's Postgres is reused for it
docker compose exec postgres psql -U postgres -c 'CREATE DATABASE loadtest_tenant'

# 3. smoke run: provisions the project, seeds, ~30 s at a few rps
k6 run \
  -e BASE_URL=http://localhost:8081 -e ADMIN_URL=http://localhost:9090 \
  -e PROJECT_ID=loadtest -e PRINT_API_KEY=1 \
  -e ADMIN_KEY=dev-only-admin-key-do-not-use-in-production \
  -e CUSTOMER_DSN=postgres://postgres:postgres@postgres:5432/loadtest_tenant \
  loadtest/embyr.js
# The project's api_key is returned once. PRINT_API_KEY=1 logs it so you can pass API_KEY below;
# without it, a second run with the same PROJECT_ID fails with 409 (project exists).

# 4. real runs (project already exists: pass its key, setup() only re-seeds)
k6 run -e API_KEY=... -e PROFILE=ramp -e TARGET_RPS=300 loadtest/embyr.js
k6 run -e API_KEY=... -e PROFILE=soak -e TARGET_RPS=300 -e SOAK_DURATION=2h loadtest/embyr.js
k6 run -e API_KEY=... -e SCENARIOS=badkeys -e BAD_KEY_RPS=50 loadtest/embyr.js

# cleanup
docker compose down -v   # also removes the Postgres volume
```

The compose stack is for **correctness and shape**, not capacity: k6, server and Postgres share
one laptop. Capacity numbers need the production-like topology from `topology.md`, with k6 on a
separate host in the same region as the load balancer.

## Environment variables

| Var | Default | Meaning |
|---|---|---|
| `BASE_URL` | `http://localhost:8081` | gRPC-Web listener (or the LB in front of it) |
| `PROJECT_ID` | `loadtest` | must match `^[a-z][a-z0-9-]{0,62}$` |
| `API_KEY` | - | project API key; if unset, `setup()` provisions the project |
| `ADMIN_URL`, `ADMIN_KEY`, `CUSTOMER_DSN` | `http://localhost:9090`, -, - | only for provisioning in `setup()` |
| `PROFILE` | `smoke` | `smoke` (40 s), `ramp`, `soak` |
| `TARGET_RPS` | `200` | total arrival rate across the data scenarios at the top of the ramp |
| `SOAK_DURATION` | `30m` | hold time of the `soak` profile |
| `SCENARIOS` | `data` | comma list of `data`, `badkeys` |
| `SEED_DOCS` | `500` | documents seeded and read/queried |
| `WRITE_KEYSPACE` | `10000` | distinct document ids `Commit` writes (bounds table growth) |
| `PAYLOAD_BYTES` | `256` | size of the `payload` string field |
| `COLLECTION`, `QUERY_LIMIT` | `loadtest`, `20` | |
| `BAD_KEY_RPS`, `BAD_KEY_DURATION_S` | `20`, `120` | bad-key scenario |
| `REPLICAS` | `1` | replicas behind `BASE_URL`; scales the Argon2 bound in the bad-key threshold |
| `P99_GET_MS`, `P99_COMMIT_MS`, `P99_QUERY_MS`, `MAX_ERROR_RATE` | `250`, `500`, `500`, `0.001` | thresholds, from the SLO doc |

Profiles:
- `smoke`: ~10 s ramp + 30 s at `min(TARGET_RPS, 20)`; checks the wiring and the encoding.
- `ramp`: 25 % -> 50 % -> 75 % -> 100 % of `TARGET_RPS` (1/2/2/2 min), 5 min hold, ramp down.
  Use it to find where p99 or the fault rate bends.
- `soak`: 2 min up, `SOAK_DURATION` at 100 %, 1 min down. Use it to find leaks, pool churn from
  the 60 s credential TTL, sweeper interference, and Postgres bloat.

## Thresholds (a failed run exits non-zero)

- `grpc_server_fault` rate < `MAX_ERROR_RATE`: fault = gRPC `unknown|deadline_exceeded|internal|
  unavailable|data_loss`, or any non-200 / missing trailer (an LB 502/503 counts). Same definition
  as SLI 1.
- p99 of `http_req_duration` per `rpc` tag: GetDocument, Commit, UpdateDocument, RunQuery.
  These are client-side (include network); the server-side histogram is the SLI of record.
- `grpc_rate_limited` count == 0: any `resource_exhausted` on the data scenarios means you
  measured the per-project limiter instead of the server. The default limit is
  `EMBYR_RATE_LIMIT_RPS=1000` per project (the compose override raises it to 100000).

## The bad-key scenario (Argon2 gate)

Why it matters: every request that misses the credential cache pays an Argon2id verification
(64 MiB, CPU-bound, process-wide gate of one permit per core). The cache TTL is **60 s**
(`CREDENTIAL_TTL` in `adapters/credential_cache.rs`), so every active key costs one verification
per minute per replica, and anything that empties the cache (restart, deploy, a cold replica)
turns into an auth-miss storm.

Defence, as coded: for a *real* project id, keys the process has never verified draw on a
per-project, per-process failed-authentication budget (burst 10, refill 1/s). Beyond it the
request is answered `resource_exhausted` ("too many failed authentication attempts") **before**
Argon2 runs. `badkeys` fires `BAD_KEY_RPS` requests with a different wrong key each, for
`BAD_KEY_DURATION_S`, and asserts:

- `badkey_unauthenticated` (Argon2 actually ran) <= `REPLICAS * (10 + duration) * 1.2 + 10`
- `badkey_resource_exhausted` > 0 (the gate engaged)
- `badkey_unexpected` == 0 (nothing but those two statuses)
- `legit_during_flood_ok` > 0.99: a legitimate key this process already verified keeps working
  at 5 rps during the flood. Run `SCENARIOS=data` (or the smoke run) first so the key is cached on
  every replica; with N replicas behind an LB, make sure each has seen the key.

Watch while it runs: CPU of the replicas (should stay low; the point is that bad keys do *not*
saturate cores), memory (Argon2 peak = permits x 64 MiB), `/healthz` still 200, data-plane p99.
Known limit, by design in the code: while the flood holds the budget at zero, a **new** legitimate
key on that project (first use after a restart) is also refused. That is a finding to weigh, not
a test failure.

Manual follow-up for the cold-cache case: restart (or roll) one replica during a `soak` and look
at p99 and `embyr_pg_pool_*` for the minute after.

## What to look at while it runs

All from `GET :9090/metrics` (bearer `EMBYR_ADMIN_KEY`), or the checked-in dashboard
([`grafana-dashboard.json`](../docs/operations/grafana-dashboard.json)).

| Question | Look at |
|---|---|
| Server-side latency/error truth | `embyr_grpc_request_duration_seconds` per `method`, `embyr_grpc_requests_total{status}` |
| System pool saturated? | `embyr_pg_pool_idle{pool="system"}` hitting 0, `embyr_pg_pool_size{pool="system"}` at `EMBYR_SYSTEM_DB_MAX_CONNECTIONS`. Every authenticated request does a rate-limiter UPDATE on this pool. |
| Rate limiter degrading | `rate(embyr_rate_limit_pg_timeout_total[1m])` (60 ms budget), `embyr_rate_limit_pg_error_total`, `embyr_rate_limit_requests_total{outcome}` |
| Tenant pool saturated? | **no metric.** Symptom: `internal` errors and p99 jumping to ~`EMBYR_TENANT_DB_ACQUIRE_TIMEOUT_SECS` (5 s). Confirm with `pg_stat_activity` on the tenant DB. |
| Postgres connections | `select datname, count(*) from pg_stat_activity group by 1` on both system and tenant DBs; compare with the formula in `topology.md` |
| Argon2 / CPU | container CPU and RSS; bad-key scenario above |
| 60 s TTL churn | a sawtooth every minute in latency or connection counts means the credential/pool rebuild is visible |
| Hot-row limiter | all requests of a project update ONE `rate_buckets` row, so one project serialises on that row lock. If `embyr_rate_limit_pg_timeout_total` rises before CPU does, that is why. (Hypothesis from the code, not yet observed.) |

## Limits of this test

- One project, one API key, one customer database. Real load is many projects across many
  customer databases; per-project limiter, credential cache and tenant pools behave differently
  at fan-out. After this works, parametrise `PROJECT_ID`/`API_KEY` per VU (the script is
  structured so that is a small change) and test 50-500 projects.
- Latency is client-observed. Read server histograms for the SLO.
- Data volume is tiny (`SEED_DOCS`). Query and index behaviour on millions of rows is untested.
- `Commit` here is a one-write upsert; no transactions (`BeginTransaction`), no `BatchGet`.

## Record these after every run

Date, git SHA / image tag, topology (replicas, vCPU/RAM, pool env vars, Postgres size and
`max_connections`), `TARGET_RPS`, the k6 summary, the server-side p99 per method, peak
`embyr_pg_pool_*`, peak Postgres connections, and the first threshold that failed. Paste them
into the "Calibrating" section of `slo.md` and the sizing table of `topology.md`.
