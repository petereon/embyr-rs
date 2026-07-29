//! Mock data layer — `data::mock::*()` functions.
//!
//! V1: all data is in-process mock returning hardcoded collections.
//! Individual slices populate this data as they are implemented.
//! V2 plan: replace with `#[server]` function calls. Component code is unchanged (ADR-007).

/// Hardcoded display-only stats for a database at a given index (V1 mock).
///
/// Used by dashboard cards and the db-detail overview. Not part of the domain model.
pub struct DisplayStats {
    pub region: &'static str,
    pub p50: u32,
    pub p95: u32,
    pub p99: u32,
    pub reads: u64,
    pub writes: u64,
    pub deletes: u64,
    /// 12-point sparkline heights (0-40) for the db card latency preview.
    pub spark: [u8; 12],
    /// 12-bar ops series for the overview bar chart.
    pub ops: [u16; 12],
    /// Human-readable connection detail string.
    pub connection_detail: &'static str,
    pub backend_mode_label: &'static str,
    pub created: &'static str,
}

pub fn display_stats(idx: usize) -> DisplayStats {
    match idx {
        0 => DisplayStats {
            region: "us-east-1", p50: 4, p95: 11, p99: 28,
            reads: 1_240_000, writes: 342_000, deletes: 12_400,
            spark: [20,22,18,25,15,28,22,30,24,18,26,20],
            ops:   [3200,4100,5400,6800,7200,6400,5100,5800,7000,8100,7600,6300],
            connection_detail: "db.prod.internal:5432/embyr_prod",
            backend_mode_label: "direct_pg",
            created: "2025-08-14",
        },
        1 => DisplayStats {
            region: "us-east-1", p50: 7, p95: 19, p99: 44,
            reads: 88_000, writes: 41_000, deletes: 3_100,
            spark: [10,14,12,18,16,14,20,16,18,14,16,14],
            ops:   [420,510,680,720,640,590,710,680,760,820,710,590],
            connection_detail: "agent.staging.embyr.dev:8443",
            backend_mode_label: "agent_mode",
            created: "2025-09-02",
        },
        2 => DisplayStats {
            region: "europe-west1", p50: 12, p95: 33, p99: 71,
            reads: 612_000, writes: 9_800, deletes: 220,
            spark: [25,30,28,35,32,30,38,34,28,32,36,30],
            ops:   [2600,3100,4200,5100,4800,4400,5200,4900,5400,5900,5100,4600],
            connection_detail: "agent.analytics.embyr.dev:8443",
            backend_mode_label: "agent_mode",
            created: "2025-10-21",
        },
        _ => DisplayStats {
            region: "us-east-1", p50: 3, p95: 6, p99: 14,
            reads: 4_200, writes: 1_800, deletes: 90,
            spark: [5,6,4,8,5,7,4,6,5,7,6,5],
            ops:   [12,18,24,30,22,18,26,22,28,32,24,20],
            connection_detail: "localhost:5432/playground",
            backend_mode_label: "direct_pg",
            created: "2026-01-09",
        },
    }
}

pub fn fmt_num(n: u64) -> String {
    if n >= 1_000_000_000 { format!("{:.1}B", n as f64 / 1_000_000_000.0) }
    else if n >= 1_000_000 { format!("{:.1}M", n as f64 / 1_000_000.0) }
    else if n >= 1_000     { format!("{:.1}K", n as f64 / 1_000.0) }
    else                   { n.to_string() }
}

pub fn sparkline_points(spark: &[u8]) -> String {
    let w = 300.0f32;
    let h = 40.0f32;
    let max = spark.iter().copied().max().unwrap_or(1) as f32;
    spark.iter().enumerate().map(|(i, &v)| {
        let x = i as f32 * w / (spark.len() - 1) as f32;
        let y = h - (v as f32 / max * h * 0.85);
        format!("{:.1},{:.1}", x, y)
    }).collect::<Vec<_>>().join(" ")
}

