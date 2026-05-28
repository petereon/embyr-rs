# JTBD Four Forces — embyr-rs

> Feature: embyr-rs full implementation
> Wave: DISCUSS
> Date: 2026-05-23

---

## JOB-01: SDK-Compat

| Force | Detail |
|---|---|
| **Push** (current frustration) | Google Firestore is the only backend the Firebase SDK officially supports. Data residency requirements or cost concerns force teams to consider alternatives, but no drop-in replacements exist. Migrating away means rewriting all data access code. |
| **Pull** (desired future) | Point `firebase.initializeApp` at an embyr endpoint; all SDK features work identically. No client code changes. Full control over data location, schema, and backups. |
| **Anxiety** (adoption concern) | "Will edge cases (offline persistence, transactions, resume tokens, BrowserChannel) really work?" Fear of subtle SDK incompatibilities discovered in production. Fear of performance regressions. |
| **Habit** (must change) | Teams currently use `@firebase/app` + `@firebase/firestore` configured against `firestore.googleapis.com`. Must change `apiKey`, `projectId`, and the service endpoint, but nothing else. |

---

## JOB-02: Tenant-Provision

| Force | Detail |
|---|---|
| **Push** | Manual DB setup per customer (creating Postgres schemas, running migrations, issuing credentials) is slow, error-prone, and doesn't scale past 20 customers. |
| **Pull** | One API call creates the project, migrates the customer DB, and returns credentials. Terraform-compatible. Scriptable at onboarding. |
| **Anxiety** | "Will the cascade-delete actually clean up everything on project deletion?" "What if the admin key leaks?" Concern about admin API exposure. |
| **Habit** | Ops teams currently Slack credentials to developers or store them in a shared spreadsheet. Must move to an API call in their IaC pipeline. |

---

## JOB-03: Live-Sync

| Force | Detail |
|---|---|
| **Push** | Building real-time features with REST polling or manual WebSockets requires significant custom infrastructure. Firebase onSnapshot works perfectly but is cloud-only. |
| **Pull** | `onSnapshot` fires within seconds on any committed write. Resume tokens survive reconnects. The Listen stream handles connection drops transparently. |
| **Anxiety** | "If a subscriber is slow, will it miss events?" "Does resume-token reconnect actually work after a 5-minute disconnect?" Concern about event delivery guarantees. |
| **Habit** | Developers currently either tolerate polling (refresh every 5 seconds) or rely on Google Firestore for real-time. Must trust that embyr's registry fan-out is reliable. |

---

## JOB-04: Credential-Isolation (agent mode)

| Force | Detail |
|---|---|
| **Push** | Security audits flag any SaaS product that receives database credentials. Passing SOC 2 or HIPAA audits requires demonstrating that DB passwords never traverse a cloud boundary. |
| **Pull** | embyr agent runs as a Kubernetes sidecar. DB credentials live in a Kubernetes Secret. embyr SaaS never learns the password. The network boundary is the only trust boundary. |
| **Anxiety** | "Is the mTLS setup complicated?" "If the agent crashes, will our app users see errors?" "How do we rotate the agent's TLS cert?" Operational complexity concerns. |
| **Habit** | Security teams currently run a manual review for every third-party credential upload. With the agent, the review gate drops out because there are no credentials to upload. |

---

## JOB-05: Cloud-Secret

| Force | Detail |
|---|---|
| **Push** | Using a separate embyr-specific credential store means another secret to audit, rotate, and monitor. AWS Secrets Manager or GCP Secret Manager is already the source of truth for all other DB credentials. |
| **Pull** | Provide the secret ARN once at project creation. When the DBA rotates the password in Secrets Manager, embyr picks it up on the next credential-cache miss — no re-provisioning. |
| **Anxiety** | "Will embyr have too-broad IAM access to my AWS account?" "What if embyr's IAM role is compromised?" Cross-account access concerns. |
| **Habit** | DevOps currently stores DSNs in Secrets Manager for all services. Must grant embyr's IAM role `secretsmanager:GetSecretValue` on one specific ARN — a familiar IAM pattern. |

---

## JOB-06: Tenant-Control

| Force | Detail |
|---|---|
| **Push** | Without a suspension mechanism, non-paying customers continue to generate infrastructure costs. Without usage metrics, billing is guesswork. |
| **Pull** | One API call suspends a project (returns PermissionDenied to all their SDK calls). Daily per-project ingress/egress bytes are in `daily_project_metrics` for the billing ETL. |
| **Anxiety** | "Will suspend take effect immediately or is there a propagation delay?" "Can a suspended customer see that they're suspended or just get generic errors?" |
| **Habit** | Currently, operators manually revoke credentials and call their payment provider. Must replace with a POST to `/admin/v1/projects/{id}/suspend`. |
