# Feature Delta: wire-secret-fetchers

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/production-readiness-audit-2026-09-08.md` — read (finding #7 row + surrounding
findings #6-8). Exact wording confirmed: "`aws_secret`/`gcp_secret` backend modes are hardcoded to
`None` fetchers in the real composition root — every request for a project in that mode returns
`Status::internal`. Admin provisioning API still allows creating such projects. Pre-existing, named
in ADR-054 §D7, but still blocks launch for that backend mode." Location cited:
`crates/embyr-server/src/main.rs:164-165,239-246`; `crates/embyr-server/src/grpc/handler.rs:268-271,
286-289`. Severity: **Blocker**. Status: "Not started."

✓ `crates/embyr-server/src/main.rs` read around lines 130-260. Confirmed exactly as cited, plus **one
additional hardcoded-`None` call site the audit finding does not name**: `FirestoreService`
construction (lines 156-167) hardcodes `aws_secret_fetcher: None, gcp_secret_fetcher: None` (164-165);
`build_admin_router(...)` (lines 228-243) passes `None, None` for the same two positional params
(236-237); and `embyr_server::sweepers::transaction_sweeper::spawn(...)` (lines 272-280) **also**
hardcodes `None, None` (274-275) for its own `aws_secret_fetcher`/`gcp_secret_fetcher` params — a
third call site sharing the identical root gap, explicitly documented (not newly discovered by this
DISCUSS) in ADR-054 §D7 (see below). A fix touching only the 2 call sites the audit finding names
would leave the transaction sweeper (JOB-12, `customer-db-transaction-sweeper`) silently skipping
every `aws_secret`/`gcp_secret` project forever — an incomplete correction of the same wiring gap.

✓ `crates/embyr-server/src/grpc/handler.rs` read around lines 240-310. Confirmed exactly as cited:
the `aws_secret` branch (263-280) returns `Status::internal("aws_secret_fetcher not configured on
this server")` when `self.aws_secret_fetcher` is `None` (268-271); the `gcp_secret` branch (281-298)
returns the GCP equivalent (286-289). Both branches otherwise call `fetcher.get_dsn(&arn_or_resource)`
and build a fresh `PostgresBackendAdapter::new(&dsn)` per request when a fetcher IS present — this
per-request adapter-construction shape is pre-existing and unchanged by this feature (see § Out of
Scope).

✓ `crates/embyr-server/src/admin/handlers/provision.rs` read in full (lines 1-310+). **Correction to
the audit finding's own wording**: the admin provisioning endpoint (`POST /admin/v1/...`) does
**not**, in fact, "still allow creating such projects" unconditionally — it already gates on the
identical fetcher-presence check: `state.aws_secret_fetcher.as_ref().ok_or_else(|| err(...,
"aws_secret_fetcher_not_configured"))?` (line ~205-208) and the GCP equivalent (line ~257-260), BEFORE
any DB write. With today's hardcoded `None, None`, provisioning an `aws_secret`/`gcp_secret` project
via the admin API **already fails immediately** (500, `aws_secret_fetcher_not_configured` /
`gcp_secret_fetcher_not_configured`) rather than silently succeeding and failing later on live
traffic. This is a materially better starting position than the audit's wording implies — no
"zombie" projects can be created via this path today — but it means the SAME wiring gap blocks BOTH
provisioning and live traffic identically; fixing composition-root construction fixes both at once.
Also confirmed: `secret_arn`/`gcp_resource_name` are already required, non-empty request fields for
these two backend modes (`BAD_REQUEST` `secret_arn_required` / `gcp_resource_name_required` if
absent) — the natural signal DESIGN can reuse, as the task anticipated.

✓ `crates/embyr-server/src/adapters/aws_secret_fetcher.rs` and `.../gcp_secret_fetcher.rs` read in
full. Confirmed the task's framing: both are real, complete, already-used adapters (`get_dsn`,
`fetch_fresh`, `get_raw_secret`, TTL-based per-key caching). Load-bearing asymmetry found by direct
reading (relevant to the missing-credentials investigation below): `AwsSecretFetcher::new(&aws_config,
ttl)` takes an `aws_config::SdkConfig` built via `aws_config::load_defaults(BehaviorVersion::latest())`
— this call is **infallible and lazy**; it never fails synchronously for missing credentials, it
returns a config that resolves credentials lazily on the FIRST real API call. `GcpSecretFetcher::new(
base_url, token, ttl)`, by contrast, takes the bearer token **synchronously, as a plain `&str`** — it
cannot be constructed at all without already having a token in hand.

✓ `crates/embyr-server/src/config.rs` read around lines 460-531 (`fetch_from_secret_manager`,
`require_gcp_access_token`). Confirmed the exact, already-established construction pattern for both
fetchers (used today only for one-shot `EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY` secrets-manager
sourcing, a *different* purpose per the task's own framing): `GcpSecretFetcher::new(
GCP_SECRET_MANAGER_BASE_URL, &token, ttl)` where `token` comes from `require_gcp_access_token`, which
reads `EMBYR_GCP_ACCESS_TOKEN` and errors only when a `*_GCP_SECRET_NAME` source var is explicitly
set elsewhere; `AwsSecretFetcher::new(&aws_config, ttl).await` where `aws_config =
aws_config::load_defaults(...).await`.

✓ `crates/embyr-server/src/lib.rs` read around lines 589-816. **Major finding**: `start_test_server_
with_aws_fetcher` (line 712) and `start_test_server_with_gcp_fetcher` (line 765) already exist as
test-harness composition helpers that build a REAL `FirestoreService` with `aws_secret_fetcher:
Some(Arc::clone(&aws_fetcher))` / `gcp_secret_fetcher: Some(Arc::clone(&gcp_fetcher))`, and pass the
same fetcher into `admin::router::build_with_aws(...)` / `build_with_gcp(...)`. This is the exact
wiring `main.rs` needs to do for production — proven correct today, just never done outside tests.

✓ `tests/acceptance/us_10_aws_secrets.rs` and `tests/acceptance/us_11_gcp_secrets.rs` read (headers +
test list). Both marked `SCAFFOLD: false` (real, not stub). US-10's own doc comment: *"As Morgan
(Tenant Admin), I want to register a project with backend_mode=aws_secret and an ARN, so that embyr
fetches the DSN from my existing secret store and never stores it."* — this is JOB-05's job story
almost verbatim (see § Persona & Job). Both files exercise the full mechanism end-to-end against real
infrastructure: US-10 uses a real LocalStack Secrets Manager container + real Postgres container,
covering provisioning success, `no_iam_access_returns_backend_secret_fetch_failed`, malformed-secret
rejection, and TTL-cache rotation transparency. US-11 uses an in-process mock GCP Secret Manager HTTP
server. **None of these test functions are `#[ignore]`d.** Together they prove the fetcher mechanism
itself is correct and already covered — the gap this feature closes is purely that `main.rs` (the
real production composition root) never constructs and threads real fetcher instances the way these
test helpers already do.

✓ `docs/product/architecture/adr-054-transaction-sweeper-raw-access-and-sweep-sql.md` § D7 read in
full (lines 211-248). Confirms the audit's own citation ("named in ADR-054 §D7") and independently
documents the exact same gap, including the third call site (`TransactionSweeper::spawn`) this
DISCUSS's own reading also found, and the exact proposed construction shape: *"constructed fresh in
main.rs using the identical pattern already used in config.rs::fetch_from_secret_manager"*, plus the
`EMBYR_GCP_ACCESS_TOKEN`-reuse note.

✓ `docs/feature/composite-index-real-creation/feature-delta.md` § Follow-Up Work and
`docs/evolution/2026-09-09-composite-index-real-creation.md` § Follow-Up Work both read. Confirmed:
*"`resolve_aws_secret_dsn` has zero test coverage anywhere in the codebase (pre-existing, not
introduced by this feature) — a real candidate for a future aws_secret-mode-specific feature"* and
*"hardcoded-None aws/gcp secret fetchers (finding #7, likely related to the aws_secret gap above)."*
`crates/embyr-server/src/adapters/customer_db_connect.rs::resolve_aws_secret_dsn` (line ~60) is a
**different call path** from `handler.rs`'s per-request dispatch (it's used by the transaction
sweeper's own DSN resolution, per ADR-054) — but the connection the sibling features guessed at is
confirmed: nothing in production ever constructs a real `aws_secret_fetcher`, so neither this
function nor `handler.rs`'s per-request path has ever been exercised outside tests. This feature's
own walking-skeleton scenarios (below) exercise `handler.rs`'s path directly; whether they also close
`resolve_aws_secret_dsn`'s coverage gap as a side effect is DESIGN/DISTILL's call (see § Out of
Scope).

✓ `docs/product/jobs.yaml` read in full (JOB-01 through JOB-16+). **JOB-05 `cloud-secret`** (persona
P3) is the exact fit — see § Persona & Job.

✓ `docs/product/journeys/tenant-admin.yaml` and `docs/product/architecture/c4-diagrams-admin-ui.md`
read — confirm persona P3's name: **Morgan (Tenant Admin / DevOps Lead)**.

