# Journey: Agent Storage Proxying — Visual Map

> Persona: Alex Chen (P1 — SDK Developer, app uses Firebase JS SDK)
> Supporting: Riley Nakamura (P4) configured the backend_mode=agent project
> Goal: SDK calls behave identically to Firestore regardless of backend_mode
> Emotional arc: Indifferent (unaware of agent) → Trusting (operations just work) → Confident (real-time works too)
> Jobs: JOB-01 (sdk-compat), JOB-03 (live-sync), JOB-04 (credential-isolation), JOB-08 (agent-livesync)
> Date: 2026-05-27

---

## Architecture of the Proxying Path

```
[Firebase SDK]         [embyr SaaS]           [embyr-agent (in VPC)]    [Postgres (in VPC)]
      |                      |                          |                        |
      |--- gRPC/REST -------->|                          |                        |
      |   Firestore RPC      |--- StorageAgent gRPC --->|                        |
      |                      |   (mTLS)                 |--- SQL --------------->|
      |                      |                          |<-- result -------------|
      |<-- Firestore resp ----|<-- agent response -------|                        |
      |                      |                          |                        |
```

---

## Journey Flow

```
[Trigger]             [Step 1]              [Step 2]              [Step 3]
Alex writes a new     setDoc("users/alex")  embyr SaaS routes     Agent executes SQL
document to finops-   SDK call via gRPC     call to agent at       INSERT into Postgres
prod project          or REST transport     10.0.1.50:9191 mTLS    version column = 1
     |                     |                     |                      |
     v                     v                     v                      v
Feels: Indifferent    SDK resolves OK        mTLS handshake         Agent returns
(no awareness of      (same as Firestore)    (SaaS cert verified,   Document with
 agent mode)                                  agent cert verified)  version=1, timestamp


      |
      v

[Step 4]              [Step 5]              [Step 6]
getDoc reads back     onSnapshot registers  Concurrent write
the same document     for live changes      by another client
                      via Subscribe stream  triggers DocChange
     |                     |                     |
     v                     v                     v
Document matches      Full snapshot sent     Agent pushes DocChange
what was written      to client               to embyr SaaS Subscribe
(round-trip ok)       (target CURRENT)        stream → fan-out →
                                              onSnapshot fires
```

---

## Step Detail with TUI Mockups

### Step 1: SDK Write (CreateDocument or UpdateDocument via agent)

```
+-- Step 1: SDK setDoc --------------------------------------------------+
|                                                                        |
|  // SDK code (unchanged from Firestore — Alex is unaware of agent)    |
|  await setDoc(doc(db, "orders/ord-2026-001"), {                       |
|    customerId: "C-489",                                               |
|    amount: 1250.00,                                                   |
|    status: "pending"                                                  |
|  });                                                                  |
|                                                                        |
|  // embyr SaaS receives CreateDocument RPC                            |
|  // Authenticates via mTLS (project finops-prod)                      |
|  // Routes to AgentAdapter → StorageAgent.CreateDocument gRPC         |
|                                                                        |
+------------------------------------------------------------------------+
  Emotional state: Indifferent (Alex's code is unchanged)
  Shared artifact: $DOCUMENT_PATH "orders/ord-2026-001" → flows through all steps
```

### Step 2: embyr SaaS → Agent mTLS Dispatch

```
+-- Step 2: SaaS→Agent Dispatch ----------------------------------------+
|                                                                        |
|  embyr SaaS resolves backend_mode=agent for project finops-prod       |
|  Establishes mTLS connection to 10.0.1.50:9191                        |
|                                                                        |
|  StorageAgent.CreateDocument(                                          |
|    parent:        "projects/finops-prod/databases/(default)/documents",|
|    collection_id: "orders",                                           |
|    document_id:   "ord-2026-001",                                     |
|    document:      { fields: { customerId: "C-489", ... } }           |
|  )                                                                    |
|                                                                        |
|  [mTLS verification happens here — both sides verify cert]            |
|  embyr SaaS cert verified against agent's EMBYR_AGENT_CA             |
|  Agent cert verified against project's backend_agent_ca               |
|                                                                        |
|  [Error: agent down → embyr SaaS returns Unavailable to SDK]          |
|                                                                        |
+------------------------------------------------------------------------+
  Emotional state: Invisible to Alex (internal dispatch)
  Shared artifact: $MTLS_SESSION → reused for subsequent calls to same project
```

### Step 3: Agent executes SQL

```
+-- Step 3: Agent Postgres Execution -----------------------------------+
|                                                                       |
|  Agent receives StorageAgent.CreateDocument                           |
|  Agent connects to local Postgres via EMBYR_AGENT_DB_DSN             |
|                                                                       |
|  INSERT INTO documents (project_id, path, collection, data, version) |
|  VALUES ('finops-prod', 'projects/finops-prod/.../orders/ord-2026-001'|
|          'orders', '{"fields":{"customerId":{"stringValue":"C-489"}}}',
|           1)                                                          |
|  ON CONFLICT → AlreadyExists returned to SaaS → SDK sees             |
|                                                                       |
|  Agent returns Document(name=..., version=1, update_time=<now>)      |
|  to embyr SaaS → translated to Firestore Document response           |
|                                                                       |
|  DocChange{Upsert, project_id="finops-prod", path="orders/ord-...",  |
|             version=1} pushed to Subscribe stream                     |
|                                                                       |
+-----------------------------------------------------------------------+
  Emotional state: Invisible to Alex (internal)
  Shared artifact: $DOCUMENT_VERSION (1) → used in OCC validation, getDoc, onSnapshot
```

