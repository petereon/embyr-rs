# Single-Host Deployment (Non-Kubernetes)

Closes High finding #45 from `docs/product/production-readiness-audit-2026-09-08.md`.

## Who this is for

An operator running `embyr-server` on one host (VM or bare metal) without Kubernetes, Helm,
or ECS. `docker-compose.yml` at the repo root is explicitly dev/evaluation-only (see its own
header comment) — it is not this document. This is the reference path for a real single-host
production deployment: systemd for process supervision, a reverse proxy for TLS termination,
and a real (non-container) Postgres for the System DB.

## Non-Goals

- **Not** a Kubernetes/Helm/ECS guide — deferred by finding #19's own resolution
  (`docs/evolution/2026-09-15-deployment-release-process.md`) to a future
  `production-hosting-topology` item.
- **Not** multi-host / high-availability guidance. One host, one `embyr-server` process.
  (embyr-server itself is stateless behind Postgres, so horizontal scaling is possible later,
  but that's out of scope here.)
- **Not** automated deploy tooling (Ansible/Terraform playbooks). The commands below are
  manual, run-once-per-host setup, matching the "not automated" honesty convention already
  used in `docs/operations/runbook.md` and `docs/operations/backup-disaster-recovery.md`.

## Open item: where does the image come from

As of this writing, findings #42/#43 (CI never pushes `embyr-server`/`embyr-agent` images to
any registry — `docker build` only, then discarded) are still **Not started**. There is no
`ghcr.io`/ECR/Docker Hub location to pull from yet. Until that lands, build the image
yourself on (or for) the target host, following the same procedure
`docs/operations/release-process.md`'s Rollback Procedure already documents:

```bash
git checkout vX.Y.Z   # tag from `git tag -l` / CHANGELOG.md, per release-process.md
docker build -t embyr-server:vX.Y.Z .
```

Once #42/#43 close, replace the local `docker build` step below with `docker pull
<registry-TBD>/embyr-server:vX.Y.Z` — the rest of this document (systemd unit, proxy config,
port bindings) does not change.

## 1. Environment file

Populate `/etc/embyr/embyr-server.env` (mode `0600`, owned by the service user) using
`.env.example` at the repo root as the index of every variable `ServerConfig` reads. At
minimum:

```
DATABASE_URL=postgres://embyr:***@systemdb.internal:5432/embyr
EMBYR_ADMIN_KEY=...
EMBYR_ENCRYPTION_KEY=...   # 64 hex chars
GRPC_PORT=8080
REST_PORT=8081
ADMIN_PORT=9090
RUST_LOG=info
```

`DATABASE_URL` must point at a real, operator-managed Postgres instance — see
`docs/operations/backup-disaster-recovery.md` for System DB backup/PITR requirements before
you go live. Do not put a plaintext `EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY` in this file if
you'd rather source them from AWS/GCP Secrets Manager — see
[`secrets-manager-setup.md`](./secrets-manager-setup.md) for the `_AWS_SECRET_ARN`/
`_GCP_SECRET_NAME` variables that replace the plain values.

## 2. systemd unit

`embyr-server` ships only as a Docker image today (see "Open item" above) — the unit below
runs that image directly under systemd, which gives you `Restart=on-failure` and journald log
capture without a separate container orchestrator.

`/etc/systemd/system/embyr-server.service`:

```ini
[Unit]
Description=embyr-server (Firestore protocol translation layer)
After=network-online.target docker.service
Wants=network-online.target
Requires=docker.service

[Service]
Type=simple
EnvironmentFile=/etc/embyr/embyr-server.env
ExecStartPre=-/usr/bin/docker rm -f embyr-server
ExecStart=/usr/bin/docker run --rm --name embyr-server \
  --env-file /etc/embyr/embyr-server.env \
  -p 8080:8080 \
  -p 8081:8081 \
  -p 127.0.0.1:9090:9090 \
  embyr-server:vX.Y.Z
ExecStop=/usr/bin/docker stop -t 30 embyr-server
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

Key points:

- **`Restart=on-failure`** — systemd restarts the container on a non-zero exit (config error,
  DB probe failure, port bind failure — all the ADR-017 exit-code-1 paths). `RestartSec=5`
  avoids a tight crash loop.
- **`-p 127.0.0.1:9090:9090`** — the admin port is bound to loopback only, never to `0.0.0.0`.
  This directly satisfies ADR-001's requirement that admin "must be unreachable from the
  public network" (finding #53's own citation) with a one-line Docker port binding, no
  separate firewall rule needed. Do not change this to `-p 9090:9090`.
- **`docker run` without `-d`** — the container stays attached in the foreground, so its
  stdout/stderr become the unit process's stdout/stderr, which systemd captures into journald
  automatically via `StandardOutput=journal`/`StandardError=journal`. No `--log-driver` flag
  needed.
