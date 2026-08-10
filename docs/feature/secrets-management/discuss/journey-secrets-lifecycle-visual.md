# Journey (Visual) — Secrets Lifecycle

> Feature: secrets-management
> Persona: Sam Chen (P2 — Service Operator / Platform Engineer)
> Goal: Source EMBYR_ADMIN_KEY and EMBYR_ENCRYPTION_KEY from a secrets manager, and rotate
> either one in production without an outage or orphaned data.
> Wave: DISCUSS
> Updated: 2026-08-09

---

## Emotional Arc

```
START                    MIDDLE                              END
Anxious                  Focused / Cautious                  Confident / In Control
"One leak away from      "Watching logs during the           "That was routine, not
 a coordinated            rotation window, ready to            a fire drill. Nothing
 flag-day outage"         roll back if anything looks off"     broke, nothing orphaned."
```

No jarring transitions: every step gives Sam an explicit, observable checkpoint
(a log line, an HTTP status, a successful client call) before he proceeds to the next step.
The riskiest moment — flipping the CURRENT key during rotation — is bracketed by two
checkpoints Sam already trusts (secrets-manager fetch succeeded; old clients still work).

---

## Flow Diagram

```
[Trigger: security policy requires        [Step 1: Configure secrets-       [Step 2: Server starts,
 secrets-manager sourcing, OR              manager sourcing]                 fetches both secrets]
 planned/forced key rotation]         →                                →
  Feels: anxious, exposed                   Feels: cautious, methodical        Feels: watchful
  Artifacts: deployment manifest            Artifacts: *_AWS_SECRET_ARN /      Artifacts: startup log,
                                             *_GCP_SECRET_NAME env vars        "embyr-server ready"

        ↓                                          ↓                                 ↓

[Step 3: Rotate                           [Step 4: Rotate                    [Step 5: Close the
 EMBYR_ENCRYPTION_KEY]                     EMBYR_ADMIN_KEY]                   rotation window]
  Feels: focused, methodical                Feels: focused, in control          Feels: confident, relieved
  Artifacts: EMBYR_ENCRYPTION_KEY_          Artifacts: EMBYR_ADMIN_KEY_         Artifacts: manifest with
  PREVIOUS; existing TOTP/OIDC/DSN          PREVIOUS; existing operator         *_PREVIOUS vars removed
  ciphertext                                clients + /metrics scraper

                                                                                        ↓

                                                                          [Goal: both secrets are
                                                                           secrets-manager-sourced
                                                                           and rotatable on demand]
                                                                            Feels: routine, not risky
```

---

## Step 1: Configure secrets-manager sourcing

```
+-- Step 1: Configure secrets-manager sourcing -----------------------------+
| $ kubectl apply -f embyr-server-deployment.yaml                          |
|                                                                           |
| # deployment manifest excerpt — no literal admin key or encryption key   |
| env:                                                                     |
|   - name: DATABASE_URL                                                  |
|     value: postgres://embyr:***@pg.internal:5432/embyr                  |
|   - name: EMBYR_ADMIN_KEY_AWS_SECRET_ARN                                |
|     value: arn:aws:secretsmanager:us-east-1:123456789012:secret:${admin_secret_ref}  |
|   - name: EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN                           |
|     value: arn:aws:secretsmanager:us-east-1:123456789012:secret:${encryption_secret_ref} |
|                                                                           |
| deployment.apps/embyr-server configured                                 |
+---------------------------------------------------------------------------+
```

Emotional state: entry = anxious ("is this manifest actually free of literal secrets now?");
exit = cautious relief (manifest review shows zero literal key values).

---

## Step 2: Server starts, fetches both secrets

```
+-- Step 2: Startup — secrets fetched, server ready ------------------------+
| $ kubectl logs deploy/embyr-server -f                                    |
|                                                                           |
| [INFO] fetched EMBYR_ADMIN_KEY from AWS Secrets Manager (arn=${admin_secret_ref})       |
| [INFO] fetched EMBYR_ENCRYPTION_KEY from AWS Secrets Manager (arn=${encryption_secret_ref}) |
| [INFO] migrations applied: 18                                            |
| [INFO] embyr-server ready grpc=0.0.0.0:8080 rest=0.0.0.0:8081 admin=0.0.0.0:9090 |
|                                                                           |
| $ curl -s -o /dev/null -w "%{http_code}\n" \                             |
|     -H "Authorization: Bearer $(aws secretsmanager get-secret-value ...)" \  |
|     https://embyr.internal:9090/metrics                                  |
| 200                                                                       |
+---------------------------------------------------------------------------+
```

Note: log lines name the ARN/resource reference (`${admin_secret_ref}`), never the fetched
secret value itself — see shared-artifacts-registry.md for the log-safety invariant.

Emotional state: entry = cautious; exit = watchful confidence (server is up, first
authenticated request succeeds using the secrets-manager-sourced key).

---

## Step 3: Rotate EMBYR_ENCRYPTION_KEY

