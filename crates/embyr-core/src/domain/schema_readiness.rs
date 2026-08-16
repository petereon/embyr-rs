//! `SchemaReadiness` — pure domain type describing a customer database's
//! migration-tracking state relative to the compiled-in expected schema
//! version.
//!
//! Provenance: feature `customer-db-onboarding`, ADR-023 (schema-readiness
//! verification). Zero IO imports — enforced by workspace `deny.toml`
//! (embyr-core IO-prohibition).
//!
//! Constructed by `embyr_pg_storage::backend_adapter::PostgresBackendAdapter::
//! verify_schema_readiness()`, which fetches `found_version` from
//! `_sqlx_migrations` and delegates the pure classification below.

/// The customer database's schema-readiness state, as determined by reading
/// sqlx's own `_sqlx_migrations` bookkeeping table (ADR-023 § Mechanism).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaReadiness {
    /// All expected migrations are applied (`found_version >= expected_version`,
    /// all rows `success = true`). Not strict equality — see ADR-023
    /// Consequences / OQ-1 (forward-compatible during rolling deploys).
    Ready { schema_version: i64 },
    /// `_sqlx_migrations` is absent or unreadable. Names the expected
    /// application tables (`documents`, `transactions`) as the missing
    /// element — the caller cannot cleanly distinguish "never prepped" from
    /// "prepped but tracking table unreadable", and both share the same
    /// remediation (re-run the prep step).
    NotPrepped { missing_tables: Vec<String> },
    /// `0 < found_version < expected_version`.
    Stale {
        expected_version: i64,
        found_version: i64,
    },
}

/// Classify a customer database's schema-readiness state from
/// already-fetched bookkeeping numbers.
///
/// Pure — no I/O. Called by `embyr_pg_storage::backend_adapter::
/// PostgresBackendAdapter::verify_schema_readiness()` after that adapter
/// fetches `found_version` from `_sqlx_migrations` (ADR-023 § Mechanism).
///
/// `found_version <= 0` means the tracking table is absent/unreadable — the
/// caller passes `0` in that case, `missing_tables` names the application
/// tables that could not be confirmed present (diagnostic-only role, not the
/// versioning source of truth).
///
/// `found_version >= expected_version` is `Ready` — deliberately
/// forward-compatible (not strict equality). See ADR-023 OQ-1 / CDO-AD-05:
/// a newer prep-tool build than this server's compiled-in `Migrator` must
/// not block provisioning during rolling deploys.
///
pub fn classify(
    found_version: i64,
    expected_version: i64,
    missing_tables: Vec<String>,
) -> SchemaReadiness {
    if found_version <= 0 {
        return SchemaReadiness::NotPrepped { missing_tables };
    }
    if found_version >= expected_version {
        return SchemaReadiness::Ready {
            schema_version: found_version,
        };
    }
    SchemaReadiness::Stale {
        expected_version,
        found_version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Invariant: classify() reports Ready iff found_version >= expected_version.
        #[test]
        fn ready_iff_found_gte_expected(found in 0i64..1000, expected in 1i64..1000) {
            let result = classify(found, expected, vec![]);
            prop_assert_eq!(
                matches!(result, SchemaReadiness::Ready { .. }),
                found >= expected
            );
        }

        /// Boundary-inclusive: found_version == expected_version must be Ready,
        /// never Stale (ADR-023 OQ-1 / CDO-AD-05 — not strict equality).
        #[test]
        fn found_equal_expected_is_always_ready_never_stale(version in 1i64..1000) {
            let result = classify(version, version, vec![]);
            prop_assert_eq!(result, SchemaReadiness::Ready { schema_version: version });
        }

        /// found_version == 0 is always NotPrepped, regardless of expected_version.
        #[test]
        fn found_zero_is_always_not_prepped(
            expected in 1i64..1000,
            missing in proptest::collection::vec("[a-z]+", 0..3),
        ) {
            let result = classify(0, expected, missing.clone());
            prop_assert_eq!(result, SchemaReadiness::NotPrepped { missing_tables: missing });
        }
    }
}
