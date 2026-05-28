# JTBD Opportunity Scores — embyr-rs

> Opportunity score = Importance + max(0, Importance − Satisfaction)
> Scale 1-10. High importance + low satisfaction = strongest opportunity.

| Job ID | Job | Importance | Satisfaction (current solutions) | Opportunity Score | Priority |
|---|---|---|---|---|---|
| JOB-01 | SDK-Compat | 10 | 2 (no drop-in Firestore alternative exists) | 18 | 🔴 Critical |
| JOB-03 | Live-Sync | 9 | 3 (only achievable via Google Firestore) | 15 | 🔴 Critical |
| JOB-04 | Credential-Isolation | 8 | 2 (no SaaS Firestore alternative supports agent mode) | 14 | 🔴 Critical |
| JOB-02 | Tenant-Provision | 8 | 4 (doable manually, just slow) | 12 | 🟠 High |
| JOB-05 | Cloud-Secret | 7 | 4 (can work around with direct_pg, just less elegant) | 10 | 🟠 High |
| JOB-06 | Tenant-Control | 6 | 3 (currently done manually with side-effects) | 9 | 🟡 Medium |

## Key Finding

JOB-01 (SDK-Compat) and JOB-03 (Live-Sync) together form the **core value proposition**: if these two jobs work correctly, embyr is useful. All other jobs (backend connectivity modes, tenant management, billing) are **operational enablers** that determine whether embyr can run as a production SaaS, but they do not differentiate the product in the user's eyes.

**Implication for slice sequencing**: Slices delivering JOB-01 + JOB-03 coverage should ship first (highest learning leverage, highest user value). Backend connectivity and tenant management slices can follow.
