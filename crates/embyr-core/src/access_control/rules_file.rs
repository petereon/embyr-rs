//! Rules-file outer-syntax parser and decomposition — BC-4 Access Control
//! (feature `security-rules-cel-parity`, Slice 01, US-01, ADR-062).
//!
//! Pure, zero-IO domain submodule (ADR-062 § Decision — Outer-Syntax
//! Parser). Parses a real Firestore `.rules` file's outer `service
//! cloud.firestore { match /databases/{database}/documents { match
//! /<collection>/{<var>}? { allow <verbs>: if <condition>; } } } }` wrapper
//! and decomposes each `match` block into a `DecomposedRule` — ready for the
//! admin handler to hand to the existing, unmodified `upsert_access_rule`/
//! `upsert_write_access_rule` calls (`admin::handlers::access_rules::
//! import_rules_file`).
//!
//! Slice 01 locked scope (feature-delta.md Resolution 1 Option C,
//! slice-01 brief): single top-level collections, at most one leaf-level
//! path-variable capture per `match` block. Rejection of out-of-v1-scope
//! constructs beyond a generic parse error (nested paths, recursive
//! wildcards, multiple wildcards, named-construct messaging) is Slice 04's
//! own concern — `RulesFileError` already carries a `Vec<OffendingBlock>`
//! per ADR-062's own accepted shape (so Slice 04 needs no type change), but
//! this slice only ever populates it with a single entry, first-encountered
//! problem.
//!
//! Hand-rolled scanner (ADR-062 § Accepted: hand-rolled scanner + recursive
//! block/verb parser) — no `pest`/`nom`, mirroring `access_control::mod`'s
//! own condition-tokenizer discipline one syntactic layer out, so the outer
//! grammar cannot silently widen via a grammar-file edit.

use crate::access_control::{parse_condition, ConditionParseError, UnsupportedConstruct};

// ---------------------------------------------------------------------------
// Types (ADR-062 § Decision — Outer-Syntax Parser / § Decision — Admin Surface)
// ---------------------------------------------------------------------------

/// One `/`-delimited segment of a `match` block's own path pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    Literal(String),
    /// `{name}` — a leaf-level path-variable capture (Slice 01's own new
    /// capability). `{name=**}` is classified as `RecursiveWildcard`
    /// instead (below), never as a named `Wildcard`.
    Wildcard(String),
    /// `{name=**}` or a bare `**}` segment — always out of Slice 01's
    /// locked scope (Epic 4b's own concern).
    RecursiveWildcard,
}

/// Firestore's own granular `allow` verb vocabulary (ADR-062 § Decision —
/// Outer-Syntax Parser). Bucketed onto embyr's 2-condition-per-collection
/// model at decompose time (§ Decision — Decomposition, Verb-bucketing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Read,
    Get,
    List,
    Write,
    Create,
    Update,
    Delete,
}

/// One parsed `match /<pattern> { allow <verbs>: if <condition>; ... }`
/// unit. `path_pattern` is the raw, as-written text — retained for
/// error-message fidelity even for shapes `segments` cannot classify
/// meaningfully.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchBlock {
    pub path_pattern: String,
    pub segments: Vec<PathSegment>,
    /// Each `allow <verb, verb, ...>: if <condition>;` clause, RAW
    /// (un-rewritten) condition text — the wildcard-name-to-`request.path.`
    /// rewrite happens at `decompose` time, not here (parsing and semantic
    /// rewriting are kept as separate passes).
    pub allow_clauses: Vec<(Vec<Verb>, String)>,
}

/// The decomposition target for ONE `match` block — ready for the admin
/// handler to hand to `upsert_access_rule`/`upsert_write_access_rule`
/// unmodified. `None` means this block's `allow` clauses named no verb in
/// that bucket at all (e.g. `allow read: if true;` alone -> `write_condition
/// = None`, so the write rule is left completely untouched).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecomposedRule {
    pub collection_path: String,
    pub read_condition: Option<String>,
    pub write_condition: Option<String>,
}