pub fn bar_chart_points(ops: &[u16]) -> Vec<(f32, f32, f32)> {
    // Returns (x, width, height_frac) tuples for each bar (0..1 height fraction)
    let max = ops.iter().copied().max().unwrap_or(1) as f32;
    let n = ops.len() as f32;
    ops.iter().enumerate().map(|(i, &v)| {
        let x = i as f32 / n;
        let w = 0.7 / n;
        let h = v as f32 / max;
        (x, w, h)
    }).collect()
}

use crate::model::{
    AdminKey, Database, DbBackendMode, DbId, DbStatus, KeyId, Member, OidcId, OidcProvider, Role,
    SdkKey, ServiceAccount, ServiceAccountId,
};
use uuid::Uuid;

pub mod mock {
    use super::*;

    /// Return a mock list of databases for the account.
    ///
    /// Two hardcoded entries covering both backend modes (direct_pg and agent_mode).
    /// Deleted databases are intentionally excluded — they would be filtered by SetDatabases.
    pub fn databases() -> Vec<Database> {
        vec![
            Database {
                id: DbId(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()),
                name: "production".to_string(),
                status: DbStatus::Active,
                backend_mode: DbBackendMode::DirectPg,
                logging_enabled: true,
                log_retention: Some(crate::model::LogRetention::SevenDays),
                created_at: None,
            },
            Database {
                id: DbId(Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap()),
                name: "staging".to_string(),
                status: DbStatus::Active,
                backend_mode: DbBackendMode::AgentMode,
                logging_enabled: false,
                log_retention: None,
                created_at: None,
            },
        ]
    }

    /// Return a mock list of members for the account.
    pub fn members() -> Vec<Member> {
        Vec::new()
    }

    /// Return mock SDK keys for the given database.
    pub fn sdk_keys(_db_id: &DbId) -> Vec<SdkKey> {
        Vec::new()
    }

    /// Return mock account-level admin API keys.
    ///
    /// Two hardcoded entries with the "embyr_adm_" prefix covering Owner and Admin roles.
    pub fn admin_keys() -> Vec<AdminKey> {
        vec![
            AdminKey {
                id: KeyId(Uuid::parse_str("00000000-0000-0000-0000-000000000010").unwrap()),
                name: "ci-pipeline-key".to_string(),
                service_account_id: Some(ServiceAccountId(
                    Uuid::parse_str("00000000-0000-0000-0000-000000000020").unwrap(),
                )),
                member_id: None,
                role: Role::Admin,
                prefix: "embyr_adm_".to_string(),
                created_at: None,
            },
            AdminKey {
                id: KeyId(Uuid::parse_str("00000000-0000-0000-0000-000000000011").unwrap()),
                name: "read-only-key".to_string(),
                service_account_id: None,
                member_id: None,
                role: Role::Viewer,
                prefix: "embyr_adm_".to_string(),
                created_at: None,
            },
        ]
    }

    /// Return mock service accounts.
    ///
    /// One hardcoded entry for a CI service account.
    pub fn service_accounts() -> Vec<ServiceAccount> {
        vec![ServiceAccount {
            id: ServiceAccountId(
                Uuid::parse_str("00000000-0000-0000-0000-000000000020").unwrap(),
            ),
            name: "ci-service-account".to_string(),
            description: Some("Used by the CI pipeline".to_string()),
            role: Role::Admin,
            created_at: None,
        }]
    }

    /// Return mock OIDC providers.
    ///
    /// One hardcoded entry (Google) with enabled=false, representing a provider
    /// that has been configured but not yet activated.
    pub fn oidc_providers() -> Vec<OidcProvider> {
        vec![OidcProvider {
            id: OidcId(Uuid::parse_str("00000000-0000-0000-0000-000000000030").unwrap()),
            issuer: "https://accounts.google.com".to_string(),
            client_id: "mock-google-client-id".to_string(),
            enabled: false,
        }]
    }
}
