//! `PartitionQuery` split-point computation (pure, no IO).
//!
//! The handler scans the (gated) query's document names into memory, then this
//! picks evenly spaced split points. Documents are identified by their stored
//! `(collection_path, document_id)`, ordered bytewise: the same order
//! `ORDER BY __name__` and `__name__` cursors use in `embyr-pg-storage`.

/// A document name as stored: `(collection_path, document_id)`.
pub type DocName = (String, String);

/// Upper bound on documents `PartitionQuery` will scan into memory. A query
/// matching more is rejected with `FailedPrecondition` rather than silently
/// returning partitions computed from a truncated scan.
// ponytail: in-memory scan, ceiling 50k docs. Upgrade path: step through the
// ordered query with OFFSET/LIMIT (or a count) instead of loading every row.
pub const MAX_PARTITION_SCAN_DOCS: usize = 50_000;

/// Up to `partition_count - 1` split points that cut `names` into
/// `partition_count` runs of near-equal size, ascending. Running the query
/// with `startAt(split_k)`/`endBefore(split_k+1)` bounds yields each run
/// (the first run has no start bound, the last no end bound).
///
/// `partition_count == 1`, or no names, gives no split points. Asking for more
/// partitions than there are documents gives one split per document after the
/// first.
pub fn split_points(mut names: Vec<DocName>, partition_count: usize) -> Vec<DocName> {
    names.sort();
    let n = names.len();
    let parts = partition_count.min(n);
    // `parts <= n`, so `i * n / parts` is strictly increasing and >= 1:
    // every split is distinct and never the very first document.
    (1..parts).map(|i| names[i * n / parts].clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(ids: &[&str]) -> Vec<DocName> {
        ids.iter().map(|id| ("c".to_string(), id.to_string())).collect()
    }

    #[test]
    fn one_partition_or_no_documents_has_no_split_points() {
        assert!(split_points(names(&["a", "b", "c"]), 1).is_empty());
        assert!(split_points(vec![], 8).is_empty());
    }

    #[test]
    fn splits_are_evenly_spaced_ascending_and_input_order_is_irrelevant() {
        let ids: Vec<String> = (0..10).map(|i| format!("d{i}")).collect();
        let mut shuffled: Vec<&str> = ids.iter().map(String::as_str).collect();
        shuffled.reverse();
        let got = split_points(names(&shuffled), 4);
        // 10 docs, 4 partitions -> boundaries at sorted indices 2, 5, 7.
        assert_eq!(got, names(&["d2", "d5", "d7"]));
    }

    #[test]
    fn partitions_reassemble_into_the_full_sorted_set_without_overlap() {
        let all = names(&["a", "b", "c", "d", "e", "f", "g"]);
        let splits = split_points(all.clone(), 3);
        assert_eq!(splits.len(), 2);
        let mut runs: Vec<Vec<DocName>> = vec![vec![]; splits.len() + 1];
        for d in &all {
            // startAt(split) semantics: a doc belongs to the last split <= it.
            let k = splits.iter().take_while(|s| *s <= d).count();
            runs[k].push(d.clone());
        }
        assert!(runs.iter().all(|r| !r.is_empty()));
        assert_eq!(runs.concat(), all);
    }

    #[test]
    fn more_partitions_than_documents_caps_at_one_split_per_extra_document() {
        let got = split_points(names(&["a", "b", "c"]), 1_000_000_000_000);
        assert_eq!(got, names(&["b", "c"]));
    }

    #[test]
    fn order_is_bytewise_over_collection_path_then_id() {
        let a = ("a-b".to_string(), "z".to_string());
        let b = ("a".to_string(), "z".to_string());
        let c = ("a".to_string(), "zz".to_string());
        // Same order as `collection_path COLLATE "C", document_id COLLATE "C"`:
        // "a" < "a-b" bytewise; within "a", "z" < "zz".
        assert_eq!(split_points(vec![a.clone(), c.clone(), b.clone()], 3), vec![c, a]);
    }
}
