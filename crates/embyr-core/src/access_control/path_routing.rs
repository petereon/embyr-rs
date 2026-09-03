//! Shared pure matching primitives for multi-segment path-pattern routing —
//! BC-4 Access Control (feature `security-rules-cel-path-matching`, ADR-063
//! § Decision — Shared Matching Primitives).
//!
//! Slice 01 (US-01) built [`literal_skeleton`], to fill the
//! `access_rule_patterns.literal_skeleton` indexed-narrowing column at
//! import time. Slice 02 (US-02) added [`bind_ancestor`] (request-time
//! routing) and its shared private per-position predicate
//! (`positions_compatible`). Slice 04 (US-04) adds [`structurally_overlap`]
//! (import-time overlap detection), built on the SAME `positions_compatible`
//! predicate — generalized from "does this concrete value satisfy this
//! pattern position" to "could some concrete value satisfy both positions" —
//! never a second, independently-maintained matching routine (Decision
//! Driver 3, DDD-PM-4).
//!
//! Pure, zero-IO (enforced identically to `rules_file`, `deny.toml` already
//! covers all of `embyr-core`).

use std::collections::BTreeMap;

use super::rules_file::PathSegment;

/// Even-position (collection-name) segments only, joined with `/` — the
/// routing-index narrowing key (ADR-063 § Decision — Schema). Identical
/// computation whichever caller supplies `ancestor_segments`: a stored
/// pattern's own ancestor (this module, import time) or a concrete
/// request's own ancestor path (Slice 02, request time) — one
/// implementation, never two (Decision Driver 3).
pub fn literal_skeleton(ancestor_segments: &[PathSegment]) -> String {
    ancestor_segments
        .iter()
        .enumerate()
        .filter_map(|(i, seg)| match (i.is_multiple_of(2), seg) {
            (true, PathSegment::Literal(s)) => Some(s.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Per-position compatibility test (ADR-063 § Decision — Shared Matching
/// Primitives, DDD-PM-4): "could SOME concrete value satisfy both `a` and
/// `b` at this position?" — literal-vs-literal only when equal;
/// wildcard-vs-anything (including wildcard-vs-wildcard) is always
/// compatible, since a wildcard accepts any concrete value; recursive
/// wildcard is never compatible with anything (fails closed — it can never
/// legally appear in an already-decomposed ancestor, but a future caller
/// passing one is refused, not silently matched). The ONE predicate both
/// [`bind_ancestor`] (request-time routing — `b` is always `Literal`, a
/// concrete request never carries a wildcard) and [`structurally_overlap`]
/// (import-time overlap detection — both sides are patterns) are built on —
/// never two independently-maintained matching routines (Decision Driver 3).
fn positions_compatible(a: &PathSegment, b: &PathSegment) -> bool {
    match (a, b) {
        (PathSegment::RecursiveWildcard, _) | (_, PathSegment::RecursiveWildcard) => false,
        (PathSegment::Literal(x), PathSegment::Literal(y)) => x == y,
        (PathSegment::Wildcard(_), _) | (_, PathSegment::Wildcard(_)) => true,
    }
}

/// Overlap-detection primitive (US-04, import-time; ADR-063 § Decision —
/// Overlap Detection). Two pattern ancestors structurally overlap iff SOME
/// concrete path could satisfy both: same length, and every position
/// pairwise compatible per [`positions_compatible`] — the SAME primitive
/// [`bind_ancestor`] is built on, never a second, independently-maintained
/// matching routine (DDD-PM-4). A different-length pair, or any pair of
/// literal positions with different literal names, never overlaps —
/// including two patterns sharing every wildcard/literal position up to a
/// different LEAF-adjacent literal collection name (AC-17-221): that
/// literal always sits at an even (collection-name) position, so it is
/// compared here exactly like any other literal position.
pub fn structurally_overlap(a: &[PathSegment], b: &[PathSegment]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| positions_compatible(x, y))
}

/// Routing primitive (US-02/03, request-time; ADR-063 § Decision — Shared
/// Matching Primitives). `concrete_ancestor` is always `Literal`-only — a
/// concrete request never carries a wildcard. `Some(bindings)` iff every
/// position is compatible (same length, every position satisfies
/// [`positions_compatible`]) — every `Wildcard(name)` position captures its
/// own concrete value by name. `None` = no structural match (different
/// length, or a literal position whose value differs).
pub fn bind_ancestor(
    pattern_ancestor: &[PathSegment],
    concrete_ancestor: &[PathSegment],
) -> Option<BTreeMap<String, String>> {
    if pattern_ancestor.len() != concrete_ancestor.len() {
        return None;
    }

    let mut bindings = BTreeMap::new();
    for (pattern_seg, concrete_seg) in pattern_ancestor.iter().zip(concrete_ancestor.iter()) {
        if !positions_compatible(pattern_seg, concrete_seg) {
            return None;
        }
        if let (PathSegment::Wildcard(name), PathSegment::Literal(value)) =
            (pattern_seg, concrete_seg)
        {
            bindings.insert(name.clone(), value.clone());
        }
    }
    Some(bindings)
}

/// Request-time routing primitive (security-rules-cel-recursive-wildcards,
/// Slice 02, US-02, ADR-064 § Decision — New Pure Primitives). `prefix` is a
/// stored recursive-wildcard pattern's own FIXED PREFIX segments (always
/// even length, the terminal `RecursiveWildcard` segment already stripped).
/// `concrete_full_path` is a CONCRETE document's own FULL path segments
/// (ancestor + document ID joined, always `Literal`-only, always even
/// length) — NOT the ancestor alone: unlike [`bind_ancestor`]'s hard
/// equal-length precondition, a recursive wildcard's own zero-remaining
/// -segments case can land EXACTLY at the prefix's own boundary document,
/// which `DocumentPath.collection_path` alone cannot represent (Resolution 1,
/// `rules_version = '2'` semantics, feature-delta.md). `Some((bindings,
/// remainder_len))` iff `concrete_full_path` is at least as long as `prefix`
/// and every prefix position is compatible with the SAME
/// [`positions_compatible`] predicate [`bind_ancestor`]/[`structurally_overlap`]
/// already use — never a third, independently-invented compatibility test
/// (Decision Driver 1). `remainder_len` is always even automatically: both
/// `prefix` (Resolution 3, locked) and `concrete_full_path` (a real document
/// path) are always even-length, so their difference is even — no separate
/// parity check is needed. `None` = no structural match (too short, or a
/// literal position within the prefix whose value differs).
pub fn bind_recursive_prefix(
    prefix: &[PathSegment],
    concrete_full_path: &[PathSegment],
) -> Option<(BTreeMap<String, String>, usize)> {
    if concrete_full_path.len() < prefix.len() {
        return None;
    }

    let mut bindings = BTreeMap::new();
    for (pattern_seg, concrete_seg) in prefix.iter().zip(concrete_full_path.iter()) {
        if !positions_compatible(pattern_seg, concrete_seg) {
            return None;
        }
        if let (PathSegment::Wildcard(name), PathSegment::Literal(value)) =
            (pattern_seg, concrete_seg)
        {
            bindings.insert(name.clone(), value.clone());
        }
    }
    Some((bindings, concrete_full_path.len() - prefix.len()))
}

#[cfg(test)]
mod tests {
    //! Test Budget: 7 behaviors — (1) compute the literal skeleton from an
    //! ancestor segment sequence, (2) `bind_ancestor` binds every wildcard
    //! position by name on a structural match (including the all-literal,
    //! zero-wildcard case), (3) `bind_ancestor` returns `None` on any
    //! structural mismatch (length or literal-value), (4) `structurally_overlap`
    //! returns `true` iff same length and every position pairwise compatible
    //! (wildcard-vs-literal, wildcard-vs-wildcard with different captured
    //! names, and equal-literal-vs-literal all overlap), (5)
    //! `structurally_overlap` returns `false` on a different length or any
    //! differing-literal position (AC-17-221: a different leaf/collection
    //! literal name never overlaps, regardless of shared wildcard positions
    //! earlier in the path), (6) `bind_recursive_prefix` matches a concrete
    //! full path at or beyond the prefix's own length (zero remainder landing
    //! exactly at the prefix's own boundary document, AND a positive
    //! remainder reaching a descendant), binding every wildcard prefix
    //! position by name — including the empty-prefix (project-wide catch-all)
    //! case, (7) `bind_recursive_prefix` returns `None` when the concrete
    //! full path is shorter than the prefix, or any literal prefix position
    //! differs — x 2 = 14 budget; 7 tests used (parametrized variations per
    //! behavior).

    use super::*;

    #[test]
    fn literal_skeleton_joins_only_the_even_position_literal_segments() {
        let ancestor = vec![
            PathSegment::Literal("expeditions".to_string()),
            PathSegment::Wildcard("expeditionId".to_string()),
            PathSegment::Literal("journal_entries".to_string()),
        ];
        assert_eq!(literal_skeleton(&ancestor), "expeditions/journal_entries");

        let single = vec![PathSegment::Literal("profiles".to_string())];
        assert_eq!(literal_skeleton(&single), "profiles");
    }

    /// AC-17-207/211: a concrete ancestor path structurally matching a
    /// stored pattern binds every wildcard segment by its own captured name.
    #[test]
    fn bind_ancestor_captures_every_wildcard_position_by_name_on_a_structural_match() {
        let pattern = vec![
            PathSegment::Literal("expeditions".to_string()),
            PathSegment::Wildcard("expeditionId".to_string()),
            PathSegment::Literal("journal_entries".to_string()),
        ];

        // Two DIFFERENT concrete ancestor paths matching the SAME pattern
        // shape must bind fully independent values (AC-17-212 non-leakage).
        let trek = vec![
            PathSegment::Literal("expeditions".to_string()),
            PathSegment::Literal("trek-2026".to_string()),
            PathSegment::Literal("journal_entries".to_string()),
        ];
        let coastal = vec![
            PathSegment::Literal("expeditions".to_string()),
            PathSegment::Literal("coastal-explorer-2026".to_string()),
            PathSegment::Literal("journal_entries".to_string()),
        ];

        let trek_bindings = bind_ancestor(&pattern, &trek).expect("must structurally match");
        let coastal_bindings = bind_ancestor(&pattern, &coastal).expect("must structurally match");

        assert_eq!(
            trek_bindings.get("expeditionId").map(String::as_str),
            Some("trek-2026")
        );
        assert_eq!(
            coastal_bindings.get("expeditionId").map(String::as_str),
            Some("coastal-explorer-2026")
        );

        // The zero-wildcard case (4a's own `[Literal(coll)]` generalization,
        // ADR-063 § Decision — The Ancestor/Leaf Split): an all-literal
        // pattern matching an identical all-literal concrete path binds an
        // EMPTY map, never `None`.
        let literal_only = vec![PathSegment::Literal("app_config".to_string())];
        assert_eq!(
            bind_ancestor(&literal_only, &literal_only),
            Some(BTreeMap::new())
        );
    }

    /// AC-17-208/209: no structural match (different length, or a literal
    /// position whose concrete value differs) returns `None`.
    #[test]
    fn bind_ancestor_returns_none_on_any_structural_mismatch() {
        let pattern = vec![
            PathSegment::Literal("expeditions".to_string()),
            PathSegment::Wildcard("expeditionId".to_string()),
            PathSegment::Literal("journal_entries".to_string()),
        ];

        // Different length — no candidate row this shape is even queried
        // against in production (skeleton/segment-count narrowing), but the
        // pure function itself must still fail closed, defensively.
        let too_short = vec![PathSegment::Literal("expeditions".to_string())];
        assert_eq!(bind_ancestor(&pattern, &too_short), None);

        // Same length, same skeleton, but a DIFFERENT literal collection
        // name at an even position.
        let wrong_literal = vec![
            PathSegment::Literal("expeditions".to_string()),
            PathSegment::Literal("trek-2026".to_string()),
            PathSegment::Literal("other_collection".to_string()),
        ];
        assert_eq!(bind_ancestor(&pattern, &wrong_literal), None);
    }

    /// AC-17-218/219: two pattern ancestors structurally overlap iff SOME
    /// concrete path could satisfy both — wildcard-vs-literal (Domain
    /// Example 1: an already-stored wildcard vs. a new literal exception),
    /// wildcard-vs-wildcard even with DIFFERENT captured names (the schema
    /// doc's own "{expeditionId} vs {expId}" collision case), and
    /// equal-literal-vs-literal all overlap.
    #[test]
    fn structurally_overlap_is_true_when_every_position_is_pairwise_compatible() {
        let wildcard = vec![
            PathSegment::Literal("expeditions".to_string()),
            PathSegment::Wildcard("expeditionId".to_string()),
            PathSegment::Literal("journal_entries".to_string()),
        ];
        let literal_exception = vec![
            PathSegment::Literal("expeditions".to_string()),
            PathSegment::Literal("trek-2026".to_string()),
            PathSegment::Literal("journal_entries".to_string()),
        ];
        assert!(structurally_overlap(&wildcard, &literal_exception));
        // Symmetric regardless of argument order.
        assert!(structurally_overlap(&literal_exception, &wildcard));

        let differently_named_wildcard = vec![
            PathSegment::Literal("expeditions".to_string()),
            PathSegment::Wildcard("expId".to_string()),
            PathSegment::Literal("journal_entries".to_string()),
        ];
        assert!(structurally_overlap(&wildcard, &differently_named_wildcard));

        assert!(structurally_overlap(&wildcard, &wildcard));
    }

    /// AC-17-221: a different length, or any differing literal position
    /// (including a different LEAF collection name, e.g. `journal_entries`
    /// vs `photos` — always an even/collection-name ancestor position),
    /// never overlaps — regardless of shared wildcard positions earlier in
    /// the path.
    #[test]
    fn structurally_overlap_is_false_on_length_or_any_literal_mismatch() {
        let journal_entries = vec![
            PathSegment::Literal("expeditions".to_string()),
            PathSegment::Wildcard("expeditionId".to_string()),
            PathSegment::Literal("journal_entries".to_string()),
        ];
        let photos = vec![
            PathSegment::Literal("expeditions".to_string()),
            PathSegment::Wildcard("expeditionId".to_string()),
            PathSegment::Literal("photos".to_string()),
        ];
        assert!(!structurally_overlap(&journal_entries, &photos));

        let shorter = vec![PathSegment::Literal("expeditions".to_string())];
        assert!(!structurally_overlap(&journal_entries, &shorter));
    }

    /// AC-17-238/240/241: a concrete full path structurally matching a
    /// recursive-wildcard pattern's own fixed prefix binds every wildcard
    /// prefix position by name, whether the match lands EXACTLY at the
    /// prefix's own boundary document (zero remainder) or reaches a
    /// descendant (positive remainder) — including the empty-prefix
    /// (project-wide catch-all) case, which matches ANY full path.
    #[test]
    fn bind_recursive_prefix_matches_at_or_beyond_the_prefix_and_binds_wildcards() {
        let prefix = vec![
            PathSegment::Literal("expeditions".to_string()),
            PathSegment::Wildcard("expeditionId".to_string()),
        ];

        // Zero remainder: the full path lands EXACTLY at the prefix's own
        // boundary document (Resolution 1's own locked `rules_version = '2'`
        // zero-or-more semantics).
        let at_boundary = vec![
            PathSegment::Literal("expeditions".to_string()),
            PathSegment::Literal("trek-2026".to_string()),
        ];
        let (bindings, remainder) =
            bind_recursive_prefix(&prefix, &at_boundary).expect("must structurally match");
        assert_eq!(bindings.get("expeditionId").map(String::as_str), Some("trek-2026"));
        assert_eq!(remainder, 0);

        // Positive remainder: the full path reaches a descendant one full
        // collection+document pair beyond the prefix.
        let descendant = vec![
            PathSegment::Literal("expeditions".to_string()),
            PathSegment::Literal("trek-2026".to_string()),
            PathSegment::Literal("photos".to_string()),
            PathSegment::Literal("img-042".to_string()),
        ];
        let (bindings, remainder) =
            bind_recursive_prefix(&prefix, &descendant).expect("must structurally match");
        assert_eq!(bindings.get("expeditionId").map(String::as_str), Some("trek-2026"));
        assert_eq!(remainder, 2);

        // Empty prefix (project-wide catch-all, AC-17-237's own routing
        // counterpart): matches ANY full path, binding nothing.
        let (bindings, remainder) = bind_recursive_prefix(&[], &descendant).expect("empty prefix always matches");
        assert!(bindings.is_empty());
        assert_eq!(remainder, 4);
    }

    /// AC-17-238: no structural match (the concrete full path is shorter
    /// than the prefix, or any literal prefix position differs) returns
    /// `None`.
    #[test]
    fn bind_recursive_prefix_returns_none_when_shorter_than_prefix_or_mismatched() {
        let prefix = vec![
            PathSegment::Literal("expeditions".to_string()),
            PathSegment::Wildcard("expeditionId".to_string()),
        ];

        // Shorter than the prefix — can never structurally match, even
        // though the pure function itself must still fail closed
        // defensively (no candidate this short is even queried against in
        // production, `list_recursive_access_rule_patterns_up_to`'s own
        // narrowing).
        let too_short = vec![PathSegment::Literal("expeditions".to_string())];
        assert_eq!(bind_recursive_prefix(&prefix, &too_short), None);

        // Same length as the prefix's own boundary, but a DIFFERENT literal
        // collection name at the prefix's own even (collection-name)
        // position.
        let wrong_literal = vec![
            PathSegment::Literal("photos".to_string()),
            PathSegment::Literal("img-042".to_string()),
        ];
        assert_eq!(bind_recursive_prefix(&prefix, &wrong_literal), None);
    }
}
