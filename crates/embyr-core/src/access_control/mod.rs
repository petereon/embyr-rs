// SCAFFOLD: true
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
//! panic in its OWN logic once implemented) — AC-17-09's "never crashes on
//! a missing field" claim is a type-level guarantee, not a tested
//! convention (see § Fail-Closed Semantics below). The two functions in
//! this module are the SOLE evaluation routine shared by real enforcement
//! (`grpc/handler.rs::handle_get_document`, US-02/03/04) and simulation
//! (`admin::handlers::access_rules::simulate_access_rule`, US-05) — ADR-029
//! § Decision — Composition, "Simulation shares the exact evaluation
//! routine".
//!
//! RED scaffold (Mandate 7, DISTILL wave `security-rules`): types are fully
//! defined (they are not "business logic" — the grammar's SHAPE is locked
//! by ADR-027, only the parsing/evaluation ALGORITHM is missing); the two
//! functions panic. This is deliberately parallel to
//! `crates/embyr-core/src/client_identity/mod.rs`'s own historical shape
//! during `client-auth`'s DISTILL wave (types real, `verify_client_identity_token`
//! scaffolded) — see that module's doc comment for the precedent this
//! mirrors.

use std::collections::BTreeMap;

use crate::domain::field_value::FieldValue;

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
    let _ = source;
    panic!(
        "embyr_core::access_control::parse_condition — RED scaffold \
         (DISTILL wave, feature security-rules, ADR-027): not yet implemented"
    );
}

/// Evaluate a parsed `Condition` against an `(auth, resource)` pair.
/// Infallible and total by construction once implemented — see § Fail-Closed
/// Semantics below (ADR-027).
///
/// Fail-closed semantics (AC-17-09, ADR-027 § Fail-Closed Semantics on
/// Missing Field): a `resource.data.<field>` reference absent from
/// `resource_fields` is a TOP-LEVEL evaluation short-circuit to `Deny` —
/// the first `FieldMissing` encountered anywhere in the condition tree
/// collapses the entire evaluation to `Deny`, regardless of `&&`/`||`/`!`
/// structure. There is no `Result::Err` branch to forget to handle, because
/// there is no `Result` in this function's return type.
pub fn evaluate(
    condition: &Condition,
    auth: Option<&AuthContext>,
    resource_fields: &BTreeMap<String, FieldValue>,
) -> EvaluationOutcome {
    let _ = (condition, auth, resource_fields);
    panic!(
        "embyr_core::access_control::evaluate — RED scaffold \
         (DISTILL wave, feature security-rules, ADR-027): not yet implemented"
    );
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
    //!
    //! RED by design (Mandate 7): every test below calls into the scaffold
    //! functions above and is expected to panic (not `#[ignore]`d — these
    //! are layer-1 inner-loop tests, run under plain `cargo test`, same
    //! one-scenario-at-a-time discipline exemption `client_identity::tests`
    //! documents for itself).

    use super::*;
    use proptest::prelude::*;

    fn resource_with(field: &str, value: FieldValue) -> BTreeMap<String, FieldValue> {
        let mut map = BTreeMap::new();
        map.insert(field.to_string(), value);
        map
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

        assert_eq!(evaluate(&condition, Some(&auth), &resource), EvaluationOutcome::Allow);
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

        assert_eq!(evaluate(&condition, Some(&auth), &resource), EvaluationOutcome::Deny);
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

        assert_eq!(evaluate(&condition, Some(&auth), &resource), EvaluationOutcome::Deny);
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

        assert_eq!(evaluate(&condition, None, &resource), EvaluationOutcome::Deny);
    }

    #[test]
    fn bare_true_literal_allows_regardless_of_auth_or_resource_ac_17_12() {
        let resource: BTreeMap<String, FieldValue> = BTreeMap::new();
        assert_eq!(
            evaluate(&Condition::Literal(true), None, &resource),
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

            prop_assert_eq!(evaluate(&condition, Some(&auth), &resource), EvaluationOutcome::Deny);
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

            prop_assert_eq!(evaluate(&condition, Some(&auth), &resource), EvaluationOutcome::Deny);
        }
    }
}
