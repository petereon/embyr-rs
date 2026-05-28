# Journey: Agent Deployment — Visual Map

> Persona: Riley Nakamura (P4 — DevSecOps Lead, FinOps Corp)
> Goal: Deploy embyr-agent in Kubernetes VPC, register with embyr SaaS, verify health
> Emotional arc: Apprehensive (unknown binary) → Methodical (clear env vars) → Relieved (green health) → Confident (audit-ready)
> Jobs: JOB-04, JOB-07, JOB-09
> Date: 2026-05-27

---

## Journey Flow

```
[Trigger]             [Step 1]              [Step 2]              [Step 3]
Security audit        Configure agent       Deploy to cluster     Verify startup
requires credential   env vars in           kubectl apply -f      kubectl logs <pod>
isolation proof       K8s Secret            agent-deployment.yaml shows health signal
     |                     |                     |                     |
     v                     v                     v                     v
Feels: Apprehensive    Feels: Methodical    Feels: Tense            Feels: Relieved
     |                     |                     |                     |
     Artifacts: none   $CERT, $KEY, $CA,   $POD_NAME           "connected to Postgres"
                        $DB_DSN, $LISTEN    $AGENT_ENDPOINT     "listening on :9191"

      |
      v

[Step 4]              [Step 5]              [Step 6]
Register project      Test SDK write        Prepare audit
with embyr SaaS       through agent         evidence
curl POST /admin/v1   sdk.setDoc(...)        system DB row +
/projects             resolves OK            log export
     |                     |                     |
     v                     v                     v
Feels: Methodical    Feels: Excited          Feels: Confident
     |                     |                     |
$PROJECT_ID,         gRPC call appears       NULL creds_enc,
$AGENT_CA_PEM        in agent logs          mTLS handshake log
```

---

## Step Detail with TUI Mockups

### Step 1: Configure environment variables

```
+-- Step 1: Configure Agent Environment ---------------------------+
|                                                                  |
|  # K8s Secret (agent-tls-secret)                                |
|  EMBYR_AGENT_CERT   = /certs/agent.crt       <- PEM cert        |
|  EMBYR_AGENT_KEY    = /certs/agent.key       <- PEM key         |
|  EMBYR_AGENT_CA     = /certs/embyr-ca.crt   <- embyr SaaS CA   |
|  EMBYR_AGENT_DB_DSN = postgres://user:***@pg.finops.internal/db |
|                                             (DSN stays in VPC)  |
|  EMBYR_AGENT_LISTEN_ADDR = 0.0.0.0:9191    <- default          |
|  EMBYR_AGENT_MAX_CONNS   = 25              <- pool size         |
|  EMBYR_AGENT_LOG_LEVEL   = info            <- production        |
|                                                                  |
|  Missing var → binary prints to stderr and exits with code 1    |
|  "embyr-agent: missing required environment variable: EMBYR_AGENT_DB_DSN"
|                                                                  |
+------------------------------------------------------------------+
  Emotional state: Methodical — clear list, no guessing
  Integration risk: DSN must NOT appear in any log line
```

### Step 2: Deploy to Kubernetes cluster

```
+-- Step 2: Deploy --------------------------------------------------+
|                                                                    |
|  $ kubectl apply -f agent-deployment.yaml                         |
|  deployment.apps/embyr-agent created                              |
|                                                                    |
|  $ kubectl rollout status deployment/embyr-agent                  |
|  Waiting for deployment "embyr-agent" rollout to finish:          |
|  1 of 1 updated replicas are available...                         |
|  deployment "embyr-agent" successfully rolled out                 |
|                                                                    |
|  [Error path: pod CrashLoopBackOff]                               |
|  $ kubectl logs embyr-agent-xxx                                   |
|  embyr-agent: missing required environment variable: EMBYR_AGENT_CA
|  → Riley fixes the missing env var; re-deploys                    |
|                                                                    |
+--------------------------------------------------------------------+
  Emotional state: Tense → Relieved (on success) / Tense → Methodical (on clear error)
  Shared artifact: $POD_NAME → used in Step 3
```

### Step 3: Verify startup logs

```
+-- Step 3: Verify Startup ------------------------------------------+
|                                                                    |
|  $ kubectl logs embyr-agent-7d9f4b-xkp9l                         |
|                                                                    |
|  {"level":"INFO","ts":"2026-05-27T14:22:01Z",                     |
|   "msg":"connected to Postgres",                                  |
|   "max_conns":25}                                                 |
|                                                                    |
|  {"level":"INFO","ts":"2026-05-27T14:22:01Z",                     |
|   "msg":"listening on :9191",                                     |
|   "tls":"mTLS enabled, client cert required"}                     |
|                                                                    |
|  No DSN string in any log line ← audit invariant                  |
|                                                                    |
|  [Error path: Postgres unreachable]                               |
|  {"level":"ERROR","msg":"failed to connect to Postgres",          |
|   "error":"connection refused"}                                   |
|  Process exits with code 1 ← pod restart loop is visible         |
|                                                                    |
+--------------------------------------------------------------------+
  Emotional state: Relieved (green) / Anxious (error path → clear message)
  Shared artifact: agent endpoint :9191 → used in Step 4
```

