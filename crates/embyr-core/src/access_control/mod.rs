//! Access-rule condition grammar and evaluation — BC-4 Access Control
//! (feature `security-rules`, ADR-027/ADR-029).
//!
//! Pure, zero-IO domain module (ADR-027 § Decision Driver 3 — `embyr-core`
//! must never import `tokio`/`sqlx`/`tonic`/`axum`, enforced by
//! `deny.toml`). Parses and evaluates the v1 constrained boolean grammar
//! locked by `docs/feature/security-rules/feature-delta.md` § Job Discovery
//! Framing Resolution, Resolution 1 (Option C):
//!
//! ```text
//! condition   := or_expr
//! or_expr     := and_expr ( '||' and_expr )*
//! and_expr    := unary ( '&&' unary )*
//! unary       := '!' unary | primary
//! primary     := comparison | 'true' | 'false' | '(' condition ')'
//! comparison  := operand ( '==' | '!=' ) operand
//! operand     := 'request.auth.uid' | 'request.auth' | 'null' | 'resource.data.' IDENT
//! ```
//!
//! Explicitly out of v1 scope (ADR-027): cross-document reads (`get()`/
//! `exists()`), custom functions, wildcard/recursive path matching, custom
//! claims, string/number literals (OQ-SR-04 — booleans only, confirmed by
//! DISTILL, see `feature-delta.md` § Wave: DISTILL / Open Question
//! Resolutions).
//!
//! `evaluate()` is infallible and total by construction (no `Result`, no
//! panic) — AC-17-09's "never crashes on a missing field" claim is a
//! type-level guarantee, not a tested convention (see § Fail-Closed
//! Semantics below). The two functions in this module are the SOLE
//! evaluation routine shared by real enforcement
//! (`grpc/handler.rs::handle_get_document`, US-02/03/04) and simulation
//! (`admin::handlers::access_rules::simulate_access_rule`, US-05) — ADR-029
//! § Decision — Composition, "Simulation shares the exact evaluation
//! routine".
//!
//! Hand-rolled recursive-descent parser (ADR-027 Option C) — no `pest`/
//! `nom`, deliberately, so the grammar cannot silently widen via a
//! grammar-file edit; widening requires a new AST variant, parser branch,
//! and evaluator arm.

use std::collections::BTreeMap;

use crate::domain::field_value::FieldValue;
use crate::domain::query::QueryFilter;

// ---------------------------------------------------------------------------
// Types (ADR-027 § Decision — Types)
// ---------------------------------------------------------------------------

/// The condition AST. `Condition::Literal(bool)` covers the bare `true`/
/// `false` grammar productions (e.g. `allow read: if true` — DISCUSS US-03
/// Domain Example 2, "public read").
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    Literal(bool),
    Compare(Operand, CompareOp, Operand),
    And(Box<Condition>, Box<Condition>),
    Or(Box<Condition>, Box<Condition>),
    Not(Box<Condition>),
}

/// The four operand shapes the locked v1 grammar admits. `ResourceField`
/// carries the referenced field name (e.g. `resource.data.owner_id` ->
/// `ResourceField("owner_id".to_string())`).
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    AuthUid,
    AuthNullSentinel,
    ResourceField(String),
    /// The proposed new document's field (security-rules-write-path,
    /// ADR-030): `request.resource.data.<field>`. Coexists with
    /// `ResourceField` (pre-write state) — non-overlapping token prefixes,
    /// no ordering risk (ADR-030 § Decision — Grammar Extension).
    RequestResourceField(String),
    BoolLiteral(bool),
    NullLiteral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Ne,
}

/// The *only* shape `request.auth` presents to the evaluator (ADR-027 §
/// Types). Constructed 1:1 from `VerifiedEndUserIdentity.end_user_id` at
/// the `handle_get_document` call site (ADR-029 § Identity reuse) — this
/// module never constructs its own identity, it only consumes an
/// already-verified one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthContext {
    pub uid: String,
}

/// `evaluate()`'s return type. No third state — the function is total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationOutcome {
    Allow,
    Deny,
}

/// The two named out-of-grammar constructs the parser specifically
/// recognizes and rejects (AC-17-03's distinguishability requirement) —
/// distinct from a generic `SyntaxError` (AC-17-04).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsupportedConstruct {
    CrossDocumentRead,
    CustomFunction,
    WildcardPath,
}

/// `parse_condition()`'s error type. `SyntaxError` = plain invalid syntax
/// (unbalanced parens, unrecognized operator). `UnsupportedConstruct` = a
/// recognized-but-out-of-v1-scope shape (`get()`/`exists()` call syntax,
/// `{...}`/`**` wildcard-path shapes) — AC-17-03 requires this be
/// distinguishable from `SyntaxError` in the caller-facing message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionParseError {
    SyntaxError { detail: String },
    UnsupportedConstruct { construct: UnsupportedConstruct, detail: String },
}

// ---------------------------------------------------------------------------
// Functions (ADR-027 § Decision — Functions; ADR-029 § Decision — Composition)
// ---------------------------------------------------------------------------

/// Parse the locked v1 grammar (see module doc) into a `Condition` AST.
///
/// Three call sites share this function (ADR-029 § Simulation shares the
/// exact evaluation routine): define/redefine's validation
/// (`admin::handlers::access_rules::define_access_rule`), real
/// enforcement's re-parse of the stored `condition_source`
/// (`grpc::handler::handle_get_document`), and simulation's parse of the
/// caller-supplied candidate condition
/// (`admin::handlers::access_rules::simulate_access_rule`).
pub fn parse_condition(source: &str) -> Result<Condition, ConditionParseError> {
    if let Some(err) = detect_unsupported_construct(source) {
        return Err(err);
    }
    let tokens = tokenize(source)?;
    let mut parser = Parser { tokens: &tokens, pos: 0 };
    let condition = parser.parse_or()?;
    if parser.pos != tokens.len() {
        return Err(syntax_error("unexpected trailing input after condition"));
    }
    Ok(condition)
}

