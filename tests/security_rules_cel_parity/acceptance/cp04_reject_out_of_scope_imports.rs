//! CP04 (Slice 04, US-04, Release 1) — An Import Containing Out-of-v1-Scope
//! Constructs Is Rejected Whole, Naming Every Offending Block.
//!
//! Acceptance criteria verified here (feature-delta.md US-04):
//!   AC-17-188: a nested/multi-segment collection path is rejected in full,
//!              naming that specific block (NESTED_PATH).
//!   AC-17-189: a recursive wildcard (`{path=**}`) is rejected in full,
//!              naming that specific block (RECURSIVE_WILDCARD).
//!   AC-17-190: a custom `function` call or `get()`/`exists()` call is
//!              rejected in full, naming that specific block and construct
//!              (CUSTOM_FUNCTION / CROSS_DOCUMENT_READ).
//!   AC-17-191: a rejection response names EVERY offending block, not only
//!              the first found.
//!   AC-17-192: a rejected import leaves every existing rule — for the
//!              file's own named collections and any other collection —
//!              completely unchanged.
//!   AC-17-193: a condition referencing a path-variable name not captured by
//!              its own block's own path pattern is rejected at import time
//!              as a named, distinguishable error (SYNTAX_ERROR, fired at
//!              import-time parsing per ADR-062 § Decision — Condition
//!              rewrite), never silently treated as a runtime missing-field
//!              denial.
//!   (DESIGN-introduced, OQ-CP-05, ADR-062 § Decision — Verb-bucketing):
//!              a single block assigning two DIFFERENT conditions to the
//!              same read/write bucket via granular verbs is rejected,
//!              named CONFLICTING_VERB_CONDITIONS.
//!
//! Driving port: Admin HTTP :9090 (`SecurityRulesAdminContext`, reused
//! unchanged from cp01's own fixture).
//!
//! Error ratio: every scenario here IS a rejection/boundary case by this
//! story's own nature (US-04 is entirely about rejection) — well over the
//! 40% mandate.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesAdminContext;

/// Journey:
///   Given: project `trailmark-prod` exists, no rules imported yet
///   When:  Alex imports a file containing a block whose collection-name
///          position is itself a wildcard (`/{expeditionId}/journal_entries`)
///   Then:  the whole import is rejected, naming that block as NESTED_PATH,
///          and no rule is stored for it
///
/// SUPERSEDED SCENARIO NOTE (fixed as part of `security-rules-cel-path-
/// matching` Slice 05, AC-17-224 non-regression audit): this test originally
/// asserted that a nested/multi-segment subcollection path
/// (`expeditions/{expeditionId}/journal_entries/{entryId}`) was ALWAYS
/// rejected as NESTED_PATH. That assumption was true under 4a's own locked
/// v1 scope (single top-level collection only) but is INTENTIONALLY
/// superseded by this feature's own Slice 01 (ADR-063): a fixed-depth
/// multi-segment pattern is now a supported, first-class import shape
/// (`DecomposedTarget::MultiSegmentPattern`), not rejected — confirmed
/// directly via `rules_file::decompose_block`'s widened
/// `validate_segment_shape` (`crates/embyr-core/src/access_control/
/// rules_file.rs`), which only rejects NESTED_PATH when a COLLECTION-NAME
/// position (an even segment index) is itself non-literal, never merely for
/// having more than 2 segments. This test is repurposed (not silently
/// deleted, matching this codebase's own "superseded, not silently deleted"
/// precedent) to assert the ONE remaining, genuinely-still-invalid NESTED_PATH
/// trigger: a wildcard sitting at a collection-name position.
///
/// AC-17-188
///
/// @error @driving_port @real-io @US-04 @AC-17-188
#[tokio::test]
async fn a_wildcard_at_a_collection_name_position_is_rejected_naming_that_block() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let rules_file = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /{expeditionId}/journal_entries {
              allow read: if true;
            }
          }
        }
    "#;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": rules_file }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(resp.status().as_u16(), 400, "AC-17-188: an out-of-scope import must be rejected");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["reason"], "IMPORT_REJECTED");
    let offending = body["offending_blocks"].as_array().expect("offending_blocks must be an array");
    assert_eq!(offending.len(), 1);
    assert_eq!(offending[0]["construct"], "NESTED_PATH", "AC-17-188: must be named NESTED_PATH specifically");
    assert_eq!(offending[0]["path_pattern"], "/{expeditionId}/journal_entries");

    assert_eq!(
        ctx.access_rule_condition_source("trailmark-prod", "expeditions").await,
        None,
        "AC-17-192: no rule may be stored for a rejected block"
    );
}

