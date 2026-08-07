# ADR-011: IEmailSender Driven Port Placement

## Status

Accepted

## Context

`POST /admin/v1/members/invite` (AC-B05-02) sends an invitation email to the invitee. The DISCUSS wave locked "SMTP email delivery with real relay (stub `IEmailSender`; `NoopEmailSender` in tests)" as out of scope for V1. V1 inserts the `invitations` row and returns 202 without sending a real email. V2 wires a real SMTP adapter.

The `IEmailSender` trait must be defined somewhere. The hexagonal principle requires that port traits live in the inner hexagon (`embyr-core`) so that domain and application logic can depend on the abstraction without importing infrastructure. The concrete adapters (SMTP client, noop) live in the outer ring (`embyr-server`).

Three placement options:
1. Trait in `embyr-core`, adapters in `embyr-server`
2. Trait in `embyr-server` (same layer as the adapter implementations)
3. Trait in a new `embyr-admin-api` crate (new infrastructure-layer crate)

## Decision

**`IEmailSender` trait in `embyr-core::admin::email`. Concrete adapters in `embyr-server::adapters::email`.**

Port trait definition (`embyr-core::admin::email`):
```
trait IEmailSender: Send + Sync {
    async fn send(&self, message: EmailMessage) -> Result<(), EmailError>;
    async fn probe(&self) -> Result<(), AdapterProbeError>;
}

struct EmailMessage {
    to: String,
    subject: String,
    text_body: String,
    html_body: Option<String>,
}
```

V1 adapter (`embyr-server::adapters::email::NoopEmailSender`):
- `send()`: logs `{msg: "email_noop", to: <email>, subject: <subject>}` and returns `Ok(())`.
- `probe()`: returns `Ok(())` immediately — no external substrate to validate.

V2 adapter (`embyr-server::adapters::email::SmtpEmailSender`):
- `send()`: delivers via `lettre` (async SMTP crate) using `EMBYR_SMTP_HOST`, `EMBYR_SMTP_PORT`, `EMBYR_SMTP_USERNAME`, `EMBYR_SMTP_PASSWORD`, `EMBYR_SMTP_FROM`.
- `probe()`: opens a TCP connection to `EMBYR_SMTP_HOST:EMBYR_SMTP_PORT` and performs an SMTP EHLO handshake (without sending any message). Hard failure (process refuses to start) if configured but unreachable.

Injection at composition root:
- V1: `Arc<NoopEmailSender>` injected into `UserAdminState.email_sender`
- V2: `Arc<SmtpEmailSender>` injected if `EMBYR_SMTP_HOST` is set in config; else falls back to `NoopEmailSender` with a startup warning

`UserAdminState.email_sender` field type: `Arc<dyn IEmailSender>` — dynamic dispatch is acceptable for an infrequent operation (invitation sending) with no hot-path impact.

## Alternatives Considered

### Option 2: Trait in `embyr-server` (rejected)

The invitation handler in `embyr-server::admin::handlers::members` would import the trait directly from the same crate, which is fine for compilation. But if the invitation logic were ever extracted to `embyr-core` (e.g., as an `InvitationService` domain service), it would then need to import from `embyr-server`, inverting the dependency direction. Placing the trait in `embyr-core` now prevents this inversion.

### Option 3: New `embyr-admin-api` crate (rejected)

Creating a new crate for admin API traits would be premature — the admin API adds one port trait in V1 (email) and potentially one more in V2 (SMTP). The overhead of a new crate (Cargo.toml, deny.toml, CI configuration) is not justified for two traits. The existing `embyr-core::admin` sub-module provides the same isolation without a new crate.

## Consequences

**Positive:**
- `IEmailSender` trait is co-located with other domain-level port traits (`IQueryLogWriter`, RBAC types) in `embyr-core::admin`, keeping the admin domain's port definitions in one place.
- `NoopEmailSender::probe()` trivially succeeds — no overhead at startup for V1.
- `SmtpEmailSender::probe()` provides Earned Trust guarantees for V2: SMTP unreachable → process refuses to start with `health.startup.refused: smtp_unreachable`.
- Swapping V1 → V2 is a single composition root change (`NoopEmailSender` → `SmtpEmailSender`); zero handler changes.

**Negative / Trade-offs:**
- `embyr-core::admin` is a new sub-module that does not currently exist. It must be created alongside this feature. This is a controlled addition — no existing code changes.
- Dynamic dispatch (`Arc<dyn IEmailSender>`) adds one vtable indirection per invitation send. Invitation sends are infrequent (human-driven) — the overhead is immeasurable in practice.

## Earned Trust (Principle 12)

`NoopEmailSender.probe()` always returns `Ok(())`. This is acceptable because the V1 adapter has no external substrate — there is nothing to lie. The probe requirement is fully satisfied by the trait structure (the adapter must implement `probe()`) even when the implementation is trivial. When V2 `SmtpEmailSender` is deployed, its `probe()` exercises the real SMTP server — at that point, Earned Trust is genuinely required and provided.

The three enforcement layers:
- **Subtype**: `impl IEmailSender for NoopEmailSender` — `probe()` must be present or compilation fails.
- **Structural**: `#[adapter]` proc-macro attribute on `SmtpEmailSender` (V2 only) — verifies non-trivial `probe()` body.
- **Behavioral (V2 CI gold-test)**: CI probe-contracts stage: spin up `SmtpEmailSender` pointing at a stopped SMTP server, assert probe returns `AdapterProbeError::SmtpUnreachable` within 3s.
