# Journey: SDK Developer — Connect Firebase App to embyr

> Persona: P1 — Alex (SDK Developer)
> Jobs: JOB-01 (SDK-Compat), JOB-03 (Live-Sync)
> Research depth: Comprehensive

---

## Emotional Arc

```
Uncertain  Focused    Relieved   Confident  Delighted  Trusting
    │          │          │          │          │          │
    ▼          ▼          ▼          ▼          ▼          ▼
[Configure] [Write]    [Read]    [Query]   [Listen]  [Reconnect]
```

---

## Journey Steps

### Step 1 — Configure SDK
**Action**: Alex changes `firebase.initializeApp` config: updates `apiKey` to the embyr project key, adds `firestoreSettings({host: "embyr.example.com:8081", ssl: false})`.
**Expected output**: No error at init; `getFirestore()` returns a Firestore instance.
**Emotion**: Uncertain → "Will this even connect?"
**Shared artifacts produced**: `${embyr_endpoint}`, `${project_id}`, `${api_key}`
**Error path**: Wrong host → SDK hangs forever on first operation → Alex checks host:port, verifies `/healthz` responds.

### Step 2 — Write a Document
**Action**: `setDoc(doc(db, "users", "alice"), {name: "Alice", age: 30})`
**Expected output**: Promise resolves; no error. Document visible in DB.
**Emotion**: Focused → "Let's see if this actually works"
**Shared artifacts consumed**: `${embyr_endpoint}`, `${project_id}`, `${api_key}`
**Shared artifacts produced**: `${document_path}` = `projects/my-proj/databases/(default)/documents/users/alice`
**Error path**: Auth failure → SDK throws `permission-denied` → Alex verifies API key matches project config.

### Step 3 — Read a Document
**Action**: `getDoc(doc(db, "users", "alice"))`
**Expected output**: `snapshot.exists() === true`; `snapshot.data()` equals the written fields.
**Emotion**: Relieved → "It round-tripped!"
**Error path**: `NotFound` → Document not written in step 2 due to auth error → resolve step 2 first.

### Step 4 — Query a Collection
**Action**: `getDocs(query(collection(db, "users"), where("age", ">=", 18), orderBy("age")))`
**Expected output**: Array of documents matching filter; ordering correct.
**Emotion**: Confident → "The query API is working"
**Shared artifacts produced**: `${query_result_set}`
**Error path**: `FailedPrecondition` — missing composite index → Alex calls admin API to create index.

### Step 5 — Listen for Changes (onSnapshot)
**Action**: `onSnapshot(collection(db, "users"), snapshot => { ... })`
**Expected output**: Initial snapshot delivered (all matching docs). After a second client writes a document, the listener fires again with the new document within 2 seconds.
**Emotion**: Delighted → "Real-time is working!"
**Shared artifacts produced**: `${resume_token}` (embedded in listener state)
**Error path**: No change event delivered → verify Postgres NOTIFY trigger is installed; verify listener is connected to the correct transport.

### Step 6 — Run a Transaction
**Action**: `runTransaction(db, async (tx) => { const snap = await tx.get(ref); tx.update(ref, {count: snap.data().count + 1}); })`
**Expected output**: Transaction commits; `count` incremented by exactly 1 even under concurrent writes.
**Emotion**: Confident → "OCC works"
**Error path**: Transaction aborted (code: `aborted`) → SDK auto-retries; visible in logs as "version mismatch". Expected under high concurrency.

### Step 7 — Reconnect with Resume Token
**Action**: SDK disconnects (network drop simulated); reconnects after 30 seconds; `onSnapshot` listener is still active.
**Expected output**: SDK reconnects; delta delivery sends only documents changed during the disconnect window (not a full re-fetch). Listener fires with correct diff.
**Emotion**: Trusting → "This handles real network conditions"
**Shared artifacts consumed**: `${resume_token}` from step 5
**Error path**: Token expired (> 24h) → full re-snapshot sent; correct behavior, no data loss.

---

## Shared Artifacts Registry

| Artifact | Produced in | Consumed in | Source of Truth |
|---|---|---|---|
| `${embyr_endpoint}` | Step 1 | Steps 2–7 | Admin API project record |
| `${project_id}` | Step 1 | Steps 2–7 | Admin API project record |
| `${api_key}` | Step 1 | Steps 2–7 | Admin API Create Project response |
| `${document_path}` | Step 2 | Steps 3–7 | Firestore resource name |
| `${query_result_set}` | Step 4 | Step 4 only | Server response |
| `${resume_token}` | Step 5 | Step 7 | Server-issued, SDK-cached |