// ---------------------------------------------------------------------------
// parse_condition internals (ADR-027 Option C — hand-rolled, closed grammar)
// ---------------------------------------------------------------------------

fn syntax_error(detail: impl Into<String>) -> ConditionParseError {
    ConditionParseError::SyntaxError { detail: detail.into() }
}

/// Recognizes named out-of-v1-grammar shapes BEFORE tokenizing, since their
/// arguments (e.g. `get()`'s Firestore document path) are not expressible
/// in the locked grammar's token set at all and would otherwise surface as
/// an undifferentiated `SyntaxError` (violating AC-17-03's distinguishability
/// requirement).
fn detect_unsupported_construct(source: &str) -> Option<ConditionParseError> {
    if source.contains("**") || source.contains('{') {
        return Some(ConditionParseError::UnsupportedConstruct {
            construct: UnsupportedConstruct::WildcardPath,
            detail: "wildcard/recursive path matching is not supported in v1".to_string(),
        });
    }

    // Call-shaped syntax: an identifier immediately followed by '('. Never
    // reachable from valid grammar (the grammar's own '(' is only used for
    // grouping, never directly preceded by an identifier character).
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_alphabetic() || chars[i] == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect();
            if i < chars.len() && chars[i] == '(' {
                return Some(if ident == "get" || ident == "exists" {
                    ConditionParseError::UnsupportedConstruct {
                        construct: UnsupportedConstruct::CrossDocumentRead,
                        detail: "cross-document reads (get()/exists()) are not supported in v1"
                            .to_string(),
                    }
                } else {
                    ConditionParseError::UnsupportedConstruct {
                        construct: UnsupportedConstruct::CustomFunction,
                        detail: format!(
                            "custom function calls ('{ident}(...)') are not supported in v1"
                        ),
                    }
                });
            }
            continue;
        }
        i += 1;
    }
    None
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    LParen,
    RParen,
    And,
    Or,
    Not,
    Eq,
    Ne,
    Word(String),
}

fn tokenize(source: &str) -> Result<Vec<Token>, ConditionParseError> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            '&' if chars.get(i + 1) == Some(&'&') => {
                tokens.push(Token::And);
                i += 2;
            }
            '|' if chars.get(i + 1) == Some(&'|') => {
                tokens.push(Token::Or);
                i += 2;
            }
            '=' if chars.get(i + 1) == Some(&'=') => {
                tokens.push(Token::Eq);
                i += 2;
            }
            '!' if chars.get(i + 1) == Some(&'=') => {
                tokens.push(Token::Ne);
                i += 2;
            }
            '!' => {
                tokens.push(Token::Not);
                i += 1;
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.')
                {
                    i += 1;
                }
                tokens.push(Token::Word(chars[start..i].iter().collect()));
            }
            other => {
                return Err(syntax_error(format!(
                    "unexpected character '{other}' at position {i}"
                )));
            }
        }
    }
    Ok(tokens)
}