/// One offending `match` block, naming what about it is out of v1 scope.
/// Slice 01 only ever produces `"SYNTAX_ERROR"` (a generic parse problem) —
/// the richer named-construct vocabulary (`"NESTED_PATH"` /
/// `"RECURSIVE_WILDCARD"` / `"CROSS_DOCUMENT_READ"` / `"CUSTOM_FUNCTION"` /
/// `"CONFLICTING_VERB_CONDITIONS"`) is reachable from this same type
/// (ADR-062's own accepted shape) but only exercised/tested starting Slice
/// 04.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffendingBlock {
    pub path_pattern: String,
    pub construct: &'static str,
    pub detail: String,
}

/// `parse_rules_file`/`decompose`'s shared error type — carries EVERY
/// offending block found, never fails fast on the first (DISCUSS Resolution
/// 2 / US-04: an import naming any out-of-v1-scope construct anywhere in
/// the file is rejected in full, every offending block named). Slice 01's
/// own hand-rolled outer-shell scanner still fails fast on the FIRST
/// structural problem it finds (a malformed brace/keyword shell has no
/// well-defined way to keep scanning past it) — multi-block error
/// aggregation is exercised starting Slice 04, once `decompose` is the only
/// place multiple, independently-valid-shell blocks can each fail on their
/// own semantic grounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesFileError {
    pub offending_blocks: Vec<OffendingBlock>,
}

impl RulesFileError {
    fn single(path_pattern: impl Into<String>, construct: &'static str, detail: impl Into<String>) -> Self {
        RulesFileError {
            offending_blocks: vec![OffendingBlock {
                path_pattern: path_pattern.into(),
                construct,
                detail: detail.into(),
            }],
        }
    }
}

fn shell_syntax_error(detail: &str) -> RulesFileError {
    RulesFileError::single(String::new(), "SYNTAX_ERROR", detail.to_string())
}

// ---------------------------------------------------------------------------
// parse_rules_file — outer service/match-documents shell + match blocks
// ---------------------------------------------------------------------------

/// Parse the whole `.rules` file text into its constituent `match` blocks.
/// Validates the fixed `service cloud.firestore { match
/// /databases/{database}/documents { ... } }` shell (a plain syntax
/// concern, not a grammar-widening one — ADR-062 § Decision — Outer-Syntax
/// Parser) before scanning inner `match` blocks.
pub fn parse_rules_file(source: &str) -> Result<Vec<MatchBlock>, RulesFileError> {
    let trimmed = source.trim();

    let after_service = trimmed
        .strip_prefix("service cloud.firestore")
        .map(str::trim_start)
        .filter(|s| s.starts_with('{'))
        .ok_or_else(|| {
            shell_syntax_error(
                "expected 'service cloud.firestore { match /databases/{database}/documents { ... } }'",
            )
        })?;
    let close1 = find_matching_close(after_service, 0)
        .ok_or_else(|| shell_syntax_error("unbalanced '{' in the 'service cloud.firestore' block"))?;
    if !after_service[close1 + 1..].trim().is_empty() {
        return Err(shell_syntax_error("unexpected content after the closing 'service' brace"));
    }
    let service_body = after_service[1..close1].trim();

    let after_match = service_body
        .strip_prefix("match /databases/{database}/documents")
        .map(str::trim_start)
        .filter(|s| s.starts_with('{'))
        .ok_or_else(|| {
            shell_syntax_error(
                "expected 'match /databases/{database}/documents { ... }' inside the service block",
            )
        })?;
    let close2 = find_matching_close(after_match, 0)
        .ok_or_else(|| shell_syntax_error("unbalanced '{' in the 'match /databases/{database}/documents' block"))?;
    if !after_match[close2 + 1..].trim().is_empty() {
        return Err(shell_syntax_error("unexpected content after the closing 'documents' brace"));
    }
    let documents_body = after_match[1..close2].trim();

    parse_match_blocks(documents_body)
}

