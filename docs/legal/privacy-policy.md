# Privacy Policy — DRAFT

> Draft for lawyer review. Not legal advice. Fill every `[BRACKETED]` item first.

**Effective date:** [DATE]  ·  **Controller:** [COMPANY], [ADDRESS]  ·  **Contact:** [PRIVACY_EMAIL]

## 1. Who this covers and our two roles

- **Account holders and team members** who use our console and API ("you"): we are the
  **controller** of the data described in section 2.
- **Your customers' data.** The service stores your documents in a PostgreSQL database that you
  provide and control. For personal data of your end users that passes through the service
  (including accounts created with hosted identity), we act as your **processor** and handle it
  only on your instructions under our data processing terms [LINK]. This policy does not
  describe how you handle your end users' data; you need your own notice for that.

## 2. What we collect and why

| Data | Examples | Purpose | Legal basis (GDPR) |
|---|---|---|---|
| Account data | email, password (stored only as an Argon2id hash), two-factor secret (encrypted) and recovery codes (hashed), role, invitations | Create and secure your account, authenticate you, send invitations and password-reset emails | Contract |
| Session data | session token (stored only as a hash), creation and last-active time, 24 h expiry | Keep you signed in | Contract |
| Billing data | plan, subscription status, Stripe customer and subscription ids, usage counts | Charge for the service, apply plan limits | Contract; legal obligation (tax) |
| Connection secrets | database connection strings or secret-manager references, agent TLS material — **stored encrypted** under a server-held key | Connect to the database you configured | Contract |
| Operational data | per-request operation type, collection path, latency, status and error code (no document contents); hourly and daily usage metrics; server logs | Run, secure and troubleshoot the service; show you usage in the console; enforce limits | Legitimate interests (security, reliability); contract |
| Network data | client IP address, used transiently to rate-limit sign-in attempts and appearing in server logs | Prevent abuse and credential stuffing | Legitimate interests |
| Support communications | what you send us | Answer you | Contract; legitimate interests |

We do **not** use advertising or analytics trackers on the console, we do not sell personal
data, and we do not read your database contents except as technically necessary to execute your
requests.

## 3. Cookies

The console sets one **strictly necessary** session cookie (HttpOnly, Secure, SameSite=Strict,
scoped to the admin path) to keep you signed in. No consent banner is needed for it. We set no
advertising or analytics cookies.

## 4. Who receives data (sub-processors and recipients)

| Recipient | Purpose | Data | Location |
|---|---|---|---|
| Stripe | Payments, invoicing, billing portal | name/email, billing details you enter on Stripe's pages, subscription data | [per Stripe] |
| [EMAIL_PROVIDER] | Transactional email (invitations, password reset) | recipient email, message content | [REGION] |
| [HOSTING_PROVIDER] | Hosting of the service and its system database, backups | all data in section 2 | [REGION] |
| Professional advisers, authorities | Legal advice, where legally required | as needed | as needed |

We require sub-processors to protect data to an equivalent standard. We will update this list
and notify account owners of material changes [30 days] in advance.

## 5. International transfers

Where data moves outside the UK/EEA we rely on [adequacy decisions / standard contractual
clauses] with the recipient. [Complete once hosting and email providers are chosen.]

## 6. How long we keep data

- Account, team and session data: while your account exists. Sessions expire after 24 hours.
- Password-reset and invitation tokens: valid for a short window (reset 1 hour) and then
  invalid; stored only as hashes.
- Deleted databases and accounts: kept for a **[7]-day grace period** so you can recover from
  mistakes, then permanently purged from live systems. Backups containing them age out within
  [N] days.
- Operational data: [LOG_RETENTION] for per-request logs; hourly usage rollups [14] days; daily
  metrics for billing history [N months].
- Billing records: as long as tax and accounting law requires ([7] years in many jurisdictions).

## 7. Security

We encrypt connection secrets at rest with AES-256-GCM using a server-held key that supports
rotation; store passwords with Argon2id; require two-factor authentication for accounts; encrypt
traffic in transit (TLS) and enforce TLS to customer databases; separate secrets from the
database that stores them; apply rate limits and lockouts; and maintain audit-oriented logging.
No system is perfectly secure. We will notify affected account owners and the relevant
authority of a personal-data breach as required by law.

## 8. Your rights

Depending on where you live (for example under the GDPR/UK GDPR), you may ask us to access,
correct, delete or export your personal data, to restrict or object to certain processing, and
to withdraw consent where we rely on it. You can delete your account in the console, or write to
[PRIVACY_EMAIL]; we respond within one month. You may also lodge a complaint with your data
protection authority. If you are an end user of one of our customers, contact that customer
first; we assist them in responding.

## 9. Children

The service is for businesses and developers and is not directed at anyone under 16. We do not
knowingly collect their data.

## 10. Changes

We will post changes here and, for material ones, email account owners at least [30] days before
they take effect.

## 11. Contact

[COMPANY] · [ADDRESS] · [PRIVACY_EMAIL]
Data protection officer / EU/UK representative (if required): [NAME / ADDRESS]