### Step 4: SDK getDoc reads back

```
+-- Step 4: SDK getDoc Round-trip --------------------------------------+
|                                                                       |
|  const snap = await getDoc(doc(db, "orders/ord-2026-001"));          |
|  console.log(snap.data());                                            |
|  // { customerId: "C-489", amount: 1250, status: "pending" }         |
|                                                                       |
|  StorageAgent.GetDocument routes through SaaS → agent → Postgres     |
|  SELECT * FROM documents WHERE project_id='finops-prod'              |
|    AND path='projects/finops-prod/.../orders/ord-2026-001'            |
|                                                                       |
|  Result matches what Alex wrote ← round-trip correctness             |
|                                                                       |
+-----------------------------------------------------------------------+
  Emotional state: Trusting (works as expected)
```

### Step 5: onSnapshot registers for live changes

```
+-- Step 5: Subscribe (Change Notification) ----------------------------+
|                                                                       |
|  onSnapshot(collection(db, "orders"), (snapshot) => {                |
|    snapshot.docChanges().forEach(change => console.log(change));     |
|  });                                                                  |
|                                                                       |
|  embyr SaaS establishes Subscribe stream to agent:                   |
|  StorageAgent.Subscribe(project_id: "finops-prod")                   |
|    → agent opens LISTEN on Postgres NOTIFY channel                   |
|    → agent starts streaming DocChange events to SaaS                 |
|                                                                       |
|  SaaS sends to SDK:                                                   |
|  targetChange{ADD} → documentChange{orders/ord-2026-001} →           |
|  targetChange{CURRENT, resumeToken=...} →                            |
|  targetChange{NO_CHANGE}                                              |
|                                                                       |
|  Full snapshot delivered ← SDK onSnapshot fires for first time       |
|                                                                       |
+-----------------------------------------------------------------------+
  Emotional state: Trusting → Confident (real-time works)
  Shared artifact: $RESUME_TOKEN → enables delta delivery on reconnect
```

### Step 6: Concurrent write triggers live update

```
+-- Step 6: Live DocChange via Subscribe Stream -------------------------+
|                                                                       |
|  // Another client (or backend service) updates the order status     |
|  await updateDoc(doc(db, "orders/ord-2026-001"), { status: "shipped" });
|                                                                       |
|  Flow:                                                                |
|  1. SDK UpdateDocument → embyr SaaS → StorageAgent.UpdateDocument    |
|  2. Agent: SQL UPDATE documents SET data=..., version=version+1      |
|  3. Postgres trigger fires: pg_notify('doc_changes', payload)        |
|  4. Agent LISTEN goroutine receives NOTIFY                            |
|  5. Agent pushes DocChange{Upsert, version=2} over Subscribe stream  |
|  6. embyr SaaS Registry receives DocChange                           |
|  7. SaaS Listen handler evaluates against active targets              |
|  8. Match found → SaaS fetches updated doc via GetDocument from agent |
|  9. SaaS sends documentChange to Alex's SDK                          |
|  10. Alex's onSnapshot callback fires:                               |
|      { status: "shipped" } ← real-time update                       |
|                                                                       |
|  [Overflow scenario: >64 buffered events]                            |
|  Agent channel full → overflow flag set                              |
|  SaaS sends targetChange{RESET} → SDK re-snapshots                   |
|                                                                       |
+-----------------------------------------------------------------------+
  Emotional state: Confident (real-time parity with direct mode)
```

---

## Critical Error Paths

### Agent Disconnect During Operation

```
+-- Error Path: Agent Disconnect ----------------------------------------+
|                                                                        |
|  embyr SaaS ←→ agent: connection drops (network failure in VPC)       |
|                                                                        |
|  SaaS behavior:                                                        |
|  - In-flight RPC returns Unavailable to SDK                            |
|  - SaaS starts reconnect loop: backoff 1s → 2s → 4s → ... → 30s      |
|  - Subscribe stream disconnected → existing Listen targets get RESET   |
|  - SDK receives RESET → re-fetches full snapshot on reconnect          |
|                                                                        |
|  After agent comes back:                                               |
|  - SaaS reconnects within one backoff interval                         |
|  - Subscribe resumes; new DocChanges flow again                        |
|  - SDK re-snapshots and resumes onSnapshot correctly                   |
|                                                                        |
+------------------------------------------------------------------------+
```

### OCC Conflict Through Agent

```
+-- Error Path: Transaction OCC Conflict --------------------------------+
|                                                                        |
|  Alex's client and a backend service both read order version=3        |
|  and attempt to commit concurrently                                    |
|                                                                        |
|  Agent CommitTransaction: re-reads version, detects mismatch          |
|  Returns codes.Aborted "version mismatch for orders/ord-2026-001"     |
|  embyr SaaS propagates Aborted to SDK                                 |
|  Firebase SDK retries the transaction automatically                    |
|                                                                        |
+------------------------------------------------------------------------+
```
