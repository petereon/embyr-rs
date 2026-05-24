/// Driven port placeholder for the real-time NOTIFY listener interface.
///
/// Implementation lives in the infrastructure layer (e.g., PostgreSQL LISTEN/NOTIFY
/// or a pub/sub bridge). The `'static` bound allows `Box<dyn NotifyPort>` use
/// in async contexts without lifetime parameters.
pub trait NotifyPort: Send + Sync + 'static {}