✓ `docs/feature/rate-limiter-project-id-validation/feature-delta.md` read in full to confirm this
project's established single-file `feature-delta.md` convention (`## Wave: DISCUSS / [REF] {Section}`
heading format, story template, DoR checklist shape). This DISCUSS mirrors that structure directly.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Reliability fix — composition-root wiring gap** ("wire up an already-built
  mechanism that was simply never connected"), not a missing-feature or security-hardening fix. Same
  shape class as `stripe-webhook-secret-required`'s own composition-root gap, but that feature was a
  fail-fast config-validation gap (JOB-13); this one is a functional-completeness gap for an
  already-documented job (JOB-05) — see § Persona & Job for why JOB-05, not JOB-13, is the fit.
- JTBD: **reuse JOB-05** (`cloud-secret`, P3 Morgan) — confirmed independently against jobs.yaml, not
  re-litigated further; see § Persona & Job for the confirming evidence.
- Decision 4 (full JTBD path vs. infrastructure-only): **Yes — full JTBD path**. This directly
  restores a real, user-invocable outcome (a working `aws_secret`/`gcp_secret`-mode project) that
  Morgan already expects per JOB-05's own functional dimension — not infrastructure scaffolding with
  no observable behavior change.
- Missing-credentials investigation: **required by this DISCUSS, mechanism choice left to DESIGN** —
  see § Business Context for the investigation result and the concrete asymmetry this DISCUSS's own
  reading surfaced (AWS fetcher construction is unconditional/lazy; GCP fetcher construction requires
  gating on `EMBYR_GCP_ACCESS_TOKEN`).
- No global startup fail-fast for missing AWS/GCP credentials — confirmed against this session's own
  established narrow-fail-fast precedent (`stripe-webhook-secret-required`) and directly verified
  against `provision.rs`'s existing per-request/per-provision-call gate (see § Reading Confirmation):
  the fetcher-presence check already happens at the point of USE (a provisioning call or a live
  request for that specific backend mode), never at global startup, and this feature preserves that
  shape.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P3 Morgan (Tenant Admin / DevOps Lead)** — confirmed via
`docs/product/journeys/tenant-admin.yaml` (`persona: P3`, `persona_name: Morgan (Tenant Admin / DevOps
Lead)`) and `docs/product/architecture/c4-diagrams-admin-ui.md` line 16 (*"Tenant Admin (P3/Morgan):
Configures backend connection and agent mode secrets via the Connections panel"*).

**Job**: **JOB-05 `cloud-secret`**, reused, made real (not new). JOB-05's job story: *"When my
database credentials are already managed in AWS/GCP secret infrastructure, I want embyr to fetch them
from there, so I don't have to operate yet another credential store."* Functional dimension, verbatim:
*"Provide secret ARN; embyr fetches DSN; password rotation is transparent."* This is an exact match,
not an approximation — confirmed further by `tests/acceptance/us_10_aws_secrets.rs`'s own doc comment
(*"As Morgan (Tenant Admin), I want to register a project with backend_mode=aws_secret and an ARN, so
that embyr fetches the DSN from my existing secret store and never stores it"*), which is JOB-05's job
story almost word-for-word. This confirms US-10/US-11 were JOB-05's own original walking-skeleton
stories — proving the mechanism in isolation via test-only composition — and this feature is the
"make it real" closure for the live production composition root, the same repeated pattern this
session has used for other JOB-05/JOB-01/JOB-12 gaps (e.g. `firestore-composite-indexes-admin-api` →
JOB-01, `customer-db-transaction-sweeper` → JOB-12).

**Candidate considered and rejected**: **JOB-13 `production-deployment`** (P2 Sam Chen) — the job
`stripe-webhook-secret-required` (finding #1, same audit) reused, for a fail-fast
missing-required-env-var startup shape (*"server exits non-zero with named missing var"*). Rejected
here: this feature adds **no new required env var** and must explicitly NOT fail startup when
AWS/GCP credentials are absent (per the orchestrator's own narrow-fail-fast reasoning, confirmed
against `provision.rs`'s existing per-call gate — see § Business Context). JOB-13's own shape does not
fit; JOB-05's "provide secret ARN, embyr fetches DSN" functional promise fits exactly, and the persona
(Morgan, the tenant admin who owns backend-connection configuration per the C4 diagram) is the correct
one, not Sam Chen (service operator).

## Wave: DISCUSS / [REF] Business Context

Today, `crates/embyr-server/src/main.rs` hardcodes `aws_secret_fetcher: None, gcp_secret_fetcher:
None` at **three** composition-root call sites — `FirestoreService` construction (lines 164-165),
`build_admin_router(...)` (lines 236-237), and `transaction_sweeper::spawn(...)` (lines 274-275) —
regardless of what AWS/GCP credentials are actually present in the deployment's environment. The
result, confirmed by direct reading:

- **Provisioning** (`POST /admin/v1/...` via `provision.rs`): creating a project with
  `backend_mode=aws_secret` or `gcp_secret` fails immediately with `500
  aws_secret_fetcher_not_configured` / `gcp_secret_fetcher_not_configured` — always, regardless of
  environment (this is a correction to the audit finding's own wording; see § Reading Confirmation).
- **Live gRPC traffic** (`handler.rs`): any request for an already-provisioned project in either mode
  returns `Status::internal("aws_secret_fetcher not configured on this server")` / the GCP
  equivalent — always.
- **The transaction sweeper** (ADR-054, JOB-12): silently skips every `aws_secret`/`gcp_secret`
  project on its own continue-on-error path — a gap the audit finding does not name but ADR-054 §D7
  already documents explicitly.

`AwsSecretFetcher` and `GcpSecretFetcher` are real, complete, already-tested adapters — proven
end-to-end via `tests/acceptance/us_10_aws_secrets.rs` / `us_11_gcp_secrets.rs` (real, non-`#[ignore]`
tests against a real LocalStack container / an in-process mock GCP server) using test-only
composition helpers (`start_test_server_with_aws_fetcher`, `start_test_server_with_gcp_fetcher` in
`lib.rs`) that already construct real `Arc<AwsSecretFetcher>`/`Arc<GcpSecretFetcher>` instances and
thread them into `FirestoreService` and the admin router exactly the way `main.rs` needs to for
production. The gap is narrowly: `main.rs` itself never does this.

### Missing-credentials investigation (task-required)

The task asked this DISCUSS to investigate — not necessarily resolve — what SHOULD happen when a
cloud's credentials are absent. Direct reading of the two adapters' own constructors (see § Reading
Confirmation) surfaces a concrete, load-bearing asymmetry between AWS and GCP that any DESIGN
mechanism will need to account for:

- **AWS**: `AwsSecretFetcher::new(&aws_config, ttl)` takes an `aws_config::SdkConfig` from
  `aws_config::load_defaults(BehaviorVersion::latest())` — this call is infallible and lazy; it never
  fails synchronously for missing credentials. Credential resolution (IAM role, env vars, shared
  config file, etc. — the standard AWS SDK credential chain) happens lazily on the FIRST real
  `GetSecretValue` call. This means an `AwsSecretFetcher` can be constructed **unconditionally**,
  with zero I/O and zero risk of blocking or failing startup, regardless of whether real AWS
  credentials exist in the environment.
- **GCP**: `GcpSecretFetcher::new(base_url, token, ttl)` takes the bearer token **synchronously, as a
  plain string** — it cannot be constructed at all without a token already in hand. Construction must
  be conditional on `EMBYR_GCP_ACCESS_TOKEN` being present and non-empty (mirroring `config.rs`'s own
  `require_gcp_access_token` pattern), leaving `gcp_secret_fetcher` genuinely `None` on deployments
  that never set that variable.

This confirms the orchestrator's option (a) framing is the right shape for GCP (construct only when
credentials are present, leave `None` otherwise) but reveals AWS doesn't need the same conditional at
all — an `AwsSecretFetcher` can always be constructed, and a deployment with no real AWS credentials
naturally fails closed later, at the point of an actual `GetSecretValue` call, via the SAME
already-implemented error path `provision.rs`/`handler.rs` already handle (`AwsSecretError::Sdk`/
`AccessDenied` → `backend_secret_fetch_failed` / `Status::internal`). No new error-handling code is
required for that failure mode — it is a natural consequence of always constructing the fetcher.

This DISCUSS locks the **required outcome**, not the mechanism (DESIGN's call, consistent with this
session's established convention): whichever cloud's credentials are genuinely available in the
environment must make that backend mode actually work, end-to-end, for both provisioning and live
traffic; a cloud with no credentials configured must continue to fail closed (never silently succeed
with a wrong/stale DSN, never panic) with the SAME overall behavior class as today (a clear,
actionable failure) — not a new, undocumented failure mode. Server startup must never hard-fail
globally over an unconfigured cloud credential, matching this session's own narrow-fail-fast
precedent and directly confirmed against `provision.rs`'s existing per-call (not per-startup) gate.

Also investigated per the task: does the admin provisioning API already require an ARN/resource-name
to be supplied for these modes? **Yes, confirmed** (`secret_arn_required` / `gcp_resource_name_required`
`BAD_REQUEST` rejections already exist in `provision.rs` — see § Reading Confirmation) — this is
unchanged by this feature and remains the natural per-project signal DESIGN can build on.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (1). >3 bounded contexts/modules? No — confined to
`embyr-server`'s own composition root (`main.rs`) and the two adapters it already imports
(`AwsSecretFetcher`, `GcpSecretFetcher`) — zero new domain type, zero new adapter trait, zero new
bounded context; `FirestoreService`, `build_admin_router`, and `transaction_sweeper::spawn` all
**already accept** `Option<Arc<AwsSecretFetcher>>`/`Option<Arc<GcpSecretFetcher>>` parameters — this
is parameter-passing, not new architecture. Walking skeleton >5 integration points? No — one real
gRPC request against a real LocalStack-backed `aws_secret` project, one real gRPC request against a
mock-GCP-backed `gcp_secret` project, one real admin provisioning call per cloud, one server-startup
check with neither cloud configured. Estimated effort >2 weeks? No — the mechanism is already proven
correct by existing, real, non-`#[ignore]`d tests (US-10/US-11) using test-only composition helpers
that already exist in `lib.rs`; `main.rs` needs to replicate that same construction using
`config.rs`'s own already-established pattern. Multiple independent user outcomes? No — "the backend
mode Morgan selected during provisioning is the mode that actually serves traffic" is a single
outcome; AWS and GCP are two instances of the identical outcome via the same wiring change, not
separate outcomes (mirrors this session's own precedent of not splitting stories that share one
mechanism, e.g. `firestore-equal-notequal-value-type-support`'s multi-type single story).

**Scope Assessment: PASS** — 1 user story, 1 bounded context (`embyr-server` composition root),
estimated ≤2 days, 6 UAT scenarios (within the 3-7 right-sized range).

## Wave: DISCUSS / [REF] System Constraints

- Zero behavior change for `backend_mode=direct_pg` or `backend_mode=agent` projects.
- No global startup fail-fast when AWS/GCP credentials are absent — confirmed against this session's
  own narrow-fail-fast precedent and `provision.rs`'s existing per-call gate (§ Business Context).
- The exact construction/gating mechanism (AWS unconditional vs. GCP gated on
  `EMBYR_GCP_ACCESS_TOKEN`) is DESIGN's call to lock; this DISCUSS names the asymmetry it found by
  direct reading as DESIGN's starting evidence, not a decided mechanism.
- The SAME constructed fetcher instances must reach all **three** known hardcoded-`None` call sites in
  `main.rs` (`FirestoreService`, `build_admin_router`, `transaction_sweeper::spawn`) — not only the
  two the audit finding names — to avoid a partial fix that leaves the transaction sweeper silently
  skipping these projects (see § Reading Confirmation).
- No change to `AwsSecretFetcher`'s or `GcpSecretFetcher`'s own internal implementation — construction
  + composition-root wiring only, reusing `config.rs`'s already-established construction pattern.
- No new external dependency (`aws-sdk-secretsmanager`, `aws-config`, `reqwest` are already in the
  dependency tree, used today by `config.rs`).

## Wave: DISCUSS / [REF] User Stories

### US-01: Cloud-Managed Secret Fetchers Actually Resolve DSNs in Production

**job_id**: JOB-05

#### Elevator Pitch
**Before**: Morgan already stores Meridian Health's production Postgres credentials in AWS Secrets
Manager, exactly as her company's secrets-hygiene policy requires — JOB-05's own push force ("I don't
have to operate yet another credential store"). She provisions an embyr project with
`backend_mode=aws_secret` and a valid ARN. Provisioning itself fails immediately with `500
aws_secret_fetcher_not_configured` — and even if it somehow succeeded, every live gRPC request from
Alex's SDK app would return `Status::internal("aws_secret_fetcher not configured on this server")`.
This happens regardless of the fact that her AWS credentials are perfectly valid and reachable — the
adapter that was built and tested for exactly this (`AwsSecretFetcher`, proven correct by
`us_10_aws_secrets.rs`) was simply never connected to the real running server.
**After**: With AWS credentials genuinely present in `embyr-server`'s environment (e.g. an attached
IAM role), Morgan provisions the `aws_secret`-mode project successfully, and Alex's SDK calls succeed
against it exactly like any `direct_pg` project. GCP-managed credentials via
`EMBYR_GCP_ACCESS_TOKEN` behave identically for `gcp_secret`-mode projects. A deployment that
configures neither cloud continues to start up normally and every `direct_pg` project is completely
unaffected.
**Decision enabled**: Morgan can commit to `backend_mode=aws_secret`/`gcp_secret` in production,
trusting that the mode she selects during provisioning is the mode that actually serves traffic — she
is not forced back onto `direct_pg` (and thus a redundant, policy-violating credential store) just to
get a working deployment.

#### Who
- Morgan (P3) | Tenant Admin / DevOps Lead operating an embyr deployment where database credentials
  are already managed in AWS Secrets Manager or GCP Secret Manager | Needs `embyr-server` itself
  (not just a test harness) to actually reach that secret store, so she never has to duplicate
  credentials into a second store.

#### Solution
Construct real `AwsSecretFetcher`/`GcpSecretFetcher` instances once at the `embyr-server` composition
root (`main.rs`), reusing the construction pattern `config.rs::fetch_from_secret_manager` already
establishes, and thread the SAME instances into all three existing `Option<Arc<...>>` parameters that
already exist for this purpose (`FirestoreService`, `build_admin_router`, `transaction_sweeper::spawn`)
— exactly the wiring `lib.rs`'s own `start_test_server_with_aws_fetcher`/`start_test_server_with_gcp_
fetcher` test helpers already do. The exact construction/gating mechanism per cloud is DESIGN's call
(see § Business Context for the asymmetry this DISCUSS found).

#### Domain Examples

**Example 1 (Happy Path — AWS)**: Morgan, Tenant Admin at Meridian Health, has stored Meridian
Health's production Postgres DSN in AWS Secrets Manager under
`arn:aws:secretsmanager:us-east-1:184729xxxxx:secret:meridian-health-prod-dsn-x7K2p`. The
`embyr-server` process runs on an EC2 instance with an IAM role attached (no explicit AWS keys set).
Morgan provisions project `meridian-health-prod` with `backend_mode=aws_secret` and that ARN —
provisioning succeeds. Alex's SDK app calls `getDoc(doc(db, "patients", "p123"))` and receives the
document, exactly as JOB-01's "all SDK calls succeed unchanged" already promises for every other
backend mode.

**Example 2 (Edge Case — GCP)**: Globex Corp's DevOps lead sets `EMBYR_GCP_ACCESS_TOKEN` on the
`embyr-server` deployment and stores Globex's DSN in GCP Secret Manager at
`projects/globex-prod-247/secrets/embyr-primary-dsn`. Morgan provisions project `globex-prod` with
`backend_mode=gcp_secret` and that resource name — provisioning succeeds, and live SDK traffic against
`globex-prod` works identically to the AWS example.

**Example 3 (Error/Boundary — credentials genuinely absent, non-regression)**: A staging deployment's
operator never attached an IAM role and set no AWS env vars. A project `legacy-staging-test`,
provisioned in `aws_secret` mode before this deployment lost its IAM role, still exists in the system
DB. Alex's SDK app calls `getDoc(...)` against `legacy-staging-test`; the request fails closed with a
clear `Status::internal` error (not a panic, not a silent fallback to a stale or wrong DSN) — the same
overall failure class as today, explicitly locked as a non-regression, not silently changed.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A project backed by AWS Secrets Manager actually works when AWS credentials are available
  Given the embyr-server deployment has AWS credentials available via the standard AWS SDK credential chain
  And Morgan has provisioned project "meridian-health-prod" with backend_mode=aws_secret and a valid secret ARN
  When Alex's SDK client calls GetDocument against "meridian-health-prod"
  Then the request succeeds and returns the document
  And no Status::internal "aws_secret_fetcher not configured" error occurs

Scenario: A project backed by GCP Secret Manager actually works when EMBYR_GCP_ACCESS_TOKEN is set
  Given the embyr-server deployment has EMBYR_GCP_ACCESS_TOKEN configured
  And Morgan has provisioned project "globex-prod" with backend_mode=gcp_secret and a valid resource name
  When Alex's SDK client calls GetDocument against "globex-prod"
  Then the request succeeds and returns the document
  And no Status::internal "gcp_secret_fetcher not configured" error occurs

Scenario: Admin provisioning of a new AWS-Secrets-Manager-backed project succeeds when AWS credentials are available
  Given the embyr-server deployment has AWS credentials available via the standard AWS SDK credential chain
  When Morgan calls the admin provisioning endpoint to create a project with backend_mode=aws_secret and a valid secret_arn
  Then the project is created successfully and an api_key is returned
  And the response is not a 500 aws_secret_fetcher_not_configured error

Scenario: A deployment with neither cloud configured starts up normally and direct_pg is unaffected
  Given the embyr-server deployment has no AWS credentials and no EMBYR_GCP_ACCESS_TOKEN configured
  When the server starts
  Then the server starts successfully and binds all three listeners
  And an existing direct_pg-mode project continues to serve GetDocument requests exactly as before

Scenario: A project already provisioned in aws_secret mode still fails closed when AWS credentials are absent
  Given the embyr-server deployment has no usable AWS credentials in its credential chain
  And a project "legacy-staging-test" already exists with backend_mode=aws_secret
  When Alex's SDK client calls GetDocument against "legacy-staging-test"
  Then the request fails with a Status::internal error, not a panic and not a stale/wrong-DSN success
  And this failure is the same overall behavior class as before this feature (explicit non-regression)

Scenario: Provisioning a new gcp_secret project is rejected with a clear error when EMBYR_GCP_ACCESS_TOKEN is unset
  Given the embyr-server deployment has no EMBYR_GCP_ACCESS_TOKEN configured
  When Morgan calls the admin provisioning endpoint to create a project with backend_mode=gcp_secret and a valid gcp_resource_name
  Then provisioning is rejected with a clear, named configuration error
  And no project row is created and no panic occurs
```

#### Acceptance Criteria
- [ ] AC-WSF-01: with AWS credentials genuinely available in the environment (standard AWS SDK
      credential chain resolves), a real gRPC request against an `aws_secret`-mode project succeeds
      (walking skeleton) — proven against a real LocalStack Secrets Manager container, mirroring
      `us_10_aws_secrets.rs`'s own existing infrastructure pattern.
- [ ] AC-WSF-02: with `EMBYR_GCP_ACCESS_TOKEN` set, a real gRPC request against a `gcp_secret`-mode
      project succeeds — proven against an in-process mock GCP Secret Manager server, mirroring
      `us_11_gcp_secrets.rs`'s own existing infrastructure pattern.
- [ ] AC-WSF-03: with AWS credentials available, admin provisioning of a new `aws_secret`-mode project
      succeeds (not `500 aws_secret_fetcher_not_configured`).
- [ ] AC-WSF-04 (regression guard): a deployment with neither AWS credentials nor
      `EMBYR_GCP_ACCESS_TOKEN` configured starts up successfully with no global fail-fast, and an
      existing `direct_pg`-mode project's `GetDocument`/`SetDocument` behavior is provably unchanged.
- [ ] AC-WSF-05 (named non-regression): a request against an already-provisioned `aws_secret`- or
      `gcp_secret`-mode project on a deployment lacking that cloud's credentials continues to fail
      closed with a clear `Status::internal`-class error — never a panic, never a silent
      stale/wrong-DSN success. The exact error message text is DESIGN's call; the fail-closed
      behavior class is locked here as unchanged.
- [ ] AC-WSF-06: the same constructed fetcher instances reach all three composition-root call sites
      (`FirestoreService`, `build_admin_router`, `transaction_sweeper::spawn`) — verified by
      confirming the transaction sweeper no longer silently skips a real `aws_secret`/`gcp_secret`
      project when that cloud's credentials are available (extends, does not replace, JOB-12's own
      sweeper coverage).
- [ ] AC-WSF-07: no other currently-passing test in the full workspace suite regresses, including
      `tests/acceptance/us_10_aws_secrets.rs` and `us_11_gcp_secrets.rs` (which already prove the
      fetcher mechanism itself; this feature must not weaken or duplicate that coverage).

#### Outcome KPIs
- **Who**: Morgan (P3, Tenant Admin / DevOps Lead) operating embyr deployments where database
  credentials are already managed in AWS Secrets Manager or GCP Secret Manager (JOB-05's own
  persona).
- **Does what**: successfully provisions AND serves live gRPC traffic for `aws_secret`/`gcp_secret`-
  mode projects, without operating a duplicate credential store, when that cloud's credentials are
  genuinely available in the deployment's environment.
- **By how much**: from 0% (today, every `aws_secret`/`gcp_secret` provisioning call and every live
  request fails, 100% of the time, regardless of environment — confirmed by direct reading of the
  hardcoded `None, None` at all three composition-root call sites) to a real gRPC request succeeding
  end-to-end whenever that cloud's credentials are present; 0% regression on `direct_pg`/`agent`-mode
  projects or on deployments with neither cloud configured.
- **Measured by**: acceptance tests using real infrastructure — a LocalStack Secrets Manager
  container for AWS (mirroring `us_10_aws_secrets.rs`'s own existing pattern) and an in-process mock
  GCP Secret Manager HTTP server for GCP (mirroring `us_11_gcp_secrets.rs`) — issuing a real
  provisioning call and a real gRPC `GetDocument`/`SetDocument` call against the production
  composition root (not the test-only helpers), asserting success.
- **Baseline**: 0% — confirmed by direct reading of `main.rs:164-165,236-237,274-275` (all three
  hardcoded `None, None`) and `handler.rs:268-271,286-289` (the resulting `Status::internal` on every
  request).

#### Technical Notes
- Reuse `config.rs::fetch_from_secret_manager`'s already-established construction pattern:
  `AwsSecretFetcher::new(&aws_config, ttl)` where `aws_config = aws_config::load_defaults(
  BehaviorVersion::latest()).await`; `GcpSecretFetcher::new(GCP_SECRET_MANAGER_BASE_URL, &token, ttl)`
  where `token` comes from `EMBYR_GCP_ACCESS_TOKEN` (mirroring `require_gcp_access_token`).
- `lib.rs::start_test_server_with_aws_fetcher`/`start_test_server_with_gcp_fetcher` (lines 712, 765)
  already demonstrate the exact target wiring shape for `FirestoreService` and the admin router —
  DESIGN/DELIVER should treat these as the reference implementation, not invent a new shape.
- The long-lived TTL for these composition-root fetcher instances (repeatedly queried across many
  requests for many projects, unlike the one-shot admin-key/encryption-key sourcing use in
  `config.rs`) is a DESIGN decision; `config.rs`'s own `SECRET_FETCHER_TTL_SECS_UNUSED` constant is
  documented as specific to that one-shot use and not necessarily reusable here.
- `resolve_aws_secret_dsn` (`customer_db_connect.rs`) is a separate call path (transaction sweeper's
  own DSN resolution, per ADR-054) from `handler.rs`'s per-request dispatch this feature's own AC-WSF-
  01/02 exercise directly. Whether this feature's changes also close that function's own pre-existing
  zero-test-coverage gap (flagged by `composite-index-real-creation`'s follow-up) as a side effect is
  DESIGN/DISTILL's call, not mandated here.
- Depends on nothing new — `AwsSecretFetcher`, `GcpSecretFetcher`, `PostgresBackendAdapter`,
  `provision.rs`'s fetcher-presence gate, and `handler.rs`'s runtime dispatch all already exist and
  are unchanged by this feature; this is composition-root construction + wiring only.

## Wave: DISCUSS / [REF] Definition of Done

1. AC-WSF-01 through AC-WSF-07 all pass, proven against real infrastructure (real LocalStack container
   for AWS, real in-process mock GCP server, real running `embyr-server` process using the PRODUCTION
   composition root, not test-only helpers) — mirrors this session's own "real, not mocked/unit-only"
   proof standard.
2. AC-WSF-04's regression guard is proven identical to pre-feature behavior for `direct_pg` traffic,
   not merely "still works."
3. AC-WSF-05's non-regression is proven as an explicit, named test case — not merely assumed
   unchanged.
4. Full regression suite clean (pre-existing flakes triaged, not assumed, per this session's own
   `feedback_triage_before_dismissing_as_flaky` practice).
5. Mutation testing runs after DELIVER, per this repo's own `per-feature` strategy (root `CLAUDE.md`).
6. Evolution doc written; `docs/product/production-readiness-audit-2026-09-08.md` row 7 updated to
   CLOSED at FINALIZE (this DISCUSS only advances it to IN PROGRESS — see § Next Wave).
7. Memory updated.

## Wave: DISCUSS / [REF] Out of Scope

- **Redesigning `handler.rs`'s per-request DSN-fetch-then-new-adapter-per-request shape** (lines
  263-298) — each request builds a fresh `PostgresBackendAdapter` even on a cache hit; this is a
  pre-existing, unchanged inefficiency, not this feature's concern.
- **`resolve_aws_secret_dsn`'s own zero test coverage** — named as a likely-related, pre-existing gap
  (see § Reading Confirmation); this feature does not mandate dedicated tests for that specific
  function beyond whatever AC-WSF-01/02 naturally exercise.
- **mTLS, IAM policy design, or GCP service-account/workload-identity setup guidance** — operator-side
  AWS/GCP configuration is assumed to already exist per JOB-05's own premise ("credentials already
  managed in AWS/GCP secret infrastructure"), not built by this feature.
- **Finding #8** (no build/release path for `embyr-agent`) — an unrelated `backend_mode=agent`
  concern, a separate tracked Blocker.
- **The exact error message text / HTTP status code refinement** for missing-fetcher failures —
  DESIGN may refine wording; the overall fail-closed behavior class (`Status::internal`-equivalent) is
  locked as an explicit non-regression here, not the literal string.
- **Any code implementation** — this is DISCUSS only, per explicit task instruction; DESIGN performs
  the investigation and implementation planning.

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy A** (real, minimal, end-to-end) — every scenario is a real gRPC request and/or real admin
provisioning call against a real running `embyr-server` process built from the PRODUCTION composition
root (`main.rs`, not the test-only `start_test_server_with_*_fetcher` helpers), using a real LocalStack
Secrets Manager container for AWS and an in-process mock GCP Secret Manager server for GCP — mirroring
`us_10_aws_secrets.rs`/`us_11_gcp_secrets.rs`'s own existing infrastructure exactly, but exercising
`main.rs` itself rather than the test-only wiring those files currently use. This feature IS the
walking skeleton — single story, no further slicing (mirrors the two most recent sibling Blocker
features' own single-story precedent for a confined composition-root wiring fix).

## Wave: DISCUSS / [REF] Driving Ports

The existing gRPC :8080 listener (any RPC handler that dispatches on `backend_mode`, e.g.
`GetDocument`/`SetDocument`) and the existing Admin :9090 `POST /admin/v1/...` provisioning endpoint
(both already exist). Zero new RPC/HTTP endpoint — this feature changes only what fetcher instances
the composition root constructs and threads into already-existing parameters.

## Wave: DISCUSS / [REF] Pre-requisites

- None beyond what already exists. `AwsSecretFetcher`, `GcpSecretFetcher`,
  `config.rs::fetch_from_secret_manager`'s construction pattern, `provision.rs`'s fetcher-presence
  gate, `handler.rs`'s runtime dispatch, and `lib.rs`'s test-only wiring reference implementation
  (`start_test_server_with_aws_fetcher`/`start_test_server_with_gcp_fetcher`) all already exist.
- LocalStack (AWS Secrets Manager emulation) and an in-process mock GCP Secret Manager server are
  already used by `us_10_aws_secrets.rs`/`us_11_gcp_secrets.rs` — no new test infrastructure to stand
  up.

## Wave: DISCUSS / [REF] Definition of Ready Validation

### DoR Checklist (9-item hard gate)
1. [x] Story traces to a job_id (JOB-05) — reused, not new, with reasoning against the nearest
   alternative (JOB-13) explicitly documented in § Persona & Job, and confirmed via
   `us_10_aws_secrets.rs`'s own doc comment matching JOB-05's job story almost verbatim.
2. [x] Story has a complete Elevator Pitch (Before / After / Decision enabled), naming real
   user-invocable entry points (gRPC `GetDocument`, admin provisioning endpoint).
3. [x] Every AC is testable without ambiguity (7 ACs, each a real gRPC/admin-API assertion against a
   real running server using real LocalStack/mock-GCP infrastructure, or a real full-suite regression
   run).
4. [x] Walking Skeleton identified (US-01 is the whole feature's walking skeleton).
5. [x] Scope Assessment passed.
6. [x] Story is not `@infrastructure`-only with no user-visible value — Decision 4 = Yes (full JTBD
   path); it directly enables Morgan's own trust decision (Elevator Pitch "Decision enabled") that the
   backend mode she provisions is the mode that actually serves traffic.
7. [x] Out of Scope explicitly named (6 items, each reasoned).
8. [x] Outcome KPIs have a numeric framing (0% → real-request-succeeds-when-credentials-present) and
   measurement methods using existing real infrastructure.
9. [x] Prior-wave artifacts read and reconciled (the audit's own finding #7, ADR-054 §D7,
   `composite-index-real-creation`'s follow-up note, JOB-05's existing job story, and the already-
   proven `us_10`/`us_11` test mechanism all directly informed this feature's shape; the audit
   finding's own wording about provisioning was corrected against direct reading, not silently
   repeated).

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Job reused: JOB-05 (`cloud-secret`), not JOB-13 (`production-deployment` — wrong shape, this
  feature adds no new required env var and must not fail startup globally).
- [D2] Scope is exactly the composition-root construction + wiring of already-existing, already-tested
  fetcher adapters into three already-existing `Option<Arc<...>>` parameters — zero new domain type,
  zero new adapter, zero new endpoint.
- [D3] Missing-credentials behavior: AWS fetcher construction can be unconditional (the SDK credential
  chain resolves lazily, confirmed by direct reading of `aws_config::load_defaults`'s own signature);
  GCP fetcher construction must be gated on `EMBYR_GCP_ACCESS_TOKEN` (the constructor takes the token
  synchronously). This asymmetry is named as DESIGN's starting evidence, not a locked mechanism.
- [D4] The audit finding's own claim that "Admin provisioning API still allows creating such projects"
  is corrected against direct reading: `provision.rs` already gates on fetcher presence and fails
  immediately (500) today — no zombie projects can be created via this path. This does not reduce the
  finding's severity (both provisioning and live traffic are blocked identically) but changes what
  "fixed" looks like: fixing composition-root construction fixes both at once, using the SAME error
  path already implemented for the fetcher-absent case.
- [D5] All THREE hardcoded-`None` call sites in `main.rs` (not only the two the audit finding cites)
  must be wired from the same constructed instances — `transaction_sweeper::spawn` shares the
  identical gap per ADR-054 §D7, confirmed independently by this DISCUSS's own reading.

### Requirements Summary
- Primary need: whichever cloud's credentials (AWS or GCP) are genuinely available in the deployment's
  environment must make that backend mode's projects actually work, end-to-end, for both provisioning
  and live gRPC traffic — restoring JOB-05's own already-documented, already-tested-in-isolation
  promise to the real production composition root.
- Walking skeleton scope: US-01, the entire feature — single story, 6 UAT scenarios, 7 ACs.
- Feature type: Reliability fix / composition-root wiring gap.

### Constraints Established
- Zero behavior change to `direct_pg`/`agent`-mode projects.
- No global startup fail-fast for missing AWS/GCP credentials.
- Fail-closed behavior for a provisioned project whose cloud's credentials are absent is preserved as
  an explicit, named non-regression — not silently changed.
- No new bounded context, no new domain type, no new RPC/HTTP endpoint.

### Upstream Changes
None — this DISCUSS extends JOB-05's existing scope (same job, same persona), consistent with this
session's own repeated "make it real / close a gap in an already-covered surface" pattern (e.g.
`customer-db-transaction-sweeper` → JOB-12, `firestore-composite-indexes-admin-api` → JOB-01).

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 5 locked Decisions (D1-D5), 1-story walking-skeleton plan, 7
ACs (AC-WSF-01 through AC-WSF-07) to design executable scenarios against. DESIGN's own investigation
scope: (1) the precise AWS-unconditional / GCP-gated-on-`EMBYR_GCP_ACCESS_TOKEN` construction
mechanism (asymmetry named, not locked, in § Business Context), (2) the long-lived TTL value for these
composition-root fetcher instances, (3) confirming whether AC-WSF-01/02's own acceptance tests close
`resolve_aws_secret_dsn`'s pre-existing zero-test-coverage gap as a side effect or whether a dedicated
test is still warranted, (4) the exact, final error-message wording for AC-WSF-05's non-regression
case (behavior class locked here, literal text is DESIGN's call).

## Wave: DESIGN / [REF] Reading Confirmation

✓ `crates/embyr-server/src/main.rs` read in full (361 lines). Line numbers confirmed **unchanged**
from DISCUSS's own citation (no drift this session): `FirestoreService` construction 156-167
(`aws_secret_fetcher: None, gcp_secret_fetcher: None` at 164-165); `build_admin_router(...)` call
228-244 (`None, None` at 236-237, the 8th/9th positional args); `transaction_sweeper::spawn(...)` call
272-280 (`None, None` at 274-275, the 2nd/3rd positional args). The existing comment block at 261-271
explicitly documents the sweeper's own `None, None` as sharing "the pre-existing gap ADR-054 § D7
already documents explicitly" — this comment goes stale the moment this feature lands and must be
removed/rewritten (see § Handoff Package).

✓ Field/parameter types independently confirmed at all three sites (task's own instruction: verify,
don't assume, since types can differ) — **they do not differ, no alignment needed**:
- `FirestoreService.aws_secret_fetcher`/`.gcp_secret_fetcher` (`grpc/handler.rs:87,89`):
  `Option<Arc<AwsSecretFetcher>>` / `Option<Arc<GcpSecretFetcher>>`.
- `build_admin_router`'s 8th/9th params (`admin/router.rs:104-105`): identical
  `Option<Arc<AwsSecretFetcher>>` / `Option<Arc<GcpSecretFetcher>>`.
- `transaction_sweeper::spawn`'s 2nd/3rd params (`sweepers/transaction_sweeper.rs:63-64`): identical
  `Option<Arc<AwsSecretFetcher>>` / `Option<Arc<GcpSecretFetcher>>`.

  All three already accept the exact same pair of types — one constructed `Option<Arc<...>>` pair
  reaches all three via `.clone()` (`Arc` is cheaply `Clone`; `Option<Arc<T>>` inherits `Clone`
  regardless of `T`'s own `Clone`-ness), no per-site wrapping/unwrapping needed.

✓ `crates/embyr-server/src/lib.rs:712-762` (`start_test_server_with_aws_fetcher`) and `:765-815`
(`start_test_server_with_gcp_fetcher`) read in full — confirmed as the task described: both construct
a real fetcher via the caller-supplied `Arc<AwsSecretFetcher>`/`Arc<GcpSecretFetcher>` (already built
by the *calling test*, not by these helpers) and thread `Some(...)`/`Arc::clone(...)` into both
`FirestoreService` and `admin::router::build_with_aws`/`build_with_gcp`. These helpers do NOT
themselves demonstrate the production *construction* pattern (they take an already-built fetcher as a
parameter) — the production construction pattern (`aws_config::load_defaults(...)` +
`AwsSecretFetcher::new(...).await`; `GcpSecretFetcher::new(base_url, &token, ttl)`) is demonstrated
instead by `config.rs::fetch_from_secret_manager` (495-531), confirmed byte-for-byte below.

✓ `crates/embyr-server/src/config.rs:495-531` (`fetch_from_secret_manager`) and `:480-488`
(`require_gcp_access_token`) read in full. Exact reusable shape:
```rust
// AWS
let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
let fetcher = AwsSecretFetcher::new(&aws_config, ttl_secs).await;

// GCP
let token = std::env::var("EMBYR_GCP_ACCESS_TOKEN").ok().filter(|v| !v.is_empty())...;
let fetcher = GcpSecretFetcher::new(GCP_SECRET_MANAGER_BASE_URL, &token, ttl_secs);
```
`GCP_SECRET_MANAGER_BASE_URL` (`config.rs:54`) and `SECRET_FETCHER_TTL_SECS_UNUSED` (`config.rs:62`)
are both **private** (`const`, not `pub const`) — `main.rs` is a separate crate (the `embyr-server`
binary target depends on the `embyr_server` library target as an external crate; `use embyr_server::
{...}` in `main.rs:24` confirms this), so neither is visible to `main.rs` today. This is a real,
load-bearing visibility gap this DESIGN must close (see § Construction Decisions D-WSF-3).

✓ `crates/embyr-server/src/adapters/aws_secret_fetcher.rs:22-44` and `gcp_secret_fetcher.rs:19-54`
read in full. Both doc comments independently state **"default 300s"** as the adapter's own intended
cache TTL semantic — `aws_secret_fetcher.rs:24-25` additionally references an
`EMBYR_AWS_SECRET_CACHE_SECONDS` env-var override "(used in tests for short TTL)". Grepped the entire
workspace for that variable name: **zero implementations exist anywhere** — it is stale/aspirational
documentation for a caller-side knob that was never built (tests construct short TTLs by passing a
literal `2`/`1` directly to `::new(...)`, not via an env var — confirmed in
`tests/acceptance/us_10_aws_secrets.rs:324,336` and `us_11_gcp_secrets.rs:317-318`). Pre-existing
doc/code mismatch, not introduced by and not this feature's concern to fix — noted, not touched.

✓ `crates/embyr-server/src/grpc/handler.rs:263-298` (`aws_secret`/`gcp_secret` dispatch branches) and
`crates/embyr-server/src/admin/handlers/provision.rs:199-215,256-267` (AWS/GCP provisioning validation
branches) re-read at DESIGN depth, specifically to verify DISCUSS's claim that no new error-handling
code is needed for the AWS-credentials-genuinely-absent case. **Confirmed true, byte-for-byte**:
- `handler.rs:272-275`: `fetcher.get_dsn(&arn).await.map_err(|e| Status::internal(format!("aws secret
  fetch failed: {e}")))?` — already implemented, already reachable the instant `aws_secret_fetcher` is
  `Some`. A real `AwsSecretError` (e.g. `AccessDenied`, `Sdk(...)`) from a credential-less environment
  flows through this exact line with zero new code.
- `provision.rs:210-215`: `fetcher.fetch_fresh(arn).await.map_err(|e| match e { AwsSecretError::
  AccessDenied => 400 backend_secret_fetch_failed, ... })` — same: already implemented, already
  reachable, zero new code.
- The GCP equivalents (`handler.rs:289`, `provision.rs:260`) are **completely unchanged** by this
  feature — `gcp_secret_fetcher` stays genuinely `None` whenever `EMBYR_GCP_ACCESS_TOKEN` is absent,
  so the existing `Status::internal("gcp_secret_fetcher not configured on this server")` /
  `500 gcp_secret_fetcher_not_configured` paths fire byte-identically to today.

✓ `crates/embyr-server/src/adapters/customer_db_connect.rs:46-47,60-` (`resolve_aws_secret_dsn`,
`resolve_gcp_secret_dsn`) read — confirmed these are called by the transaction sweeper's own DSN
resolution whenever `aws_secret_fetcher`/`gcp_secret_fetcher` are `Some`. This resolves DISCUSS's open
question (3): AC-WSF-06's own sweeper scenario (proving the sweeper no longer skips a real
`aws_secret`/`gcp_secret` project once fetchers are wired) **necessarily exercises
`resolve_aws_secret_dsn`/`resolve_gcp_secret_dsn` as a side effect** — closing
`composite-index-real-creation`'s follow-up-flagged zero-test-coverage gap for free. **No dedicated
test is warranted** — DISTILL should design AC-WSF-06's scenario knowing this, not duplicate coverage.

✓ `docs/product/architecture/adr-054-transaction-sweeper-raw-access-and-sweep-sql.md` § D7 (211-249)
re-read at DESIGN depth. It already specifies the exact `TransactionSweeper::spawn` signature (matches
the independently-confirmed signature above) and already names "constructed fresh in `main.rs` using
the identical pattern already used in `config.rs::fetch_from_secret_manager`" and the
`EMBYR_GCP_ACCESS_TOKEN`-reuse choice — this DESIGN's own conclusions (below) are **confirmations of an
already-specified plan**, not new invention, for the sweeper call site specifically.

## Wave: DESIGN / [REF] Blast Radius (full-workspace, every call site individually verified)

Grepped the entire workspace (not just `main.rs`) for all three symbols, per the task's explicit
"a naive grep... can miss call sites; verify EVERY match" instruction — this session's own
`stripe-webhook-secret-required` DESIGN lesson applied directly.

**`FirestoreService { ... }` struct-literal construction — 2 files, exactly as DISCUSS found:**
- `crates/embyr-server/src/main.rs:156` — **the production site. Needs wiring.**
- `crates/embyr-server/src/lib.rs` — 6 test-harness helper functions (`alloc_test_components`-based
  `start_test_server*`), each its own struct literal. Only 2 of the 6 already pass real fetchers
  (`start_test_server_with_aws_fetcher:732`, `start_test_server_with_gcp_fetcher:786`) — by design,
  each exercises exactly one cloud, `None` for the other. The remaining 4 (`start_test_server:593`→
  `start_test_server_with_keepalive`, `start_test_server_with_email_sender:613`,
  `start_test_server_with_oauth:671`, `start_test_server_with_distributed_rate_limit:835`) pass
  `None, None` for **both** — deliberately: none of these helpers' own callers (client-auth,
  oauth-providers, distributed-rate-limiting test suites) exercise `aws_secret`/`gcp_secret`
  backend_mode at all. **My call: leave unchanged.** Wiring real fetchers into these would require
  every one of those unrelated test suites to stand up LocalStack/mock-GCP for zero incremental
  coverage — the opposite of this project's own token/container-discipline conventions
  (`feedback_container_cleanup`, root `CLAUDE.md` test-run discipline).
- `crates/embyr-server/src/grpc/handler.rs` — matched only on the **struct/impl definition**
  (`pub struct FirestoreService {`, `impl FirestoreService {`), zero construction sites. Confirms
  DISCUSS's own count (2 files, not 3).

**`build_admin_router(...)` / `build_with_aws(...)` / `build_with_gcp(...)` — every call site checked:**
- `crates/embyr-server/src/main.rs:228` — **the production `build_admin_router` site. Needs wiring.**
- `crates/embyr-server/src/lib.rs` — 8 `build_with_aws`/`build_with_gcp` call sites (562, 626, 684,
  737, 790, 848, 909, 965), each inside one of the same 6 `start_test_server*` helpers above (some
  helpers call it twice across overloaded variants). Same reasoning as above: 2 already pass real
  fetchers (their dedicated US-10/US-11 helpers), 6 deliberately pass `None`. Unchanged.
- **8 direct `build_admin_router(...)` call sites**, one each in `tests/{client_auth_hosted_identity,
  anonymous_sessions, security_rules, client_auth, oauth_providers, admin_api_v2,
  card_payments_backend}×2}/common/mod.rs` — every one individually opened and confirmed: all pass
  `None, None` for the 8th/9th (`aws_secret_fetcher`/`gcp_secret_fetcher`) positional args
  (spot-checked `client_auth/common/mod.rs:181-182`, `admin_api_v2/common/mod.rs:373-374`, both
  literal `None, None`). **My call: leave unchanged.** These 8 test suites cover client auth, hosted
  identity, anonymous sessions, security rules, OAuth providers, the admin API v2 surface, and card
  payments — none provision or exercise `aws_secret`/`gcp_secret` backend_mode projects; they are
  deliberately testing the fetcher-absent/uninvolved path (or, more precisely, a path that never
  touches these fields at all). Wiring real fetchers here is unrequested scope with a real cost
  (8 more test suites needing LocalStack/mock-GCP) and zero coverage benefit — ponytail: don't add
  infrastructure a test doesn't exercise.
- `docs/**` matches (`brief.md`, `adr-017`, `adr-018`, 3× `feature-delta.md`) are documentation
  cross-references, not code — no action.

**`transaction_sweeper::spawn(...)` — every call site checked (smallest blast radius of the three):**
- `crates/embyr-server/src/main.rs:272` — **the production site. Needs wiring.**
- `tests/customer_db_transaction_sweeper/acceptance/us01_reclaim_orphaned_transactions.rs:213-221` —
  the ONE other call site in the entire workspace. Passes `None, None` (confirmed, lines 215-216).
  This test proves reclaim/purge semantics for `direct_pg`-mode transactions, unrelated to secret
  fetchers. **My call: leave unchanged**, same reasoning as above — this feature does not need or
  benefit from LocalStack in the transaction-sweeper reclaim test suite.

**Conclusion: exactly 3 production-code edit points, all in `main.rs`. Zero test files require any
change as a result of this feature's own wiring decision** (test files exercising the fetcher
mechanism itself — `us_10_aws_secrets.rs`, `us_11_gcp_secrets.rs` — are unchanged inputs this feature
must not regress, per AC-WSF-07; DISTILL adds NEW test files for AC-WSF-01 through 06 exercised
against `main.rs`'s real composition root, not edits to any file enumerated above).

## Wave: DESIGN / [REF] Construction Decisions

**D-WSF-1 — Construction site and shape.** One new construction block in `main.rs`, inserted between
Step 8 (bind the 3 TCP listeners) and Step 9 (build `FirestoreService`) — both fetchers must exist
before `FirestoreService` is built, and neither depends on anything bound in Step 8; placing it
immediately before Step 9 keeps the startup-sequence doc comment (`main.rs:1-17`) accurate with one
new line rather than renumbering existing steps. Illustrative shape (DELIVER owns exact
formatting/comment wording):

```rust
// Step 8b: construct cloud secret fetchers (wire-secret-fetchers,
// closes production-readiness-audit finding #7).
let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
let aws_secret_fetcher = Some(Arc::new(
    AwsSecretFetcher::new(&aws_config, CLOUD_SECRET_FETCHER_TTL_SECS).await,
));

// `None` here is a deliberate deferral, not an oversight: this feature's own
// § System Constraints locks "no global startup fail-fast for missing
// AWS/GCP credentials" (narrow-fail-fast precedent, same as
// stripe-webhook-secret-required's JOB-13 shape rejected in § Persona &
// Job). A gcp_secret-mode request/provision call fails closed later,
// exactly as today (handler.rs:289 / provision.rs:260) — this line must
// carry that comment verbatim in main.rs so the silent-None mapping reads
// as intentional, not a missed validation.
let gcp_secret_fetcher = std::env::var("EMBYR_GCP_ACCESS_TOKEN")
    .ok()
    .filter(|v| !v.is_empty())
    .map(|token| {
        Arc::new(GcpSecretFetcher::new(
            GCP_SECRET_MANAGER_BASE_URL,
            &token,
            CLOUD_SECRET_FETCHER_TTL_SECS,
        ))
    });
```

Then thread the SAME `Option<Arc<...>>` pair into all 3 sites (`Option<Arc<T>>: Clone` — no manual
`match`/`as_ref().map(Arc::clone)` boilerplate needed):
- `FirestoreService { ..., aws_secret_fetcher: aws_secret_fetcher.clone(), gcp_secret_fetcher:
  gcp_secret_fetcher.clone(), ... }` (replaces the `None, None` at 164-165).
- `build_admin_router(..., aws_secret_fetcher.clone(), gcp_secret_fetcher.clone(), ...)` (replaces the
  `None, None` 8th/9th args at 236-237).
- `transaction_sweeper::spawn(system_db.clone(), aws_secret_fetcher, gcp_secret_fetcher, ...)`
  (replaces the `None, None` at 274-275) — final use in program order, so this one **moves** rather
  than clones; no third `.clone()` needed.

**Asymmetry rationale (confirms DISCUSS's own finding, now locked as the mechanism):**
- **AWS: unconditional construction.** `aws_config::load_defaults(...)` is infallible/lazy — zero I/O,
  zero risk of blocking or failing startup, regardless of whether real credentials exist. A deployment
  with no AWS credentials still starts normally; the first `aws_secret`-mode request or provisioning
  call fails closed later via already-implemented code (§ Reading Confirmation above) — satisfying
  AC-WSF-05 (fail-closed non-regression) and § System Constraints (no global startup fail-fast) with
  zero new error-handling code.
- **GCP: gated construction.** `GcpSecretFetcher::new` takes the bearer token synchronously — cannot
  be constructed without one. Gate on `EMBYR_GCP_ACCESS_TOKEN` non-empty, mirroring
  `config.rs::require_gcp_access_token`'s established idiom (not reusing the function itself — it
  returns a `ConfigError` shaped for `ServerConfig::from_env()`'s different fail-fast context, per the
  task's own instruction not to force-fit it). `None` here reproduces today's exact `gcp_secret_fetcher
  not configured` failure class — zero regression, zero new code.

**D-WSF-2 — TTL value.** New named constant in `config.rs`, `pub const
CLOUD_SECRET_FETCHER_TTL_SECS: u64 = 300` (300s / 5 minutes), placed beside the existing
`SECRET_FETCHER_TTL_SECS_UNUSED` (which stays private and unchanged — it backs a genuinely different,
one-shot, cache-never-consulted bootstrap use per its own doc comment). **Not reused** for this
feature: reusing a constant named `_UNUSED` for a fetcher that is now long-lived and genuinely caching
would be a naming lie the moment this feature lands — a new, correctly-named constant is the lazier
and more honest fix (one `const` line, not a rename that touches `config.rs`'s 4 existing bootstrap
call sites). 300s is not arbitrary: both adapters' own doc comments (`aws_secret_fetcher.rs:24`,
`gcp_secret_fetcher.rs:26`) independently document "default 300s" as their intended value, and it
matches the order of magnitude `us_10_aws_secrets.rs`/`us_11_gcp_secrets.rs`'s own rotation-
transparency tests already validate (at an accelerated 1-2s TTL for test speed) — JOB-05's "password
rotation is transparent" promise holds within this window. **No env-var override built** — the
`EMBYR_AWS_SECRET_CACHE_SECONDS` mentioned in `aws_secret_fetcher.rs`'s own doc comment is pre-existing
dead documentation (§ Reading Confirmation) for a knob nothing in this feature's scope requires;
building it now would be unrequested scope (ponytail). If real operational need for a shorter/longer
production TTL emerges, it is a 1-line follow-up, not a gap this feature leaves unhandled — the
correctness property (rotation eventually picked up) already holds at the fixed value.

**Production rationale, made explicit (peer-review condition — must appear as a `main.rs` comment
next to the constructed fetchers, not left implicit):** AWS/GCP secret rotation in the target
deployments (Meridian Health / Globex-shaped operators, per JOB-05's own premise) happens on the
order of hours-to-days (manual or scheduled rotation), never sub-minute — a 300s cache means a
rotated DSN is picked up within, worst case, one TTL window (≤5 min) after rotation, which is
acceptable staleness for a background credential, not a per-request hot value. 300s also bounds
`GetSecretValue`/GCP REST call volume to at most 1 per project per 5 minutes regardless of request
rate, protecting against AWS/GCP API throttling under load — the actual operational reason a TTL
exists at all here, not merely "the adapter's own default." `us_10`/`us_11`'s own accelerated
1-2s TTL scenarios prove the ROTATION MECHANISM works, not that 300s is the right production value —
this paragraph is that missing justification, per peer review.

**D-WSF-3 — Base URL visibility.** `GCP_SECRET_MANAGER_BASE_URL` (`config.rs:54`) changes from private
`const` to `pub const` — the minimal visibility bump needed for `main.rs` (a separate crate from the
`embyr_server` library target) to reuse it directly, avoiding a duplicate literal string. No other
change to the constant; its value and existing 4 in-crate call sites (`fetch_from_secret_manager`'s
GCP branch) are untouched.

**D-WSF-4 — Stale comment removal.** `main.rs:261-271`'s existing comment (explaining that the
sweeper's `aws_secret_fetcher`/`gcp_secret_fetcher` are `None`, "mirroring the pre-existing gap ADR-054
§ D7 names explicitly") becomes factually wrong the moment this feature lands and must be removed or
rewritten as part of implementation — flagged here so DELIVER doesn't leave a comment asserting a gap
that no longer exists.

## Wave: DESIGN / [REF] ADR Decision

**No new ADR.** Evaluated against this session's own bar (e.g. ADR-069 for `rate-limiter-project-id-
validation`, which introduced a genuinely new mechanism — a `known_existing` metrics-label signal) and
found this feature does not clear it: zero new component, zero new port/adapter, zero new domain type,
zero new architectural pattern. The one asymmetric decision (AWS unconditional vs. GCP gated
construction) was **already fully specified** by `docs/product/architecture/adr-054-transaction-
sweeper-raw-access-and-sweep-sql.md` § D7 for the sweeper call site, and this DESIGN's own independent
re-derivation (§ Construction Decisions above) reached the identical conclusion for all 3 call sites —
this is confirmation and generalization of an existing, already-recorded decision, not a new one. The
2 remaining decisions (TTL value, base-URL visibility) are narrow, fully justified, single-paragraph
choices with no viable competing alternative worth a comparison table (TTL: reuse-a-lying-constant-name
vs. a correctly-named new one; visibility: `pub` vs. duplicate the literal — neither is a real design
fork). Per ADR immutability convention, `adr-054`'s § D7 is left unmodified — its "pre-existing gap,
out of scope" framing was accurate at the time it was written; this feature's own evolution doc (at
FINALIZE) is the correct place to record the gap's closure, not a retroactive ADR edit.

**Peer-review condition, locked**: the evolution doc MUST explicitly name the AWS-unconditional /
GCP-gated asymmetry and its evidence (§ Construction Decisions D-WSF-1 above), not just "wired the
fetchers" — this is the durable, SSOT-searchable record the reviewer required in place of a
dedicated ADR. If DELIVER or a future maintainer finds the `main.rs` code comments insufficient to
explain the asymmetry on their own, escalate to a proper `adr-074-*.md` at FINALIZE rather than
leaving it comment-only — this DESIGN's own call was that comments + evolution doc suffice, not that
documentation is unnecessary.

## Wave: DESIGN / Handoff Package

**Files requiring a change (3 production files, exactly):**
1. `crates/embyr-server/src/main.rs` — new construction block (§ D-WSF-1) inserted before Step 9; 3
   call-site edits (`FirestoreService` literal, `build_admin_router` call, `transaction_sweeper::spawn`
   call); import block extended (`adapters::{aws_secret_fetcher::AwsSecretFetcher,
   gcp_secret_fetcher::GcpSecretFetcher, ...}`, `config::{ServerConfig,
   CLOUD_SECRET_FETCHER_TTL_SECS, GCP_SECRET_MANAGER_BASE_URL}`); stale comment at 261-271 removed/
   rewritten (§ D-WSF-4).
2. `crates/embyr-server/src/config.rs` — `GCP_SECRET_MANAGER_BASE_URL` (line 54) becomes `pub const`
   (§ D-WSF-3); new `pub const CLOUD_SECRET_FETCHER_TTL_SECS: u64 = 300` added beside
   `SECRET_FETCHER_TTL_SECS_UNUSED` (§ D-WSF-2). `fetch_from_secret_manager`/
   `require_gcp_access_token`/`SECRET_FETCHER_TTL_SECS_UNUSED` themselves: **unchanged**.
3. No third production file — `handler.rs`, `provision.rs`, `admin/router.rs`,
   `sweepers/transaction_sweeper.rs`, `customer_db_connect.rs`, `lib.rs` all confirmed to need **zero**
   change (§ Reading Confirmation, § Blast Radius); their existing `Option<Arc<...>>`-typed signatures
   and already-implemented error paths are the entire reason this is a pure wiring fix.

**Files confirmed to need NO change:** the 6 `lib.rs` test-harness helpers, the 8 direct
`build_admin_router` test call sites, and the 1 `transaction_sweeper::spawn` test call site — full
list and reasoning in § Blast Radius.

**Documentation changes made this wave:** none beyond this feature-delta.md DESIGN section — no new
ADR (§ ADR Decision), no existing ADR modified.

**Regression guards DISTILL/DELIVER must run:** `tests/acceptance/us_10_aws_secrets.rs` and
`us_11_gcp_secrets.rs` (AC-WSF-07 — must keep passing unmodified, they prove the fetcher mechanism
itself and are the reference pattern this design reuses, not duplicates). Full workspace `cargo test`
once at the pre-commit gate per this repo's own root `CLAUDE.md` discipline.

**New test scenarios DISTILL must design (none exist today, all against `main.rs`'s real composition
root, not test-only helpers):** the 6 UAT scenarios as written (AC-WSF-01 through 06) — a real gRPC
request against a LocalStack-backed `aws_secret` project via a running `embyr-server` binary process;
the GCP equivalent against an in-process mock GCP server; a real admin-provisioning call for each
cloud; a startup test with neither cloud configured proving `direct_pg` traffic is unaffected; the
`legacy-staging-test` fail-closed non-regression scenario (AWS credentials absent, existing
`aws_secret`-mode project, asserts `Status::internal`, asserts the response is NOT the old "fetcher not
configured" string but IS still `Status::internal`-class); the GCP provisioning-rejected-when-token-
absent scenario (byte-identical to today, confirms zero regression).

## Wave: DISTILL / [REF] Prior Wave Consultation — Reading Confirmation

✓ Read this file's own DISCUSS + DESIGN sections in full (all 921 lines pre-DISTILL) — 5 locked
Decisions (D1-D5), 4 Construction Decisions (D-WSF-1..4), Blast Radius (3 production edit points, zero
test files require change), Handoff Package.

✓ `docs/architecture/atdd-infrastructure-policy.md` read in full. Confirmed pre-existing rows already
cover every port this feature needs: `embyr-server` binary (subprocess) driving-port row (added
2026-08-08, feature `production-readiness`), and the AWS Secrets Manager (LocalStack) / GCP Secret
Manager (fake) driven-external rows. Zero new policy rows needed — this feature inherits, does not
extend, the policy.

✓ `tests/production_readiness/mod.rs` + `tests/production_readiness/common/mod.rs` read in full.
`ServerProcess::start`/`start_env_only`/`wait_for_healthy`/`sigterm`/`wait_for_exit`/`drain_stderr`/
`port_is_bound`, `start_postgres_container`, `TEST_ENCRYPTION_KEY`, and the `state_delta` re-export +
`universe` module are all reused as-is — zero changes to shared test infrastructure.

✓ `tests/production_readiness/acceptance/pr01_config_from_env.rs`, `pr04_graceful_shutdown.rs`,
`pr05_tls_support.rs`, `pr06_stripe_webhook_secret_required.rs`, `pr07_stripe_webhook_body_limit.rs`
read for established conventions: exactly one walking skeleton per module (`pr01`'s own, NOT
`#[ignore]`), all other subprocess tests `#[ignore]` (DELIVER unskips one at a time); real gRPC calls
via `tonic::transport::Channel::from_shared(format!("http://127.0.0.1:{}", server.grpc_port))` +
`FirestoreClient::new(channel)`; sibling customer-database creation pattern (`pr04`) for `direct_pg`
provisioning against a system-Postgres-hosted second database; direct-SQL project-row seeding (`pr05`'s
`setup_tls_server`) for scenarios whose precondition is "already provisioned," not "provision it here";
layer classification precedent (`pr07`'s own header comment): subprocess/real-I/O tests at this layer
use traditional assertions, not `assert_state_delta` — `nw-test-design-mandates` Layered Test Discipline
table names this the correct choice for "Subprocess / FS acceptance" vs. the WS/`@wiring_e2e` layer,
and this file family's own established precedent already applies it uniformly across pr01-pr08.

✓ `tests/acceptance/us_10_aws_secrets.rs` read in full (`start_localstack`, `make_sm_client`,
LocalStack secret-creation shape `{"dsn": "<url>"}`). `tests/acceptance/us_11_gcp_secrets.rs` read in
full (in-process mock GCP Secret Manager via `axum::Router`, `GcpSecretFetcher::new(&base_url, "test-
token", ttl)` constructed directly by the test, handed to `start_test_server_with_gcp_fetcher` — a
TEST-ONLY composition path, confirmed NOT reachable from `main.rs`'s own construction).

✓ `crates/embyr-server/src/admin/handlers/provision.rs` read in full (`ProvisionRequest` fields:
`secret_arn`, `gcp_resource_name`; error codes `aws_secret_fetcher_not_configured` /
`gcp_secret_fetcher_not_configured` (500), `secret_arn_required`/`gcp_resource_name_required`/
`backend_secret_fetch_failed` (400)). `crates/embyr-server/src/grpc/handler.rs:260-298` re-read to
confirm the per-request DSN-fetch-then-adapter-construct dispatch (unchanged by this feature, per
DESIGN's own § Out of Scope).

✓ `tests/customer_db_transaction_sweeper/acceptance/us01_reclaim_orphaned_transactions.rs` read in
full. Confirmed DESIGN's own § Reading Confirmation claim precisely: this file's ONE
`transaction_sweeper::spawn(...)` call site passes `None, None` (the `direct_pg` walking skeleton); a
SEPARATE `gcp_secret_orphaned_transaction_is_reclaimed_without_api_key` test exercises the GCP path via
`TransactionSweeper::run_cycle` directly (not `spawn`) with a test-constructed fetcher — meaning
`resolve_gcp_secret_dsn` already has coverage, but **no `aws_secret` equivalent test exists in this file
at all**, confirming `composite-index-real-creation`'s follow-up note that `resolve_aws_secret_dsn` has
zero coverage anywhere. `EMBYR_TRANSACTION_SWEEP_INTERVAL_SECS` (`config.rs:347`) confirmed as the env
var controlling `main.rs`'s own sweeper interval — used to keep AC-WSF-06's own test fast (1s).

✓ `crates/embyr-server/src/config.rs:52-54` (`GCP_SECRET_MANAGER_BASE_URL`'s own doc comment) read at
DISTILL depth specifically to verify whether AC-WSF-02 (GCP walking-skeleton-equivalent scenario) can
be proven against the real subprocess the same way AC-WSF-01 is proven against LocalStack for AWS. It
cannot — see § Upstream Issue below, a genuine finding this DISTILL pass surfaces for DESIGN/DELIVER's
attention.

## Wave: DISTILL / [REF] Upstream Issue — GCP subprocess testability gap (untestable-as-written AC)

**Finding**: AC-WSF-02, as worded in DISCUSS ("a real gRPC request against a `gcp_secret`-mode project
succeeds" — mirroring `us_11_gcp_secrets.rs`'s own mock-GCP mechanism, proven against the REAL
`main.rs` subprocess), is **not fully provable** given DESIGN's own locked construction shape
(D-WSF-1), and this is a structural asymmetry, not a test-authoring gap.

**Evidence**:
- AWS: `main.rs` calls `aws_config::load_defaults(BehaviorVersion::latest()).await` — the standard AWS
  SDK credential/config resolution chain, which respects the standard `AWS_ENDPOINT_URL` env var (in
  addition to `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`/`AWS_REGION`). This lets a REAL subprocess of
  `main.rs` be pointed at LocalStack with **zero production-code changes** — proven working end-to-end
  by this DISTILL pass's own `aws_secret_project_serves_real_grpc_request_when_aws_credentials_available`
  walking skeleton (RED-verified below).
- GCP: `main.rs` calls `GcpSecretFetcher::new(GCP_SECRET_MANAGER_BASE_URL, &token, ttl)` where
  `GCP_SECRET_MANAGER_BASE_URL` is the hardcoded `pub const "https://secretmanager.googleapis.com"`
  (`config.rs:54`). That constant's own pre-existing doc comment states plainly: *"No env-var override
  exists yet (OQ-SM-4 / ADR-018 Alternatives A6) — out of scope for this feature."* This feature's own
  DESIGN wave (D-WSF-3) only bumps the constant's visibility to `pub` for `main.rs` to reuse it — it
  does **not** add an override mechanism. `us_11_gcp_secrets.rs`'s own mock-GCP mechanism only works
  because it constructs `GcpSecretFetcher` directly with a mock `base_url` and hands the already-built
  instance to `start_test_server_with_gcp_fetcher` — a TEST-ONLY composition path this feature exists
  to prove is insufficient for production.

**Consequence**: a real-subprocess test for AC-WSF-02 can only reach the REAL
`secretmanager.googleapis.com` over the network — it cannot fabricate a real secret the way the AWS
walking skeleton does via LocalStack. `gcp_secret_project_wiring_reached_when_token_configured` (below)
proves WIRING (the fetcher is genuinely `Some` once `EMBYR_GCP_ACCESS_TOKEN` is set — provisioning no
longer returns the old `gcp_secret_fetcher_not_configured`), not a full round-trip against a real
document. It is tagged `@requires_external`, `#[ignore]`d, and excluded from the deterministic CI
regression run by convention (this project has exactly one other precedent for a REAL external
dependency in tests — Stripe, per D-13 in `atdd-infrastructure-policy.md`, and that was an explicit,
reasoned exception; a live dependency on Google's own production API was not similarly reasoned through
by this feature's own DISCUSS/DESIGN and is flagged here rather than silently added).

**Not resolved by this DISTILL pass** (scope: DISTILL does not modify production code). Two paths
forward, left for a human/DESIGN decision before DELIVER implements AC-WSF-02 fully:
1. Accept the wiring-only proof this DISTILL pass ships (`gcp_secret_project_wiring_reached_when_token_configured`)
   as sufficient evidence for AC-WSF-02, given the walking skeleton (AC-WSF-01) already proves the
   IDENTICAL wiring pattern for AWS and the asymmetry is a pre-existing, independently-documented gap
   (`config.rs`'s own comment), not something this feature introduces.
2. Amend DESIGN to add a test-injectable base-URL override for `GcpSecretFetcher`'s construction in
   `main.rs` (e.g. an `EMBYR_GCP_SECRET_MANAGER_BASE_URL` env var, mirroring `AWS_ENDPOINT_URL`'s own
   role for AWS), enabling a fully-deterministic subprocess+mock-GCP test — a small, narrowly-scoped
   DESIGN amendment, not a re-litigation of D-WSF-1's own asymmetric-construction decision.

## Wave: DISTILL / [REF] Scenario List

| # | Scenario | Tags | Ignore? |
|---|----------|------|---------|
| 1 | `aws_secret_project_serves_real_grpc_request_when_aws_credentials_available` | `@walking_skeleton @driving_port @real-io @US-01 @AC-WSF-01 @AC-WSF-03` | No |
| 2 | `gcp_secret_project_wiring_reached_when_token_configured` | `@error @boundary @US-01 @AC-WSF-02 @requires_external` | Yes |
| 3 | `server_with_neither_cloud_configured_starts_and_direct_pg_unaffected` | `@error @boundary @US-01 @AC-WSF-04` | Yes |
| 4 | `aws_secret_project_fails_closed_when_aws_credentials_absent` | `@error @US-01 @AC-WSF-05` | Yes |
| 5 | `gcp_secret_provisioning_rejected_when_token_unset` | `@error @US-01 @AC-WSF-05` | Yes |
| 6 | `transaction_sweeper_reclaims_aws_secret_project_when_aws_credentials_available` | `@error @US-01 @AC-WSF-06` | Yes |

AC-WSF-03 is covered as a subset of scenario 1 (the walking skeleton's own provisioning step), not
duplicated as a separate LocalStack-spinning test — ponytail: avoid a near-identical second container
spin-up for an assertion the walking skeleton already makes. AC-WSF-07 is the full regression suite,
run by the orchestrator once, not a scenario in this file — confirmed clean below (§ RED-State
Verification).

## Wave: DISTILL / [REF] Walking Skeleton Strategy (applied)

Architecture of Reference: driving port (gRPC :8080, Admin :9090) = real adapter via the
`embyr-server` binary subprocess (Project Infrastructure Policy, pre-existing row). Driven external
(AWS Secrets Manager) = LocalStack container, `@real_io` (pre-existing policy row). Driven external
(GCP Secret Manager) = fake/mock for the mechanism-proof tests (`us_11_gcp_secrets.rs`, unmodified); for
THIS feature's own subprocess wiring proof, no fake is reachable (see § Upstream Issue) — the one
`@requires_external` scenario reaches the real API instead. Exactly one walking skeleton
(`@walking_skeleton @driving_port`), scenario 1, NOT `#[ignore]`, mirroring `pr01`'s own established
convention for this file family.

## Wave: DISTILL / [REF] Adapter Coverage

| Adapter | `@real-io` scenario | Covered by |
|---------|---------------------|------------|
| `AwsSecretFetcher` (via `main.rs`'s own construction) | YES | scenario 1 (walking skeleton) + scenario 6 (sweeper) |
| `GcpSecretFetcher` (via `main.rs`'s own construction) | Partial — wiring only, not full round-trip (§ Upstream Issue) | scenario 2 |
| `PostgresBackendAdapter` (per-request, unchanged) | YES (indirect, exercised by every scenario's own gRPC call) | scenario 1 |
| `TransactionSweeper` / `resolve_aws_secret_dsn` | YES | scenario 6 — closes `composite-index-real-creation`'s follow-up-flagged zero-coverage gap for `resolve_aws_secret_dsn` |

## Wave: DISTILL / [REF] Test Placement

`tests/production_readiness/acceptance/pr09_wire_secret_fetchers.rs`, registered via `mod
pr09_wire_secret_fetchers;` in `tests/production_readiness/mod.rs` (no new Cargo.toml `[[test]]`
entry needed — `production_readiness` is already one registered test binary covering pr01-pr08;
pr09 is the natural continuation of the SAME production-readiness-audit-driven test suite, closing
finding #7 exactly as pr01-pr08 closed findings #1-6/8). Precedent: `tests/production_readiness/`'s own
established convention as the strong precedent for "spawn the real binary" style tests in this
codebase, reused rather than creating a dedicated `tests/wire_secret_fetchers/` directory (which would
duplicate `ServerProcess`/`start_postgres_container`/`TEST_ENCRYPTION_KEY` test infrastructure this
feature needs unchanged).

## Wave: DISTILL / [REF] Driving Adapter Coverage

Both driving ports named in DESIGN (gRPC :8080, Admin :9090 `POST /admin/v1/projects`) are exercised via
their real wire protocol in every scenario: `reqwest::Client` HTTP POST for provisioning, real
`tonic::transport::Channel` + `FirestoreClient` for gRPC — no scenario calls an internal service
function directly. Zero new driving adapter is introduced by this feature (DESIGN confirmed: zero new
RPC/HTTP endpoint).

## Wave: DISTILL / [REF] Pre-requisites

Docker daemon (LocalStack + Postgres testcontainers), `embyr-server` binary built via
`CARGO_BIN_EXE_embyr-server` (automatic for `[[test]]` targets) — both already required by pr01-pr08
and `us_10_aws_secrets.rs`; nothing new. `EMBYR_TRANSACTION_SWEEP_INTERVAL_SECS` env var (pre-existing,
`config.rs:347`) used by scenario 6 to keep the sweep interval short for fast test feedback.

## Wave: DISTILL / [REF] RED-State Verification

Run against **today's unfixed code** (`main.rs` still hardcodes `None, None` at all 3 call sites) —
`cargo test --test production_readiness -p embyr-server`:

| Scenario | Result | Classification | Why |
|---|---|---|---|
| 1. `aws_secret_project_serves_real_grpc_request_when_aws_credentials_available` | **FAILED** | `MISSING_FUNCTIONALITY` (correct RED) | Provisioning returns `500 aws_secret_fetcher_not_configured` — exact assertion message: *"provisioning an aws_secret project must succeed (AC-WSF-03) ... got 500 Internal Server Error: {"error":"aws_secret_fetcher_not_configured"}"* |
| 2. `gcp_secret_project_wiring_reached_when_token_configured` (`--ignored`) | **FAILED** | `MISSING_FUNCTIONALITY` (correct RED) | `gcp_secret_fetcher_not_configured` still returned even with `EMBYR_GCP_ACCESS_TOKEN` set — proves `main.rs` never reaches the `Some(...)` construction arm |
| 3. `server_with_neither_cloud_configured_starts_and_direct_pg_unaffected` (`--ignored`) | **PASSED** | Expected (non-regression guard) | AC-WSF-04 asserts behavior that is UNCHANGED by this feature — `direct_pg` never touches the AWS/GCP fetchers, so this holds identically before and after the fix; correctly green today, must stay green post-DELIVER |
| 4. `aws_secret_project_fails_closed_when_aws_credentials_absent` (`--ignored`) | **PASSED** | Expected (non-regression guard) | AC-WSF-05 locks TODAY's fail-closed behavior (`Status::internal`) as unchanged — the current bug already produces this exact class of failure (fetcher `None` → `Status::internal`); post-DELIVER the SAME class of failure occurs for a different reason (real fetch against a nonexistent ARN) — the test must stay green across the fix, which it does |
| 5. `gcp_secret_provisioning_rejected_when_token_unset` (`--ignored`) | **PASSED** | Expected (non-regression guard) | Byte-identical to today per DESIGN's own confirmation — `gcp_secret_fetcher` stays `None` regardless of this feature, so this scenario is unaffected by the fix and correctly green now and after |
| 6. `transaction_sweeper_reclaims_aws_secret_project_when_aws_credentials_available` (`--ignored`) | **FAILED** | `MISSING_FUNCTIONALITY` (correct RED) | Transaction status remained `"active"` (never reclaimed) — proves `transaction_sweeper::spawn(...)` inside `main.rs` still hardcodes `None, None` for its own 2nd/3rd params |

3 of 6 scenarios are non-regression guards that correctly pass BOTH before and after DELIVER's fix (by
design — that is what "non-regression" means); 3 correctly fail today for the right reason
(`MISSING_FUNCTIONALITY`, not a fixture/import/setup error) and are expected to flip to PASSED once
DELIVER implements DESIGN's Handoff Package. Zero scenarios in `IMPORT_ERROR`/`FIXTURE_BROKEN`/
`WRONG_ASSERTION` categories — pre-DELIVER fail-for-the-right-reason gate: **PASSED**.

**AC-WSF-07 regression guard confirmed**: `cargo test --test us_10_aws_secrets --test us_11_gcp_secrets
-p embyr-server` → both green, 4/4 passed each, 0 failed — unmodified by this DISTILL pass, exactly as
required.

Docker cleanup confirmed: `docker ps -a` shows zero stray LocalStack/Postgres containers after the run
(testcontainers auto-removed all of them).

## Wave: DISTILL / [REF] Mandate Compliance Evidence

- **CM-A** (Mandate 1, hexagonal boundary): every scenario enters through a driving port only — real
  HTTP POST to `/admin/v1/projects` (admin port) and real gRPC calls via `FirestoreClient` (data port)
  against the REAL `embyr-server` binary subprocess. Zero import of an internal service/handler
  function directly.
- **CM-B** (Mandate 2, business language): scenario/function names speak in terms of "aws_secret
  project," "AWS credentials available," "transaction sweeper reclaims" — zero HTTP-verb or
  status-code-only naming. Technical detail (JSON bodies, gRPC message types) lives inside the test
  bodies, not in scenario names.
- **CM-C** (Mandate 3, user journey completeness): scenario 1 is a complete journey (Morgan provisions
  → Alex's SDK creates and reads back a document); non-regression scenarios each assert an observable,
  user-facing outcome (starts up, fails closed, rejected with a named error), not an isolated technical
  check.
- **CM-D** (Mandate 4, pure function extraction): not applicable — this feature has zero new business
  logic to extract; it is composition-root wiring only (DESIGN's own confirmation, § ADR Decision:
  "zero new component, zero new port/adapter, zero new domain type").
- Error/edge ratio: 5 of 6 scenarios are error/boundary/non-regression (83%), well above the 40% target
  — appropriate for a reliability-fix feature whose entire value is "this class of failure must not
  regress" plus "this class of failure must finally stop happening."

## Wave: DISTILL / [REF] Next Wave

**Handoff To**: nw-software-crafter (DELIVER wave)
**Deliverables**: `tests/production_readiness/acceptance/pr09_wire_secret_fetchers.rs` (6 scenarios, 1
walking skeleton + 5 `#[ignore]`d, one-at-a-time), RED-state verification (above), the GCP subprocess
testability finding (§ Upstream Issue — needs a human decision before AC-WSF-02 is considered fully
closed, not before DELIVER starts on AC-WSF-01/03/04/05/06). DELIVER implements DESIGN's fully-specified
Handoff Package (§ D-WSF-1..4), unskips scenarios one at a time starting with scenario 2, and runs
`cargo test --test production_readiness -p embyr-server` (all, not `--ignored`-only) plus the full
workspace suite once at the pre-commit gate per this repo's own root `CLAUDE.md` discipline.
