# Evolution: backup-disaster-recovery-docs

**Date:** 2026-09-15
**Feature:** A new operational document, `docs/operations/backup-disaster-recovery.md`,
states backup ownership across all 4 backend modes, System DB recovery procedure and
RTO/RPO targets, and a previously-undocumented encryption-key recovery precondition.
**ADR:** None new — pure documentation, no code/architecture change.

## This closes finding #18 from `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

No backup/disaster-recovery documentation existed anywhere in the repo — RTO/RPO, PITR
guarantees, and backup ownership (embyr's vs. customer's, for BYOC/`direct_pg`) were
entirely unstated. This is the first finding closed this session with a confirmed
zero-code-change scope: embyr-rs holds no local filesystem state and touches only
externally-managed Postgres instances, so the finding is genuinely about writing down an
ownership boundary and a set of preconditions, not building new capability.

## Key Decisions

| Decision | Verdict |
|---|---|
| Scope | Confirmed pure documentation via investigation, not assumed — grepped for hidden backup infrastructure/tooling (none exists), traced all 4 backend modes' code to confirm customer DB is never embyr-managed in any mode. |
| Pipeline adaptation | DESIGN skipped (no component to design — deployment topology is already architecturally locked by ADR-001). DISTILL's BDD/Gherkin authoring skipped in favor of a 5-item verification checklist traceable to DISCUSS's own investigation findings. DELIVER staffed with `nw-documentarist` instead of a crafter agent. QUALITY_GATE replaced with the verification checklist plus the documentarist's own DIVIO/Diataxis validation pass — there is nothing to mutation-test in a markdown file. |
| RTO/RPO framing | Stated as targets (≤5min RPO, ≤30min RTO) explicitly qualified as "pending hosting decision" (finding #19, tracked separately) rather than asserted as a locked SLA — avoids fabricating a guarantee no hosting/IaC decision in-repo actually backs. |
| New finding surfaced | System DB backup alone is insufficient for a complete restore — `EMBYR_ENCRYPTION_KEY`/`EMBYR_ENCRYPTION_KEY_PREVIOUS` (never stored in Postgres, per ADR-018) must also be recoverable, or 3 encrypted columns (`users.totp_secret_enc`, `oidc_providers.client_secret_enc`, `projects.backend_pg_dsn_enc`) become permanently undecryptable. Never documented anywhere before this feature. Positioned as an explicit restore-runbook precondition (step 4 of the recovery procedure), not a code fix. |

## Steps Completed

Adapted nWave pipeline for a documentation-shaped deliverable (DISCUSS=`nw-product-owner` with peer review, DELIVER=`nw-documentarist` with self-run DIVIO/Diataxis validation):

1. **DISCUSS**: confirmed pure-documentation scope via code investigation (zero filesystem state, all databases externally-managed across all 4 backend modes). Surfaced the encryption-key recovery precondition as a genuinely new finding. Produced a deliverable spec (7 required sections) in place of traditional user stories, with DoR item 4 (UAT/Gherkin) explicitly marked ADAPTED (replaced with a verification checklist) rather than silently skipped. Recommended the pipeline adaptation explicitly rather than forcing a code-shaped process onto docs-shaped work.
2. **Peer review** (`nw-product-owner-reviewer`): approved, 0 critical/high/medium/low issues — validated the pure-documentation determination, the deliverable spec's completeness against the audit finding's own asks (RTO/RPO, PITR, ownership), the encryption-key finding's validity and scoping, the legitimacy of the adapted DoR item, and the soundness of the pipeline recommendation.
3. **DELIVER** (`nw-documentarist`): authored `docs/operations/backup-disaster-recovery.md` against all 7 required sections. Interrupted once by a session rate limit almost immediately after starting (essentially no work lost), relaunched cleanly. Self-ran a DIVIO/Diataxis classification (REFERENCE, 87% reference/13% how-to, above the 80% type-purity threshold, justified hybrid per DISCUSS's own approval) and validation pass (9.2/10 across accuracy/completeness/clarity/consistency/correctness/usability).
4. **Independent spot-check** (by the orchestrator, not a subagent — cheap enough for a single markdown file): read the full document directly rather than trusting the self-graded validation alone. Found and fixed one real inaccuracy the documentarist's own self-review missed: the doc referenced a `GET /readyz` endpoint that doesn't exist in this codebase — the actual endpoints (established by `healthz-dependency-checks`/ADR-078, closed 2 features earlier this same session) are `/healthz` (readiness) and `/livez` (liveness). Corrected the recovery-procedure step and added the missing ADR-078 cross-reference.

## Lessons Learned

1. **A pure-documentation finding is a legitimate nWave pipeline shape, not a shortcut around it** — DISCUSS explicitly determined and justified the zero-code-change scope through investigation (not assumption), and every downstream wave was deliberately adapted (not skipped without reason) to fit: DESIGN skipped with justification, DISTILL's BDD authoring replaced with a traceable verification checklist, DELIVER staffed with the right agent type for the work's actual shape, QUALITY_GATE replaced with a real validation pass rather than silently dropped. The discipline of "adapt visibly, never silently cut" held even for a feature shape this session hadn't hit before.
2. **Self-graded validation from the same agent that authored the content is not a substitute for an independent read, even for "just documentation."** The documentarist's own DIVIO/Diataxis pass scored the document 9.2/10 and found zero issues, but a direct independent read (cheap here — one file, no cargo needed) caught a real factual error: a cited endpoint (`/readyz`) that was never built, contradicting a decision from a feature closed only 2 items earlier in this SAME session's own memory. Worth generalizing: even a "low risk" deliverable benefits from a second, independent read against the actual current codebase, not just a self-report — this session's own recent work is exactly the kind of fast-moving ground truth a same-agent self-review can miss.
3. **This session's own history is itself a source of drift risk** — the `/readyz` mistake happened because the doc's author (correctly) drew on general Kubernetes-probe conventions rather than checking this specific codebase's own just-established (and deliberately inverted) naming. When citing operational surfaces (endpoints, env vars, CLI flags) that a recent feature in the SAME session touched, grep the actual current code rather than relying on either prior knowledge or general convention.

## Key Files

- `docs/operations/backup-disaster-recovery.md` (new) — the deliverable itself.
- `docs/feature/backup-disaster-recovery-docs/feature-delta.md` — DISCUSS's full investigation and deliverable spec.

## Follow-Up Work

- Finding #19 (hosting/IaC decision) will let the RTO/RPO targets in this document move from "aspirational, pending hosting decision" to a locked, evidenced SLA — this document should be revisited once that finding closes.
- Finding #20+ (Medium/Low) remain after the High-severity arc closes.
