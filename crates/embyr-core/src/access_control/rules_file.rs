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
//!
//! Widened (feature `security-rules-cel-path-matching`, Slice 01, US-01,
//! ADR-063): `parse_match_blocks` now recurses into nested `match { match
//! { ... } } }` shells (§ Decision — Nested Match-Block Flattening),
//! prepending each ancestor's own already-parsed `PathSegment`s so the
//! flattened result is byte-for-byte identical to the equivalent flat
//! multi-segment syntax. `decompose_block`'s own shape allow-list widens
//! from 4a's `[Literal]`/`[Literal, Wildcard]` two-shape check to any
//! length ≥ 1 alternating literal-collection/wildcard-or-literal-document-ID
//! sequence (§ Decision — Widened `decompose_block` Shape-Check), splitting
//! into an ancestor (`> 1` segment ⇒ `DecomposedTarget::MultiSegmentPattern`)
//! and 4a's own single-collection shape (`== 1` segment ⇒
//! `DecomposedTarget::SingleCollection`, `DecomposedRule` unchanged).

use std::collections::BTreeMap;

use crate::access_control::path_routing;
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
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// The decomposition target for ONE multi-segment `match` block (ancestor
/// segment count > 1) — feature `security-rules-cel-path-matching`, Slice
/// 01, US-01, ADR-063 § Decision — Schema. `collection_path_pattern` is the
/// ANCESTOR path only (e.g. `"expeditions/{expeditionId}/journal_entries"`)
/// — the leaf (if any) is split off separately into `leaf_variable`, never
/// part of this text. Ready for `SystemDb::upsert_access_rule_pattern`
/// unmodified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecomposedPatternRule {
    pub collection_path_pattern: String,
    pub ancestor_segment_count: u16,
    pub literal_skeleton: String,
    /// `Some(name)` iff the pattern's leaf (final) segment is a named
    /// wildcard capture; `None` for both "no leaf position" (odd total
    /// segment length) and "a literal-valued leaf" (routing never inspects
    /// the leaf structurally, ADR-063 § Decision — Routing Composition —
    /// only a captured NAME is ever bound).
    pub leaf_variable: Option<String>,
    pub read_condition: Option<String>,
    pub write_condition: Option<String>,
}

/// The decomposition target for ONE terminal, even-prefix recursive-wildcard
/// `match` block (feature `security-rules-cel-recursive-wildcards`, Slice
/// 01, US-01, ADR-064 § Decision — Parser). `fixed_prefix_pattern` is the
/// pattern's own FIXED PREFIX only (e.g. `"expeditions/{expeditionId}"`) —
/// possibly EMPTY (`""`) for a project-wide `{document=**}` catch-all
/// (AC-17-237). The recursive wildcard's own captured name is never
/// retained — it carries no usable binding, structurally (Resolution 3:
/// no condition may reference the captured remainder). Ready for
/// `SystemDb::upsert_access_rule_pattern` (`is_recursive: true`) unmodified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecomposedRecursivePattern {
    pub fixed_prefix_pattern: String,
    pub fixed_prefix_segment_count: u16,
    pub literal_skeleton_prefix: String,
    pub read_condition: Option<String>,
    pub write_condition: Option<String>,
}

/// One `match` block's decomposition target — additive over 4a's own
/// `DecomposedRule` (ADR-063 § Decision — Widened `decompose_block`
/// Shape-Check). `decompose()`'s return type carries this enum so the admin
/// import handler can branch to the correct storage call
/// (`upsert_access_rule`/`upsert_write_access_rule` vs.
/// `upsert_access_rule_pattern`) without either storage shape needing to
/// know about the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecomposedTarget {
    /// `ancestor_segments.len() == 1` — 4a's own shape, `DecomposedRule`
    /// completely unchanged.
    SingleCollection(DecomposedRule),
    /// `ancestor_segments.len() > 1` — this feature's own new shape.
    MultiSegmentPattern(DecomposedPatternRule),
    /// A terminal, even-prefix recursive-wildcard pattern (feature
    /// `security-rules-cel-recursive-wildcards`, ADR-064) — additive,
    /// 4a's/4b's own variants untouched.
    RecursiveWildcardPattern(DecomposedRecursivePattern),
}

/// One offending `match` block, naming what about it is out of v1 scope.
/// Slice 01 only ever produces `"SYNTAX_ERROR"` (a generic parse problem) —
/// the richer named-construct vocabulary (`"NESTED_PATH"` /
/// `"RECURSIVE_WILDCARD"` / `"CUSTOM_FUNCTION"` /
/// `"CONFLICTING_VERB_CONDITIONS"` / `"UNSUPPORTED_EXPRESSION_GRAMMAR"`) is
/// reachable from this same type (ADR-062's own accepted shape) but only
/// exercised/tested starting Slice 04. `"CROSS_DOCUMENT_READ"` was removed
/// (security-rules-cel-cross-document-reads, ADR-066) — a narrowly-scoped
/// `get()`/`exists()` idiom is now a supported construct, never
/// unconditionally rejected.
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
    /// `security-rules-cel-chaining-detection`: the `MatchBlock`s that DID
    /// parse successfully at Stage 1, alongside the ones that didn't —
    /// carried so the caller can still hand them to `decompose()` (Stage 2)
    /// instead of losing the opportunity to find THEIR own offending
    /// constructs (`NESTED_PATH`, `UNSUPPORTED_EXPRESSION_GRAMMAR`, etc.).
    /// Empty for a genuinely-structural failure (unbalanced brace, wrong
    /// shell keyword) — there is no well-defined partial block list in that
    /// case, matching today's unchanged fail-fast behavior for those.
    pub partial_blocks: Vec<MatchBlock>,
}

