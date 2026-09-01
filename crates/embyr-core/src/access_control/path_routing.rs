//! Shared pure matching primitives for multi-segment path-pattern routing —
//! BC-4 Access Control (feature `security-rules-cel-path-matching`, ADR-063
//! § Decision — Shared Matching Primitives).
//!
//! Slice 01 (US-01) needs only [`literal_skeleton`], to fill the
//! `access_rule_patterns.literal_skeleton` indexed-narrowing column at
//! import time. `bind_ancestor` (request-time routing, US-02/03) and
//! `structurally_overlap` (import-time overlap detection, US-04) land in
//! later slices — this file's location and scope are fixed by ADR-063, not
//! duplicated ahead of need (Principle 8).
//!
//! Pure, zero-IO (enforced identically to `rules_file`, `deny.toml` already
//! covers all of `embyr-core`).

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

#[cfg(test)]
mod tests {
    //! Test Budget: 1 behavior (compute the literal skeleton from an
    //! ancestor segment sequence) x 2 = 2 budget; 1 test used (parametrized
    //! variations).

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
}
