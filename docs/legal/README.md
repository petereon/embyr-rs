# Legal drafts (not legal advice)

`terms-of-service.md` and `privacy-policy.md` are **drafts** written from what the code stores
and does. They are meant to give a lawyer a complete, accurate starting point, not to be
published as-is. Have a qualified lawyer in your jurisdiction review them before launch.

## Fill in before publishing

Every `[BRACKETED]` item. The ones that need a decision:

| Placeholder | Decision |
|---|---|
| `[COMPANY]`, `[ADDRESS]`, `[JURISDICTION]` | Legal entity, registered address, governing law and courts |
| `[SUPPORT_EMAIL]`, `[PRIVACY_EMAIL]` | Mailboxes that exist and are monitored |
| `[HOSTING_PROVIDER]`, `[EMAIL_PROVIDER]` | The real sub-processors once hosting and SMTP are chosen |
| `[REGION]` | Where the service and its system database run (drives transfer wording) |
| `[FREE_PLAN_LIMITS]`, `[PRO_PRICE]` | Plan terms; the code has a Free cap but no project-count cap |
| `[LOG_RETENTION]` | Query-log retention default (see below) |
| `[SLA]` | Left as "no SLA"; replace once `docs/operations/slo.md` targets are validated |

## Facts the drafts rely on (verify they stay true)

Derived from `migrations/` and `.env.example` on master, 2026-11-17:

- **Account data stored:** email, password hash (Argon2id), TOTP secret (encrypted), recovery
  codes (hashed), sessions (24 h cookie, token hashed), invitations, password-reset tokens
  (1 h), subscription state and the Stripe customer/subscription ids. No card data touches
  the service; payment is handled by Stripe Checkout/Portal.
- **Customer data:** lives in the customer's own Postgres (direct DSN, secret-manager DSN, or
  the customer-VPC agent). The service stores the DSN only encrypted under the server key.
  Hosted-identity end-user accounts are stored in the customer's database, so the service is
  a processor for them.
- **Operational data:** `query_logs` (operation, collection path, latency, status; no document
  contents), hourly/daily project metrics (hourly rollup kept 14 days), rate-limit state
  (client IP as an in-memory/DB key for sign-in throttling), server logs.
- **Retention:** project soft-delete 7 days, account deletion 7 days grace
  (`EMBYR_SOFT_DELETE_GRACE_DAYS`, `EMBYR_ACCOUNT_DELETE_GRACE_DAYS`).
- **Gaps to resolve:** there is **no sweeper for `query_logs` retention** that I could find
  (a per-project `log_retention_days` column exists); decide the real default and make the
  policy text match. Backups: the policy says backups are purged on the provider's schedule;
  confirm with `docs/operations/backup-disaster-recovery.md` once hosting is chosen.
- **Trademark:** "Firestore" and "Google" belong to Google. The terms state the service is
  independent and API-compatible only. Check your use of the name in marketing.

## Slovakia-specific notes (sole trader, not legal advice)

- **Provider identification.** A Slovak online service provider must publish identifying
  details (name, address, IČO, trade-register entry, contact). The drafts' header and the
  site footer need them; a PO box or virtual office address is acceptable only if it is a
  valid address for service — confirm with your accountant. Do not publish these (or register
  as a trader) until you decide to; see the cost discussion in the project notes.
- **B2B only.** The terms restrict the service to businesses and professionals, which keeps
  consumer-protection rules (withdrawal rights, ODR/alternative dispute resolution notices)
  out of scope. Keep the sign-up flow consistent with that (a checkbox confirming
  professional use is a cheap way to back it).
- **Supervisory authority** for privacy complaints: Úrad na ochranu osobných údajov SR.
- **Before charging anyone:** trade licence (živnosť) or company, VAT position, Stripe account
  in the right name, and updated `[FULL NAME]`/IČO in these documents.