/// Journey:
///   Given: project `trailmark-prod` exists
///   When:  Alex imports a file containing `match /{path=**} { allow read:
///          if false; }` — a terminal, even-prefix (empty-prefix) recursive
///          wildcard
///   Then:  the import now succeeds and is stored as a recursive-wildcard
///          pattern with an empty fixed prefix
///
/// SUPERSEDED SCENARIO NOTE (found during `security-rules-cel-recursive-
/// wildcards` Slice 05, AC-17-255 full-baseline regression run): this test
/// originally asserted that ANY recursive-wildcard path (`{path=**}`) was
/// unconditionally rejected as RECURSIVE_WILDCARD. That assumption was true
/// under 4a's/4b's own locked v1 scope but is INTENTIONALLY superseded by
/// this feature's own Slice 01 (ADR-064, Resolution 3 Option B): a
/// terminal, even-prefix recursive wildcard — including the empty-prefix,
/// project-wide catch-all shape exercised here — is now a supported,
/// first-class import shape (confirmed directly via rw01's own
/// `a_project_wide_recursive_wildcard_catch_all_is_imported_and_active`,
/// AC-17-232/AC-17-237, identical rules-file shape). The bare
/// `RECURSIVE_WILDCARD` rejection construct this test asserted no longer
/// exists in the implementation; it was replaced by two narrower,
/// distinguishable constructs (`RECURSIVE_WILDCARD_ODD_PREFIX`,
/// `RECURSIVE_WILDCARD_NOT_TERMINAL` — both already proven by rw01's own
/// AC-17-233/AC-17-234). This test is repurposed (not silently deleted,
/// matching this codebase's own "superseded, not silently deleted"
/// precedent) to lock 4a's/cp04's own regression baseline to the NEW
/// behavior — proving cp04's own historical rejection claim is stale —
/// without duplicating rw01's own more granular storage assertions.
///
/// AC-17-189 (superseded; see note above)
///
/// @driving_port @real-io @US-04 @AC-17-189 @security-regression
#[tokio::test]
async fn a_recursive_wildcard_path_is_now_accepted_superseding_the_original_rejection() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let rules_file = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /{path=**} {
              allow read: if false;
            }
          }
        }
    "#;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": rules_file }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-17-189 (superseded): a terminal, even-prefix recursive wildcard must now import cleanly"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["imported"].as_array().expect("imported must be an array").len(), 1);

    let row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM access_rule_patterns WHERE project_id = $1 AND collection_path_pattern = $2 AND is_recursive",
    )
    .bind("trailmark-prod")
    .bind("")
    .fetch_one(&ctx.pool)
    .await
    .expect("count access_rule_patterns rows");
    assert_eq!(
        row_count, 1,
        "AC-17-189 (superseded): the recursive-wildcard pattern must be stored, not rejected"
    );
}

