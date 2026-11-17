# agent-mode-listen

Listen (real-time) for `backend_mode=agent` projects.

## Problem

`handle_listen` started `PostgresNotifyListener` from the credential cache's DSN. Agent projects cache an empty DSN (credentials never leave the customer VPC), so Listen failed.

## Design

- `embyr-core`: `storage::change_feed` (`ChangeEvent`, `ChangeFeed`; pure, no IO) and a default-provided `BackendAdapter::open_change_feed` returning `Ok(None)`. Backends the server cannot `LISTEN` on directly override it.
- `AgentBackendAdapter::open_change_feed` calls the agent's existing `Subscribe` RPC over the existing mTLS channel and maps `DocChange` to `ChangeEvent` (unintelligible -> `Reset`).
- `realtime::change_feed_listener::ChangeFeedListener`: per-project task, same `notify_channel(project_id)` registry channel and same `ListenEvent` shape as `PostgresNotifyListener`. `Changed` re-fetches via `BackendAdapter::get_document`. Reconnect uses the shared `reconnect_backoff` (1,2,4,8,16,30s), first attempt immediate, 5s per-attempt timeout, same metric names as the Postgres listener. Every successful reconnect and every `Reset` calls `ListenRegistry::reset` (subscribers resync) because a re-opened stream has no replay.
- `handle_listen`: tries `ChangeFeedListener::start` first; `Ok(None)` falls through to the unchanged Postgres path. `active_listeners` now holds a type-erased `ListenerGuard`.

## Proto

Initially zero changes: the agent's `Subscribe` rejects an empty `collection_path` and filters by substring match against the full document name, so the server subscribes with `projects/{pid}/databases/(default)/documents/`, which matches every document of the project. Works with already-deployed agents.

Later, one additive field: `DocChange.old_fields` (tag 5, `map<string, Value>`). The agent fills it on DELETE from the soft-deleted row (deletes keep `fields`; the agent's NOTIFY translation no longer filters `NOT deleted`, so no extra query). The server maps it to `ChangeEvent::Deleted { old_fields }` -> `ListenEvent::Removed { fields }`, so a rule that reads the document is evaluated on the old document. Interop: an old agent omits the field (empty = unknown) and keeps the fail-closed behaviour (rule-gated deletes withheld); an old server ignores the unknown field. A row already purged by the sweeper, or any value the server cannot convert, also yields empty (fail closed).

## Known gaps

- Deletes from an agent that predates `DocChange.old_fields` (or whose row was already purged) carry no pre-delete snapshot: delivered when the collection's rule does not read the document, withheld (fail closed) when it does. Upgrade the agent to close this.
- A Listen client sees RESET after any agent-stream reconnect (correct, but noisier than the Postgres path).
- Agent-side `AgentNotifyBridge` still has no reconnect of its own: on a `PgListener` error it ends the stream and the server reconnects (new bridge connection). Fine, but each server reconnect opens a fresh agent-side PG connection.
- The Docker-backed acceptance test (`tests/agent_mode_listen`) was only compiled locally (`--no-run`); first run is in CI.
- `docs/operations/runbook.md` "Known limitation" (PR #61) should be removed once both merge.