/// Find the byte index of the `}` matching the `{` at `open_idx` in `s`, by
/// simple depth counting — correct regardless of what the braces
/// themselves "mean" (service shell, match block, or a `{var}` path
/// capture), since every brace in a well-formed file nests properly.
fn find_matching_close(s: &str, open_idx: usize) -> Option<usize> {
    if s.as_bytes().get(open_idx) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    for (i, c) in s.char_indices().filter(|(i, _)| *i >= open_idx) {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Repeatedly scan `body` (the content directly inside `match
/// /databases/{database}/documents { ... }`) for `match /<pattern> { ...
/// }` blocks, in order, until exhausted.
fn parse_match_blocks(mut body: &str) -> Result<Vec<MatchBlock>, RulesFileError> {
    let mut blocks = Vec::new();
    loop {
        body = body.trim_start();
        if body.is_empty() {
            break;
        }
        let after_match = body
            .strip_prefix("match")
            .filter(|rest| rest.starts_with(char::is_whitespace))
            .ok_or_else(|| shell_syntax_error("expected a 'match /<path> { ... }' block"))?
            .trim_start();
        let pattern_end = find_path_pattern_end(after_match)
            .ok_or_else(|| shell_syntax_error("expected a '/'-prefixed match block path pattern"))?;
        let path_pattern = after_match[..pattern_end].trim().to_string();
        let after_pattern = after_match[pattern_end..].trim_start();
        if !after_pattern.starts_with('{') {
            return Err(shell_syntax_error("expected '{' after a match block's path pattern"));
        }
        let rest = after_pattern;
        let close_idx = find_matching_close(rest, 0)
            .ok_or_else(|| shell_syntax_error(&format!("unbalanced '{{' in match block '{path_pattern}'")))?;
        let block_body = &rest[1..close_idx];

        let segments = parse_path_segments(&path_pattern)?;
        let allow_clauses = parse_allow_clauses(block_body, &path_pattern)?;

        blocks.push(MatchBlock { path_pattern, segments, allow_clauses });

        body = &rest[close_idx + 1..];
    }

    if blocks.is_empty() {
        return Err(shell_syntax_error("expected at least one 'match /<path> { ... }' block"));
    }
    Ok(blocks)
}

/// Find the byte offset where a match block's `/`-delimited path pattern
/// ends and the block's own opening `{` begins — NOT simply the first `{`
/// in the remaining text, since a `{userId}` (or `{database}`) path
/// -variable segment contains braces of its own. Consumes `/segment`
/// repeats, where each `segment` is either a `{...}` token (single,
/// non-nested pair) or a bare run of non-`/`/non-whitespace/non-`{`
/// characters, stopping as soon as the next character is not `/`.
fn find_path_pattern_end(after_match: &str) -> Option<usize> {
    if !after_match.starts_with('/') {
        return None;
    }
    let mut idx = 0usize;
    loop {
        if after_match[idx..].as_bytes().first() != Some(&b'/') {
            break;
        }
        idx += 1;
        if after_match[idx..].starts_with('{') {
            let rel_close = after_match[idx..].find('}')?;
            idx += rel_close + 1;
        } else {
            let rel_end = after_match[idx..]
                .find(|c: char| c == '/' || c.is_whitespace() || c == '{')
                .unwrap_or(after_match[idx..].len());
            idx += rel_end;
        }
        if after_match[idx..].as_bytes().first() != Some(&b'/') {
            break;
        }
    }
    Some(idx)
}

fn parse_path_segments(pattern: &str) -> Result<Vec<PathSegment>, RulesFileError> {
    let stripped = pattern.strip_prefix('/').unwrap_or(pattern);
    if stripped.is_empty() {
        return Err(RulesFileError::single(pattern, "SYNTAX_ERROR", "empty match path"));
    }
    stripped.split('/').map(|seg| parse_one_segment(seg, pattern)).collect()
}

fn parse_one_segment(seg: &str, pattern: &str) -> Result<PathSegment, RulesFileError> {
    if let Some(inner) = seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
        if inner.ends_with("=**") {
            return Ok(PathSegment::RecursiveWildcard);
        }
        if inner.is_empty() {
            return Err(RulesFileError::single(pattern, "SYNTAX_ERROR", "empty path-variable name"));
        }
        return Ok(PathSegment::Wildcard(inner.to_string()));
    }
    if seg.contains("**") {
        return Ok(PathSegment::RecursiveWildcard);
    }
    if seg.is_empty() {
        return Err(RulesFileError::single(pattern, "SYNTAX_ERROR", "empty path segment"));
    }
    Ok(PathSegment::Literal(seg.to_string()))
}

/// Split a match block's body on `;` into `allow <verbs>: if <condition>`
/// clauses. Comments/blank clauses (empty after trim) are skipped.
fn parse_allow_clauses(block_body: &str, path_pattern: &str) -> Result<Vec<(Vec<Verb>, String)>, RulesFileError> {
    let mut clauses = Vec::new();
    for raw_clause in block_body.split(';') {
        let clause = raw_clause.trim();
        if clause.is_empty() {
            continue;
        }

        let after_allow = clause
            .strip_prefix("allow")
            .filter(|rest| rest.starts_with(char::is_whitespace))
            .map(str::trim_start)
            .ok_or_else(|| {
                RulesFileError::single(path_pattern, "SYNTAX_ERROR", format!("expected an 'allow' clause, got '{clause}'"))
            })?;

        let colon_idx = after_allow
            .find(':')
            .ok_or_else(|| RulesFileError::single(path_pattern, "SYNTAX_ERROR", "expected ':' after the verb list"))?;
        let verbs: Vec<Verb> = after_allow[..colon_idx]
            .split(',')
            .map(|v| parse_verb(v.trim(), path_pattern))
            .collect::<Result<_, _>>()?;
        if verbs.is_empty() {
            return Err(RulesFileError::single(path_pattern, "SYNTAX_ERROR", "empty verb list"));
        }

        let condition_text = after_allow[colon_idx + 1..]
            .trim()
            .strip_prefix("if")
            .filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
            .map(str::trim)
            .ok_or_else(|| {
                RulesFileError::single(path_pattern, "SYNTAX_ERROR", "expected 'if <condition>' after the verb list")
            })?;
        if condition_text.is_empty() {
            return Err(RulesFileError::single(path_pattern, "SYNTAX_ERROR", "empty condition"));
        }

        clauses.push((verbs, condition_text.to_string()));
    }

    if clauses.is_empty() {
        return Err(RulesFileError::single(path_pattern, "SYNTAX_ERROR", "match block has no 'allow' clauses"));
    }
    Ok(clauses)
}

fn parse_verb(v: &str, path_pattern: &str) -> Result<Verb, RulesFileError> {
    match v {
        "read" => Ok(Verb::Read),
        "get" => Ok(Verb::Get),
        "list" => Ok(Verb::List),
        "write" => Ok(Verb::Write),
        "create" => Ok(Verb::Create),
        "update" => Ok(Verb::Update),
        "delete" => Ok(Verb::Delete),
        other => Err(RulesFileError::single(path_pattern, "SYNTAX_ERROR", format!("unrecognized verb '{other}'"))),
    }
}

// ---------------------------------------------------------------------------
// decompose — path-shape validation, condition rewrite, verb-bucketing
// (ADR-062 § Decision — Decomposition)
// ---------------------------------------------------------------------------

/// Decompose every parsed `match` block into a `DecomposedRule`, or collect
/// every offending block's reason (Resolution 2 / US-04: reject in full,
/// every offending block named — Slice 01 only ever produces one entry per
/// call today, since `parse_match_blocks` itself fails fast on the first
/// shell problem; `decompose`'s own per-block loop is what will carry
/// multiple entries starting Slice 04, once several independently
/// -well-formed blocks can each fail on their own semantic grounds).
pub fn decompose(blocks: Vec<MatchBlock>) -> Result<Vec<DecomposedRule>, RulesFileError> {
    let mut offending = Vec::new();
    let mut rules = Vec::new();

    for block in &blocks {
        match decompose_block(block) {
            Ok(rule) => rules.push(rule),
            Err(mut err) => offending.append(&mut err.offending_blocks),
        }
    }

    if !offending.is_empty() {
        return Err(RulesFileError { offending_blocks: offending });
    }
    Ok(rules)
}

fn decompose_block(block: &MatchBlock) -> Result<DecomposedRule, RulesFileError> {
    let (collection, wildcard_var) = match block.segments.as_slice() {
        [PathSegment::Literal(coll)] => (coll.clone(), None),
        [PathSegment::Literal(coll), PathSegment::Wildcard(var)] => (coll.clone(), Some(var.clone())),
        segments if segments.iter().any(|s| matches!(s, PathSegment::RecursiveWildcard)) => {
            return Err(RulesFileError::single(
                &block.path_pattern,
                "RECURSIVE_WILDCARD",
                "recursive wildcard path matching is not supported in this slice",
            ));
        }
        _ => {
            return Err(RulesFileError::single(
                &block.path_pattern,
                "NESTED_PATH",
                "only a single top-level collection with at most one leaf-level path variable is supported in this slice",
            ));
        }
    };

    let mut read_condition: Option<String> = None;
    let mut write_condition: Option<String> = None;

    for (verbs, raw_condition) in &block.allow_clauses {
        let rewritten = match &wildcard_var {
            Some(var) => rewrite_path_variable(raw_condition, var),
            None => raw_condition.clone(),
        };

        if let Err(e) = parse_condition(&rewritten) {
            return Err(RulesFileError::single(&block.path_pattern, construct_for(&e), detail_for(e)));
        }

        for verb in verbs {
            let bucket = match verb {
                Verb::Read | Verb::Get | Verb::List => &mut read_condition,
                Verb::Write | Verb::Create | Verb::Update | Verb::Delete => &mut write_condition,
            };
            match bucket {
                Some(existing) if *existing != rewritten => {
                    return Err(RulesFileError::single(
                        &block.path_pattern,
                        "CONFLICTING_VERB_CONDITIONS",
                        "differing conditions for the same read/write bucket are not supported",
                    ));
                }
                _ => *bucket = Some(rewritten.clone()),
            }
        }
    }

    Ok(DecomposedRule { collection_path: collection, read_condition, write_condition })
}

fn construct_for(e: &ConditionParseError) -> &'static str {
    match e {
        ConditionParseError::SyntaxError { .. } => "SYNTAX_ERROR",
        ConditionParseError::UnsupportedConstruct { construct, .. } => match construct {
            UnsupportedConstruct::CrossDocumentRead => "CROSS_DOCUMENT_READ",
            UnsupportedConstruct::CustomFunction => "CUSTOM_FUNCTION",
            UnsupportedConstruct::WildcardPath => "NESTED_PATH",
        },
    }
}