impl RulesFileError {
    fn single(path_pattern: impl Into<String>, construct: &'static str, detail: impl Into<String>) -> Self {
        RulesFileError {
            offending_blocks: vec![OffendingBlock {
                path_pattern: path_pattern.into(),
                construct,
                detail: detail.into(),
            }],
            partial_blocks: Vec::new(),
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

    // security-rules-cel-functions (Slice 01, US-01, ADR-067 § Decision —
    // Threading): zero or more `function <name>() { return <expr>; }`
    // blocks, siblings of the `match /databases/{database}/documents { ...
    // }` sub-block, mirroring real Firestore's own declaration position
    // (§ Reading Confirmation). Consumed FIRST, leaving the remaining text
    // for the existing `match /databases/{database}/documents` shell check
    // below, completely unmodified.
    let (functions, service_body) = parse_function_blocks(service_body)?;

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

    parse_match_blocks(documents_body, &functions)
}

/// security-rules-cel-functions (Slice 01, US-01, ADR-067 § Decision —
/// Types): repeatedly scans the LEADING portion of `service_body` for
/// `function <name>() { return <expr>; }` blocks — reuses
/// `find_matching_close` unchanged for the `{...}` body, mirroring
/// `parse_nested_match_blocks`'s own "repeatedly scan a keyword-prefixed
/// block until exhausted" loop shape exactly, just for a flat
/// (non-nesting) result and a different keyword. Stops at the first
/// non-`function`-prefixed content (the `match /databases/{database}/
/// documents { ... }` sub-block, in every well-formed file) and returns
/// that REMAINING slice unmodified, alongside the consumed definitions.
///
/// A function body is validated via the UNCHANGED `parse_condition` at
/// THIS scan step (fail-fast, before any `match` block is even reached) —
/// this is what makes nesting/recursion structurally impossible
/// (Resolution 3): a body containing a call-shaped identifier is rejected
/// by `detect_unsupported_construct`'s own existing, unmodified scan,
/// since no expansion pass is ever applied to a function body itself.
fn parse_function_blocks(service_body: &str) -> Result<(BTreeMap<String, String>, &str), RulesFileError> {
    let mut functions = BTreeMap::new();
    let mut body = service_body;
    loop {
        body = body.trim_start();
        let after_function = match body.strip_prefix("function").filter(|rest| rest.starts_with(char::is_whitespace)) {
            Some(rest) => rest.trim_start(),
            None => break,
        };
        let paren_open = after_function
            .find('(')
            .ok_or_else(|| shell_syntax_error("expected '(' after a function name"))?;
        let name = after_function[..paren_open].trim();
        if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(shell_syntax_error(&format!("'{name}' is not a valid function name")));
        }
        if matches!(name, "get" | "exists" | "duration" | "true" | "false" | "in") {
            return Err(shell_syntax_error(&format!(
                "'{name}' collides with an existing reserved word/construct and cannot be used as \
                 a function name"
            )));
        }
        let paren_close = after_function[paren_open..]
            .find(')')
            .map(|i| paren_open + i)
            .ok_or_else(|| shell_syntax_error(&format!("unterminated '(' in function '{name}'")))?;
        let params = after_function[paren_open + 1..paren_close].trim();
        if !params.is_empty() {
            return Err(RulesFileError::single(
                String::new(),
                "FUNCTION_PARAMETERS_UNSUPPORTED",
                format!(
                    "function '{name}' declares parameter(s) ('{params}') — only zero-parameter \
                     functions are supported in v1"
                ),
            ));
        }
        let after_params = after_function[paren_close + 1..].trim_start();
        if !after_params.starts_with('{') {
            return Err(shell_syntax_error(&format!("expected '{{' after function '{name}'s own '()'")));
        }
        let close_idx = find_matching_close(after_params, 0)
            .ok_or_else(|| shell_syntax_error(&format!("unbalanced '{{' in function '{name}'")))?;
        let fn_body = after_params[1..close_idx].trim();

        let expr = fn_body
            .strip_prefix("return")
            .filter(|rest| rest.starts_with(char::is_whitespace))
            .map(str::trim)
            .and_then(|rest| rest.strip_suffix(';'))
            .map(str::trim)
            .ok_or_else(|| {
                shell_syntax_error(&format!(
                    "function '{name}'s own body must be exactly 'return <expr>;' — no 'let' \
                     bindings, no other statements, in v1"
                ))
            })?;
        if expr.is_empty() {
            return Err(shell_syntax_error(&format!("function '{name}'s own 'return' has no expression")));
        }
        // Validated via the UNCHANGED parse_condition — fail-fast on a
        // malformed body, and structurally forbids nesting/recursion (a
        // call-shaped identifier inside `expr` is rejected here, unchanged,
        // by `detect_unsupported_construct`).
        parse_condition(expr).map_err(|e| {
            RulesFileError::single(String::new(), construct_for(&e), format!("function '{name}': {}", detail_for(e)))
        })?;

        if functions.insert(name.to_string(), expr.to_string()).is_some() {
            return Err(RulesFileError::single(
                String::new(),
                "DUPLICATE_FUNCTION",
                format!("function '{name}' is defined more than once"),
            ));
        }

        body = &after_params[close_idx + 1..];
    }
    Ok((functions, body))
}

/// security-rules-cel-functions (Slice 01, US-01, ADR-067 § Decision —
/// Types): a single quote-aware linear scan over `condition_text`, splicing
/// `(<body>)` in place of each `<name>()` call site found OUTSIDE a `"..."`
/// or `'...'` literal, for every `name` present in `functions`. Mirrors
/// `tokenize`'s own quote-handling discipline as a DESIGN PRINCIPLE (track
/// literal-state, skip scanning inside it) — not shared code, this scanner
/// operates on raw text at a structurally earlier pipeline stage.
///
/// Applied EXACTLY ONCE per condition, never recursively: a function body
/// is pre-validated flat by `parse_function_blocks`'s own `parse_condition`
/// call, so the spliced-in body text can never itself contain a further
/// `name()` needing expansion.
fn expand_function_calls(
    condition_text: &str,
    functions: &BTreeMap<String, String>,
    path_pattern: &str,
) -> Result<String, RulesFileError> {
    let chars: Vec<char> = condition_text.chars().collect();
    let mut out = String::with_capacity(condition_text.len());
    let mut i = 0;
    let mut in_string: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if let Some(quote) = in_string {
            out.push(c);
            if c == quote {
                in_string = None;
            }
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' {
            in_string = Some(c);
            out.push(c);
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            // Includes '.' as a continuation character — mirrors
            // `tokenize`'s own dotted-identifier Word scan exactly, so a
            // compound name (`duration.value`, `resource.data.role`,
            // `request.auth.uid`) is always captured as ONE ident here too,
            // never mis-split into a bare trailing segment that could be
            // mistaken for its own standalone function-call candidate
            // (`value(` inside `duration.value(24, 'h')`, e.g.).
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect();
            let followed_by_call = chars.get(i) == Some(&'(');

            if let Some(body) = functions.get(&ident) {
                if followed_by_call {
                    let close = find_char_paren_close(&chars, i).ok_or_else(|| {
                        RulesFileError::single(
                            path_pattern,
                            "SYNTAX_ERROR",
                            format!("unterminated '(' in call to function '{ident}'"),
                        )
                    })?;
                    let args = chars[i + 1..close].iter().collect::<String>();
                    if !args.trim().is_empty() {
                        return Err(RulesFileError::single(
                            path_pattern,
                            "FUNCTION_PARAMETERS_UNSUPPORTED",
                            format!(
                                "call to function '{ident}' passes argument(s) ('{}') — only \
                                 zero-argument calls are supported in v1",
                                args.trim()
                            ),
                        ));
                    }
                    out.push('(');
                    out.push_str(body);
                    out.push(')');
                    i = close + 1;
                    continue;
                }
                // A bare reference to a known function name with no
                // following '(' at all — left untouched; the existing
                // grammar handles (or rejects) it exactly as it already
                // does today.
                out.push_str(&ident);
                continue;
            }

            // Not a known function. `get`/`exists`/`duration.value` are
            // ALSO call-shaped identifiers, but pre-existing, tokenizer
            // -level constructs (never this feature's own concern) — passed
            // through unchanged, exactly like today. Any OTHER call-shaped
            // identifier is a call to an UNDEFINED function — named here,
            // distinguishable from the generic `CUSTOM_FUNCTION` rejection
            // a bare, non-imported condition still gets (Resolution 1's own
            // safety-net corollary covers the case this check does NOT
            // catch: a bug that leaves a KNOWN function's own call site
            // unexpanded — structurally impossible here, since every
            // `followed_by_call` branch above always either expands or
            // returns `Err` before reaching this point).
            if followed_by_call && ident != "get" && ident != "exists" && ident != "duration.value" {
                return Err(RulesFileError::single(
                    path_pattern,
                    "UNDEFINED_FUNCTION",
                    format!("no function named '{ident}' is defined in this file"),
                ));
            }
            out.push_str(&ident);
            continue;
        }
        out.push(c);
        i += 1;
    }
    Ok(out)
}

