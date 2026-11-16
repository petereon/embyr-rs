//! Pure bookkeeping for a multiplexed Listen stream: target-id assignment,
//! which changes a target covers, and the diff of a query window.
//!
//! No IO — `embyr-server`'s `listen_session` drives these from the stream.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetIdError {
    /// Negative ids are never valid.
    Negative(i32),
    /// The id is already used by an active target on this stream.
    AlreadyActive(i32),
}

/// Active target ids on one stream. A client-supplied id is kept as is; `0`
/// means "server assigns" (Firestore semantics). Assigned ids are monotonic
/// and never reuse a removed id, so a late response for a removed target can
/// never be mistaken for a newer one.
#[derive(Debug, Default)]
pub struct TargetIds {
    active: BTreeSet<i32>,
    last_assigned: i32,
}

impl TargetIds {
    pub fn activate(&mut self, requested: i32) -> Result<i32, TargetIdError> {
        if requested < 0 {
            return Err(TargetIdError::Negative(requested));
        }
        let id = if requested == 0 { self.next_free() } else { requested };
        if !self.active.insert(id) {
            return Err(TargetIdError::AlreadyActive(id));
        }
        Ok(id)
    }

    /// Returns whether `id` was active.
    pub fn release(&mut self, id: i32) -> bool {
        self.active.remove(&id)
    }

    fn next_free(&mut self) -> i32 {
        loop {
            self.last_assigned = self.last_assigned.saturating_add(1);
            if !self.active.contains(&self.last_assigned) {
                return self.last_assigned;
            }
        }
    }
}

/// Which document changes a target can possibly be interested in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetScope {
    /// Every document directly in this collection path.
    Collection(String),
    /// Exactly these `(collection_path, document_id)` pairs.
    Documents(BTreeSet<(String, String)>),
}

impl TargetScope {
    pub fn covers(&self, collection_path: &str, document_id: &str) -> bool {
        match self {
            Self::Collection(path) => path == collection_path,
            Self::Documents(docs) => {
                docs.contains(&(collection_path.to_string(), document_id.to_string()))
            }
        }
    }
}

/// Difference between the documents already sent for a query and its freshly
/// recomputed result.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WindowDiff {
    /// Ids new to the window, or whose version moved — in `current` order.
    pub entered_or_changed: Vec<String>,
    /// Ids that were sent but are no longer in the result — sorted.
    pub left: Vec<String>,
}

/// `known`: id -> version already delivered. `current`: the recomputed result
/// as `(id, version)` in query order.
pub fn diff_window(known: &BTreeMap<String, i64>, current: &[(String, i64)]) -> WindowDiff {
    let present: BTreeSet<&str> = current.iter().map(|(id, _)| id.as_str()).collect();
    WindowDiff {
        entered_or_changed: current
            .iter()
            .filter(|(id, version)| known.get(id) != Some(version))
            .map(|(id, _)| id.clone())
            .collect(),
        left: known
            .keys()
            .filter(|id| !present.contains(id.as_str()))
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn client_ids_are_kept_and_zero_gets_fresh_unused_ids() {
        let mut ids = TargetIds::default();
        assert_eq!(ids.activate(1), Ok(1));
        assert_eq!(ids.activate(0), Ok(2), "assigned id skips the one in use");
        assert_eq!(ids.activate(0), Ok(3));
        assert_eq!(ids.activate(3), Err(TargetIdError::AlreadyActive(3)));
        assert_eq!(ids.activate(-4), Err(TargetIdError::Negative(-4)));
    }

    #[test]
    fn assigned_ids_are_not_reused_after_release() {
        let mut ids = TargetIds::default();
        let first = ids.activate(0).unwrap();
        assert!(ids.release(first));
        assert!(!ids.release(first), "second release is a no-op");
        assert_ne!(ids.activate(0).unwrap(), first);
        assert_eq!(ids.activate(first), Ok(first), "a client may re-add a released id");
    }

    #[test]
    fn scope_covers_only_its_own_documents() {
        let docs = TargetScope::Documents(
            [("users".to_string(), "u1".to_string())].into_iter().collect(),
        );
        assert!(docs.covers("users", "u1"));
        assert!(!docs.covers("users", "u2"));
        assert!(!docs.covers("posts", "u1"));
        let collection = TargetScope::Collection("users/u1/posts".into());
        assert!(collection.covers("users/u1/posts", "anything"));
        assert!(!collection.covers("users", "anything"));
    }

    proptest! {
        /// Applying the diff to what was already sent reproduces the new
        /// result exactly, and an unchanged result diffs to nothing.
        #[test]
        fn applying_diff_to_known_yields_current(
            known in prop::collection::btree_map("[a-e]", 0i64..3, 0..5),
            current in prop::collection::btree_map("[a-e]", 0i64..3, 0..5),
        ) {
            let rows: Vec<(String, i64)> = current.clone().into_iter().collect();
            let diff = diff_window(&known, &rows);

            let mut applied = known.clone();
            for id in &diff.left {
                applied.remove(id);
            }
            for id in &diff.entered_or_changed {
                applied.insert(id.clone(), current[id]);
            }
            prop_assert_eq!(&applied, &current);
            prop_assert_eq!(diff_window(&applied, &rows), WindowDiff::default());
        }
    }
}
