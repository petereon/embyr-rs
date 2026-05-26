/// Subscriber registry for real-time fan-out.
///
/// Step 05-01: type definition only — populated in 05-02 for NOTIFY fan-out.
use std::{
    collections::HashMap,
    sync::Arc,
};

use tokio::sync::{mpsc, RwLock};
use tonic::Status;

use embyr_proto::firestore::ListenResponse;

/// Map of project_id → list of active subscriber channels.
pub type ListenRegistry = Arc<RwLock<HashMap<String, Vec<mpsc::Sender<Result<ListenResponse, Status>>>>>>;

/// Create an empty listener registry.
pub fn new_registry() -> ListenRegistry {
    Arc::new(RwLock::new(HashMap::new()))
}
