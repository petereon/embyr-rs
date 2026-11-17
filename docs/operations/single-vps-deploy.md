# Single-VPS Deployment (closed beta, one operator, minimum cost)

Copy-paste runbook from a blank VPS to a working admin console and data plane on **one small
box** (2 vCPU / 4 GB, Debian 12 or Ubuntu 22.04/24.04, e.g. Hetzner): `embyr-server`, its system
Postgres and Caddy (TLS), all under Docker Compose. The bundle lives in
[`deploy/single-vps/`](../../deploy/single-vps/).

> **Status: written, not run.** None of the compose file, Caddyfile or scripts has been executed
> (no Docker, Caddy or shellcheck was available when they were written). Expect to fix small
> things on the first real run; the "Untested" list in the PR that added this lists what to
> watch. Every step below ends with a check that tells you whether it worked.

This document links to existing docs instead of repeating them:
[`single-host-deployment.md`](./single-host-deployment.md) (the nginx + systemd + external-Postgres
variant),
[`network-topology.md`](./network-topology.md), [`topology.md`](./topology.md),
[`resource-sizing.md`](./resource-sizing.md), [`runbook.md`](./runbook.md),
[`release-process.md`](./release-process.md),
[`backup-disaster-recovery.md`](./backup-disaster-recovery.md),
[`migration-policy.md`](./migration-policy.md),
[`secrets-manager-setup.md`](./secrets-manager-setup.md), [`slo.md`](./slo.md),
[`alert-rules.yml`](./alert-rules.yml).

## What you are building

```
 internet ──443/80──> Caddy ──h2c──> embyr-server :8080   native gRPC      (api.<domain>)
                        │      └───> embyr-server :8081   gRPC-Web, REST, BrowserChannel, /healthz
                        └──────────> embyr-server :9090   console SPA + session API (allowlist!)
                                          │ (db network, no internet, no host port)
                                       Postgres 15 (system DB, named volume)
 host loopback 127.0.0.1:9090 ──> embyr-server :9090   operator API + /metrics (you, over SSH only)
 off-box: rclone remote (B2/S3, ideally crypt)  <── nightly pg_dump
```

| Bundle file | Purpose |
|---|---|
| `docker-compose.prod.yml` | the three services, networks, limits, healthchecks, log rotation |
| `Caddyfile` | TLS, host routing, **the public path allowlist for :9090** |
| `.env.production.example` | every variable, with generation commands |
| `backup.env.example` | rclone remote, retention, heartbeat URLs |
| `scripts/` | `firewall.sh`, `backup.sh`, `restore.sh` (+ `restore-test.md`), `update.sh`, `check.sh`, `verify-exposure.sh`, systemd backup unit + timer |

Hostnames: **`api.<domain>`** is the Firestore data plane (SDK clients connect here).
**`console.<domain>`** is the admin console (the SPA is served at `/admin/`).

## 0. Decide and gather (15 minutes, before touching the server)

- A domain you control and can add A records to.
- A VPS: 2 vCPU / 4 GB / 40 GB+ disk, Debian 12 or Ubuntu LTS, with your SSH public key
  installed at creation time. Note its IPv4 address.
- An SMTP relay (invites and password resets are emails; the server refuses to start without
  one, `EMBYR_REQUIRE_SMTP=true`). SES, Brevo, Resend, Postmark all have free/cheap tiers.
- An S3-compatible bucket for backups (Backblaze B2 is the cheapest common choice) and its keys.
- A password manager entry ready for `EMBYR_ENCRYPTION_KEY` (step 6).
- Which image to run: a `sha-<7 hex>` tag of a green `master` build, or a `vX.Y.Z` release tag
  ([`release-process.md`](./release-process.md) → Container Registry). Never `master`.
- GitHub access to pull the image: the ghcr.io package is private until a repo admin makes it
  public, so create a classic PAT with only `read:packages`.

## 1. DNS

Create **A records only**, TTL 300 while you are setting up:

| Name | Type | Value |
|---|---|---|
| `api.<domain>` | A | VPS IPv4 |
| `console.<domain>` | A | VPS IPv4 |