fn detail_for(e: ConditionParseError) -> String {
    match e {
        ConditionParseError::SyntaxError { detail } | ConditionParseError::UnsupportedConstruct { detail, .. } => detail,
    }
}

/// Rewrite every WHOLE-WORD occurrence of `var` in `condition` to
/// `request.path.<var>` (ADR-062 § Decision — Condition rewrite) — a plain
/// string substitution over word boundaries, so `userIdSuffix` is never
/// partially matched inside a rewrite of `userId`.
fn rewrite_path_variable(condition: &str, var: &str) -> String {
    let is_word_char = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let chars: Vec<char> = condition.chars().collect();
    let var_chars: Vec<char> = var.chars().collect();
    let mut result = String::with_capacity(condition.len());
    let mut i = 0;
    while i < chars.len() {
        let matches_here = chars[i..].starts_with(var_chars.as_slice())
            && (i == 0 || !is_word_char(chars[i - 1]))
            && !chars.get(i + var_chars.len()).is_some_and(|c| is_word_char(*c));
        if matches_here {
            result.push_str("request.path.");
            result.push_str(var);
            i += var_chars.len();
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    //! Layer 1 (unit) coverage per Mandate 9 — pinned examples over
    //! `parse_rules_file`/`decompose`'s own hand-rolled scanner, mirroring
    //! `access_control::mod::tests`'s established shape for this codebase's
    //! non-PBT-amenable, string-scanning grammar layer (input space here is
    //! syntactic shell shape, not a quantifiable numeric/boolean domain).
    //!
    //! Test Budget: distinct behaviors in Slice 01's own IN-scope AC set —
    //! (1) parse a single no-wildcard block, (2) parse a single
    //! wildcard-bearing block, (3) parse multiple independent blocks, (4)
    //! reject a malformed shell, (5) decompose bucket verbs into
    //! read/write conditions correctly (no-wildcard), (6) decompose
    //! rewrites the wildcard variable and the rewritten condition parses,
    //! (7) decompose rejects a nested/multi-segment path. 7 behaviors x 2 =
    //! 14 budget; 8 tests used (parametrized where variations share one
    //! behavior).

    use super::*;

    const PROFILES_FILE: &str = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /profiles/{userId} {
              allow read, write: if request.auth.uid == userId;
            }
          }
        }
    "#;

    #[test]
    fn parses_a_single_block_with_no_path_variable() {
        let source = r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /app_config {
                  allow read: if true;
                }
              }
            }
        "#;
        let blocks = parse_rules_file(source).expect("must parse");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].segments, vec![PathSegment::Literal("app_config".to_string())]);
        assert_eq!(blocks[0].allow_clauses, vec![(vec![Verb::Read], "true".to_string())]);
    }

    #[test]
    fn parses_a_single_block_with_a_leaf_level_wildcard() {
        let blocks = parse_rules_file(PROFILES_FILE).expect("must parse");
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].segments,
            vec![PathSegment::Literal("profiles".to_string()), PathSegment::Wildcard("userId".to_string())]
        );
        assert_eq!(
            blocks[0].allow_clauses,
            vec![(vec![Verb::Read, Verb::Write], "request.auth.uid == userId".to_string())]
        );
    }

    #[test]
    fn parses_multiple_independent_match_blocks() {
        let source = r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /profiles/{userId} {
                  allow read, write: if request.auth.uid == userId;
                }
                match /journal_entries {
                  allow read: if request.auth.uid == resource.data.owner_id;
                }
                match /trail_guides {
                  allow read: if true;
                }
              }
            }
        "#;
        let blocks = parse_rules_file(source).expect("must parse");
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].path_pattern, "/profiles/{userId}");
        assert_eq!(blocks[1].path_pattern, "/journal_entries");
        assert_eq!(blocks[2].path_pattern, "/trail_guides");
    }

    #[test]
    fn malformed_outer_shell_is_a_syntax_error() {
        for bad_source in [
            "match /profiles/{userId} { allow read: if true; }",
            "service cloud.firestore { match /wrong/shell { allow read: if true; } }",
            "service cloud.firestore { match /databases/{database}/documents { } }",
        ] {
            let result = parse_rules_file(bad_source);
            match result {
                Err(RulesFileError { offending_blocks }) => {
                    assert_eq!(offending_blocks[0].construct, "SYNTAX_ERROR");
                }
                Ok(_) => panic!("expected a SYNTAX_ERROR for malformed shell '{bad_source}'"),
            }
        }
    }

    #[test]
    fn decompose_buckets_a_no_wildcard_block_unchanged_into_read_and_write() {
        let blocks = parse_rules_file(
            r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /journal_entries {
                  allow read: if request.auth.uid == resource.data.owner_id;
                }
              }
            }
        "#,
        )
        .expect("must parse");

        let rules = decompose(blocks).expect("must decompose");
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0],
            DecomposedRule {
                collection_path: "journal_entries".to_string(),
                read_condition: Some("request.auth.uid == resource.data.owner_id".to_string()),
                write_condition: None,
            }
        );
    }

    #[test]
    fn decompose_rewrites_the_wildcard_variable_and_shares_it_across_read_and_write() {
        let blocks = parse_rules_file(PROFILES_FILE).expect("must parse");
        let rules = decompose(blocks).expect("must decompose");

        assert_eq!(rules.len(), 1);
        let expected_condition = "request.auth.uid == request.path.userId".to_string();
        assert_eq!(
            rules[0],
            DecomposedRule {
                collection_path: "profiles".to_string(),
                read_condition: Some(expected_condition.clone()),
                write_condition: Some(expected_condition.clone()),
            }
        );
        // The rewritten condition must itself parse via the SAME,
        // unmodified parse_condition real enforcement/simulation use —
        // proves the rewrite step produces grammar-valid text, not just a
        // plausible-looking string.
        assert!(parse_condition(&expected_condition).is_ok());
    }

    #[test]
    fn decompose_rejects_a_nested_multi_segment_path() {
        let blocks = parse_rules_file(
            r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /expeditions/{id}/journal_entries/{entryId} {
                  allow read: if true;
                }
              }
            }
        "#,
        )
        .expect("must parse");

        let result = decompose(blocks);
        match result {
            Err(RulesFileError { offending_blocks }) => {
                assert_eq!(offending_blocks[0].construct, "NESTED_PATH");
            }
            Ok(_) => panic!("expected NESTED_PATH rejection"),
        }
    }
}