- **Log format** — `embyr-server` emits structured JSON log lines (finding #25,
  `docs/evolution/2026-09-19-structured-json-logging.md`) to stderr. journald stores each line
  as-is in the `MESSAGE` field; point a log shipper (Promtail, Vector, `systemd-journal-remote`)
  at journald if you need those JSON fields indexed downstream — this document does not set
  one up, matching the "no automation exists" convention in `runbook.md`.

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now embyr-server
sudo journalctl -u embyr-server -f
```

### Optional: `embyr-agent`

If this host also runs `embyr-agent` for a self-hosted, single-tenant `backend_mode=agent`
deployment, the same pattern applies with `crates/embyr-agent/Dockerfile`'s image, its own
`/etc/embyr/embyr-agent.env` (see `.env.example`'s `embyr-agent` section for
`EMBYR_AGENT_DB_DSN`/`EMBYR_AGENT_CERT`/`EMBYR_AGENT_KEY`/`EMBYR_AGENT_CA`), and a unit
binding only `EMBYR_AGENT_LISTEN_ADDR` (default `0.0.0.0:9191`) — no admin port exists on this
binary, so there's no equivalent loopback restriction to apply.

## 3. Reverse proxy (TLS termination)

`embyr-server` can terminate TLS itself (`EMBYR_TLS_CERT_PATH`/`EMBYR_TLS_KEY_PATH`, both-or-
neither — see `.env.example`), but the more common single-host pattern is a reverse proxy in
front, so cert renewal (e.g. Let's Encrypt/ACME) is decoupled from the application process.
This example uses nginx in front of `:8080` (gRPC) and `:8081` (REST/gRPC-Web/BrowserChannel).
**`:9090` (admin) is never proxied or exposed here** — it stays loopback-only per §2 above, and
an operator who needs remote admin access should reach it over SSH port-forwarding or a VPN,
not a public reverse-proxy route.

`/etc/nginx/sites-available/embyr.conf`:

```nginx
upstream embyr_grpc {
    server 127.0.0.1:8080;
    keepalive 32;
}

upstream embyr_rest {
    server 127.0.0.1:8081;
    keepalive 32;
}

server {
    listen 443 ssl http2;
    server_name grpc.embyr.example.com;

    ssl_certificate     /etc/letsencrypt/live/embyr.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/embyr.example.com/privkey.pem;

    location / {
        grpc_pass grpc://embyr_grpc;
        grpc_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    }
}

server {
    listen 443 ssl;
    server_name rest.embyr.example.com;

    ssl_certificate     /etc/letsencrypt/live/embyr.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/embyr.example.com/privkey.pem;

    location / {
        proxy_pass http://embyr_rest;
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}

# No server block for :9090 — intentionally absent.
```

`http2` on the gRPC server block requires nginx ≥ 1.25.1 for `grpc_pass` over plain `listen
443 ssl http2` (older nginx needs the separate `listen ... http2` directive form — check
`nginx -v`). If your gRPC clients need to reach the same hostname as REST, split by path
prefix or run gRPC on its own subdomain as shown above; `docs/operations/runbook.md`'s BrowserChannel sticky-routing note (ADR-001) applies if you later put a load
balancer in front of more than one host — out of scope for this single-host document.

Reload after editing:

```bash
sudo ln -s /etc/nginx/sites-available/embyr.conf /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx
```

## 4. Verify

```bash
curl -s https://rest.embyr.example.com/healthz   # readiness — 200 once DB probe passes
sudo journalctl -u embyr-server -n 50 --no-pager  # confirm JSON startup log, correct version
curl -s http://127.0.0.1:9090/healthz              # admin port only reachable locally
```

## Cross-References

- [`secrets-manager-setup.md`](./secrets-manager-setup.md) — sourcing `EMBYR_ADMIN_KEY`/
  `EMBYR_ENCRYPTION_KEY` from AWS/GCP instead of the plaintext env file above.
- [`release-process.md`](./release-process.md) — version/tag convention referenced by the
  `docker build -t embyr-server:vX.Y.Z` step above, and the rollback procedure this document
  extends with concrete restart mechanics (`systemctl restart embyr-server` after rebuilding
  the older tag).
- [`backup-disaster-recovery.md`](./backup-disaster-recovery.md) — System DB backup
  requirements for the Postgres instance `DATABASE_URL` points at.
- [`runbook.md`](./runbook.md) — incident triage once the service is running (pool exhaustion,
  rate-limiter degradation, gRPC error spikes).
- ADR-001 (Process Topology) — the "`:9090` must be unreachable from the public network"
  requirement this document's loopback binding and absent nginx block satisfy.
- ADR-017 (Production Startup) — the env vars, exit-code-1 startup failure semantics, and
  graceful-shutdown (SIGTERM) behavior the systemd unit's `Restart=`/`ExecStop=` rely on.
