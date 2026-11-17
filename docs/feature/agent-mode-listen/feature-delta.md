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

Zero changes. The agent's `Subscribe` rejects an empty `collection_path` and filters by substring match against the full document name, so the server subscribes with `projects/{pid}/databases/(default)/documents/`, which matches every document of the project. Works with already-deployed agents.

## Known gaps

- Deletes carry no pre-delete snapshot (`DocChange` has no old fields; agent `get_document` hides soft-deleted rows). `Removed` is delivered with empty fields: delivered when the collection's rule does not read the document, withheld (fail closed) when it does. Fix: additive `old_fields` on `DocChange`.
- A Listen client sees RESET after any agent-stream reconnect (correct, but noisier than the Postgres path).
- Agent-side `AgentNotifyBridge` still has no reconnect of its own: on a `PgListener` error it ends the stream and the server reconnects (new bridge connection). Fine, but each server reconnect opens a fresh agent-side PG connection.
- The Docker-backed acceptance test (`tests/agent_mode_listen`) was only compiled locally (`--no-run`); first run is in CI.
- `docs/operations/runbook.md` "Known limitation" (PR #61) should be removed once both merge.