### Step 4: Register project with embyr SaaS admin API

```
+-- Step 4: Register Project -----------------------------------------+
|                                                                     |
|  $ curl -X POST https://embyr.saas/admin/v1/projects \             |
|    -H "Authorization: Bearer $ADMIN_KEY" \                         |
|    -d '{                                                           |
|          "project_id": "finops-prod",                             |
|          "auth_mode": "mtls",                                      |
|          "auth_mtls_ca": "-----BEGIN CERTIFICATE-----\n...",       |
|          "backend_mode": "agent",                                  |
|          "backend_agent_endpoint": "10.0.1.50:9191",              |
|          "backend_agent_ca": "-----BEGIN CERTIFICATE-----\n..."    |
|        }'                                                          |
|                                                                     |
|  HTTP 201 Created                                                   |
|  {"project_id":"finops-prod","status":"active",                    |
|   "backend_mode":"agent","backend_agent_endpoint":"10.0.1.50:9191"}|
|                                                                     |
|  Note: response contains NO backend_pg_creds_enc field ← no DSN   |
|                                                                     |
|  [Error path: agent unreachable during health check]               |
|  HTTP 400 Bad Request                                               |
|  {"error":"agent endpoint unreachable: 10.0.1.50:9191"}            |
|  → Riley checks firewall rules (port 9191 must be open from SaaS)  |
|                                                                     |
+---------------------------------------------------------------------+
  Emotional state: Methodical → Confident (201)
  Shared artifact: $PROJECT_ID "finops-prod" → used in Step 5, Step 6
```

### Step 5: Test SDK write through agent

```
+-- Step 5: SDK Write Test ------------------------------------------+
|                                                                    |
|  // Node.js test script                                            |
|  const db = getFirestore(app); // pointed at embyr SaaS           |
|  await setDoc(doc(db, "users/riley"), { name: "Riley", role: "devops" });
|  const snap = await getDoc(doc(db, "users/riley"));               |
|  console.log(snap.data()); // { name: 'Riley', role: 'devops' }   |
|                                                                    |
|  [Agent logs show the RPC was proxied]                             |
|  {"level":"INFO","msg":"GetDocument","name":"users/riley",         |
|   "project":"finops-prod","duration_ms":3}                        |
|                                                                    |
|  SDK sees no difference from direct mode ← parity invariant       |
|                                                                    |
+--------------------------------------------------------------------+
  Emotional state: Excited (it works!)
  Shared artifact: $PROJECT_DATA (users/riley doc) → used in Subscribe test
```

### Step 6: Collect audit evidence

```
+-- Step 6: Audit Evidence ------------------------------------------+
|                                                                    |
|  # Evidence 1: System DB — no DSN stored                          |
|  SELECT project_id, backend_mode, backend_pg_creds_enc            |
|  FROM projects WHERE project_id = 'finops-prod';                  |
|                                                                    |
|  project_id  | backend_mode | backend_pg_creds_enc               |
|  ------------|--------------|--------------------                 |
|  finops-prod | agent        | (null)               <- no DSN     |
|                                                                    |
|  # Evidence 2: Agent log — no DSN string                          |
|  $ grep -i "postgres://" <(kubectl logs embyr-agent-...)          |
|  (no matches)                                                      |
|                                                                    |
|  # Evidence 3: mTLS enforced                                       |
|  $ openssl s_client -connect 10.0.1.50:9191 (no client cert)     |
|  140: error: ssl3_read_bytes: handshake failure <- cert required  |
|                                                                    |
+--------------------------------------------------------------------+
  Emotional state: Confident — three artifacts, unambiguous
```

---

## Error Paths Summary

| Step | Failure | Signal | Recovery |
|------|---------|--------|----------|
| 1 | Missing env var | `embyr-agent: missing required environment variable: EMBYR_AGENT_DB_DSN` | Add env var to K8s Secret |
| 1 | Unreadable cert file | `embyr-agent: failed to read cert: /certs/agent.crt: No such file` | Fix volume mount path |
| 2 | Postgres unreachable | `{"level":"ERROR","msg":"failed to connect to Postgres","error":"..."}` + exit code 1 | Fix DB_DSN or network firewall |
| 4 | Agent unreachable from SaaS | HTTP 400 `{"error":"agent endpoint unreachable"}` | Open port 9191 in VPC security group |
| 4 | mTLS CA mismatch | HTTP 400 `{"error":"TLS handshake failed: certificate verify failed"}` | Re-check backend_agent_ca vs agent's cert issuer |
| 5 | Operation returns Unimplemented | `Status(code=Unimplemented)` in SDK | Agent RPC not yet implemented — update agent binary |
