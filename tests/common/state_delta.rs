// SCAFFOLD: true
//! State-delta port — embyr-rs (Rust bootstrap)
//!
//! Universe-bound assertion contract per nw-test-design-mandates Mandate 8.
//! Every state-mutating test at layers 1-3 must call `assert_state_delta`.
//!
//! Universe entries are port-exposed observable names only (exit codes, public
//! read-model fields, captured outputs, event counts) — never internal struct fields.
//!
//! Bootstrapped: 2026-05-24 (DISTILL wave, feature embyr-rs)
//! [port-mode] bootstrapped

use std::collections::HashMap;
use std::fmt::Debug;

/// Predicate types for expected state changes.
#[derive(Debug)]
pub enum Predicate<T: Debug + PartialEq> {
    /// The value is set to exactly this value.
    SetTo(T),
    /// The value is unchanged from before.
    Unchanged,
    /// The collection has this new element appended.
    AppendedWith(T),
    /// The collection contains this element (among possibly others).
    Containing(T),
}

/// Assert that the state delta between `before` and `after` matches `expected`
/// for every key declared in `universe`.
///
/// - Keys in `universe` that appear in `expected` are checked with their predicate.
/// - Keys in `universe` that do NOT appear in `expected` must remain unchanged
///   (fail-closed: unexpected mutations are a violation).
///
/// # Panics
/// Panics with a descriptive message if any assertion fails.
pub fn assert_state_delta<T>(
    before: &HashMap<&str, T>,
    after: &HashMap<&str, T>,
    universe: &[&str],
    expected: &HashMap<&str, Predicate<T>>,
) where
    T: Debug + PartialEq + Clone,
{
    for &key in universe {
        match expected.get(key) {
            Some(Predicate::SetTo(expected_val)) => {
                let actual = after.get(key).unwrap_or_else(|| {
                    panic!("state_delta: key '{}' missing in after-state", key)
                });
                assert_eq!(
                    actual, expected_val,
                    "state_delta: key '{}' expected SetTo({:?}), got {:?}",
                    key, expected_val, actual
                );
            }
            Some(Predicate::Unchanged) | None => {
                // Must equal before value.
                let before_val = before.get(key);
                let after_val = after.get(key);
                assert_eq!(
                    before_val, after_val,
                    "state_delta: key '{}' expected unchanged ({:?}) but changed to {:?}",
                    key, before_val, after_val
                );
            }
            Some(Predicate::AppendedWith(_)) | Some(Predicate::Containing(_)) => {
                // Placeholder — specialized implementations needed per concrete type.
                // For now, assert key is present in after-state.
                assert!(
                    after.contains_key(key),
                    "state_delta: key '{}' missing in after-state for AppendedWith/Containing check",
                    key
                );
            }
        }
    }
}

/// Convenience constructor: value was set to this value.
pub fn set_to<T: Debug + PartialEq>(value: T) -> Predicate<T> {
    Predicate::SetTo(value)
}

/// Convenience constructor: value is unchanged.
pub fn unchanged<T: Debug + PartialEq>() -> Predicate<T> {
    Predicate::Unchanged
}

/// Convenience constructor: collection had this element appended.
pub fn appended_with<T: Debug + PartialEq>(value: T) -> Predicate<T> {
    Predicate::AppendedWith(value)
}

/// Convenience constructor: collection contains this element.
pub fn containing<T: Debug + PartialEq>(value: T) -> Predicate<T> {
    Predicate::Containing(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_to_passes_when_value_matches() {
        let mut before = HashMap::new();
        before.insert("status", "inactive");
        let mut after = HashMap::new();
        after.insert("status", "active");

        let universe = &["status"];
        let mut expected = HashMap::new();
        expected.insert("status", set_to("active"));

        // Should not panic.
        assert_state_delta(&before, &after, universe, &expected);
    }

    #[test]
    #[should_panic(expected = "state_delta: key 'status' expected unchanged")]
    fn unchanged_fails_when_value_changes() {
        let mut before = HashMap::new();
        before.insert("status", "active");
        let mut after = HashMap::new();
        after.insert("status", "suspended");

        let universe = &["status"];
        let expected: HashMap<&str, Predicate<&str>> = HashMap::new(); // no expected entry → must be unchanged

        assert_state_delta(&before, &after, universe, &expected);
    }
}
