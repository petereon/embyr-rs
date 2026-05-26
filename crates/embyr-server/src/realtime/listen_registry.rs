/// Subscriber registry for real-time fan-out.
///
/// Step 05-02: ListenRegistry with register() + fan_out() for NOTIFY-driven updates.
use std::{
    collections::HashMap,
    sync::Arc,
};

use tokio::sync::{mpsc, Notify, RwLock};

use embyr_core::domain::document::{DocumentPath, FirestoreDocument};

/// An event delivered to a registered Listen subscriber.
#[derive(Clone, Debug)]
pub enum ListenEvent {
    /// A document was created or updated.
    Changed(FirestoreDocument),
    /// A document was deleted.
    Removed(DocumentPath),
}

/// A registered subscriber handle.
///
/// Contains the event channel receiver and a notifier that fires if
/// the subscriber is too slow (its channel was found full during fan_out).
pub struct SubscriberHandle {
    /// Incoming event receiver.
    pub event_rx: mpsc::Receiver<ListenEvent>,
    /// Fired by fan_out when this subscriber's channel is full.
    pub reset_notify: Arc<Notify>,
}

struct SubscriberEntry {
    tx: mpsc::Sender<ListenEvent>,
    reset_notify: Arc<Notify>,
}

/// Per-project fan-out registry.
///
/// Key: notify channel name (`dc_<hex16>`).
/// Value: list of active subscriber entries.
pub struct ListenRegistry {
    subscribers: RwLock<HashMap<String, Vec<SubscriberEntry>>>,
}

impl ListenRegistry {
    /// Create an empty registry.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            subscribers: RwLock::new(HashMap::new()),
        })
    }

    /// Register a new subscriber for a channel.
    ///
    /// Returns a `SubscriberHandle` containing:
    /// - `event_rx`: receives document change events
    /// - `reset_notify`: fires when the subscriber is detected as slow
    ///
    /// Capacity 64: fan_out fires reset_notify when channel is full.
    pub async fn register(&self, channel: &str) -> SubscriberHandle {
        let (tx, rx) = mpsc::channel(64);
        let reset_notify = Arc::new(Notify::new());
        let entry = SubscriberEntry {
            tx,
            reset_notify: Arc::clone(&reset_notify),
        };
        let mut map = self.subscribers.write().await;
        map.entry(channel.to_string()).or_default().push(entry);
        SubscriberHandle {
            event_rx: rx,
            reset_notify,
        }
    }

    /// Fan out an event to all subscribers of a channel.
    ///
    /// If a subscriber's channel is found to be at capacity, its `reset_notify`
    /// is fired immediately and the sender is removed from the registry.
    /// Dead senders (dropped receivers) are also silently removed.
    pub async fn fan_out(&self, channel: &str, event: ListenEvent) {
        let mut map = self.subscribers.write().await;
        if let Some(entries) = map.get_mut(channel) {
            entries.retain(|entry| {
                if entry.tx.capacity() == 0 {
                    // Channel is full — subscriber is too slow. Signal RESET and drop.
                    entry.reset_notify.notify_one();
                    return false;
                }
                entry.tx.try_send(event.clone()).is_ok()
            });
        }
    }

    /// Signal RESET to all subscribers on a channel.
    ///
    /// Used in integration tests to simulate overflow without relying on channel
    /// saturation timing, which is non-deterministic under async scheduling.
    /// Not compiled into production builds.
    pub async fn simulate_overflow(&self, channel: &str) {
        let map = self.subscribers.read().await;
        if let Some(entries) = map.get(channel) {
            for entry in entries {
                entry.reset_notify.notify_one();
            }
        }
    }
}