fn word_to_operand(word: &str) -> Result<Operand, ConditionParseError> {
    match word {
        "request.auth.uid" => Ok(Operand::AuthUid),
        "request.auth" => Ok(Operand::AuthNullSentinel),
        "null" => Ok(Operand::NullLiteral),
        w if w.starts_with("request.resource.data.") => {
            let field = &w["request.resource.data.".len()..];
            if field.is_empty() {
                return Err(syntax_error("'request.resource.data.' requires a field name"));
            }
            Ok(Operand::RequestResourceField(field.to_string()))
        }
        w if w.starts_with("resource.data.") => {
            let field = &w["resource.data.".len()..];
            if field.is_empty() {
                return Err(syntax_error("'resource.data.' requires a field name"));
            }
            Ok(Operand::ResourceField(field.to_string()))
        }
        other => Err(syntax_error(format!("unrecognized operand '{other}'"))),
    }
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.pos);
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn parse_or(&mut self) -> Result<Condition, ConditionParseError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(Token::Or)) {
            self.advance();
            let right = self.parse_and()?;
            left = Condition::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Condition, ConditionParseError> {
        let mut left = self.parse_unary()?;
        while matches!(self.peek(), Some(Token::And)) {
            self.advance();
            let right = self.parse_unary()?;
            left = Condition::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Condition, ConditionParseError> {
        if matches!(self.peek(), Some(Token::Not)) {
            self.advance();
            let inner = self.parse_unary()?;
            return Ok(Condition::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Condition, ConditionParseError> {
        match self.peek() {
            Some(Token::LParen) => {
                self.advance();
                let inner = self.parse_or()?;
                match self.advance() {
                    Some(Token::RParen) => Ok(inner),
                    _ => Err(syntax_error("expected closing ')'")),
                }
            }
            Some(Token::Word(w)) if w == "true" => {
                self.advance();
                Ok(Condition::Literal(true))
            }
            Some(Token::Word(w)) if w == "false" => {
                self.advance();
                Ok(Condition::Literal(false))
            }
            Some(Token::Word(_)) => self.parse_comparison(),
            _ => Err(syntax_error("expected a condition (operand, 'true'/'false', or '(')")),
        }
    }

    fn parse_comparison(&mut self) -> Result<Condition, ConditionParseError> {
        let left = match self.advance() {
            Some(Token::Word(w)) => word_to_operand(w)?,
            _ => return Err(syntax_error("expected an operand")),
        };
        let op = match self.advance() {
            Some(Token::Eq) => CompareOp::Eq,
            Some(Token::Ne) => CompareOp::Ne,
            _ => return Err(syntax_error("expected '==' or '!='")),
        };
        let right = match self.advance() {
            Some(Token::Word(w)) => word_to_operand(w)?,
            _ => return Err(syntax_error("expected an operand")),
        };
        Ok(Condition::Compare(left, op, right))
    }
}

/// Evaluate a parsed `Condition` against an `(auth, resource, request_resource)`
/// triple. Infallible and total by construction once implemented — see §
/// Fail-Closed Semantics below (ADR-027, extended ADR-030).
///
/// Fail-closed semantics (AC-17-09, ADR-027 § Fail-Closed Semantics on
/// Missing Field; AC-17-28, ADR-030 — the identical mechanism reused
/// verbatim): a `resource.data.<field>` reference absent from
/// `resource_fields`, OR a `request.resource.data.<field>` reference absent
/// from `request_resource_fields`, is a TOP-LEVEL evaluation short-circuit
/// to `Deny` — the first `FieldMissing` encountered anywhere in the
/// condition tree collapses the entire evaluation to `Deny`, regardless of
/// `&&`/`||`/`!` structure. There is no `Result::Err` branch to forget to
/// handle, because there is no `Result` in this function's return type.
///
/// `request_resource_fields` is the proposed new document (Create/Update) —
/// empty for Read (`handle_get_document`, ADR-030: no "proposed new
/// document" concept) and Delete (no proposed new state). `resource_fields`
/// is the pre-write document — empty for Create (no document exists yet).
pub fn evaluate(
    condition: &Condition,
    auth: Option<&AuthContext>,
    resource_fields: &BTreeMap<String, FieldValue>,
    request_resource_fields: &BTreeMap<String, FieldValue>,
) -> EvaluationOutcome {
    match eval_bool(condition, auth, resource_fields, request_resource_fields) {
        Ok(true) => EvaluationOutcome::Allow,
        Ok(false) | Err(FieldMissing) => EvaluationOutcome::Deny,
    }
}

// ---------------------------------------------------------------------------
// evaluate() internals (ADR-027 § Fail-Closed Semantics on Missing Field)
// ---------------------------------------------------------------------------

/// Module-local field-lookup error, distinct from `ConditionParseError`
/// (parse-time only). The FIRST `FieldMissing` encountered anywhere in the
/// condition tree short-circuits (via `?`) the whole evaluation to `Deny` —
/// no per-operator null-propagation semantics beyond plain `&&`/`||`
/// short-circuit evaluation (ADR-027).
struct FieldMissing;

fn eval_bool(
    condition: &Condition,
    auth: Option<&AuthContext>,
    resource_fields: &BTreeMap<String, FieldValue>,
    request_resource_fields: &BTreeMap<String, FieldValue>,
) -> Result<bool, FieldMissing> {
    match condition {
        Condition::Literal(value) => Ok(*value),
        Condition::Not(inner) => {
            Ok(!eval_bool(inner, auth, resource_fields, request_resource_fields)?)
        }
        Condition::And(left, right) => Ok(eval_bool(left, auth, resource_fields, request_resource_fields)?
            && eval_bool(right, auth, resource_fields, request_resource_fields)?),
        Condition::Or(left, right) => Ok(eval_bool(left, auth, resource_fields, request_resource_fields)?
            || eval_bool(right, auth, resource_fields, request_resource_fields)?),
        Condition::Compare(left, op, right) => {
            let equal = compare_operands(left, right, auth, resource_fields, request_resource_fields)?;
            Ok(match op {
                CompareOp::Eq => equal,
                CompareOp::Ne => !equal,
            })
        }
    }
}

/// Comparison semantics (ADR-027 § Comparison Semantics, extended
/// ADR-030). Named pairings (`AuthUid`/`ResourceField`,
/// `AuthUid`/`RequestResourceField`, `{AuthUid,AuthNullSentinel}`/
/// `NullLiteral`) use identity-specific rules; every other grammar-legal
/// pairing falls through to ordinary `FieldValue::PartialEq` — including
/// `ResourceField`/`RequestResourceField` cross-map comparisons (the
/// immutable-field pattern, Slice 03's concern), which need no
/// special-casing here (ADR-030 § Decision — Grammar Extension).
fn compare_operands(
    left: &Operand,
    right: &Operand,
    auth: Option<&AuthContext>,
    resource_fields: &BTreeMap<String, FieldValue>,
    request_resource_fields: &BTreeMap<String, FieldValue>,
) -> Result<bool, FieldMissing> {
    match (left, right) {
        (Operand::AuthUid, Operand::ResourceField(name))
        | (Operand::ResourceField(name), Operand::AuthUid) => {
            let auth = auth.ok_or(FieldMissing)?;
            let field = resource_fields.get(name).ok_or(FieldMissing)?;
            Ok(matches!(field, FieldValue::String(v) if v == &auth.uid))
        }
        // security-rules-write-path (ADR-030): the proposed-new-document
        // analog of the pairing above — `request.resource.data.<field> ==
        // request.auth.uid` (US-02's own domain example).
        (Operand::AuthUid, Operand::RequestResourceField(name))
        | (Operand::RequestResourceField(name), Operand::AuthUid) => {
            let auth = auth.ok_or(FieldMissing)?;
            let field = request_resource_fields.get(name).ok_or(FieldMissing)?;
            Ok(matches!(field, FieldValue::String(v) if v == &auth.uid))
        }
        // The `request.auth == null` / `!= null` idiom (AC-17-11/12): auth
        // presence, NOT a literal string comparison against "null".
        (Operand::AuthNullSentinel, Operand::NullLiteral)
        | (Operand::NullLiteral, Operand::AuthNullSentinel)
        | (Operand::AuthUid, Operand::NullLiteral)
        | (Operand::NullLiteral, Operand::AuthUid) => Ok(auth.is_none()),
        _ => {
            let left_value = resolve_field_value(left, auth, resource_fields, request_resource_fields)?;
            let right_value = resolve_field_value(right, auth, resource_fields, request_resource_fields)?;
            Ok(left_value == right_value)
        }
    }
}

fn resolve_field_value(
    operand: &Operand,
    auth: Option<&AuthContext>,
    resource_fields: &BTreeMap<String, FieldValue>,
    request_resource_fields: &BTreeMap<String, FieldValue>,
) -> Result<FieldValue, FieldMissing> {
    match operand {
        Operand::ResourceField(name) => resource_fields.get(name).cloned().ok_or(FieldMissing),
        // security-rules-write-path (ADR-030): identical fail-closed shape
        // to `ResourceField` above, resolved against the OTHER map — the
        // SAME `FieldMissing` short-circuit, reused verbatim (AC-17-28).
        Operand::RequestResourceField(name) => {
            request_resource_fields.get(name).cloned().ok_or(FieldMissing)
        }
        Operand::BoolLiteral(value) => Ok(FieldValue::Boolean(*value)),
        Operand::NullLiteral => Ok(FieldValue::Null),
        Operand::AuthUid | Operand::AuthNullSentinel => {
            auth.map(|a| FieldValue::String(a.uid.clone())).ok_or(FieldMissing)
        }
    }
}

// ---------------------------------------------------------------------------
// check_query_compliance (security-rules-query-path, Slice 01, ADR-031)
// ---------------------------------------------------------------------------

/// Outcome of a query-shape compliance check (security-rules-query-path).
/// Distinct from `EvaluationOutcome` (Allow/Deny only) — a `RunQuery`'s
/// compliance decision is made from the query's own filter SHAPE, before any
/// document is fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryComplianceOutcome {
    /// The query's filter tree, together with the caller's auth context,
    /// satisfies every decidable conjunct of the rule. The query may proceed.
    Admitted,
    /// The rule is a fully decidable shape, but the query does not satisfy
    /// one or more conjuncts. Carries every conjunct that failed.
    Rejected { unsatisfied_conjuncts: Vec<UnsatisfiedConjunct> },
    /// The rule's `Condition` tree contains a shape outside this slice's
    /// locked decidable set (only pure ownership-equality, Slice 01) —
    /// the entire rule is undecidable; every query against the collection
    /// is rejected, regardless of filter shape.
    RejectedUnsupportedRuleShape,
}

/// One AND-conjunct that failed to be satisfied by the query's filter tree
/// and/or auth context. Slice 01 scope: only `OwnershipFilterMissing` — the
/// other reasons (`DenyAll`/`AuthRequired`/`AuthForbidden`) belong to later
/// slices' own decidable shapes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsatisfiedConjunct {
    /// `request.auth.uid == resource.data.<field>` conjunct unmet — no
    /// query filter binds `<field>` with `==` to the caller's own verified
    /// uid.
    OwnershipFilterMissing { field_path: String },
}

/// Decide whether a `RunQuery`'s filter tree, together with the caller's
/// auth context, satisfies a rule's `Condition` — WITHOUT fetching or
/// inspecting any document. Total and infallible by construction: every
/// unrecognized `Condition` shape resolves to `RejectedUnsupportedRuleShape`
/// via `decompose_decidable`'s own explicit catch-all — reject is the ONLY
/// reachable outcome for a shape this function does not name, never an
/// accidental allow-list gap.
///
/// `filter` is the query's ALREADY-TRANSLATED domain `QueryFilter` (or
/// `None` for an unfiltered query) — this function performs zero proto
/// translation and zero IO.
///
/// `auth` is the caller's server-VERIFIED identity (or `None`), the
/// identical `Option<&AuthContext>` `evaluate()` already takes — this
/// function never derives or accepts an identity from anywhere else. This is
/// the load-bearing security property behind AC-17-51: the caller's own uid
/// used for the ownership-equality comparison always comes from this
/// parameter, NEVER from the query filter's own bound value. The filter's
/// bound value is only ever the thing being CHECKED against `auth.uid`,
/// never trusted as proof of `auth.uid`'s own value.
pub fn check_query_compliance(
    condition: &Condition,
    filter: Option<&QueryFilter>,
    auth: Option<&AuthContext>,
) -> QueryComplianceOutcome {
    let atoms = match decompose_decidable(condition) {
        Err(Undecidable) => return QueryComplianceOutcome::RejectedUnsupportedRuleShape,
        Ok(atoms) => atoms,
    };

    let mut unsatisfied = Vec::new();
    for atom in &atoms {
        match atom {
            Atom::OwnershipEquality(field) => {
                let satisfied = auth
                    .map(|a| filter_binds_field_to_uid(filter, field, &a.uid))
                    .unwrap_or(false);
                if !satisfied {
                    unsatisfied.push(UnsatisfiedConjunct::OwnershipFilterMissing {
                        field_path: field.clone(),
                    });
                }
            }
        }
    }

    if unsatisfied.is_empty() {
        QueryComplianceOutcome::Admitted
    } else {
        QueryComplianceOutcome::Rejected { unsatisfied_conjuncts: unsatisfied }
    }
}

/// The one shape `decompose_decidable` recognizes in Slice 01: pure
/// ownership equality. Later slices (03/04/05) add `AlwaysTrue`/
/// `AlwaysFalse`/`AuthPresence` variants and their own `Condition` match
/// arms — Slice 01 deliberately does not build them ahead of a test that
/// requires them.
enum Atom {
    OwnershipEquality(String),
}

struct Undecidable;

/// Recursively decomposes a `Condition` into a flat list of atoms. Returns
/// `Err(Undecidable)` for ANY shape outside Slice 01's own locked set (pure
/// ownership equality, either operand order) — this is the ONLY path to
/// `RejectedUnsupportedRuleShape`, reached by an explicit wildcard match arm,
/// not by the absence of a match (Decision Driver 1, ADR-031).
fn decompose_decidable(condition: &Condition) -> Result<Vec<Atom>, Undecidable> {
    match condition {
        // `request.auth.uid == resource.data.<field>`, either operand
        // order. `CompareOp::Eq` ONLY.
        Condition::Compare(Operand::AuthUid, CompareOp::Eq, Operand::ResourceField(f))
        | Condition::Compare(Operand::ResourceField(f), CompareOp::Eq, Operand::AuthUid) => {
            Ok(vec![Atom::OwnershipEquality(f.clone())])
        }

        // Everything else (Slice 01 scope): `Condition::Literal`, `Or`,
        // `Not`, `And`, `Ne` on this pairing, `RequestResourceField`, the
        // `request.auth != null` idiom, etc. — undecidable for THIS slice.
        _ => Err(Undecidable),
    }
}

/// Does the query's filter tree (recursively, through `QueryFilter::
/// Composite`'s AND structure — no other composite shape exists) contain an
/// equality filter on `field_path` whose bound VALUE equals `caller_uid`?
///
/// SECURITY-CRITICAL (AC-17-51): `caller_uid` is ALWAYS `auth.uid` — the
/// server-verified identity threaded in from `check_query_compliance`'s own
/// `auth` parameter, never from anything client-supplied. This function
/// reads the filter's bound value ONLY to compare it against that
/// already-server-verified uid — it never treats the filter's presence, or
/// its field name matching, as sufficient proof of entitlement on its own. A
/// filter reading `owner_id == "maria-santos"` issued by Dana does NOT match
/// here, because `caller_uid` is `"dana-kim"` — the field name is right, the
/// VALUE is wrong, and value is what this check binds on.
///
/// Field-path matching is exact-string, case-sensitive (AC-17-52) — plain
/// `==` on `field_path: String`, no normalization.
fn filter_binds_field_to_uid(
    filter: Option<&QueryFilter>,
    field_path: &str,
    caller_uid: &str,
) -> bool {
    match filter {
        None => false,
        Some(QueryFilter::Field(ff)) => {
            ff.field_path == field_path
                && ff.op == crate::domain::query::FilterOp::Equal
                && ff.value == FieldValue::String(caller_uid.to_string())
        }
        Some(QueryFilter::Composite(filters)) => filters
            .iter()
            .any(|f| filter_binds_field_to_uid(Some(f), field_path, caller_uid)),
    }
}

#[cfg(test)]
mod tests {
    //! Layer 1 (unit) coverage per Mandate 9 — PBT full where the input
    //! space is quantifiable, pinned examples for the specific AC-17-03/04
    //! (grammar distinguishability), AC-17-09 (fail-closed), and AC-17-10
    //! (existence-non-leakage mechanism, via `evaluate`'s `Deny` uniformity)
    //! contracts. Mirrors `client_identity::mod::tests`'s own shape
    //! (pinned examples + `proptest!` block) — the DIRECT structural
    //! precedent for a layer-1 test module in this codebase.

    use super::*;
    use proptest::prelude::*;

    fn resource_with(field: &str, value: FieldValue) -> BTreeMap<String, FieldValue> {
        let mut map = BTreeMap::new();
        map.insert(field.to_string(), value);
        map
    }

    /// `evaluate()`'s new `request_resource_fields` parameter (ADR-030),
    /// empty for every pre-existing `security-rules`-era test in this file
    /// — none of them reference `request.resource.data.<field>`.
    fn empty_fields() -> BTreeMap<String, FieldValue> {
        BTreeMap::new()
    }

    // ── parse_condition: grammar acceptance + distinguishable rejection reasons ──

    #[test]
    fn ownership_equality_condition_parses_into_a_compare_ast() {
        let result = parse_condition("request.auth.uid == resource.data.owner_id");
        assert_eq!(
            result,
            Ok(Condition::Compare(
                Operand::AuthUid,
                CompareOp::Eq,
                Operand::ResourceField("owner_id".to_string()),
            ))
        );
    }

    #[test]
    fn request_resource_field_condition_parses_into_a_compare_ast() {
        // security-rules-write-path (ADR-030) Slice 02: `request.resource.data.<field>`
        // (proposed new document, US-02 domain example) must parse distinctly
        // from `resource.data.<field>` (pre-existing document).
        let result = parse_condition("request.resource.data.owner_id == request.auth.uid");
        assert_eq!(
            result,
            Ok(Condition::Compare(
                Operand::RequestResourceField("owner_id".to_string()),
                CompareOp::Eq,
                Operand::AuthUid,
            ))
        );
    }

    #[test]
    fn null_auth_idiom_parses_into_a_compare_against_null_literal() {
        // US-03 Domain Example: `request.auth != null` (auth-required rule).
        let result = parse_condition("request.auth != null");
        assert_eq!(
            result,
            Ok(Condition::Compare(
                Operand::AuthNullSentinel,
                CompareOp::Ne,
                Operand::NullLiteral,
            ))
        );
    }

    #[test]
    fn bare_true_literal_parses_as_public_read() {
        // US-03 Domain Example: `allow read: if true`.
        let result = parse_condition("true");
        assert_eq!(result, Ok(Condition::Literal(true)));
    }

    #[test]
    fn cross_document_read_call_syntax_is_rejected_as_unsupported_not_syntax_error() {
        // AC-17-03: `get()`/`exists()` must be a NAMED unsupported
        // construct, distinguishable from a plain syntax error (AC-17-04).
        let result = parse_condition(
            "get(/databases/(default)/documents/users/$(request.auth.uid)) != null",
        );
        assert_eq!(
            result,
            Err(ConditionParseError::UnsupportedConstruct {
                construct: UnsupportedConstruct::CrossDocumentRead,
                detail: "cross-document reads (get()/exists()) are not supported in v1"
                    .to_string(),
            })
        );
    }

    #[test]
    fn wildcard_path_condition_is_rejected_as_unsupported_not_syntax_error() {
        // AC-17-03 (mutation-testing gap, security-rules DELIVER Phase 5):
        // `**`/`{...}` wildcard-path shapes must be a NAMED unsupported
        // construct, distinguishable from a plain syntax error (AC-17-04) —
        // same distinguishability requirement as
        // `cross_document_read_call_syntax_is_rejected_as_unsupported_not_syntax_error`
        // above, but for the OTHER named construct (`detect_unsupported_construct`'s
        // `**`/`{` branch had no direct pinned coverage; only the call-syntax
        // branch did).
        let result = parse_condition("resource.data.path.matches('/users/**')");
        assert_eq!(
            result,
            Err(ConditionParseError::UnsupportedConstruct {
                construct: UnsupportedConstruct::WildcardPath,
                detail: "wildcard/recursive path matching is not supported in v1".to_string(),
            })
        );
    }

    #[test]
    fn unbalanced_parentheses_is_rejected_as_a_plain_syntax_error() {
        // AC-17-04: distinguishable from AC-17-03's UnsupportedConstruct.
        let result = parse_condition("(request.auth.uid == resource.data.owner_id");
        match result {
            Err(ConditionParseError::SyntaxError { .. }) => {}
            other => panic!("expected SyntaxError, got {other:?}"),
        }
    }

    // ── evaluate: the four-way truth table DISCUSS's domain examples exercise ──

    #[test]
    fn owner_uid_matching_resource_field_allows() {
        let condition = Condition::Compare(
            Operand::AuthUid,
            CompareOp::Eq,
            Operand::ResourceField("owner_id".to_string()),
        );
        let auth = AuthContext { uid: "maria-santos".to_string() };
        let resource = resource_with("owner_id", FieldValue::String("maria-santos".to_string()));

        assert_eq!(evaluate(&condition, Some(&auth), &resource, &empty_fields()), EvaluationOutcome::Allow);
    }

    #[test]
    fn non_owner_uid_mismatching_resource_field_denies() {
        let condition = Condition::Compare(
            Operand::AuthUid,
            CompareOp::Eq,
            Operand::ResourceField("owner_id".to_string()),
        );
        let auth = AuthContext { uid: "dana-kim".to_string() };
        let resource = resource_with("owner_id", FieldValue::String("maria-santos".to_string()));

        assert_eq!(evaluate(&condition, Some(&auth), &resource, &empty_fields()), EvaluationOutcome::Deny);
    }

    #[test]
    fn missing_referenced_field_denies_never_panics_ac_17_09() {
        let condition = Condition::Compare(
            Operand::AuthUid,
            CompareOp::Eq,
            Operand::ResourceField("owner_id".to_string()),
        );
        let auth = AuthContext { uid: "maria-santos".to_string() };
        let resource: BTreeMap<String, FieldValue> = BTreeMap::new(); // owner_id absent

        assert_eq!(evaluate(&condition, Some(&auth), &resource, &empty_fields()), EvaluationOutcome::Deny);
    }

    // ── evaluate: request.resource.data.<field> (security-rules-write-path, ADR-030) ──

    #[test]
    fn owner_uid_matching_proposed_new_document_field_allows_ac_17_26() {
        // security-rules-write-path US-02 domain example: `request.resource.data.owner_id
        // == request.auth.uid`, evaluated with an EMPTY `resource_fields`
        // map (Create — no document exists yet) and a POPULATED
        // `request_resource_fields` map (the proposed new document).
        let condition = Condition::Compare(
            Operand::RequestResourceField("owner_id".to_string()),
            CompareOp::Eq,
            Operand::AuthUid,
        );
        let auth = AuthContext { uid: "maria-santos".to_string() };
        let proposed = resource_with("owner_id", FieldValue::String("maria-santos".to_string()));

        assert_eq!(
            evaluate(&condition, Some(&auth), &empty_fields(), &proposed),
            EvaluationOutcome::Allow
        );
    }

    #[test]
    fn missing_referenced_proposed_field_denies_never_panics_ac_17_28() {
        // AC-17-28: `request.resource.data.<field>` absent from
        // `request_resource_fields` fails closed — the SAME `FieldMissing`
        // short-circuit AC-17-09 already proves for `resource.data.<field>`,
        // now exercised against the OTHER map.
        let condition = Condition::Compare(
            Operand::RequestResourceField("owner_id".to_string()),
            CompareOp::Eq,
            Operand::AuthUid,
        );
        let auth = AuthContext { uid: "maria-santos".to_string() };

        assert_eq!(
            evaluate(&condition, Some(&auth), &empty_fields(), &empty_fields()),
            EvaluationOutcome::Deny
        );
    }

    #[test]
    fn resource_field_reference_on_a_nonexistent_create_time_document_denies_ac_17_28() {
        // AC-17-28's OTHER half: a write rule referencing the OLD
        // `resource.data.<field>` operand, evaluated at CREATE time (no
        // document exists yet -> `resource_fields` is empty) — denies via
        // the identical fail-closed mechanism, never a crash, never a new
        // special case for "document doesn't exist yet".
        let condition = Condition::Compare(
            Operand::AuthUid,
            CompareOp::Eq,
            Operand::ResourceField("owner_id".to_string()),
        );
        let auth = AuthContext { uid: "maria-santos".to_string() };
        let proposed = resource_with("owner_id", FieldValue::String("maria-santos".to_string()));

        assert_eq!(
            evaluate(&condition, Some(&auth), &empty_fields(), &proposed),
            EvaluationOutcome::Deny
        );
    }

    #[test]
    fn null_auth_sentinel_denies_when_auth_is_none() {
        // `request.auth != null` — anonymous session (AC-17-11).
        let condition = Condition::Compare(
            Operand::AuthNullSentinel,
            CompareOp::Ne,
            Operand::NullLiteral,
        );
        let resource: BTreeMap<String, FieldValue> = BTreeMap::new();

        assert_eq!(evaluate(&condition, None, &resource, &empty_fields()), EvaluationOutcome::Deny);
    }

    #[test]
    fn bare_true_literal_allows_regardless_of_auth_or_resource_ac_17_12() {
        let resource: BTreeMap<String, FieldValue> = BTreeMap::new();
        assert_eq!(
            evaluate(&Condition::Literal(true), None, &resource, &empty_fields()),
            EvaluationOutcome::Allow
        );
    }

    // ── PBT full (Mandate 9, layer 1) — quantified over the resource-field input space ──

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Property (AC-17-09): for ANY condition that references a
        /// resource field, evaluating against an empty resource-field map
        /// always denies — never panics, never allows. This is the
        /// type-level "never crashes" claim, exercised generatively over
        /// arbitrary field names and uids.
        #[test]
        fn any_ownership_condition_against_an_empty_resource_map_always_denies(
            uid in "[a-z][a-z0-9-]{0,20}",
            field_name in "[a-z][a-z0-9_]{0,20}",
        ) {
            let condition = Condition::Compare(
                Operand::AuthUid,
                CompareOp::Eq,
                Operand::ResourceField(field_name),
            );
            let auth = AuthContext { uid };
            let resource: BTreeMap<String, FieldValue> = BTreeMap::new();

            prop_assert_eq!(evaluate(&condition, Some(&auth), &resource, &empty_fields()), EvaluationOutcome::Deny);
        }

        /// Property (AC-17-10's underlying mechanism): for ANY
        /// ownership-style condition and ANY non-matching uid/owner pair,
        /// the outcome is Deny regardless of whether other, unrelated
        /// fields are present on the resource — no field beyond the
        /// referenced one influences the outcome.
        #[test]
        fn mismatched_owner_denies_regardless_of_unrelated_resource_fields(
            auth_uid in "[a-z][a-z0-9-]{1,20}",
            owner_uid in "[a-z][a-z0-9-]{1,20}",
            unrelated_value in "[a-z]{0,10}",
        ) {
            prop_assume!(auth_uid != owner_uid);
            let condition = Condition::Compare(
                Operand::AuthUid,
                CompareOp::Eq,
                Operand::ResourceField("owner_id".to_string()),
            );
            let auth = AuthContext { uid: auth_uid };
            let mut resource = resource_with("owner_id", FieldValue::String(owner_uid));
            resource.insert("title".to_string(), FieldValue::String(unrelated_value));

            prop_assert_eq!(evaluate(&condition, Some(&auth), &resource, &empty_fields()), EvaluationOutcome::Deny);
        }
    }

    // ── check_query_compliance (security-rules-query-path, Slice 01, ADR-031) ──
    //
    // Pure sibling to `evaluate()` — decides whether a RunQuery's filter tree
    // satisfies an ownership-equality rule WITHOUT fetching any document.
    // Slice 01 scope: only the ownership-equality shape
    // (`request.auth.uid == resource.data.<field>`, either operand order).
    // Everything else is undecidable for this slice (Slices 03/04/05 extend
    // `decompose_decidable` with the remaining shapes).

    use crate::domain::query::{FieldFilter, FilterOp, QueryFilter};

    fn owner_id_field_equals(value: &str) -> QueryFilter {
        QueryFilter::Field(FieldFilter {
            field_path: "owner_id".to_string(),
            op: FilterOp::Equal,
            value: FieldValue::String(value.to_string()),
        })
    }

    #[test]
    fn ownership_equality_admits_when_filter_binds_field_to_callers_own_uid_ac_17_49() {
        let condition = Condition::Compare(
            Operand::AuthUid,
            CompareOp::Eq,
            Operand::ResourceField("owner_id".to_string()),
        );
        let auth = AuthContext { uid: "maria-santos".to_string() };
        let filter = owner_id_field_equals("maria-santos");

        assert_eq!(
            check_query_compliance(&condition, Some(&filter), Some(&auth)),
            QueryComplianceOutcome::Admitted
        );
    }

    #[test]
    fn ownership_equality_admits_regardless_of_operand_order() {
        // `resource.data.owner_id == request.auth.uid` — the reverse
        // pairing, grammar-legal and equally decidable (this slice's own
        // scope: "either operand order").
        let condition = Condition::Compare(
            Operand::ResourceField("owner_id".to_string()),
            CompareOp::Eq,
            Operand::AuthUid,
        );
        let auth = AuthContext { uid: "maria-santos".to_string() };
        let filter = owner_id_field_equals("maria-santos");

        assert_eq!(
            check_query_compliance(&condition, Some(&filter), Some(&auth)),
            QueryComplianceOutcome::Admitted
        );
    }

    #[test]
    fn additional_filters_beyond_the_required_one_do_not_affect_compliance_ac_17_50() {
        // A composite (AND) filter carrying the required ownership filter
        // PLUS an unrelated extra filter — the query has MORE filters than
        // strictly required and must still be admitted.
        let condition = Condition::Compare(
            Operand::AuthUid,
            CompareOp::Eq,
            Operand::ResourceField("owner_id".to_string()),
        );
        let auth = AuthContext { uid: "maria-santos".to_string() };
        let filter = QueryFilter::Composite(vec![
            owner_id_field_equals("maria-santos"),
            QueryFilter::Field(FieldFilter {
                field_path: "status".to_string(),
                op: FilterOp::Equal,
                value: FieldValue::String("active".to_string()),
            }),
        ]);

        assert_eq!(
            check_query_compliance(&condition, Some(&filter), Some(&auth)),
            QueryComplianceOutcome::Admitted
        );
    }

    #[test]
    fn filter_bound_to_someone_elses_uid_is_rejected_ac_17_51() {
        // THE single most security-critical property in this feature: Dana
        // (uid "dana-kim") submits a filter `owner_id == "maria-santos"` —
        // syntactically on the right field, with the right operator, but
        // bound to a value OTHER than her own verified uid. A compliance
        // check that only verifies "a filter exists on the right field"
        // would wrongly admit this and let Dana enumerate Maria's data.
        let condition = Condition::Compare(
            Operand::AuthUid,
            CompareOp::Eq,
            Operand::ResourceField("owner_id".to_string()),
        );
        let dana = AuthContext { uid: "dana-kim".to_string() };
        let filter_naming_someone_elses_uid = owner_id_field_equals("maria-santos");

        let outcome =
            check_query_compliance(&condition, Some(&filter_naming_someone_elses_uid), Some(&dana));

        assert_eq!(
            outcome,
            QueryComplianceOutcome::Rejected {
                unsatisfied_conjuncts: vec![UnsatisfiedConjunct::OwnershipFilterMissing {
                    field_path: "owner_id".to_string(),
                }],
            },
            "a filter on the right field bound to ANOTHER caller's uid must be REJECTED, \
             not admitted — field-name presence alone is never proof of entitlement"
        );
    }

    #[test]
    fn field_reference_matching_is_exact_string_case_sensitive_ac_17_52() {
        // A rule referencing `owner_id` must NOT be satisfied by a filter on
        // `Owner_Id` — no fuzzy/partial/case-insensitive matching.
        let condition = Condition::Compare(
            Operand::AuthUid,
            CompareOp::Eq,
            Operand::ResourceField("owner_id".to_string()),
        );
        let auth = AuthContext { uid: "maria-santos".to_string() };
        let filter = QueryFilter::Field(FieldFilter {
            field_path: "Owner_Id".to_string(),
            op: FilterOp::Equal,
            value: FieldValue::String("maria-santos".to_string()),
        });

        assert_eq!(
            check_query_compliance(&condition, Some(&filter), Some(&auth)),
            QueryComplianceOutcome::Rejected {
                unsatisfied_conjuncts: vec![UnsatisfiedConjunct::OwnershipFilterMissing {
                    field_path: "owner_id".to_string(),
                }],
            }
        );
    }

    #[test]
    fn no_signed_in_caller_can_never_satisfy_an_ownership_equality_rule() {
        // `auth` is `None` (no signed-in caller) — there is no uid to bind
        // to, so no filter, however shaped, can ever satisfy the rule.
        let condition = Condition::Compare(
            Operand::AuthUid,
            CompareOp::Eq,
            Operand::ResourceField("owner_id".to_string()),
        );
        let filter = owner_id_field_equals("maria-santos");

        assert_eq!(
            check_query_compliance(&condition, Some(&filter), None),
            QueryComplianceOutcome::Rejected {
                unsatisfied_conjuncts: vec![UnsatisfiedConjunct::OwnershipFilterMissing {
                    field_path: "owner_id".to_string(),
                }],
            }
        );
    }

    #[test]
    fn an_unfiltered_query_is_rejected_when_the_rule_requires_ownership_equality() {
        let condition = Condition::Compare(
            Operand::AuthUid,
            CompareOp::Eq,
            Operand::ResourceField("owner_id".to_string()),
        );
        let auth = AuthContext { uid: "maria-santos".to_string() };

        assert_eq!(
            check_query_compliance(&condition, None, Some(&auth)),
            QueryComplianceOutcome::Rejected {
                unsatisfied_conjuncts: vec![UnsatisfiedConjunct::OwnershipFilterMissing {
                    field_path: "owner_id".to_string(),
                }],
            }
        );
    }

    #[test]
    fn a_rule_shape_outside_the_locked_decidable_set_is_rejected_as_unsupported() {
        // Fail-closed default arm (Decision Driver 1): a rule shape this
        // slice does not name (e.g. bare `true`) is REJECTED, never
        // silently admitted — the ONLY reachable path for an unrecognized
        // `Condition` shape.
        let condition = Condition::Literal(true);
        let auth = AuthContext { uid: "maria-santos".to_string() };
        let filter = owner_id_field_equals("maria-santos");

        assert_eq!(
            check_query_compliance(&condition, Some(&filter), Some(&auth)),
            QueryComplianceOutcome::RejectedUnsupportedRuleShape
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Property (AC-17-51, strengthened): for ANY two DIFFERENT uids and
        /// ANY field name, a filter binding that field to the OTHER uid
        /// never satisfies an ownership-equality rule for the caller's own
        /// uid — generatively, not just the one pinned Dana/Maria example.
        #[test]
        fn filter_bound_to_a_different_uid_never_admits(
            caller_uid in "[a-z][a-z0-9-]{1,20}",
            other_uid in "[a-z][a-z0-9-]{1,20}",
            field_name in "[a-z][a-z0-9_]{0,20}",
        ) {
            prop_assume!(caller_uid != other_uid);
            let condition = Condition::Compare(
                Operand::AuthUid,
                CompareOp::Eq,
                Operand::ResourceField(field_name.clone()),
            );
            let auth = AuthContext { uid: caller_uid };
            let filter = QueryFilter::Field(FieldFilter {
                field_path: field_name,
                op: FilterOp::Equal,
                value: FieldValue::String(other_uid),
            });

            prop_assert_ne!(
                check_query_compliance(&condition, Some(&filter), Some(&auth)),
                QueryComplianceOutcome::Admitted
            );
        }
    }
}