- **No AAAA records.** Docker's userland proxy hides the real source address of IPv6 clients
  from published ports, so every IPv6 user would share one IP in embyr's sign-in throttles. Add
  AAAA only after you have enabled and tested Docker IPv6 properly.
- Verify before starting Caddy (it requests certificates immediately; failed attempts count
  against Let's Encrypt rate limits): `dig +short api.<domain> console.<domain>` shows the VPS IP.
- Do not put Cloudflare's orange-cloud proxy in front without changing
  `EMBYR_TRUSTED_PROXY_HOPS` and the Caddyfile together (gRPC and long-lived streams also have
  limits there).

## 2. Harden the server

As root on first login (replace the key and names):

```bash
# a non-root deploy user with sudo and your key
adduser --disabled-password --gecos "" deploy
usermod -aG sudo deploy
install -d -m 700 -o deploy -g deploy /home/deploy/.ssh
cp /root/.ssh/authorized_keys /home/deploy/.ssh/authorized_keys
chown deploy:deploy /home/deploy/.ssh/authorized_keys && chmod 600 /home/deploy/.ssh/authorized_keys
echo 'deploy ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/deploy && chmod 440 /etc/sudoers.d/deploy

# keys only, no root login
cat > /etc/ssh/sshd_config.d/10-hardening.conf <<'EOF'
PermitRootLogin no
PasswordAuthentication no
KbdInteractiveAuthentication no
EOF
sshd -t && systemctl reload ssh      # (service name is "ssh" on Debian/Ubuntu)
```

**Open a second terminal and log in as `deploy` before closing the root session.**

As `deploy`:

```bash
sudo apt-get update && sudo apt-get -y upgrade
sudo apt-get install -y ufw rclone curl ca-certificates unattended-upgrades
sudo timedatectl set-ntp true

# 1 GB swap: Argon2 sign-in bursts (64 MiB each) and pg_dump can spike; swap beats the OOM killer
sudo fallocate -l 1G /swapfile && sudo chmod 600 /swapfile && sudo mkswap /swapfile && sudo swapon /swapfile
echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab
echo 'vm.swappiness=10' | sudo tee /etc/sysctl.d/99-swappiness.conf && sudo sysctl --system
```

The firewall comes after the bundle is copied (step 5). Optional: `sudo apt-get install -y
fail2ban` (default sshd jail needs no config; it helps little if SSH is already key-only and
restricted to your IP by ufw).

### Unattended security updates

```bash
sudo dpkg-reconfigure -plow unattended-upgrades          # answer Yes
sudoedit /etc/apt/apt.conf.d/50unattended-upgrades       # optional, see below
```

- Default origins are the distro's **security** pocket only: kernel, openssl, openssh arrive
  automatically. Docker Engine (from Docker's apt repo) and your images do **not** auto-update:
  `sudo apt-get upgrade` monthly, and update images with `scripts/update.sh`.
- Kernel updates need a reboot. Either reboot by hand monthly, or set
  `Unattended-Upgrade::Automatic-Reboot "true";` and `Unattended-Upgrade::Automatic-Reboot-Time "04:30";`
  (a reboot is a 1-2 minute outage; containers use `restart: unless-stopped` and come back on
  their own; the Postgres container gets 60 s to shut down cleanly).
- Check it works: `sudo unattended-upgrade --dry-run -d | tail`.

## 3. Install Docker Engine + Compose

Official apt repository (works for Debian and Ubuntu; see docs.docker.com/engine/install for
changes):

```bash
sudo install -m 0755 -d /etc/apt/keyrings
. /etc/os-release
sudo curl -fsSL "https://download.docker.com/linux/$ID/gpg" -o /etc/apt/keyrings/docker.asc
echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/$ID ${UBUNTU_CODENAME:-$VERSION_CODENAME} stable" \
  | sudo tee /etc/apt/sources.list.d/docker.list >/dev/null
sudo apt-get update
sudo apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
sudo usermod -aG docker deploy      # log out and back in; "docker" group is root-equivalent: keep deploy's SSH key private
docker compose version              # a recent v2 (the scripts use `up --wait` and `ps --format`)
```

## 4. Copy the bundle

From your laptop, inside a checkout of this repo (the bundle is independent of the image tag; it
changes rarely, and updating it is a plain re-run of this command):

```bash
rsync -av --exclude .env.production --exclude backup.env --exclude backups/ \
  deploy/single-vps/ deploy@<vps-ip>:/opt/embyr/
```

(`sudo mkdir /opt/embyr && sudo chown deploy: /opt/embyr` first, on the server.) Everything below
runs in `/opt/embyr` as `deploy`. Define a shortcut for the long compose command (the scripts
bring their own):

```bash
cd /opt/embyr
dc() { docker compose --env-file .env.production -f docker-compose.prod.yml "$@"; }
```

## 5. Firewall

```bash
# your current public IP as seen by the server: `who -m` / the SSH_CLIENT variable
sudo SSH_SOURCE="${SSH_CLIENT%% *}/32" ./scripts/firewall.sh      # or SSH_SOURCE=any
```

Allows SSH from that source, 80 and 443 from anywhere, denies the rest. **Know what it does not
do:** Docker-published ports bypass ufw. The compose file therefore publishes only 80, 443 and
`127.0.0.1:9090`. After every compose edit, `sudo ss -tlnp` must show nothing public except 22,
80, 443. Test a second SSH session before closing the first. If your home IP changes, use the
provider's web console to reach the box.

## 6. Fill the environment

```bash
cp .env.production.example .env.production
chmod 600 .env.production
# generate; paste the output into .env.production (never into chat, tickets or git):
openssl rand -hex 24     # POSTGRES_PASSWORD
openssl rand -hex 32     # EMBYR_ADMIN_KEY
openssl rand -hex 32     # EMBYR_ENCRYPTION_KEY
$EDITOR .env.production  # also: DOMAIN, ACME_EMAIL, EMBYR_IMAGE_TAG, SMTP, CORS origins
grep -n REPLACE_ME .env.production    # must print nothing
```

**Back up `EMBYR_ENCRYPTION_KEY` right now**, before the first start: copy it into your password
manager (and, ideally, one offline copy: printed or on a USB stick in another place). Never in
the backup bucket, never in the same place as the database dumps. The database holds only
ciphertext: **if the key is lost, every stored customer DSN is unrecoverable, every user's TOTP
secret is undecryptable (nobody can sign in), and no restore can fix it**
([`backup-disaster-recovery.md`](./backup-disaster-recovery.md) → Encryption-Key Recovery
Precondition). Rotation procedure: [`secrets-manager-setup.md`](./secrets-manager-setup.md) /
ADR-018. `EMBYR_ADMIN_KEY` is replaceable (rotate with `EMBYR_ADMIN_KEY_PREVIOUS`); the
encryption key is not.

Notes on the variables that need judgement (everything else has its reasoning in the example
file):

- **`EMBYR_TRUSTED_PROXY_HOPS=1`**: Caddy is exactly one trusted hop, and the Caddyfile replaces
  `X-Forwarded-For` with the single address Caddy saw. Leaving it at 0 would make every sign-in
  and invitation throttle key on Caddy's container address (one shared bucket for the world).
- **Stripe is left unset.** What the server does exactly in that state is spelled out in the
  example file: webhook route not mounted, a harmless WARN at every boot, and the Upgrade
  buttons answer 502, so do not offer paid plans yet. Setting `STRIPE_SECRET_KEY` without
  `STRIPE_WEBHOOK_SIGNING_SECRET` refuses startup.
- **`EMBYR_PUBLIC_BASE_URL`, `DATABASE_URL`, `EMBYR_ALLOW_PRIVATE_DSN_HOSTS`** are set by the
  compose file, not the env file, so they cannot drift or be switched on by accident.

Backup config:

```bash
cp backup.env.example backup.env && chmod 600 backup.env
rclone config            # create the B2/S3 remote, then a "crypt" remote on top of it
$EDITOR backup.env       # BACKUP_REMOTE=cryptremote:embyr-prod, optional PING_URL
```

## 7. Start

```bash
echo "$GITHUB_PAT" | docker login ghcr.io -u <github-username> --password-stdin   # read:packages PAT
# optional but cheap: validate the Caddyfile syntax before anything touches the internet
dc run --rm --no-deps caddy caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
dc pull
dc up -d
dc ps                      # postgres healthy, embyr-server healthy (can take ~60 s), caddy running
dc logs --tail 50 embyr-server
```

Expected in the server log: a JSON line like `embyr-server vX.Y.Z ready grpc=0.0.0.0:8080
rest=0.0.0.0:8081 admin=0.0.0.0:9090`, a WARN that TLS is disabled on the listeners (correct:
Caddy terminates TLS; the listeners are on a private network), and a WARN that the Stripe probe
failed (correct while Stripe is unset). Caddy's log shows certificates being obtained for both
hostnames.

To test without burning Let's Encrypt limits, uncomment `acme_ca` (staging) in the Caddyfile
first; the browser will warn about the staging CA, which is expected.

**Verify:**

```bash
curl -s https://api.<domain>/healthz            # {"status":"ok"}
./scripts/verify-exposure.sh                    # every line "ok"; this is the :9090 allowlist test
sudo ss -tlnp | grep -E ':(80|443|9090|8080|8081|5432)\b'
#   80/443 on *, 9090 on 127.0.0.1 only, nothing on 8080/8081/5432
```

Open `https://console.<domain>/`: it redirects to `/admin/` and shows the sign-in page.

## 8. Which console routes are public (and what to re-check)

The admin port serves three kinds of routes on the same listener, and the internet may only see
the first:

| Kind | Examples | On `console.<domain>` |
|---|---|---|
| Static SPA | `/admin/`, `/admin/*` (not `/admin/v1`) | exposed |
| Session / public API used by the SPA | `/admin/v1/auth/*`, `/admin/v1/projects[/...]`, `/members`, `/service_accounts`, `/admin_keys`, `/account`, `/metrics/summary`, `/billing`, `/billing/{subscription,checkout_session,portal_session}` | exposed |
| Stripe webhook | `POST /admin/v1/webhooks/stripe` (only exists when Stripe is configured) | exposed |
| **Operator only** | `/admin/v1/accounts` (bootstrap), `/admin/v1/billing/run-metering`, `/admin/v1/projects/:id/suspend`, `.../activate` | **404** |
| **Operator / internal** | `/metrics`, `/healthz`, `/livez` (on this host), `/admin/v1/oidc_providers` (unused; OIDC sign-in is disabled server-side) | **404** |
| Anything not listed | | **404** (default deny) |

The Caddyfile header comments hold the authoritative list and the reasons. The operator routes
are also protected by `EMBYR_ADMIN_KEY` in the server; the proxy allowlist is the second layer,
so the internet cannot even try keys against them. Residual risk by design: the two dual-auth
project routes also accept the operator key as Bearer, and session routes accept admin-API
keys as Bearer; they cannot be hidden without breaking the SPA.

**When `crates/embyr-server/src/admin/router.rs` (or the `/healthz`/static-UI wiring in
`main.rs`) changes, before you deploy that release:**

1. `git diff <old-tag>..<new-tag> -- crates/embyr-server/src/admin/router.rs`, and list routes with
   `grep -nE '\.route\(|operator_router|bootstrap_router|session_router|dual_auth_router|public_router' crates/embyr-server/src/admin/router.rs`.
2. A new route in `operator_router` or `bootstrap_router` → add its path to `@operator` in the
   Caddyfile. A new top-level prefix in a session/public router → add it to `@spa_api`, or the
   SPA gets 404s. A new sub-route under an exposed prefix is exposed automatically: confirm it is
   session-authed.
3. Add a line for it to `scripts/verify-exposure.sh`, `dc exec caddy caddy reload --config
   /etc/caddy/Caddyfile`, run the script.

`update.sh` runs `verify-exposure.sh` after every successful update and warns loudly if it
fails; it cannot protect a route it does not know about, so step 2 is manual.

## 9. First operator account (bootstrap) and design partners

There is no account until the operator creates one. `POST /admin/v1/accounts` is
operator-only, so it is **not** reachable through the public hostname: call it **from the VPS**
through the loopback-published admin port. The call creates the account plus one Owner
**invitation**; no password or TOTP secret ever passes through you (feature design:
[`docs/feature/account-bootstrap-and-invitations/feature-delta.md`](../feature/account-bootstrap-and-invitations/feature-delta.md)).

```bash
cd /opt/embyr
# read the key into a shell variable (not into your shell history), and pass it to curl via a
# process substitution so it never shows up in `ps`
ADMIN_KEY="$(grep -m1 '^EMBYR_ADMIN_KEY=' .env.production | cut -d= -f2-)"
opcurl() { curl -sS -w '\nHTTP %{http_code}\n' -H @<(printf 'Authorization: Bearer %s\n' "$ADMIN_KEY") "$@"; }

opcurl -X POST http://127.0.0.1:9090/admin/v1/accounts \
  -H 'content-type: application/json' \
  -d '{"account_name":"Acme beta","owner_email":"you@example.com"}'
# 201 {"account_id":"...","invitation_id":"...","invite_token":"<TOKEN>","expires_at":"..."}
```

Responses: `201` ok; `409` the email already has a user or open invitation; `422` bad email or
empty name; `401/403` wrong key. The token is shown **once** and the invitation is valid for 7
days. If SMTP works the owner also gets an email, but it only contains a relative link, so send
the full URL yourself:

```
https://console.<domain>/admin/?invite=<TOKEN>
```

The owner opens it and completes two steps: set a display name and a password of at least 12
characters, enrol an authenticator app (scan the otpauth link or enter the secret), then enter a
current TOTP code. The server then shows **10 recovery codes once**: they must be saved. After
that, sign in normally at `https://console.<domain>/admin/`.

Troubleshooting the invitation: five wrong TOTP codes invalidate the pending enrolment, and the
owner simply redoes the first step with the same link. An unknown, expired or used token always
gives the same "invalid or expired" answer, on purpose. An email with an open invitation cannot
be invited again (`409`), and there is no operator revoke call (revoking is a session route), so
if a token is lost before use, either bootstrap with another email, wait out the 7 days, or
revoke it directly in the database from the VPS:

```bash
dc exec -T postgres psql -U embyr -d embyr -c \
  "UPDATE invitations SET revoked_at = now() WHERE lower(email) = lower('you@example.com') AND accepted_at IS NULL AND revoked_at IS NULL"
```

**Inviting a design partner** = the same call with their account name and email: each partner is
its own account with its own Owner, and that Owner invites their own team from the console
(Members). Use your own address first as a dry run. Lost authenticator device: see
[`runbook.md`](./runbook.md) → "Support: admin user lost their TOTP device".

**Creating the partner's database (project):** the Owner uses "new database" in the console.
Their data lives in **their own Postgres**, not in your system DB: it must be reachable from
your VPS over the internet by public DNS name or IP and require TLS (`sslmode=require` or
stricter); private/loopback addresses and non-TLS DSNs are rejected by design (audit #65), which
also means a partner cannot point at your system Postgres. The schema must be prepared once with
`embyr-db-prep` (ADR-022; build it from this repo, `cargo run --release -p embyr-db-prep` with
`EMBYR_DB_PREP_DSN`), otherwise provisioning answers `customer_db_not_prepped`. The project's API
key is shown **once** at creation.

## 10. Smoke test with a real client

Native gRPC against `api.<domain>:443` with the project's API key as a Bearer token
(`authorization: Bearer <api key>`; the database name is `projects/<project_id>/databases/(default)`).
`grpcurl` is the lowest-friction real gRPC client; the proto files are in the repo's `proto/`:

```bash
export HOST=api.<domain>:443 PROJECT=<project_id> KEY=<api key>     # do not paste the key into shared logs
DB="projects/$PROJECT/databases/(default)"
grpcurl -import-path proto -proto google/firestore/v1/firestore.proto \
  -H "authorization: Bearer $KEY" \
  -d "{\"database\":\"$DB\",\"writes\":[{\"update\":{\"name\":\"$DB/documents/smoke/hello\",\"fields\":{\"msg\":{\"stringValue\":\"hi\"}}}}]}" \
  "$HOST" google.firestore.v1.Firestore/Commit
grpcurl -import-path proto -proto google/firestore/v1/firestore.proto \
  -H "authorization: Bearer $KEY" \
  -d "{\"name\":\"$DB/documents/smoke/hello\"}" \
  "$HOST" google.firestore.v1.Firestore/GetDocument
```

Both calls should succeed (the read returns the document you wrote). Then repeat once with a
**Firestore SDK** of the partner's choice, configured with host `api.<domain>`, TLS on, project
id = `<project_id>` and the API key as the bearer credential: the SDK is what the partner will
really run, and it exercises `Listen` streams, which `grpcurl` does not. Also open a browser
client once if the partner uses the web SDK (it falls back to BrowserChannel, which also needs
their origin in `EMBYR_CORS_ALLOWED_ORIGINS`; edit `.env.production`, then
`dc up -d --no-deps embyr-server`).

Failure map: TLS or DNS errors → step 1/7 (Caddy log); `Unauthenticated` → wrong key or project
id; `backend_unavailable`/`Unavailable` → the partner's database is unreachable from the VPS;
a hang on `Listen` through a web client → check `docker compose logs caddy` for timeouts and that
no extra proxy sits in front.

## 11. Backups, and the first restore drill

```bash
./scripts/backup.sh                     # run once by hand: must end with "backup OK" and a file in the remote
( set -a; . ./backup.env; set +a; rclone lsf --files-only "$BACKUP_REMOTE/daily" )
sudo cp scripts/embyr-backup.service scripts/embyr-backup.timer /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now embyr-backup.timer
systemctl list-timers embyr-backup.timer
```

The unit assumes the bundle is in `/opt/embyr` and the user is `deploy`; edit the `.service` if
not. A cron alternative is in the comments of that file. What you get: a custom-format
`pg_dump` of the **system DB** every night, uploaded to the remote, newest 7 daily + 4 weekly
kept (count-based, so a silent outage never prunes your last good files), exit non-zero and a
`/fail` ping on any error. **RPO is up to 24 hours, not minutes**: there is no WAL archiving or
PITR on this setup; run the timer more often if that is too long (see the timer file).
Customer databases are the customers' responsibility, as in
[`backup-disaster-recovery.md`](./backup-disaster-recovery.md).

Do the **restore drill now and then monthly**: [`scripts/restore-test.md`](../../deploy/single-vps/scripts/restore-test.md)
(restores the newest dump into a scratch database, never touches production, and includes the
encryption-key check, which is the part that actually loses data).

## 12. Updating (manual, by you)

```bash
./scripts/update.sh sha-1a2b3c4          # or vX.Y.Z. Never master.
```

It validates the tag, pulls the image, takes a backup (aborts if that fails), switches
`EMBYR_IMAGE_TAG`, waits for the container healthcheck, and **rolls back to the previous tag
automatically** if the new one is not healthy within 120 s. History: `deploy-history.log`.

Why rolling the image back is safe: migrations are forward-only and run at server start, but
both migrators use sqlx `ignore_missing`, so the previous image boots against a newer schema
([`migration-policy.md`](./migration-policy.md), "Older binary on a newer schema"; the
expand/contract rule and its CI lint keep the old code compatible). If a release's migration
damaged data, an image rollback is not enough: `./scripts/restore.sh --prod --latest` from the
pre-update backup (accepting the data-loss window). Read the release's `CHANGELOG.md` entry
before updating. If the route files changed, do step 8's re-check first.

Expect a few seconds of failed requests during the restart (Caddy retries connections for 10
s; gRPC and Listen clients reconnect on their own; BrowserChannel clients re-handshake: see
"does not give you"). Do not update during a partner demo.

Other updates: Postgres/Caddy minor versions `dc pull postgres caddy && dc up -d` (never change
Postgres to a new **major** by editing the tag: dump, replace the volume, restore). OS packages:
monthly `sudo apt-get upgrade`.

## 13. Monitoring on a budget

**Do these three, all free, in the first hour:**

1. **External uptime monitor** (UptimeRobot, Better Stack free, or similar, 1-5 min interval)
   with two checks: `https://api.<domain>/healthz` (expects 200 and `ok`; this is the real
   readiness endpoint: it queries the system DB with a 3 s timeout) and
   `https://console.<domain>/admin/` (expects 200). The console host deliberately has no
   `/healthz`; the API host's one is the same handler. Alert to email and a phone push.
2. **Heartbeat for the box and backups**: a free [healthchecks.io](https://healthchecks.io)
   account, two checks. Put one URL in `backup.env` as `PING_URL` (backup success/fail) and one as
   `CHECK_PING_URL` (below). When the VPS dies or cron stops, the pings stop and you get mailed.
3. **Local checker**: `./scripts/check.sh` (containers running/healthy, public `/healthz`,
   disk under 85%, newest backup under 36 h) every 5 minutes:
   `*/5 * * * * /opt/embyr/scripts/check.sh >>/opt/embyr/backups/check.log 2>&1` in the `deploy`
   user's crontab (`mkdir -p /opt/embyr/backups` first).

On-host, the metrics are one command away for ad-hoc looks:
`opcurl http://127.0.0.1:9090/metrics | head -50` (the `opcurl` helper from step 9).

**Mapping to [`alert-rules.yml`](./alert-rules.yml).** Those rules are PromQL on `/metrics`, so
**none of them evaluates without Prometheus**. What replaces them:

| Alert | Without Prometheus |
|---|---|
| `EmbyrTargetDown`, `EmbyrMetricsAbsent` | external monitor on `api.<domain>/healthz` + heartbeats (covers process down, box down) |
| `EmbyrReadinessFailing` (commented out in the file) | the same external `/healthz` check is exactly what it wants |
| `EmbyrRestartLoop` | `check.sh` flags a `restarting`/unhealthy container; `docker inspect -f '{{.RestartCount}}' $(docker compose ps -q embyr-server)` |
| `EmbyrSystemDbPoolExhausted` | partly: pool exhaustion makes `/healthz` slow or 503 |
| `EmbyrRateLimiterPg*`, `EmbyrSweeperFailingRepeatedly`, `EmbyrGrpcErrorRateHigh`, `EmbyrGrpcLatencySLOBreach` | **not covered.** You learn about these from users, or from `dc logs` (JSON lines) |

**Recommendation: skip Prometheus for the first beta**, and add the free Grafana Cloud tier when
you want error-rate and latency alerts (the SLOs in [`slo.md`](./slo.md) need metrics). The
lightest route is a Grafana Alloy container on the `edge` network scraping
`http://embyr-server:9090/metrics` with the admin key as a bearer token and `remote_write`ing to
Grafana Cloud, with `job_name = "embyr-server"` so the rule file's selectors match. Alloy costs
roughly 150-250 MB of RAM, which is most of your headroom, so give it a memory limit and watch
`free -m`. Sketch, **untested**:

```
prometheus.scrape "embyr" {
  job_name     = "embyr-server"
  targets      = [{"__address__" = "embyr-server:9090"}]
  metrics_path = "/metrics"
  authorization { type = "Bearer", credentials_file = "/secrets/admin_key" }
  forward_to   = [prometheus.remote_write.cloud.receiver]
}
prometheus.remote_write "cloud" {
  endpoint { url = "<from your Grafana Cloud stack>", basic_auth { username = "<id>", password_file = "/secrets/gc_token" } }
}
```

(Grafana Cloud then evaluates the rules from `alert-rules.yml`, and the dashboard is
`grafana-dashboard.json`; see [`runbook.md`](./runbook.md) → "What's actually exported".) If you
rotate `EMBYR_ADMIN_KEY`, update the secret file Alloy reads.

## 14. When something is wrong

First: `./scripts/check.sh`, `dc ps`, `dc logs --tail 100 embyr-server`. Then the matching section of
[`runbook.md`](./runbook.md) (Postgres down, readiness failing, restart loop, key compromise,
lost TOTP). Quick table:

| Symptom | Likely cause |
|---|---|
| Caddy cannot get a certificate | DNS not pointing at the VPS yet, port 80/443 blocked at the provider's firewall, or Let's Encrypt rate limit (use staging) |
| embyr-server restart loop | wrong `EMBYR_ENCRYPTION_KEY` length (needs 64 hex), admin key under 32 chars, SMTP half-configured or unset with `REQUIRE_SMTP=true`; the first log line says which |
| Console sign-in loops | cookie is `Secure`: you must use `https://console.<domain>`, not an IP or http |
| Everyone gets 429 on sign-in | `EMBYR_TRUSTED_PROXY_HOPS` is 0 (all traffic keyed on Caddy's address) |
| Invites "sent" but not received | SMTP credentials/URL encoding; the server logs the delivery failure; use the token from the bootstrap/invite response |
| Out-of-memory kills | `dmesg -T | grep -i oom`, `docker stats --no-stream`; sign-in bursts cost 64 MiB each; check swap is on |
| Disk filling | `docker system df`, `du -sh /var/lib/docker/volumes/*`, old images (`docker image prune -a` is safe for tags not in use) |

## 15. Rebuilding on a new VPS (provider failure)

1. Steps 0-5 on a fresh VPS; copy `.env.production` and `backup.env` from your password manager
   or secure copy (the encryption key must be the original).
2. `dc pull && dc up -d postgres`, then `./scripts/restore.sh --prod --latest`.
3. `dc up -d`; move the DNS A records (TTL 300) to the new IP; run `verify-exposure.sh`.

Budget a couple of hours, plus up to 24 h of lost data (last backup).

## Cost (approximate; check each provider's page, prices change)

| Item | Approx. cost |
|---|---|
| VPS 2 vCPU / 4 GB shared (Hetzner CX-class or similar) | roughly 4-8 EUR/month |
| Backup storage (B2/S3; dumps are small, well under 1 GB for a beta) | about 0-1 USD/month (B2 has a free allowance) |
| Domain | roughly 10-15 USD/year |
| SMTP relay (SES, Brevo, Resend free tiers) | 0-1 USD/month at beta volume |
| Uptime monitor + healthchecks.io | free tiers |
| Optional: provider snapshots/backups of the VPS | roughly 20% of the VPS price |
| Optional: Grafana Cloud free tier | free, within its limits |
| **Total** | **roughly 6-12 EUR/month** |

## What this does NOT give you

- **No high availability.** One VM, one Postgres, one embyr-server: any of them failing is an
  outage until you fix or rebuild. Provider maintenance, a bad kernel update or your own `docker
  compose down` is downtime (target: minutes, but nothing guarantees it).
- **No point-in-time recovery.** RPO is the backup interval (up to 24 h with the default timer).
  Restoring means `restore.sh`, an outage, and the loss of everything since the last dump.
- **Single point of failure for the data plane too:** every request touches the system DB
  (rate limiter, credential lookups); there is no replica to fall back to.
- **BrowserChannel works only because there is one replica.** Its sessions live in the memory
  of the process that handled the handshake ([`network-topology.md`](./network-topology.md) §3).
  Every restart (updates included) drops web-SDK sessions; the SDK re-handshakes and re-listens.
  Do not add a second replica without reading that section and
  [`topology.md`](./topology.md).
- **Manual upgrades.** Nothing watches for new releases; you run `update.sh` and read the
  changelog. Docker Engine and Caddy/Postgres images are also on you.
- **You are on call.** There is no pager rotation; the external monitor emails you. Alerts that
  need Prometheus (error rate, latency SLOs) are off until you add metrics.
- **Not covered by this bundle:** customer database backups (theirs), a WAF or DDoS protection
  (a shared 2-vCPU box can be saturated by a determined client; Argon2 sign-in bursts cost
  64 MiB and a core each), log shipping (logs are rotated JSON files on the box:
  `docker compose logs`), staging environment, secrets-manager integration (env file only; see
  [`secrets-manager-setup.md`](./secrets-manager-setup.md) if you outgrow it), Kubernetes.
- **It is not the hardened single-host recipe.** [`single-host-deployment.md`](./single-host-deployment.md)
  recommends an operator-managed Postgres on separate infrastructure; here Postgres shares the
  box (and its disk) with the app to save money. Moving to a managed Postgres later means
  dropping the `postgres` service, setting `DATABASE_URL` in the compose file, and restoring a
  dump; it is the first upgrade to make when the beta turns into real revenue.