/// Journey (feature-delta.md UAT "Multiple offending blocks are all named in
/// a single response"):
///   Given: project `trailmark-prod` exists
///   When:  Alex imports a file containing a wildcard-at-collection-position
///          block (still NESTED_PATH — see supersession note above), a
///          custom-function-call block (`isEditor()`), and a `get()`-call
///          block
///   Then:  the rejection response names all three, each with its own
///          specific reason — not just the first one found
///
/// SUPERSEDED SCENARIO NOTE: the original offending block here was a
/// nested/multi-segment path (`expeditions/{expeditionId}/journal_entries/
/// {entryId}`), which this feature (Epic 4b) now accepts (see the
/// supersession note on the test above). Repurposed to a
/// wildcard-at-collection-position block — the surviving NESTED_PATH
/// trigger — so this test still proves 3 independently-offending blocks are
/// ALL named in one response, not just 2.
///
/// SUPERSEDED SCENARIO NOTE (found during `security-rules-cel-chaining-
/// detection` DESIGN Root Cause Analysis, Finding 2): the `isEditor()`
/// block's own expected construct was originally `CUSTOM_FUNCTION`. That
/// assertion is stale — `security-rules-cel-functions` (ADR-067) introduced
/// `expand_function_calls`, which always runs first during a rules-file
/// IMPORT and unconditionally intercepts any undeclared call-shaped
/// identifier as `UNDEFINED_FUNCTION` before `decompose_block`'s own
/// `CUSTOM_FUNCTION` path can ever see it. `CUSTOM_FUNCTION` remains live
/// and correct for the OTHER direct-condition routes (`define_access_rule`
/// and siblings), just structurally unreachable via THIS import path. Same
/// "superseded, not silently deleted" precedent this file already applies
/// twice above.
///
/// AC-17-190, AC-17-191
///
/// @error @driving_port @real-io @US-04 @AC-17-190 @AC-17-191
#[tokio::test]
async fn multiple_offending_blocks_are_all_named_in_a_single_rejection_response() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    // SUPERSEDED SCENARIO NOTE (found during `security-rules-cel-cross-
    // document-reads` Slice 01, AC-CDR-01 full-baseline regression run):
    // the 3rd `match` block below originally used
    // `get(/databases/x/documents/users/y).data.admin` — a well-formed
    // 4a-era CROSS_DOCUMENT_READ rejection fixture. That construct is
    // INTENTIONALLY superseded (ADR-066, Resolution 1): a narrowly-scoped
    // `get()`/`exists()` idiom is now a supported, first-class construct
    // (confirmed directly via CDR01's own real enforcement test,
    // AC-CDR-01/02, identical shape). The bare `CROSS_DOCUMENT_READ`
    // rejection this test asserted no longer exists in the implementation
    // (mirrors `security-rules-cel-recursive-wildcards`' own identical
    // "superseded, not silently deleted" precedent applied to THIS SAME
    // file already, `a_recursive_wildcard_path_is_now_accepted_
    // superseding_the_original_rejection`, above). This test is
    // repurposed to prove its OWN original claim ("every offending block
    // in a multi-block rejection is named, not just the first") using a
    // construct that IS still genuinely out of scope: chaining (a `get()`
    // whose own path is built from ANOTHER `get()`'s own result,
    // Resolution 2) — proven unsupported directly by CDR01's own unit
    // test `a_substitution_beyond_auth_uid_or_path_variable_is_a_named_
    // rejection`.
    let rules_file = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /{expeditionId}/journal_entries {
              allow read: if true;
            }
            match /trail_guides/{guideId} {
              allow write: if isEditor();
            }
            match /journal_entries {
              allow read: if exists(/databases/$(database)/documents/orgs/$(get(/databases/$(database)/documents/users/$(request.auth.uid)).data.orgId));
            }
          }
        }
    "#;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": rules_file }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(resp.status().as_u16(), 400, "AC-17-190/191: any offending block rejects the whole import");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    let offending = body["offending_blocks"].as_array().expect("offending_blocks must be an array");
    assert_eq!(offending.len(), 3, "AC-17-191: every offending block must be named, not just the first");

    let constructs: Vec<&str> = offending.iter().map(|b| b["construct"].as_str().unwrap()).collect();
    assert!(constructs.contains(&"NESTED_PATH"), "AC-17-188 (within multi-block): {constructs:?}");
    assert!(
        constructs.contains(&"UNDEFINED_FUNCTION"),
        "AC-17-190 (superseded by `security-rules-cel-functions`, ADR-067: an undeclared \
         call-shaped identifier inside a rules-file IMPORT is UNDEFINED_FUNCTION, not \
         CUSTOM_FUNCTION — see cf01's own a_call_to_an_undefined_function_is_rejected_at_import_time): \
         undefined function call: {constructs:?}"
    );
    assert!(
        constructs.contains(&"UNSUPPORTED_EXPRESSION_GRAMMAR"),
        "AC-17-190 (superseded; see note above): a chained get() call: {constructs:?}"
    );

    // AC-17-192: none of the 3 offending blocks' collections received a rule.
    for collection in ["expeditions", "trail_guides", "journal_entries"] {
        assert_eq!(
            ctx.access_rule_condition_source("trailmark-prod", collection).await,
            None,
            "AC-17-192: no rule may be stored for collection '{collection}' from a rejected import"
        );
    }
}