```
+-- Step 3: Rotate the encryption key (dual-key window) --------------------+
| # Sam adds the retiring key as EMBYR_ENCRYPTION_KEY_PREVIOUS,            |
| # and points EMBYR_ENCRYPTION_KEY at the new secret version.            |
| $ kubectl apply -f embyr-server-deployment-rotate-enc.yaml               |
| $ kubectl logs deploy/embyr-server -f                                    |
| [INFO] fetched EMBYR_ENCRYPTION_KEY from AWS Secrets Manager (new version) |
| [INFO] EMBYR_ENCRYPTION_KEY_PREVIOUS configured — decrypt fallback active |
| [INFO] embyr-server ready ...                                            |
|                                                                           |
| # Maria Santos, enrolled before rotation, signs in with her existing     |
| # authenticator app — TOTP secret decrypts via the previous key.        |
| $ curl -s -X POST https://embyr.internal:9090/admin/v1/auth/signin \     |
|     -d '{"email":"maria.santos@finops.example","totp_code":"482913"}'    |
| {"status":"ok"}                                                          |
+---------------------------------------------------------------------------+
```

Emotional state: entry = focused, methodical (this is the riskiest step — Sam watches for
any decrypt-failure log line before declaring success); exit = relief (an existing user's
sign-in proves the dual-key fallback works against real data, not just a unit test).

---

## Step 4: Rotate EMBYR_ADMIN_KEY

```
+-- Step 4: Rotate the admin bearer token (dual-token window) --------------+
| $ kubectl apply -f embyr-server-deployment-rotate-admin.yaml             |
|                                                                           |
| # Grafana scraper (still on the OLD token) — unaffected during window   |
| $ curl -s -o /dev/null -w "%{http_code}\n" \                             |
|     -H "Authorization: Bearer ${old_admin_token}" \                      |
|     https://embyr.internal:9090/metrics                                  |
| 200                                                                       |
|                                                                           |
| # CI pipeline (already updated to the NEW token) — also works           |
| $ curl -s -o /dev/null -w "%{http_code}\n" \                             |
|     -H "Authorization: Bearer ${new_admin_token}" \                      |
|     https://embyr.internal:9090/admin/v1/projects                        |
| 200                                                                       |
+---------------------------------------------------------------------------+
```

Emotional state: entry = focused; exit = in control (both old and new clients keep working —
no coordinated flag-day required).

---

## Step 5: Close the rotation window

```
+-- Step 5: Close the window — rotation complete ---------------------------+
| # Sam confirms (via his own client inventory) that every consumer has   |
| # migrated to the new tokens/key, then removes both *_PREVIOUS vars.    |
| $ kubectl apply -f embyr-server-deployment-final.yaml                    |
| $ kubectl logs deploy/embyr-server -f                                    |
| [INFO] embyr-server ready ...  # no "_PREVIOUS configured" log line     |
|                                                                           |
| $ curl -s -o /dev/null -w "%{http_code}\n" \                             |
|     -H "Authorization: Bearer ${old_admin_token}" \                      |
|     https://embyr.internal:9090/metrics                                  |
| 401                                                                       |
+---------------------------------------------------------------------------+
```

Emotional state: entry = watchful; exit = confident ("that was routine" — the retired token
is cleanly rejected, and no TOTP/OIDC/DSN row was orphaned during the window).

---

## Error Paths (acknowledged across the journey)

| Step | Failure | Symptom | Recovery |
|------|---------|---------|----------|
| 1 | Both a plain env var and a secret-ref var set for the same key | Startup exits 1 immediately | Stderr names both conflicting vars; Sam removes one |
| 2 | Secrets-manager fetch fails (IAM/access, not found, malformed) | Startup exits 1, no port bound | Stderr names the failure; Sam fixes IAM policy or secret content, retries |
| 3 | Corrupted ciphertext (not a stale-key case) | Decrypt fails under both current and previous key | Same decrypt-failure response as today — never a false-positive "success" |
| 3 | `EMBYR_ENCRYPTION_KEY_PREVIOUS` equals `EMBYR_ENCRYPTION_KEY` | Startup exits 1 | Stderr flags the values as identical; Sam corrects the manifest |
| 4 | `EMBYR_ADMIN_KEY_PREVIOUS` equals `EMBYR_ADMIN_KEY` | Startup exits 1 | Stderr flags the values as identical; Sam corrects the manifest |
| 4/5 | Client still using a retired token after the window closes | 401 Unauthorized | Same clean 401 as any invalid token today — no new failure mode |

---

## Shared Artifacts Referenced (see shared-artifacts-registry.md for full detail)

`${admin_secret_ref}`, `${encryption_secret_ref}`, `EMBYR_ADMIN_KEY` / `EMBYR_ADMIN_KEY_PREVIOUS`,
`EMBYR_ENCRYPTION_KEY` / `EMBYR_ENCRYPTION_KEY_PREVIOUS`, startup log lines, `/metrics` and
`/admin/v1/*` Bearer auth outcome.
