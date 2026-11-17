# Terms of Service — DRAFT

> Draft for lawyer review. Not legal advice. Fill every `[BRACKETED]` item first.

**Effective date:** [DATE]  ·  **Provider:** [FULL NAME], sole trader, [ADDRESS], Slovakia, IČO [ICO], registered in the Trade Register of [DISTRICT OFFICE] under no. [NUMBER] ("we", "us")

## 1. Agreement

The service is offered **only to businesses and professionals** (developers and organisations acting in the course of their trade, business or profession). It is not offered to consumers. By creating an account or using the service you ("Customer", "you") confirm you act in that capacity and agree to these terms. If you
accept for an organisation, you confirm you have authority to bind it. If you do not agree, do
not use the service.

## 2. The service

We provide a hosted protocol-translation service that accepts requests from Firestore-compatible
client libraries and applies them to a PostgreSQL database ("the service"). The service is
**not a database**: your documents are stored in a PostgreSQL database that **you** provide and
control (directly, through a secret manager reference, or through an agent running in your
network). We store the connection details you give us, encrypted, and run queries on your
behalf.

The service is independent software. It is API-compatible with Firestore; it is not made,
endorsed or supported by Google. "Firestore" and "Google" are trademarks of Google LLC.

## 3. Accounts and security

- You must provide accurate information and keep it current.
- Owner accounts must enable two-factor authentication. You are responsible for all activity
  under your account, your team members, service accounts and API keys.
- Keep API keys and passwords secret. Rotate a key immediately if you suspect exposure
  (the console provides rotation and revocation). Tell us promptly at [SUPPORT_EMAIL] about
  any unauthorised use.
- You decide who on your team has which role; role changes take effect immediately.

## 4. Your data and your database

- **Ownership.** As between us, you own your data and the contents of your database. We claim
  no rights in them beyond what is needed to provide the service.
- **Your database, your responsibility.** You are responsible for the availability, backups,
  security, capacity and legal compliance of the PostgreSQL database you connect, and for the
  security rules you configure. We are not responsible for data loss in your database.
- **Access we need.** You authorise us to connect to your database with the credentials you
  provide solely to operate the service.
- **Private networks.** Connections to non-public hosts may be refused by default for security
  reasons; see the documentation for how to connect a private database.
- **End users.** If you use hosted identity or otherwise process personal data of your own users
  through the service, you are the controller and we are your processor; our
  [data processing terms](#) apply. You are responsible for your own privacy notices and for
  the lawfulness of the data you process.

## 5. Acceptable use

You will not (and will not allow others to): break the law or infringe others' rights; upload or
process content you have no right to; attempt to probe, disrupt or bypass security, rate limits
or usage metering; use the service to attack other systems; resell the service except as
agreed in writing; or use it in a way that materially degrades the service for others. We may
throttle, suspend or block access that we reasonably believe breaches this section or
threatens the service, with notice where practical.

## 6. Plans, fees and billing

- **Plans.** The Free plan is limited to [FREE_PLAN_LIMITS]. The Pro plan is charged at
  [PRO_PRICE] [per month plus usage — describe metering]. We may change prices on [30] days'
  notice; changes apply from your next billing period.
- **Payment.** Payments are processed by Stripe; we do not receive or store your card number.
  You authorise recurring charges for your plan. Fees are exclusive of taxes, which we add
  where required.
- **Failed payment.** If a payment fails we will notify you. If it stays unpaid your account
  can be placed on hold, which blocks creating new databases and may suspend data access
  until payment is resolved. Your database itself is not affected.
- **Refunds.** Except where the law requires otherwise, fees are non-refundable.

## 7. Availability and support

The service is provided **without a service-level agreement** [SLA]. We aim for high
availability and publish operational targets, but they are goals, not commitments. We may
perform maintenance and will try to give notice of planned downtime. Support is available at
[SUPPORT_EMAIL].

## 8. Intellectual property

We and our licensors own the service, its software and documentation. We grant you a
limited, non-exclusive, non-transferable right to use the service during the term under these
terms. You grant us a right to process your data only as needed to provide the service. If you
send us feedback, we may use it without obligation.

## 9. Confidentiality

Each party will protect the other's non-public information with reasonable care and use it
only for the purposes of these terms, except where disclosure is required by law.

## 10. Term, deletion and termination

- You may stop using the service at any time. You can delete a database or your whole account
  in the console. Deleting a database or account starts a **grace period of [7] days** during
  which it can be restored by contacting support; after that the associated records are
  permanently purged. Deletion removes our records and cached connection details; it does
  not delete data inside **your** PostgreSQL database.
- We may suspend or terminate your access for material breach (including non-payment) that is
  not cured within [14] days of notice, immediately where needed to protect the service or
  others, or for convenience on [30] days' notice (with a pro-rata refund of prepaid fees).
- Sections that by their nature should survive termination (including 4, 8, 9, 11, 12, 13)
  survive.

## 11. Disclaimer

To the maximum extent permitted by law, the service is provided "as is" and "as available".
We disclaim all implied warranties, including merchantability, fitness for a particular purpose
and non-infringement, and we do not warrant that the service will be uninterrupted, error-free
or that results from queries or security rules will meet your requirements. You are
responsible for testing your security rules before relying on them.

## 12. Limitation of liability

To the maximum extent permitted by law: (a) neither party is liable for indirect, incidental,
special, consequential or punitive damages, or for loss of profits, revenue, data or goodwill;
and (b) each party's total liability arising from these terms is limited to the amounts you paid
us in the [12] months before the event giving rise to the claim. Nothing limits liability that
cannot be limited by law (for example for fraud or death/personal injury caused by negligence).

## 13. Indemnity

You will defend and indemnify us against third-party claims arising from your data, your use of
the service in breach of these terms, or your end users' use of your applications, except to the
extent caused by our breach.

## 14. Changes to these terms

We may update these terms. For material changes we will give at least [30] days' notice by
email or in the console. Continuing to use the service after the effective date means you
accept the update; if you do not, you may terminate before then.

## 15. General

These terms, the privacy policy and any order form are the entire agreement. If a provision is
unenforceable the rest stays in force. You may not assign these terms without our consent; we
may assign them in a merger or sale. No waiver is effective unless in writing. Governing law: the law of the Slovak Republic (the
Commercial Code applies to these business-to-business terms), excluding its conflict-of-law
rules; courts: the competent court in the Slovak Republic [for the provider's seat — confirm with
your lawyer]. Notices go to the email on your account, and to
[SUPPORT_EMAIL] for us.

## 16. Contact

[FULL NAME] · [ADDRESS] · IČO [ICO] · [SUPPORT_EMAIL]