/// Bounds-safe: finds the `)` matching the `(` at `chars[open_idx]`,
/// honoring nesting — mirrors `access_control::mod`'s own
/// `find_matching_paren` shape exactly, for a `Vec<char>` scan over raw
/// (pre-tokenized) rules-file text rather than a condition string.
fn find_char_paren_close(chars: &[char], open_idx: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, &c) in chars.iter().enumerate().skip(open_idx) {
        match c {
            '(' => depth += 1,
            ')' => {
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
/// }` blocks, in order, until exhausted. Entry point — always the
/// top-level scan, no ancestor to prepend (ADR-063 § Decision — Nested
/// Match-Block Flattening).
fn parse_match_blocks(body: &str, functions: &BTreeMap<String, String>) -> Result<Vec<MatchBlock>, RulesFileError> {
    let blocks = parse_nested_match_blocks(body, "", &[], functions)?;
    if blocks.is_empty() {
        return Err(shell_syntax_error("expected at least one 'match /<path> { ... }' block"));
    }
    Ok(blocks)
}

/// Repeatedly scan `body` for `match /<pattern> { ... }` blocks, prepending
/// `parent_path_text`/`parent_segments` onto each one found — the recursive
/// step that flattens a nested `match { match { ... } } }` shell (ADR-063 §
/// Decision — Nested Match-Block Flattening) to the identical
/// `Vec<PathSegment>`/path-pattern-text shape flat multi-segment syntax
/// already produces (`Some("")`/`&[]` at the top level).
fn parse_nested_match_blocks(
    mut body: &str,
    parent_path_text: &str,
    parent_segments: &[PathSegment],
    functions: &BTreeMap<String, String>,
) -> Result<Vec<MatchBlock>, RulesFileError> {
    let mut blocks = Vec::new();
    let mut offending: Vec<OffendingBlock> = Vec::new();
    loop {
        body = body.trim_start();
        if body.is_empty() {
            break;
        }
        // ── Structural delineation — TRUE fail-fast, no well-defined resume
        //    point if any of these fail (unchanged from today). ──
        let after_match = body
            .strip_prefix("match")
            .filter(|rest| rest.starts_with(char::is_whitespace))
            .ok_or_else(|| {
                shell_syntax_error(
                    "expected a 'match /<path> { ... }' block — a match block body may not mix \
                     'allow' clauses with nested 'match' blocks",
                )
            })?
            .trim_start();
        let pattern_end = find_path_pattern_end(after_match)
            .ok_or_else(|| shell_syntax_error("expected a '/'-prefixed match block path pattern"))?;
        let local_path_pattern = after_match[..pattern_end].trim().to_string();
        let after_pattern = after_match[pattern_end..].trim_start();
        if !after_pattern.starts_with('{') {
            return Err(shell_syntax_error("expected '{' after a match block's path pattern"));
        }
        let rest = after_pattern;
        let close_idx = find_matching_close(rest, 0).ok_or_else(|| {
            shell_syntax_error(&format!("unbalanced '{{' in match block '{local_path_pattern}'"))
        })?;
        let block_body = &rest[1..close_idx];

        // ── Content parsing — this block's own byte range is now fully
        //    known (`&rest[close_idx + 1..]` is always a safe resume point,
        //    computed purely from brace-depth counting above, independent
        //    of whether THIS block's own content is valid). A problem here
        //    is this ONE block's own concern, never a reason to abandon its
        //    siblings — mirrors decompose()'s own accumulate-and-continue
        //    loop, one syntactic layer earlier. ──
        match parse_path_segments(&local_path_pattern) {
            Ok(local_segments) => {
                let full_path_pattern = format!("{parent_path_text}{local_path_pattern}");
                let mut full_segments = parent_segments.to_vec();
                full_segments.extend(local_segments);

                match parse_block_body(block_body, &full_path_pattern, &full_segments, functions) {
                    Ok(mut nested) => blocks.append(&mut nested),
                    Err(mut err) => offending.append(&mut err.offending_blocks),
                }
            }
            Err(mut err) => offending.append(&mut err.offending_blocks),
        }

        body = &rest[close_idx + 1..];
    }
    if !offending.is_empty() {
        return Err(RulesFileError { offending_blocks: offending, partial_blocks: blocks });
    }
    Ok(blocks)
}

/// A match block's own body is either EXCLUSIVELY `allow` clauses (a leaf
/// block) or EXCLUSIVELY further nested `match /<pattern> { ... }` blocks —
/// never a mix (ADR-063 § Decision — Nested Match-Block Flattening,
/// DDD-PM-8). Dispatches on the body's first token.
fn parse_block_body(
    block_body: &str,
    full_path_pattern: &str,
    full_segments: &[PathSegment],
    functions: &BTreeMap<String, String>,
) -> Result<Vec<MatchBlock>, RulesFileError> {
    let trimmed = block_body.trim_start();
    let starts_with_nested_match = trimmed
        .strip_prefix("match")
        .is_some_and(|rest| rest.starts_with(char::is_whitespace));

    if starts_with_nested_match {
        return parse_nested_match_blocks(block_body, full_path_pattern, full_segments, functions);
    }

    let allow_clauses = parse_allow_clauses(block_body, full_path_pattern, functions)?;
    Ok(vec![MatchBlock {
        path_pattern: full_path_pattern.to_string(),
        segments: full_segments.to_vec(),
        allow_clauses,
    }])
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

/// Split a `/`-delimited path-pattern (or a concrete request path — the SAME
/// scanner, ADR-063 § Decision — Shared Matching Primitives) into segments.
/// `pub` (Slice 02, `security-rules-cel-path-matching`, ADR-063): reused
/// verbatim by `access_control::path_routing::bind_ancestor`'s own caller
/// (`grpc::handler`) to represent a concrete request's own ancestor path as
/// `Vec<PathSegment>` — the identical splitter proven on stored pattern
/// text, never a second one.
pub fn parse_path_segments(pattern: &str) -> Result<Vec<PathSegment>, RulesFileError> {
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
fn parse_allow_clauses(
    block_body: &str,
    path_pattern: &str,
    functions: &BTreeMap<String, String>,
) -> Result<Vec<(Vec<Verb>, String)>, RulesFileError> {
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

        // security-rules-cel-functions (Slice 01, US-01, ADR-067 § Decision
        // — Threading): the LAST point before `condition_text` becomes
        // `MatchBlock`'s own stored, owned data — every call site of a
        // known function is expanded here, so `decompose()`/`evaluate()`
        // never learn this feature exists.
        let expanded = expand_function_calls(condition_text, functions, path_pattern)?;

        clauses.push((verbs, expanded));
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
pub fn decompose(blocks: Vec<MatchBlock>) -> Result<Vec<DecomposedTarget>, RulesFileError> {
    let mut offending = Vec::new();
    let mut targets = Vec::new();

    for block in &blocks {
        match decompose_block(block) {
            Ok(target) => targets.push(target),
            Err(mut err) => offending.append(&mut err.offending_blocks),
        }
    }

    if !offending.is_empty() {
        return Err(RulesFileError { offending_blocks: offending, partial_blocks: Vec::new() });
    }
    Ok(targets)
}

/// Widened shape-check (ADR-063 § Decision — Widened `decompose_block`
/// Shape-Check; further widened by ADR-064 § Decision — Parser):
/// `segments` is valid iff every even index holds `Literal` and every odd
/// index holds `Wildcard` or `Literal`, for any length ≥ 1 — with ONE
/// additional accepted shape (feature `security-rules-cel-recursive-
/// wildcards`, Slice 01, US-01): a `RecursiveWildcard` segment is valid iff
/// it is the pattern's own FINAL segment AND sits at an even index (i.e.
/// the segments preceding it have EVEN total length — the direct
/// generalization of "every even index holds `Literal`", since prefix
/// -length-even means the recursive wildcard's own index equals the
/// (even) prefix length). A `RecursiveWildcard` anywhere but the final
/// position is `RECURSIVE_WILDCARD_NOT_TERMINAL`; at an odd index it is
/// `RECURSIVE_WILDCARD_ODD_PREFIX` (out of this feature's own locked v1
/// scope, Resolution 3 — deferred, unevidenced).
///
/// `pub` (security-rules-cel-recursive-wildcards, Slice 06, US-06, ADR-064):
/// `embyr_server`'s `simulate_routed_access_rule` (a different crate — deny.
/// toml's `embyr-core` IO-free boundary is unaffected, this is pure) needs
/// this IDENTICAL shape check for a candidate pattern that never goes
/// through `decompose_block` (simulate's `pattern`/`condition` are separate
/// fields, not rules-file block text) — reused directly rather than
/// re-implemented, never a second, independently-maintained copy.
pub fn validate_segment_shape(segments: &[PathSegment], path_pattern: &str) -> Result<(), RulesFileError> {
    for (i, seg) in segments.iter().enumerate() {
        let is_last = i == segments.len() - 1;
        match seg {
            PathSegment::RecursiveWildcard if !is_last => {
                return Err(RulesFileError::single(
                    path_pattern,
                    "RECURSIVE_WILDCARD_NOT_TERMINAL",
                    "a recursive wildcard segment must be the pattern's own final segment",
                ));
            }
            PathSegment::RecursiveWildcard if !i.is_multiple_of(2) => {
                return Err(RulesFileError::single(
                    path_pattern,
                    "RECURSIVE_WILDCARD_ODD_PREFIX",
                    "a recursive wildcard segment must occupy an even-indexed (collection-name) \
                     position — an odd-prefix recursive wildcard is out of this feature's own \
                     locked v1 scope",
                ));
            }
            PathSegment::RecursiveWildcard => {} // terminal, even prefix — valid (v1 scope)
            other if i.is_multiple_of(2) && !matches!(other, PathSegment::Literal(_)) => {
                return Err(RulesFileError::single(
                    path_pattern,
                    "NESTED_PATH",
                    "a collection-name position must be a literal segment, never a wildcard",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Every `/`-delimited segment, rendered back to its canonical textual
/// form (`Literal(s) -> s`, `Wildcard(name) -> "{name}"`) and joined with
/// `/` — used to build `DecomposedPatternRule::collection_path_pattern`
/// (ADR-063 § Decision — Schema).
fn render_ancestor_pattern(ancestor: &[PathSegment]) -> String {
    ancestor
        .iter()
        .map(|seg| match seg {
            PathSegment::Literal(s) => s.clone(),
            PathSegment::Wildcard(name) => format!("{{{name}}}"),
            PathSegment::RecursiveWildcard => unreachable!("rejected by validate_segment_shape"),
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn decompose_block(block: &MatchBlock) -> Result<DecomposedTarget, RulesFileError> {
    validate_segment_shape(&block.segments, &block.path_pattern)?;

    let segments = block.segments.as_slice();

    if matches!(segments.last(), Some(PathSegment::RecursiveWildcard)) {
        // security-rules-cel-recursive-wildcards (Slice 01, US-01, ADR-064 §
        // Decision — Parser): terminal, even-prefix recursive wildcard —
        // guaranteed by validate_segment_shape above. Split off the fixed
        // prefix (possibly EMPTY, the project-wide `{document=**}` case,
        // AC-17-237) and decompose it via the IDENTICAL wildcard-name
        // -collection + condition-rewrite + verb-bucketing loop the
        // ancestor branch below already runs — inlined here, not a new
        // function (ADR-064). The recursive wildcard's own captured name is
        // never added to the rewrite list — `PathSegment::RecursiveWildcard`
        // is unit-like, carrying no name data at all — directly enforcing
        // Resolution 3's own lock structurally.
        let fixed_prefix = &segments[..segments.len() - 1];

        let wildcard_names: Vec<String> = fixed_prefix
            .iter()
            .filter_map(|s| match s {
                PathSegment::Wildcard(name) => Some(name.clone()),
                _ => None,
            })
            .collect();

        let mut read_condition: Option<String> = None;
        let mut write_condition: Option<String> = None;

        for (verbs, raw_condition) in &block.allow_clauses {
            let mut rewritten = raw_condition.clone();
            for var in &wildcard_names {
                rewritten = rewrite_path_variable(&rewritten, var);
            }

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

        return Ok(DecomposedTarget::RecursiveWildcardPattern(DecomposedRecursivePattern {
            fixed_prefix_pattern: render_ancestor_pattern(fixed_prefix),
            fixed_prefix_segment_count: fixed_prefix.len() as u16,
            literal_skeleton_prefix: path_routing::literal_skeleton(fixed_prefix),
            read_condition,
            write_condition,
        }));
    }

    let len = segments.len();
    // Ancestor/leaf split (ADR-063 § Decision — The Ancestor/Leaf Split):
    // a leaf position is present iff `len` is even; ancestor length is
    // always odd by construction.
    let (ancestor, leaf): (&[PathSegment], Option<&PathSegment>) = if len.is_multiple_of(2) {
        (&segments[..len - 1], segments.last())
    } else {
        (segments, None)
    };

    // A named wildcard leaf capture is retained by name (AC-17-204); a
    // literal-valued leaf (or no leaf position at all) contributes no
    // binding — routing never inspects the leaf structurally (ADR-063 §
    // Decision — Routing Composition, "Routing touches ONLY the ancestor").
    let leaf_variable: Option<String> = match leaf {
        Some(PathSegment::Wildcard(name)) => Some(name.clone()),
        _ => None,
    };

    // Every DISTINCT wildcard name across the whole pattern (ancestor AND
    // leaf) must be rewritten in the condition text, never just one
    // (ADR-063 § Decision — Widened `decompose_block` Shape-Check, "a loop
    // over Wildcard segments instead of a single optional one").
    let mut wildcard_names: Vec<String> = ancestor
        .iter()
        .filter_map(|s| match s {
            PathSegment::Wildcard(name) => Some(name.clone()),
            _ => None,
        })
        .collect();
    wildcard_names.extend(leaf_variable.clone());

    let mut read_condition: Option<String> = None;
    let mut write_condition: Option<String> = None;

    for (verbs, raw_condition) in &block.allow_clauses {
        let mut rewritten = raw_condition.clone();
        for var in &wildcard_names {
            rewritten = rewrite_path_variable(&rewritten, var);
        }

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

    if ancestor.len() == 1 {
        // 4a's own shape (ADR-062), completely unchanged.
        let PathSegment::Literal(collection) = &ancestor[0] else {
            unreachable!("validate_segment_shape guarantees a literal at an even index")
        };
        Ok(DecomposedTarget::SingleCollection(DecomposedRule {
            collection_path: collection.clone(),
            read_condition,
            write_condition,
        }))
    } else {
        Ok(DecomposedTarget::MultiSegmentPattern(DecomposedPatternRule {
            collection_path_pattern: render_ancestor_pattern(ancestor),
            ancestor_segment_count: ancestor.len() as u16,
            literal_skeleton: path_routing::literal_skeleton(ancestor),
            leaf_variable,
            read_condition,
            write_condition,
        }))
    }
}

fn construct_for(e: &ConditionParseError) -> &'static str {
    match e {
        ConditionParseError::SyntaxError { .. } => "SYNTAX_ERROR",
        ConditionParseError::UnsupportedConstruct { construct, .. } => match construct {
            UnsupportedConstruct::CustomFunction => "CUSTOM_FUNCTION",
            UnsupportedConstruct::WildcardPath => "NESTED_PATH",
            UnsupportedConstruct::UnsupportedExpressionGrammar => "UNSUPPORTED_EXPRESSION_GRAMMAR",
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
    //!
    //! Widened (`security-rules-cel-path-matching`, Slice 01, US-01,
    //! ADR-063) — NEW distinct behaviors this slice introduces: (8)
    //! decompose accepts a flat multi-segment pattern, splitting
    //! ancestor/leaf correctly (supersedes old behavior 7's rejection —
    //! DESIGN-authorized scope widening, ADR-063, not a test weakening);
    //! (9) a nested `match { match { ... } } }` shell flattens to the
    //! IDENTICAL `DecomposedTarget` the flat form produces; (10) every
    //! distinct wildcard name (ancestor AND leaf) is rewritten without
    //! collision; (11) a wildcard at a collection-name position is still
    //! rejected (`NESTED_PATH`); (12) a recursive wildcard inside a
    //! multi-segment pattern is still rejected (`RECURSIVE_WILDCARD`); (13)
    //! one file mixing a single-collection and a multi-segment pattern
    //! decomposes both correctly; (14) a nested match-block body mixing
    //! `allow` with a further nested `match` is a `SYNTAX_ERROR`. 6 NEW
    //! behaviors x 2 = 12 budget (behavior 8 subsumes old behavior 7, not
    //! double-counted); 7 new tests used.

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
                Err(RulesFileError { offending_blocks, .. }) => {
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
            DecomposedTarget::SingleCollection(DecomposedRule {
                collection_path: "journal_entries".to_string(),
                read_condition: Some("request.auth.uid == resource.data.owner_id".to_string()),
                write_condition: None,
            })
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
            DecomposedTarget::SingleCollection(DecomposedRule {
                collection_path: "profiles".to_string(),
                read_condition: Some(expected_condition.clone()),
                write_condition: Some(expected_condition.clone()),
            })
        );
        // The rewritten condition must itself parse via the SAME,
        // unmodified parse_condition real enforcement/simulation use —
        // proves the rewrite step produces grammar-valid text, not just a
        // plausible-looking string.
        assert!(parse_condition(&expected_condition).is_ok());
    }

    // -----------------------------------------------------------------------
    // security-rules-cel-path-matching (Slice 01, US-01, ADR-063) — widened
    // decompose_block, nested match-block flattening.
    // -----------------------------------------------------------------------

    /// Supersedes the old `decompose_rejects_a_nested_multi_segment_path`
    /// test — DESIGN-authorized scope widening (ADR-063), the identical
    /// input this feature exists to accept.
    #[test]
    fn decompose_accepts_a_flat_multi_segment_pattern_and_splits_ancestor_leaf() {
        let blocks = parse_rules_file(
            r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /expeditions/{expeditionId}/journal_entries/{entryId} {
                  allow read, write: if request.auth.uid == resource.data.owner_id;
                }
              }
            }
        "#,
        )
        .expect("must parse");

        let targets = decompose(blocks).expect("must decompose");
        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0],
            DecomposedTarget::MultiSegmentPattern(DecomposedPatternRule {
                collection_path_pattern: "expeditions/{expeditionId}/journal_entries".to_string(),
                ancestor_segment_count: 3,
                literal_skeleton: "expeditions/journal_entries".to_string(),
                leaf_variable: Some("entryId".to_string()),
                read_condition: Some("request.auth.uid == resource.data.owner_id".to_string()),
                write_condition: Some("request.auth.uid == resource.data.owner_id".to_string()),
            })
        );
    }

    #[test]
    fn nested_match_block_flattens_to_the_identical_decomposed_target_as_flat_syntax() {
        let flat = parse_rules_file(
            r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /expeditions/{expeditionId}/journal_entries/{entryId} {
                  allow read, write: if request.auth.uid == resource.data.owner_id;
                }
              }
            }
        "#,
        )
        .expect("must parse flat form");
        let nested = parse_rules_file(
            r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /expeditions/{expeditionId} {
                  match /journal_entries/{entryId} {
                    allow read, write: if request.auth.uid == resource.data.owner_id;
                  }
                }
              }
            }
        "#,
        )
        .expect("must parse nested form");

        let flat_targets = decompose(flat).expect("must decompose flat form");
        let nested_targets = decompose(nested).expect("must decompose nested form");
        assert_eq!(
            flat_targets, nested_targets,
            "AC-17-203: a nested match-block shell must decompose to the identical DecomposedTarget the flat form produces"
        );
    }

    #[test]
    fn decompose_rewrites_every_distinct_wildcard_name_across_ancestor_and_leaf_without_collision() {
        let blocks = parse_rules_file(
            r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /expeditions/{expeditionId}/journal_entries/{entryId} {
                  allow read: if expeditionId != entryId;
                }
              }
            }
        "#,
        )
        .expect("must parse");

        let targets = decompose(blocks).expect("must decompose");
        let DecomposedTarget::MultiSegmentPattern(pattern) = &targets[0] else {
            panic!("expected a MultiSegmentPattern target");
        };
        let expected = "request.path.expeditionId != request.path.entryId".to_string();
        assert_eq!(
            pattern.read_condition,
            Some(expected.clone()),
            "AC-17-204: both distinct wildcard names must be rewritten, never colliding with each other"
        );
        assert!(parse_condition(&expected).is_ok());
    }

    #[test]
    fn decompose_rejects_a_wildcard_at_a_collection_name_position() {
        let blocks = parse_rules_file(
            r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /{expeditionId}/journal_entries {
                  allow read: if true;
                }
              }
            }
        "#,
        )
        .expect("must parse");

        match decompose(blocks) {
            Err(RulesFileError { offending_blocks, .. }) => {
                assert_eq!(offending_blocks[0].construct, "NESTED_PATH");
            }
            Ok(_) => panic!("expected NESTED_PATH rejection for a wildcard at a collection-name position"),
        }
    }

    /// Taxonomy note (security-rules-cel-recursive-wildcards, ADR-064 §
    /// Decision — Parser, pre-flagged by DESIGN): this input's own recursive
    /// wildcard sits at index 3 (an ODD prefix, `expeditions/{expeditionId}/
    /// journal_entries` = 3 preceding segments) — under the widened
    /// taxonomy this is `RECURSIVE_WILDCARD_ODD_PREFIX`, not the old bare
    /// `RECURSIVE_WILDCARD`. The SHAPE is still rejected; only the construct
    /// label narrows to name the specific reason. Not a regression.
    #[test]
    fn decompose_still_rejects_a_recursive_wildcard_at_an_odd_prefix_position() {
        let blocks = parse_rules_file(
            r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /expeditions/{expeditionId}/journal_entries/{name=**} {
                  allow read: if true;
                }
              }
            }
        "#,
        )
        .expect("must parse");

        match decompose(blocks) {
            Err(RulesFileError { offending_blocks, .. }) => {
                assert_eq!(offending_blocks[0].construct, "RECURSIVE_WILDCARD_ODD_PREFIX");
            }
            Ok(_) => panic!("expected RECURSIVE_WILDCARD_ODD_PREFIX rejection"),
        }
    }

    // -----------------------------------------------------------------------
    // security-rules-cel-recursive-wildcards (Slice 01, US-01, ADR-064) —
    // widened validate_segment_shape + decompose_block RecursiveWildcardPattern
    // branch.
    //
    // Test Budget: 4 NEW distinct behaviors this slice introduces — (1) a
    // terminal, even-prefix recursive wildcard decomposes into a
    // `RecursiveWildcardPattern`, including the empty-fixed-prefix case
    // (AC-17-232/237, parametrized as 2 variations of the SAME behavior); (2)
    // an odd-prefix recursive wildcard is rejected with
    // `RECURSIVE_WILDCARD_ODD_PREFIX` (AC-17-233, covered above by the
    // updated pre-flagged test — not double-counted here); (3) a non-terminal
    // recursive wildcard is rejected with `RECURSIVE_WILDCARD_NOT_TERMINAL`
    // (AC-17-234); (4) a file mixing a recursive-wildcard pattern with 4a's
    // own shape decomposes both correctly (AC-17-236). 4 behaviors x 2 = 8
    // budget; 3 new tests used (behavior 1 parametrized).
    // -----------------------------------------------------------------------

    #[test]
    fn decompose_accepts_a_terminal_even_prefix_recursive_wildcard() {
        // Narrower, prefix-scoped case (AC-17-232): fixed prefix
        // `expeditions/{expeditionId}`, the prefix's own wildcard capture
        // rewritten via the IDENTICAL mechanism 4a/4b already use.
        let blocks = parse_rules_file(
            r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /expeditions/{expeditionId}/{path=**} {
                  allow read: if expeditionId != "";
                }
              }
            }
        "#,
        )
        .expect("must parse");
        let targets = decompose(blocks).expect("must decompose");
        assert_eq!(
            targets[0],
            DecomposedTarget::RecursiveWildcardPattern(DecomposedRecursivePattern {
                fixed_prefix_pattern: "expeditions/{expeditionId}".to_string(),
                fixed_prefix_segment_count: 2,
                literal_skeleton_prefix: "expeditions".to_string(),
                read_condition: Some("request.path.expeditionId != \"\"".to_string()),
                write_condition: None,
            })
        );

        // Empty-fixed-prefix, project-wide catch-all case (AC-17-237) — a
        // valid, importable shape, not a degenerate error.
        let catch_all_blocks = parse_rules_file(
            r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /{document=**} {
                  allow read, write: if false;
                }
              }
            }
        "#,
        )
        .expect("must parse");
        let catch_all_targets = decompose(catch_all_blocks).expect("must decompose");
        assert_eq!(
            catch_all_targets[0],
            DecomposedTarget::RecursiveWildcardPattern(DecomposedRecursivePattern {
                fixed_prefix_pattern: String::new(),
                fixed_prefix_segment_count: 0,
                literal_skeleton_prefix: String::new(),
                read_condition: Some("false".to_string()),
                write_condition: Some("false".to_string()),
            }),
            "AC-17-237: the empty-fixed-prefix case must decompose to a valid, storable shape"
        );
    }

    /// Mutation-testing gap closed (security-rules-cel-recursive-wildcards,
    /// Slice 06 QUALITY_GATE): the recursive branch's own copy of the
    /// same-bucket conflicting-condition guard (`decompose_block`'s
    /// `Some(existing) if *existing != rewritten`, fixed-prefix loop) had
    /// no test exercising a genuine conflict — two `allow read` clauses in
    /// the SAME recursive-wildcard block with DIFFERENT conditions.
    #[test]
    fn decompose_rejects_conflicting_conditions_for_the_same_verb_bucket_in_a_recursive_wildcard_block()
    {
        let blocks = parse_rules_file(
            r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /expeditions/{expeditionId}/{path=**} {
                  allow read: if expeditionId == "trek-2026";
                  allow read: if expeditionId == "trek-2027";
                }
              }
            }
        "#,
        )
        .expect("must parse");

        match decompose(blocks) {
            Err(RulesFileError { offending_blocks, .. }) => {
                assert_eq!(offending_blocks[0].construct, "CONFLICTING_VERB_CONDITIONS");
            }
            Ok(_) => panic!("expected CONFLICTING_VERB_CONDITIONS rejection"),
        }
    }

    #[test]
    fn decompose_rejects_a_recursive_wildcard_that_is_not_the_final_segment() {
        let blocks = parse_rules_file(
            r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /expeditions/{path=**}/journal_entries {
                  allow read: if true;
                }
              }
            }
        "#,
        )
        .expect("must parse");

        match decompose(blocks) {
            Err(RulesFileError { offending_blocks, .. }) => {
                assert_eq!(offending_blocks[0].construct, "RECURSIVE_WILDCARD_NOT_TERMINAL");
            }
            Ok(_) => panic!("expected RECURSIVE_WILDCARD_NOT_TERMINAL rejection"),
        }
    }

    #[test]
    fn decompose_handles_a_file_mixing_a_recursive_wildcard_pattern_with_a_4a_shaped_block() {
        let blocks = parse_rules_file(
            r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /{document=**} {
                  allow read, write: if false;
                }
                match /profiles/{userId} {
                  allow read, write: if request.auth.uid == userId;
                }
              }
            }
        "#,
        )
        .expect("must parse");

        let targets = decompose(blocks).expect("must decompose");
        assert_eq!(targets.len(), 2);
        assert!(matches!(targets[0], DecomposedTarget::RecursiveWildcardPattern(_)));
        assert!(matches!(targets[1], DecomposedTarget::SingleCollection(_)));
    }

    #[test]
    fn decompose_handles_a_file_mixing_single_collection_and_multi_segment_patterns() {
        let blocks = parse_rules_file(
            r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /profiles/{userId} {
                  allow read, write: if request.auth.uid == userId;
                }
                match /expeditions/{expeditionId}/journal_entries/{entryId} {
                  allow read: if true;
                }
              }
            }
        "#,
        )
        .expect("must parse");

        let targets = decompose(blocks).expect("must decompose");
        assert_eq!(targets.len(), 2);
        assert!(matches!(targets[0], DecomposedTarget::SingleCollection(_)));
        assert!(matches!(targets[1], DecomposedTarget::MultiSegmentPattern(_)));
    }

    #[test]
    fn nested_match_block_body_mixing_allow_and_nested_match_is_a_syntax_error() {
        let source = r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /expeditions/{expeditionId} {
                  allow read: if true;
                  match /journal_entries/{entryId} {
                    allow read: if true;
                  }
                }
              }
            }
        "#;

        match parse_rules_file(source) {
            Err(RulesFileError { offending_blocks, .. }) => {
                assert_eq!(offending_blocks[0].construct, "SYNTAX_ERROR");
            }
            Ok(_) => panic!("expected SYNTAX_ERROR: a match block body may not mix 'allow' with nested 'match'"),
        }
    }

    // ── security-rules-cel-functions (Slice 01, US-01, ADR-067) ──

    const EDITOR_FILE: &str = r#"
        service cloud.firestore {
          function isEditor() {
            return request.auth.uid == resource.data.editor_id;
          }
          match /databases/{database}/documents {
            match /trail_guides/{guideId} {
              allow write: if isEditor();
            }
          }
        }
    "#;

    #[test]
    fn a_function_call_site_expands_into_the_functions_own_body_wrapped_in_parens() {
        let blocks = parse_rules_file(EDITOR_FILE).expect("must parse");
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].allow_clauses,
            vec![(
                vec![Verb::Write],
                "(request.auth.uid == resource.data.editor_id)".to_string()
            )]
        );
    }

    #[test]
    fn a_call_to_an_undefined_function_is_a_named_rejection() {
        let source = r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /trail_guides/{guideId} {
                  allow write: if isEditor();
                }
              }
            }
        "#;
        match parse_rules_file(source) {
            Err(RulesFileError { offending_blocks, .. }) => {
                assert_eq!(offending_blocks[0].construct, "UNDEFINED_FUNCTION");
                assert!(offending_blocks[0].detail.contains("isEditor"));
            }
            Ok(_) => panic!("expected UNDEFINED_FUNCTION"),
        }
    }

    #[test]
    fn two_function_definitions_sharing_the_same_name_is_a_named_rejection() {
        let source = r#"
            service cloud.firestore {
              function isEditor() { return true; }
              function isEditor() { return false; }
              match /databases/{database}/documents {
                match /trail_guides/{guideId} {
                  allow write: if isEditor();
                }
              }
            }
        "#;
        match parse_rules_file(source) {
            Err(RulesFileError { offending_blocks, .. }) => {
                assert_eq!(offending_blocks[0].construct, "DUPLICATE_FUNCTION");
            }
            Ok(_) => panic!("expected DUPLICATE_FUNCTION"),
        }
    }

    #[test]
    fn a_function_definition_with_a_parameter_is_a_named_rejection() {
        let source = r#"
            service cloud.firestore {
              function isEditor(userId) { return request.auth.uid == userId; }
              match /databases/{database}/documents {
                match /trail_guides/{guideId} {
                  allow write: if isEditor();
                }
              }
            }
        "#;
        match parse_rules_file(source) {
            Err(RulesFileError { offending_blocks, .. }) => {
                assert_eq!(offending_blocks[0].construct, "FUNCTION_PARAMETERS_UNSUPPORTED");
            }
            Ok(_) => panic!("expected FUNCTION_PARAMETERS_UNSUPPORTED"),
        }
    }

    #[test]
    fn a_call_site_passing_an_argument_to_a_defined_zero_parameter_function_is_a_named_rejection() {
        let source = r#"
            service cloud.firestore {
              function isEditor() { return request.auth.uid == resource.data.editor_id; }
              match /databases/{database}/documents {
                match /trail_guides/{guideId} {
                  allow write: if isEditor(guideId);
                }
              }
            }
        "#;
        match parse_rules_file(source) {
            Err(RulesFileError { offending_blocks, .. }) => {
                assert_eq!(offending_blocks[0].construct, "FUNCTION_PARAMETERS_UNSUPPORTED");
            }
            Ok(_) => panic!("expected FUNCTION_PARAMETERS_UNSUPPORTED"),
        }
    }

    #[test]
    fn a_function_body_that_is_not_exactly_return_expr_is_a_syntax_error() {
        let source = r#"
            service cloud.firestore {
              function isEditor() { let x = 1; return x == 1; }
              match /databases/{database}/documents {
                match /trail_guides/{guideId} {
                  allow write: if isEditor();
                }
              }
            }
        "#;
        match parse_rules_file(source) {
            Err(RulesFileError { offending_blocks, .. }) => {
                assert_eq!(offending_blocks[0].construct, "SYNTAX_ERROR");
            }
            Ok(_) => panic!("expected SYNTAX_ERROR: 'let' bindings are not supported in v1"),
        }
    }

    #[test]
    fn a_function_body_calling_another_function_is_rejected_via_the_pre_existing_custom_function_construct() {
        // Resolution 3's own "no new detection code" guarantee: a function
        // body is validated via the UNMODIFIED `parse_condition`, so a
        // nested call is caught by `detect_unsupported_construct`'s own
        // pre-existing, unmodified rejection — the SAME tag a bare,
        // non-imported condition already gets today.
        let source = r#"
            service cloud.firestore {
              function isOwner() { return true; }
              function isEditor() { return isOwner(); }
              match /databases/{database}/documents {
                match /trail_guides/{guideId} {
                  allow write: if isEditor();
                }
              }
            }
        "#;
        match parse_rules_file(source) {
            Err(RulesFileError { offending_blocks, .. }) => {
                assert_eq!(offending_blocks[0].construct, "CUSTOM_FUNCTION");
            }
            Ok(_) => panic!("expected CUSTOM_FUNCTION: function bodies may not call other functions in v1"),
        }
    }

    #[test]
    fn a_function_named_after_a_reserved_word_is_a_syntax_error() {
        let source = r#"
            service cloud.firestore {
              function exists() { return true; }
              match /databases/{database}/documents {
                match /trail_guides/{guideId} {
                  allow write: if true;
                }
              }
            }
        "#;
        match parse_rules_file(source) {
            Err(RulesFileError { offending_blocks, .. }) => {
                assert_eq!(offending_blocks[0].construct, "SYNTAX_ERROR");
            }
            Ok(_) => panic!("expected SYNTAX_ERROR: 'exists' collides with a reserved construct"),
        }
    }

    #[test]
    fn a_function_name_starting_with_a_digit_is_a_syntax_error() {
        let source = r#"
            service cloud.firestore {
              function 1abc() { return true; }
              match /databases/{database}/documents {
                match /trail_guides/{guideId} {
                  allow read: if true;
                }
              }
            }
        "#;
        match parse_rules_file(source) {
            Err(RulesFileError { offending_blocks, .. }) => {
                assert_eq!(offending_blocks[0].construct, "SYNTAX_ERROR");
            }
            Ok(_) => panic!("expected SYNTAX_ERROR: '1abc' does not start with a letter or underscore"),
        }
    }

    #[test]
    fn a_function_name_starting_with_an_underscore_is_valid() {
        let source = r#"
            service cloud.firestore {
              function _helper() { return true; }
              match /databases/{database}/documents {
                match /trail_guides/{guideId} {
                  allow read: if _helper();
                }
              }
            }
        "#;
        let blocks = parse_rules_file(source).expect("a leading underscore must be a valid function name");
        assert_eq!(blocks[0].allow_clauses, vec![(vec![Verb::Read], "(true)".to_string())]);
    }

    #[test]
    fn a_function_name_containing_an_underscore_is_valid() {
        let source = r#"
            service cloud.firestore {
              function is_editor() { return true; }
              match /databases/{database}/documents {
                match /trail_guides/{guideId} {
                  allow read: if is_editor();
                }
              }
            }
        "#;
        let blocks = parse_rules_file(source).expect("an underscore mid-name must be valid");
        assert_eq!(blocks[0].allow_clauses, vec![(vec![Verb::Read], "(true)".to_string())]);
    }

    #[test]
    fn expand_function_calls_leaves_exists_and_get_and_duration_value_untouched() {
        let functions = BTreeMap::new();
        let condition = "exists(/databases/$(database)/documents/organizations/$(request.auth.uid)) \
                          && request.time < resource.data.submitted_at + duration.value(24, 'h')";
        let expanded = expand_function_calls(condition, &functions, "").expect("must not error");
        assert_eq!(expanded, condition, "no function calls present — text must be byte-for-byte unchanged");
    }

    #[test]
    fn expand_function_calls_does_not_substitute_inside_a_string_literal() {
        let mut functions = BTreeMap::new();
        functions.insert("isEditor".to_string(), "true".to_string());
        let condition = r#"resource.data.note == "isEditor()" && isEditor()"#;
        let expanded = expand_function_calls(condition, &functions, "").expect("must not error");
        assert_eq!(
            expanded,
            r#"resource.data.note == "isEditor()" && (true)"#,
            "the string-literal occurrence must be left untouched; only the real call site expands"
        );
    }

    #[test]
    fn expand_function_calls_wraps_the_body_so_surrounding_negation_composes_correctly() {
        let mut functions = BTreeMap::new();
        functions.insert("isEditor".to_string(), "request.auth.uid == resource.data.editor_id".to_string());
        let expanded = expand_function_calls("!isEditor()", &functions, "").expect("must not error");
        assert_eq!(expanded, "!(request.auth.uid == resource.data.editor_id)");
    }

    // ── security-rules-cel-chaining-detection (US-01, DESIGN § Root Cause
    // Analysis / § Architecture Design — root-cause fix regression guard) ──

    /// Proves the DESIGN-identified gap directly, at the fastest feedback
    /// loop (`embyr-core`'s own unit tests, no HTTP/DB round-trip). Two
    /// blocks in ONE file: block 1 has a Stage-1-only content error
    /// (`isEditor()`, an undeclared call-shaped identifier —
    /// `UNDEFINED_FUNCTION`, caught by `expand_function_calls` inside
    /// `parse_block_body`); block 2 has a Stage-2-only error (a wildcard at
    /// a collection-name position — `NESTED_PATH`, only detectable once
    /// `decompose` runs). Today, `parse_nested_match_blocks`'s loop still
    /// uses `?` on a per-block CONTENT error, so it fails fast on block 1
    /// and abandons the loop — block 2 is never even scanned, and
    /// `decompose` (the only place `NESTED_PATH` is found) is never
    /// reached. This mirrors `import_rules_file`'s own intended Stage 1 ->
    /// Stage 2 wiring (DESIGN § Architecture Design item 4, not yet
    /// implemented): both stages' offending blocks should end up in ONE
    /// final list. Correct RED today: only `UNDEFINED_FUNCTION` is found,
    /// `NESTED_PATH` never gets the chance to be.
    #[test]
    fn a_stage_1_only_error_and_a_stage_2_only_error_in_the_same_file_are_both_named() {
        let source = r#"
            service cloud.firestore {
              match /databases/{database}/documents {
                match /trail_guides/{guideId} {
                  allow write: if isEditor();
                }
                match /{expeditionId}/journal_entries {
                  allow read: if true;
                }
              }
            }
        "#;

        // Mirrors import_rules_file's own intended Stage 1 -> Stage 2
        // wiring: whatever Stage 1 hands back gets fed to Stage 2, and both
        // stages' offending blocks are merged into one final list.
        let offending = match parse_rules_file(source) {
            Ok(blocks) => decompose(blocks).expect_err("expected at least one Stage 2 rejection").offending_blocks,
            Err(RulesFileError { mut offending_blocks, partial_blocks }) => {
                if let Err(mut stage2_err) = decompose(partial_blocks) {
                    offending_blocks.append(&mut stage2_err.offending_blocks);
                }
                offending_blocks
            }
        };

        let constructs: Vec<&str> = offending.iter().map(|b| b.construct).collect();
        assert!(
            constructs.contains(&"UNDEFINED_FUNCTION"),
            "the Stage-1-only error (undeclared isEditor() call) must survive to the final offending list: {constructs:?}"
        );
        assert!(
            constructs.contains(&"NESTED_PATH"),
            "the Stage-2-only error (wildcard at a collection-name position) must survive to the final offending list: {constructs:?}"
        );
    }
}
