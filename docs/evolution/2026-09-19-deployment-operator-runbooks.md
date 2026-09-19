# Evolution: deployment-operator-runbooks

Closes High findings #45 and #46 (DevOps / Deployment) from
`docs/product/production-readiness-audit-2026-09-08.md`'s Follow-Up Scan.

## Business Context

Two documentation gaps blocked a non-Kubernetes operator from actually running `embyr-server`
in production: no reference single-host deployment path (#45), and no IAM/secret-creation
steps for the AWS/GCP secrets-manager sourcing ADR-018 already implemented in code (#46). Pure
documentation — no code changes.

## Key Decisions

| Decision | Rationale |
|---|---|
| systemd unit runs the Docker image directly (`docker run` under `Type=simple`) | `embyr-server` ships only as a Docker image today; no bare-binary release artifact exists, and findings #42/#43 (image registry publishing) are still Not started, so a local `docker build` per `release-process.md`'s own rollback procedure is the only available path |
| Admin port bound `127.0.0.1:9090:9090`, no reverse-proxy route for `:9090` | Satisfies ADR-001's "must be unreachable from the public network" with one Docker flag instead of a separate firewall rule |
| nginx chosen for the reverse-proxy example (not Caddy) | Task called for one concrete example; nginx's `grpc_pass` directive is the more commonly deployed option for gRPC+REST split-listener fronting |
| Registry location left explicitly TBD | Findings #42/#43 (CI never pushes images anywhere) are unresolved; inventing a registry URL would be dishonest documentation |
| GCP secrets documented with an explicit no-refresh caveat | ADR-018's own Alternative A6 scopes workload-identity token refresh out as future work (OQ-SM-4); `EMBYR_GCP_ACCESS_TOKEN` is a static, startup-only bearer token that expires in ~1h — operators need this called out, not discovered via a failed restart |

## Key Files

- `docs/operations/single-host-deployment.md` — systemd unit(s), nginx TLS-termination
  example, journald log handling note (JSON logs from finding #25), `DATABASE_URL`/env wiring
- `docs/operations/secrets-manager-setup.md` — AWS IAM policy JSON + `aws secretsmanager
  create-secret`, GCP `roles/secretsmanager.secretAccessor` + `gcloud secrets create`, the
  `get_raw_secret()` (plain string) vs `get_dsn()` (JSON-wrapped) distinction, `_PREVIOUS`
  rotation window
- `docs/product/production-readiness-audit-2026-09-08.md` — rows #45/#46 marked CLOSED

## Follow-Up

- Findings #42/#43 (no image registry) block the "pull a released image" step in
  `single-host-deployment.md` — once closed, replace the local `docker build` instructions
  there with a `docker pull <registry>/embyr-server:vX.Y.Z` step.
- OQ-SM-4 (ADR-018): upgrading `GcpSecretFetcher`/`AwsSecretFetcher` to the documented
  `SecretFetcher` trait with real `probe()` and workload-identity refresh would remove the
  manual token-refresh step `secrets-manager-setup.md` currently documents.
- Finding #53 (DNS/LB topology, `:9090` firewalling) remains open at the multi-host level;
  this evolution closes the single-host instance of that gap only.
