# JTBD Job Stories — embyr-rs

> Feature: embyr-rs full implementation
> Wave: DISCUSS
> Date: 2026-05-23

---

## JOB-01: SDK-Compat — Run existing Firebase apps against a self-operated endpoint

**Job story**: When I have an app that uses the Firebase SDK, I want to point it at an embyr endpoint and have it behave identically to Google Firestore, so I can eliminate vendor lock-in and keep data in a region I control.

**Persona**: P1 — Alex (SDK Developer)

| Dimension | Detail |
|---|---|
| Functional | Use `firebase.initializeApp({apiKey, projectId})` pointing at embyr; all SDK calls succeed unchanged |
| Emotional | Feel confident the migration won't break production; feel in control of data residency |
| Social | Be seen as an infrastructure-independent architect; avoid "locked into Google" reputation |

---

## JOB-02: Tenant-Provision — Onboard a new customer project in under 5 minutes

**Job story**: When I need to give a new customer their Firestore-compatible endpoint, I want to create a project via the admin API and hand them credentials, so they are up and running without manual database setup.

**Persona**: P2 — Sam (Service Operator)

| Dimension | Detail |
|---|---|
| Functional | POST /admin/v1/projects → project created, customer DB migrated, credentials returned |
| Emotional | Feel confident the isolation invariant holds; feel efficient onboarding at scale |
| Social | Be trusted by customers that their data is isolated; be seen as operating a reliable service |

---

## JOB-03: Live-Sync — Deliver real-time document changes to all connected clients

**Job story**: When multiple clients are reading and writing a shared collection, I want all connected `onSnapshot` listeners to receive changes within seconds, so I can build collaborative features without a separate pub-sub layer.

**Persona**: P1 — Alex (SDK Developer)

| Dimension | Detail |
|---|---|
| Functional | `onSnapshot` fires for every committed write; resume tokens allow reconnect without re-fetching the full dataset |
| Emotional | Feel confident that users see a consistent view; feel the product is "alive" |
| Social | Deliver a responsive, collaborative UX that rivals Google Firestore |

---

## JOB-04: Credential-Isolation — Keep database credentials inside my network perimeter

**Job story**: When my security policy prohibits DB credentials from leaving my VPC, I want to deploy the embyr agent alongside my Postgres instance, so I can use the Firestore SDK from my app without exposing credentials to any third party.

**Persona**: P4 — Riley (Compliance-first Tenant)

| Dimension | Detail |
|---|---|
| Functional | embyr agent runs in-VPC, holds DSN in env, proxies operations; embyr SaaS never receives DSN |
| Emotional | Feel secure that I will pass my next audit; feel the architecture is principled |
| Social | Demonstrate to security auditors that zero credentials traverse the cloud boundary |

---

## JOB-05: Cloud-Secret — Use existing AWS/GCP secret infrastructure for DB credentials

**Job story**: When my database credentials are already managed in AWS Secrets Manager or GCP Secret Manager, I want embyr to fetch them from there, so I don't have to operate yet another credential store.

**Persona**: P3 — Morgan (Tenant Admin)

| Dimension | Detail |
|---|---|
| Functional | Provide secret ARN/resource name; embyr fetches DSN on demand; credentials rotate in secret manager without re-provisioning embyr |
| Emotional | Feel that the operational surface area has not grown; feel familiar patterns are reused |
| Social | Be seen as running a consistent, auditable secrets hygiene process |

---

## JOB-06: Tenant-Control — Suspend and monitor projects without touching customer data

**Job story**: When a customer is overdue on payment or abusing the service, I want to suspend their project and see their usage metrics, so I can enforce SLAs without touching their documents.

**Persona**: P2 — Sam (Service Operator)

| Dimension | Detail |
|---|---|
| Functional | POST /admin/v1/projects/{id}/suspend → all data requests return PermissionDenied; GET usage from daily_project_metrics |
| Emotional | Feel in control of costs; feel the service can be operated fairly |
| Social | Be trusted by investors that the service is self-sustaining |
