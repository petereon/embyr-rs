//! Backend-neutral change notifications (real-time Listen).
//!
//! A backend that cannot be reached by a direct Postgres `LISTEN` (e.g.
//! `backend_mode=agent`, where the database lives in the customer's VPC)
//! exposes its own change stream through `BackendAdapter::open_change_feed`.
//! Pure trait + value types: no IO.

use async_trait::async_trait;

use crate::{domain::document::DocumentPath, error::CoreError};

/// One change notification. Carries the document identity only: the server
/// re-fetches the document through `BackendAdapter::get_document`, exactly as
/// the direct-Postgres listener does after a `NOTIFY`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeEvent {
    /// The document was created, updated or (possibly) deleted: re-fetch it.
    Changed(DocumentPath),
    /// The document was deleted.
    Deleted(DocumentPath),
    /// The source lost events (overflow, malformed message): subscribers
    /// must resync from a fresh snapshot.
    Reset,
}

/// A live stream of `ChangeEvent`s for one project.
#[async_trait]
pub trait ChangeFeed: Send {
    /// The next event. `Err` means the stream is broken (transport error or
    /// closed by the peer); the caller reconnects via `open_change_feed`.
    async fn next(&mut self) -> Result<ChangeEvent, CoreError>;
}