/// Journey (feature-delta.md UAT "A rejected import leaves all existing
/// rules completely unchanged"):
///   Given: `trail_guides` already has an active rule (unrelated to this
///          import) and `profiles` has no rule yet
///   When:  Alex imports a file naming `profiles` (in-scope) and a
///          wildcard-at-collection-position block (still out-of-scope, see
///          supersession note above)
///   Then:  the whole import is rejected; `trail_guides`'s pre-existing rule
///          is completely unaffected, AND `profiles`'s own in-scope block
///          from THIS SAME file received no rule either (zero partial
///          application within one rejected import)
///
/// SUPERSEDED SCENARIO NOTE: the original offending block here was a
/// nested/multi-segment path, which this feature (Epic 4b) now accepts (see
/// the supersession note above). Repurposed to a
/// wildcard-at-collection-position block — the surviving NESTED_PATH
/// trigger — so this test still proves AC-17-192's atomicity guarantee.
///
/// AC-17-192
///
/// @error @driving_port @real-io @US-04 @AC-17-192
#[tokio::test]
async fn a_rejected_import_leaves_every_existing_and_would_be_rule_completely_unchanged() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;
    ctx.seed_access_rule("trailmark-prod", "trail_guides", "true").await;

    let rules_file = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /profiles/{userId} {
              allow read, write: if request.auth.uid == userId;
            }
            match /{expeditionId}/journal_entries {
              allow read: if true;
            }
          }
        }
    "#;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": rules_file }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(resp.status().as_u16(), 400, "AC-17-192: any offending block rejects the whole import");

    assert_eq!(
        ctx.access_rule_condition_source("trailmark-prod", "trail_guides").await.as_deref(),
        Some("true"),
        "AC-17-192: a pre-existing rule for a collection NOT named in the rejected import must be unchanged"
    );
    assert_eq!(
        ctx.access_rule_condition_source("trailmark-prod", "profiles").await,
        None,
        "AC-17-192: the in-scope block's own collection in a rejected import must receive NO rule (zero partial application)"
    );
}

/// Journey (feature-delta.md UAT "A path-variable name referenced in a
/// condition but not captured by that block's own path is rejected"):
///   Given: project `trailmark-prod` exists
///   When:  Alex imports a file containing `match /profiles/{userId} {
///          allow read: if request.auth.uid == postId; }` (a typo — the
///          captured name is `userId`, the condition references `postId`)
///   Then:  the request is rejected naming the undefined variable
///          reference, distinguishable from a missing-document-field
///          runtime denial (a 400 IMPORT_REJECTED, never a stored rule that
///          could later 403/deny at request time)
///
/// AC-17-193
///
/// @error @driving_port @real-io @US-04 @AC-17-193
#[tokio::test]
async fn a_condition_referencing_an_undefined_path_variable_is_rejected_at_import_time() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let rules_file = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /profiles/{userId} {
              allow read: if request.auth.uid == postId;
            }
          }
        }
    "#;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": rules_file }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "AC-17-193: an undefined path-variable reference must be rejected at import time, not stored"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["reason"], "IMPORT_REJECTED");
    let offending = body["offending_blocks"].as_array().expect("offending_blocks must be an array");
    assert_eq!(offending.len(), 1);
    assert_eq!(
        offending[0]["construct"], "SYNTAX_ERROR",
        "AC-17-193: an undefined bare identifier is a named, distinguishable import-time construct rejection"
    );

    assert_eq!(
        ctx.access_rule_condition_source("trailmark-prod", "profiles").await,
        None,
        "AC-17-193: no rule may be stored — this must never fall through to a runtime missing-field denial"
    );
}

/// Journey (ADR-062 § Decision — Verb-bucketing, OQ-CP-05 — orchestrator-
/// resolved DESIGN scoping, not a DISCUSS-authored AC, but explicit test
/// coverage requested for this slice):
///   Given: project `trailmark-prod` exists
///   When:  Alex imports a file whose single `match` block assigns two
///          DIFFERENT conditions to the same read bucket via granular verbs
///          (`get` vs `list`)
///   Then:  the whole import is rejected, naming that block
///          CONFLICTING_VERB_CONDITIONS — embyr's storage has no way to
///          express two conditions for one bucket, and the two conditions
///          are never silently OR'd together
///
/// OQ-CP-05
///
/// @error @driving_port @real-io @US-04 @OQ-CP-05
#[tokio::test]
async fn differing_conditions_for_the_same_verb_bucket_are_rejected_as_conflicting() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let rules_file = r#"
        service cloud.firestore {
          match /databases/{database}/documents {
            match /journal_entries {
              allow get: if request.auth.uid == resource.data.owner_id;
              allow list: if true;
            }
          }
        }
    "#;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/import"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({ "rules_file": rules_file }))
        .send()
        .await
        .expect("import request failed");

    assert_eq!(resp.status().as_u16(), 400, "OQ-CP-05: conflicting per-verb conditions must reject the whole import");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    let offending = body["offending_blocks"].as_array().expect("offending_blocks must be an array");
    assert_eq!(offending.len(), 1);
    assert_eq!(
        offending[0]["construct"], "CONFLICTING_VERB_CONDITIONS",
        "OQ-CP-05: differing get/list conditions on the same bucket must be named CONFLICTING_VERB_CONDITIONS"
    );

    assert_eq!(
        ctx.access_rule_condition_source("trailmark-prod", "journal_entries").await,
        None,
        "OQ-CP-05: no rule may be stored — the two conditions must never be silently OR'd or one silently picked"
    );
}
