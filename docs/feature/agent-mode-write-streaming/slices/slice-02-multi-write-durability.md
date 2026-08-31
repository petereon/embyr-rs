# Slice 02: Multi-Write Session and Disconnect Recovery

**Story**: US-02 | **Release**: 2 | **Estimate**: 1.5 days

## Goal

Prove the streaming mechanism from Slice 01 correctly handles multiple sequential writes in one session and survives a stream disconnect without losing or double-applying writes.

## IN Scope

- 3+ sequential writes in one stream session, each independently acknowledged
- Stream disconnect does not roll back already-acknowledged writes
- A newly-opened stream after disconnect accepts further writes normally
- Two sequential writes to the same document apply in order
- Idle stream timeout without spurious client-facing error

## OUT Scope

- SDK-side resend/retry logic (Firebase SDK's own responsibility, not embyr's)
- Cross-version graceful degradation (Escalation 2, resolved once at the feature level, not per-slice)

## Learning Hypothesis

Disproves: a stream carrying multiple writes, or surviving a mid-session disconnect, silently drops or double-applies a write.
Confirms (if it succeeds): the mechanism delivers the actual offline-durability contract the SDK depends on, not just a single-shot proof.

## Acceptance Criteria

- [ ] 3+ sequential writes in one stream session are each independently acknowledged with correct `updateTime` values
- [ ] A stream disconnect does not roll back or lose writes already acknowledged before the disconnect
- [ ] A new stream opened after a disconnect accepts further writes normally, with no duplicate application
- [ ] Two sequential writes to the same document apply in order, `generation` incrementing once per write
- [ ] An idle stream times out without surfacing a client-facing error for the idle period alone

## Dependencies

Depends on Slice 01 shipping first (reuses its streaming mechanism unchanged).

## Effort Estimate

1.5 days.

## Reference Class

`firestore-write-streaming` Slice 02+ (non-agent modes) — same durability-contract scenario class.
