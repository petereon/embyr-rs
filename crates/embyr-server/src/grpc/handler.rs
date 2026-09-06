// tonic::Status (~176 bytes: code + message + metadata map + source) is the
// idiomatic error type for gRPC handler functions across this file — boxing
// it at every one of these call sites would add noise without a real
// correctness or performance benefit at this request volume.
#![allow(clippy::result_large_err)]

use std::{collections::HashMap, sync::Arc};

use prost_types::Timestamp;
use tonic::{Request, Response, Status};

use embyr_core::{
    auth::{argon2, blake3, ecies},
    domain::{
        document::{CollectionPath, DocumentPath, FirestoreDocument},
        field_value::FieldValue,
        project::CredentialCacheKey,
        query::{
            validate_field_path, AggregateValue, AggregationKind, AggregationQuery, Cursor,
            FieldFilter, FilterOp, OrderBy, OrderDirection, QueryFilter, StructuredQuery,
        },
        transaction::TransactionOptions,
    },
    error::CoreError,
    storage::backend_adapter::{FieldTransform, Write as DomainWrite, WritePrecondition},
};
use embyr_proto::firestore::{
    firestore_server::Firestore, precondition::ConditionType, AggregationResult,
    BatchGetDocumentsRequest, BatchGetDocumentsResponse, BatchWriteRequest, BatchWriteResponse,
    BeginTransactionRequest, BeginTransactionResponse, CommitRequest, CommitResponse,
    CreateDocumentRequest, DeleteDocumentRequest, Document, GetDocumentRequest,
    ListCollectionIdsRequest, ListCollectionIdsResponse,
    ListDocumentsRequest, ListDocumentsResponse, ListenRequest,
    ListenResponse, RollbackRequest, RunAggregationQueryRequest, RunAggregationQueryResponse,
    RunQueryRequest, RunQueryResponse, UpdateDocumentRequest, WriteRequest, WriteResponse,
    get_document_request::ConsistencySelector as GetDocConsistencySelector,
    run_aggregation_query_request::QueryType as AggregationQueryType,
    run_query_request::QueryType,
    structured_aggregation_query::{aggregation::Operator as AggregationOperator, QueryType as StructuredAggQueryType},
    structured_query::{
        composite_filter::Operator as CompositeOp,
        field_filter::Operator as FieldOp,
        unary_filter::Operator as UnaryOp,
        Direction,
        filter::FilterType,
    },
};
use tokio_stream::StreamExt as _;

use crate::{
    admin::handlers::composite_indexes::{IndexFieldOrder, IndexFieldSpec},
    adapters::{
        agent_backend::AgentBackendAdapter,
        aws_secret_fetcher::AwsSecretFetcher,
        credential_cache::{CachedEntry, CredentialCache, SharedBackendAdapter},
        gcp_secret_fetcher::GcpSecretFetcher,
        index_manager::IndexManager,
        metrics_adapter::MetricsAdapter,
        postgres_backend::PostgresBackendAdapter,
        postgres_notify_listener::{notify_channel, PostgresNotifyListener},
        system_db::SystemDb,
    },
    encoding::firestore_proto::{
        document_to_proto, field_value_to_proto, fields_to_proto, proto_fields_to_domain,
    },
    middleware::{obs_helpers, rate_limit::RateLimiter},
    realtime::listen_registry::ListenRegistry,
};

#[derive(Clone)]
pub struct FirestoreService {
    pub system_db: Arc<SystemDb>,
    pub credential_cache: Arc<CredentialCache>,
    pub index_manager: Arc<IndexManager>,
    /// Records per-project daily operation counts in the system DB.
    pub metrics_adapter: Arc<MetricsAdapter>,
    /// Interval between NO_CHANGE keep-alive messages on idle Listen streams.
    /// Default: 30s for production. Tests use a shorter interval (e.g. 500ms).
    pub keepalive_interval: std::time::Duration,
    /// Shared fan-out registry for real-time Listen subscribers.
    pub listen_registry: Arc<ListenRegistry>,
    /// Active `PostgresNotifyListener` handles, keyed by project_id.
    /// Started on first Listen stream for each project.
    pub active_listeners: Arc<tokio::sync::Mutex<HashMap<String, PostgresNotifyListener>>>,
    /// AWS Secrets Manager fetcher — Some for servers configured with aws_secret support.
    pub aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>,
    /// GCP Secret Manager fetcher — Some for servers configured with gcp_secret support.
    pub gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
    /// Per-project token bucket rate limiter. Applied before authentication to skip
    /// Argon2id on requests that would be rate-limited anyway.
    pub rate_limiter: Arc<RateLimiter>,
}

impl FirestoreService {
    /// Extract `project_id` from a Firestore resource name.
    ///
    /// Format: `projects/{pid}/databases/(default)/documents/...`
    fn extract_project_id(name: &str) -> Result<&str, Status> {
        let mut parts = name.splitn(5, '/');
        match (parts.next(), parts.next()) {
            (Some("projects"), Some(pid)) if !pid.is_empty() => Ok(pid),
            _ => Err(Status::invalid_argument(format!(
                "invalid resource name: {name}"
            ))),
        }
    }

    /// Parse `DocumentPath` from a full Firestore resource name.
    fn parse_document_path(
        name: &str,
    ) -> Result<embyr_core::domain::document::DocumentPath, Status> {
        let project_id_str = Self::extract_project_id(name)?;
        let after_docs = name
            .split("/documents/")
            .nth(1)
            .ok_or_else(|| Status::invalid_argument("resource name missing /documents/"))?;

        let mut segments: Vec<&str> = after_docs.split('/').collect();
        if segments.is_empty() {
            return Err(Status::invalid_argument("missing document path segments"));
        }
        let document_id = segments
            .pop()
            .expect("non-empty vec always has last element")
            .to_string();
        let collection_path = segments.join("/");

        let project_id = embyr_core::domain::project::ProjectId::new(project_id_str)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        Ok(embyr_core::domain::document::DocumentPath {
            project_id,
            collection_path,
            document_id,
        })
    }

    /// Parse `parent` into `(project_id, prefix)` for `ListDocuments` (and,
    /// eventually, `ListCollectionIds`). Distinct from `parse_document_path`
    /// above: `parent` here IS the complete document-or-root path — there is
    /// no trailing `document_id` to pop.
    ///
    /// `parent` formats:
    ///   `projects/{pid}/databases/(default)/documents` -> prefix = ""
    ///   `projects/{pid}/databases/(default)/documents/a/b` -> prefix = "a/b"
    fn parse_parent_prefix(parent: &str) -> Result<(String, String), Status> {
        let project_id_str = Self::extract_project_id(parent)?.to_string();
        let marker = "/documents";
        let marker_pos = parent
            .find(marker)
            .ok_or_else(|| Status::invalid_argument("parent missing /documents"))?;
        let prefix = parent[marker_pos + marker.len()..]
            .trim_start_matches('/')
            .to_string();
        Ok((project_id_str, prefix))
    }

    /// Extract the bearer API key from the `authorization` metadata header.
    fn extract_api_key<T>(request: &Request<T>) -> Result<String, Status> {
        let auth_val = request
            .metadata()
            .get("authorization")
            .ok_or_else(|| Status::unauthenticated("missing authorization header"))?
            .to_str()
            .map_err(|_| Status::unauthenticated("invalid authorization header encoding"))?;

        auth_val
            .strip_prefix("bearer ")
            .or_else(|| auth_val.strip_prefix("Bearer "))
            .map(|k| k.to_string())
            .ok_or_else(|| Status::unauthenticated("authorization must be bearer token"))
    }

    /// client-auth (ADR-026 step 4, additive): extract the OPTIONAL
    /// client-identity token from the `x-embyr-client-identity` gRPC
    /// metadata key (REST's `X-Embyr-Client-Identity` header arrives here
    /// too, once tonic-web lowercases it into gRPC metadata).
    ///
    /// Unlike `extract_api_key`, absence is NOT an error — this is a new,
    /// separate, optional credential slot; the existing `authorization`
    /// metadata key (and everything `extract_api_key`/`authenticate` do
    /// with it) is completely untouched by this function's existence
    /// (ADR-026 § Decision — Wire Composition).
    fn extract_client_identity_token<T>(request: &Request<T>) -> Option<String> {
        let val = request.metadata().get("x-embyr-client-identity")?;
        let val = val.to_str().ok()?;
        val.strip_prefix("Bearer ")
            .or_else(|| val.strip_prefix("bearer "))
            .map(|s| s.to_string())
    }

    /// Authenticate the request and resolve the backend adapter.
    ///
    /// Checks the credential cache first; on miss performs Argon2id verification
    /// and ECIES DSN decryption before building a new adapter.
    ///
    /// Returns `(adapter, project_status, dsn)`.
    async fn authenticate(
        &self,
        project_id_str: &str,
        api_key: &str,
    ) -> Result<(SharedBackendAdapter, String, String), Status> {
        let api_key_blake3 = blake3::derive_cache_key(api_key.as_bytes());
        let project_id = embyr_core::domain::project::ProjectId::new(project_id_str)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let cache_key = CredentialCacheKey {
            project_id,
            api_key_blake3,
        };

        // Cache hit — avoid Argon2id cost.
        if let Some(cached) = self.credential_cache.get(&cache_key).await {
            if cached.1 == "suspended" {
                return Err(Status::permission_denied("project is suspended"));
            }
            if cached.1 == "deleted" {
                return Err(Status::not_found("project not found"));
            }
            return Ok(cached);
        }

        // Cache miss — load from system DB.
        let row = self
            .system_db
            .get_project_for_auth(project_id_str)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::unauthenticated("project not found"))?;

        // Fast-path status checks before the expensive Argon2id verification.
        // Suspended/deleted projects are rejected immediately without CPU cost.
        if row.status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }
        if row.status == "deleted" {
            return Err(Status::not_found("project not found"));
        }

        // Argon2id verification is CPU-intensive; run on blocking thread pool.
        let api_key_bytes = api_key.as_bytes().to_vec();
        let hash_current = row.api_key_hash_current.clone();
        let hash_previous = row.api_key_hash_previous.clone();
        let verified = tokio::task::spawn_blocking(move || {
            if argon2::verify_api_key(&api_key_bytes, &hash_current).unwrap_or(false) {
                return true;
            }
            if let Some(prev) = &hash_previous {
                if argon2::verify_api_key(&api_key_bytes, prev).unwrap_or(false) {
                    return true;
                }
            }
            false
        })
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

        if !verified {
            return Err(Status::unauthenticated("invalid api key"));
        }

        // Build adapter based on backend_mode.
        let (shared, dsn) = if row.backend_mode == "aws_secret" {
            // AWS Secrets Manager mode: fetch DSN via the fetcher, create PostgresBackendAdapter.
            let arn = row
                .backend_secret_arn
                .ok_or_else(|| Status::internal("aws_secret project missing backend_secret_arn"))?;
            let fetcher = self
                .aws_secret_fetcher
                .as_ref()
                .ok_or_else(|| Status::internal("aws_secret_fetcher not configured on this server"))?;
            let dsn = fetcher
                .get_dsn(&arn)
                .await
                .map_err(|e| Status::internal(format!("aws secret fetch failed: {e}")))?;
            let adapter = PostgresBackendAdapter::new(&dsn)
                .await
                .map_err(|e| Status::internal(e.to_string()))?;
            let shared: SharedBackendAdapter = Arc::new(adapter);
            (shared, dsn)
        } else if row.backend_mode == "gcp_secret" {
            // GCP Secret Manager mode: fetch DSN via the fetcher, create PostgresBackendAdapter.
            let resource_name = row
                .backend_secret_gcp
                .ok_or_else(|| Status::internal("gcp_secret project missing backend_secret_gcp"))?;
            let fetcher = self
                .gcp_secret_fetcher
                .as_ref()
                .ok_or_else(|| Status::internal("gcp_secret_fetcher not configured on this server"))?;
            let dsn = fetcher
                .get_dsn(&resource_name)
                .await
                .map_err(|e| Status::internal(format!("gcp secret fetch failed: {e}")))?;
            let adapter = PostgresBackendAdapter::new(&dsn)
                .await
                .map_err(|e| Status::internal(e.to_string()))?;
            let shared: SharedBackendAdapter = Arc::new(adapter);
            (shared, dsn)
        } else if row.backend_mode == "agent" {
            // Agent-mode: decrypt TLS bundle and create AgentBackendAdapter.
            let endpoint = row
                .backend_agent_endpoint
                .ok_or_else(|| Status::internal("agent project missing backend_agent_endpoint"))?;
            let encrypted_bundle = row
                .agent_tls_bundle_enc
                .ok_or_else(|| Status::internal("agent project missing agent_tls_bundle_enc"))?;
            let bundle_bytes = ecies::decrypt(api_key.as_bytes(), &encrypted_bundle)
                .map_err(|e| Status::internal(e.to_string()))?;
            let bundle_str = String::from_utf8(bundle_bytes)
                .map_err(|_| Status::internal("TLS bundle is not valid UTF-8"))?;
            let bundle: serde_json::Value = serde_json::from_str(&bundle_str)
                .map_err(|e| Status::internal(format!("TLS bundle JSON parse error: {e}")))?;
            let ca_pem = bundle["ca_pem"]
                .as_str()
                .ok_or_else(|| Status::internal("TLS bundle missing ca_pem"))?
                .as_bytes()
                .to_vec();
            let client_cert_pem = bundle["client_cert_pem"]
                .as_str()
                .ok_or_else(|| Status::internal("TLS bundle missing client_cert_pem"))?
                .as_bytes()
                .to_vec();
            let client_key_pem = bundle["client_key_pem"]
                .as_str()
                .ok_or_else(|| Status::internal("TLS bundle missing client_key_pem"))?
                .as_bytes()
                .to_vec();

            let adapter =
                AgentBackendAdapter::new(&endpoint, &ca_pem, &client_cert_pem, &client_key_pem)
                    .await
                    .map_err(|e| Status::internal(e.to_string()))?;
            let shared: SharedBackendAdapter = Arc::new(adapter);
            (shared, String::new())
        } else {
            // Direct-PG mode: decrypt DSN via ECIES and create PostgresBackendAdapter.
            let encrypted_dsn = row
                .ecies_encrypted_dsn
                .ok_or_else(|| Status::internal("project has no stored DSN"))?;
            let dsn_bytes = ecies::decrypt(api_key.as_bytes(), &encrypted_dsn)
                .map_err(|e| Status::internal(e.to_string()))?;
            let dsn = String::from_utf8(dsn_bytes)
                .map_err(|_| Status::internal("DSN is not valid UTF-8"))?;

            let adapter = PostgresBackendAdapter::new(&dsn)
                .await
                .map_err(|e| Status::internal(e.to_string()))?;
            let shared: SharedBackendAdapter = Arc::new(adapter);
            (shared, dsn)
        };

        self.credential_cache
            .insert(
                cache_key,
                CachedEntry {
                    adapter: Arc::clone(&shared),
                    project_status: row.status.clone(),
                    dsn: dsn.clone(),
                },
            )
            .await;

        Ok((shared, row.status, dsn))
    }

    /// client-auth (ADR-026 step 4, additive): optional, best-effort
    /// client-identity resolution — appended AFTER the existing three-role
    /// `api_key` check (`authenticate()`, above) completes exactly as it
    /// does today, per ADR-026's structural non-regression argument.
    ///
    ///   absent  -> `None`, and — load-bearing for AC-16-08(c) — ZERO calls
    ///              into `embyr_core::client_identity`. The routing check
    ///              below (`extract_client_identity_token` returning `None`)
    ///              short-circuits via `?` before
    ///              `embyr_core::client_identity::verify_client_identity_token`
    ///              (fully implemented — GREEN) is ever reached.
    ///   present -> verified or not, but NEVER rejects the caller's request
    ///              either way (ADR-026: "failure -> attach nothing; DOES
    ///              NOT reject the request"). Callers that want to surface a
    ///              rejection reason to the end user use the dedicated
    ///              sign-in action (US-02) instead.
    ///
    /// DISTILL scope note: wired into `handle_get_document` only (the exact
    /// call every DISCUSS/DESIGN domain example uses — Maria's `getDoc`).
    /// Extending the identical additive call to the other 8 RPC methods for
    /// AC-16-09's full "available to embyr's own request handling" surface
    /// is explicit DELIVER-wave follow-through, not a DISTILL gap — see
    /// feature-delta.md § DISTILL scaffolds note.
    ///
    /// client-auth-hosted-identity (ADR-036 Decision 4, additive): after the
    /// ORIGINAL `client_identity_credentials` lookup above (unchanged in
    /// shape — a `client-auth`-only project's successful-verification code
    /// path is untouched, byte-for-byte) is absent or fails to verify, also
    /// attempts `hosted_identity_signing_keys`.
    ///
    /// oauth-providers (ADR-037 Decision 8, a THIRD widening): after the
    /// hosted-identity attempt is also absent or fails to verify, also
    /// attempts `oauth_signing_keys`. All three attempts call the
    /// IDENTICAL, unchanged `verify_client_identity_token()` — a
    /// correctly-formed token from any one source only ever verifies against
    /// its own signer's public key; a wrong-source attempt fails
    /// deterministically. A project with none of the three tables populated
    /// (today's default) never queries any of them for a request with no
    /// `x-embyr-client-identity` header — the early `?` above short-circuits
    /// before any lookup, preserving AC-16-08(c)'s structural
    /// unreachability guarantee a third time. Each attempt below is a
    /// fall-through `if let Some(...) { ... }` block (not a hard `?`
    /// short-circuit) precisely so a later attempt can still run after an
    /// earlier one is absent or fails.
    async fn attach_client_identity_if_present<T>(
        &self,
        request: &Request<T>,
        project_id_str: &str,
    ) -> Option<embyr_core::client_identity::VerifiedEndUserIdentity> {
        let token = Self::extract_client_identity_token(request)?;

        if let Some(row) = self
            .system_db
            .get_client_identity_credential(project_id_str)
            .await
            .ok()
            .flatten()
        {
            let public_key_current: Option<[u8; 32]> = row.public_key_current.try_into().ok();
            let public_key_previous: Option<[u8; 32]> = row
                .public_key_previous
                .and_then(|v| v.try_into().ok());
            if let Some(public_key_current) = public_key_current {
                let credential = embyr_core::client_identity::ClientIdentityCredential {
                    public_key_current,
                    public_key_previous,
                };
                if let Ok(identity) = embyr_core::client_identity::verify_client_identity_token(
                    Some(&token),
                    project_id_str,
                    &credential,
                ) {
                    return Some(identity);
                }
            }
        }

        // Fall through: no client_identity_credentials row, or it failed to
        // verify — try the embyr-owned hosted-identity signing key.
        if let Some(hosted_row) = self
            .system_db
            .get_hosted_identity_signing_key(project_id_str)
            .await
            .ok()
            .flatten()
        {
            if let Ok(public_key_current) = hosted_row.public_key.try_into() {
                let credential = embyr_core::client_identity::ClientIdentityCredential {
                    public_key_current,
                    public_key_previous: None,
                };
                if let Ok(identity) = embyr_core::client_identity::verify_client_identity_token(
                    Some(&token),
                    project_id_str,
                    &credential,
                ) {
                    return Some(identity);
                }
            }
        }

        // Fall through: neither prior source matched or verified — try the
        // embyr-owned oauth (Google sign-in) signing key.
        if let Some(oauth_row) = self
            .system_db
            .get_oauth_signing_key(project_id_str)
            .await
            .ok()
            .flatten()
        {
            if let Ok(public_key_current) = oauth_row.public_key.try_into() {
                let credential = embyr_core::client_identity::ClientIdentityCredential {
                    public_key_current,
                    public_key_previous: None,
                };
                if let Ok(identity) = embyr_core::client_identity::verify_client_identity_token(
                    Some(&token),
                    project_id_str,
                    &credential,
                ) {
                    return Some(identity);
                }
            }
        }

        // Fall through: none of the three prior sources matched or
        // verified — try the embyr-owned anonymous-sessions signing key
        // (anonymous-sessions, ADR-043 Decision 6, a FOURTH widening).
        if let Some(anonymous_row) = self
            .system_db
            .get_anonymous_signing_key(project_id_str)
            .await
            .ok()
            .flatten()
        {
            if let Ok(public_key_current) = anonymous_row.public_key.try_into() {
                let credential = embyr_core::client_identity::ClientIdentityCredential {
                    public_key_current,
                    public_key_previous: None,
                };
                if let Ok(identity) = embyr_core::client_identity::verify_client_identity_token(
                    Some(&token),
                    project_id_str,
                    &credential,
                ) {
                    return Some(identity);
                }
            }
        }

        None
    }

    /// Convert a proto `Precondition` to a domain `WritePrecondition`.
    fn convert_precondition(
        p: Option<embyr_proto::firestore::Precondition>,
    ) -> Option<WritePrecondition> {
        let p = p?;
        match p.condition_type? {
            ConditionType::Exists(true) => Some(WritePrecondition::MustExist),
            ConditionType::Exists(false) => Some(WritePrecondition::MustNotExist),
            ConditionType::UpdateTime(ts) => {
                Some(WritePrecondition::UpdateTime(ts.seconds, ts.nanos))
            }
        }
    }

    /// Build a `RESOURCE_EXHAUSTED` `Status` with rate-limit trailing metadata.
    ///
    /// Called when `RateLimiter::check()` returns `Err(info)`.  Attaches
    /// `x-ratelimit-*` headers plus `retry-after-ms` so clients can back off.
    fn rate_limit_rejection(info: &embyr_core::rate_limit::RateLimitInfo) -> Status {
        use tonic::metadata::{Ascii, MetadataValue};
        let mut md = tonic::metadata::MetadataMap::new();
        Self::attach_rate_limit_headers(&mut md, info);
        if let Ok(v) = info.reset_ms.to_string().parse::<MetadataValue<Ascii>>() {
            let _ = md.insert("retry-after-ms", v);
        }
        Status::with_metadata(
            tonic::Code::ResourceExhausted,
            "rate limit exceeded",
            md,
        )
    }

    /// Attach `x-ratelimit-limit`, `x-ratelimit-remaining`, and `x-ratelimit-reset`
    /// headers to the given metadata map.
    ///
    /// Called on both allowed and rejected responses.  Header values are ASCII
    /// integers (floored for `remaining`); a parsing failure silently skips that
    /// header so it never causes a panic on the hot path.
    fn attach_rate_limit_headers(
        md: &mut tonic::metadata::MetadataMap,
        info: &embyr_core::rate_limit::RateLimitInfo,
    ) {
        use tonic::metadata::{Ascii, MetadataValue};
        if let Ok(v) = info.limit.to_string().parse::<MetadataValue<Ascii>>() {
            let _ = md.insert("x-ratelimit-limit", v);
        }
        if let Ok(v) = (info.remaining.floor() as i64)
            .to_string()
            .parse::<MetadataValue<Ascii>>()
        {
            let _ = md.insert("x-ratelimit-remaining", v);
        }
        if let Ok(v) = info.reset_ms.to_string().parse::<MetadataValue<Ascii>>() {
            let _ = md.insert("x-ratelimit-reset", v);
        }
    }

    /// Return `true` if the query requires a composite index.
    ///
    /// A composite index is required when there is at least one field filter
    /// AND at least one orderBy on a field that differs from the filtered field.
    fn requires_composite_index(query: &StructuredQuery) -> bool {
        // composite-index-requirement-rules (Slice 01, US-01, AC-CIR-01/02):
        // real Firestore always requires a composite index for 2+ orderBy
        // fields, regardless of any filter — checked first, unconditionally,
        // so a filter-only-looking bypass (no filter at all, or every
        // orderBy field coincidentally already filtered) can never suppress
        // it.
        if query.order_by.len() >= 2 {
            return true;
        }

        let Some(filter) = &query.filter else { return false };
        let filter_fields = Self::collect_filter_fields(filter);

        // composite-index-requirement-rules (Slice 02, US-02, AC-CIR-04):
        // an `IN` filter combined with a range comparison (</<=/>/>=) on a
        // DIFFERENT field requires a composite index, independent of
        // whether an orderBy is even present — checked BEFORE the
        // order_by-emptiness early return below, which is exactly the
        // short-circuit that let this shape through uncaught before this
        // slice. A compound-equality-only combination (IN + a SEPARATE
        // EQUALITY filter, live-verified NOT to need composite, AC-CIR-05)
        // is correctly left alone: only the presence of a RANGE operator on
        // a field the IN filter doesn't already cover trips this rule.
        let in_fields: Vec<&str> = filter_fields
            .iter()
            .filter(|(_, op)| matches!(op, FilterOp::In))
            .map(|(f, _)| *f)
            .collect();
        if !in_fields.is_empty() {
            let has_range_on_a_different_field = filter_fields.iter().any(|(f, op)| {
                matches!(
                    op,
                    FilterOp::LessThan
                        | FilterOp::LessThanOrEqual
                        | FilterOp::GreaterThan
                        | FilterOp::GreaterThanOrEqual
                ) && !in_fields.contains(f)
            });
            if has_range_on_a_different_field {
                return true;
            }
        }

        if query.order_by.is_empty() {
            return false;
        }
        // If any orderBy field is NOT in the filter fields, composite index required.
        query
            .order_by
            .iter()
            .any(|ob| !filter_fields.iter().any(|(f, _)| *f == ob.field_path.as_str()))
    }

    /// Collect all (field path, operator) pairs referenced by a filter
    /// (recursively). Carries the operator (Slice 02, composite-index
    /// -requirement-rules) so `requires_composite_index` can distinguish
    /// `IN`/range/equality for the compound-filter rule above — the
    /// original single-`orderBy`-field-vs-filter-field rule below remains
    /// operator-agnostic, unchanged.
    fn collect_filter_fields(filter: &QueryFilter) -> Vec<(&str, FilterOp)> {
        match filter {
            QueryFilter::Field(ff) => vec![(ff.field_path.as_str(), ff.op)],
            QueryFilter::Composite(sub) => {
                sub.iter().flat_map(Self::collect_filter_fields).collect()
            }
        }
    }

    /// composite-index-requirement-rules (Slice 03, US-03, AC-CIR-07):
    /// derives the `Vec<IndexFieldSpec>` a `CreateIndex` call would need
    /// from the SAME `query.filter`/`query.order_by` data `requires_
    /// composite_index` already inspected — no new query analysis. Filtered
    /// fields (in filter order) come first as `Asc`, followed by any
    /// `orderBy` fields not already included (in their own declared
    /// direction) — matches real Firestore's own convention of listing
    /// equality fields before the sort field(s) in a composite index
    /// definition. Only ever called once `requires_composite_index` has
    /// already returned `true` for the same query.
    fn missing_index_fields(query: &StructuredQuery) -> Vec<IndexFieldSpec> {
        let mut fields = Vec::new();
        let mut seen = std::collections::HashSet::new();

        if let Some(filter) = &query.filter {
            for (field, _op) in Self::collect_filter_fields(filter) {
                if seen.insert(field) {
                    fields.push(IndexFieldSpec { field: field.to_string(), order: IndexFieldOrder::Asc });
                }
            }
        }

        for ob in &query.order_by {
            if seen.insert(ob.field_path.as_str()) {
                let order = match ob.direction {
                    OrderDirection::Ascending => IndexFieldOrder::Asc,
                    OrderDirection::Descending => IndexFieldOrder::Desc,
                };
                fields.push(IndexFieldSpec { field: ob.field_path.clone(), order });
            }
        }

        fields
    }

    /// Build a proto `Document` from path, fields, create_time, and update_time.
    fn build_document_response(
        path: &embyr_core::domain::document::DocumentPath,
        fields: &std::collections::BTreeMap<String, embyr_core::domain::field_value::FieldValue>,
        create_time: (i64, i32),
        update_time: (i64, i32),
    ) -> Document {
        Document {
            name: format!(
                "projects/{}/databases/(default)/documents/{}/{}",
                path.project_id.as_str(),
                path.collection_path,
                path.document_id,
            ),
            fields: fields_to_proto(fields),
            create_time: Some(Timestamp { seconds: create_time.0, nanos: create_time.1 }),
            update_time: Some(Timestamp { seconds: update_time.0, nanos: update_time.1 }),
        }
    }

    /// security-rules-cel-path-matching (Slice 02, US-02, ADR-063 §
    /// Decision — Routing Composition, steps 2-3): the routing lookup
    /// itself. Runs ONLY when the caller's own EXISTING exact-match lookup
    /// (`get_access_rule`/`get_write_access_rule`, unchanged, step 1) has
    /// already returned `None` for `collection_path` — this function is
    /// never a substitute for that lookup, only its fallback. Computes the
    /// request's own ancestor `(ancestor_segment_count, literal_skeleton)`
    /// from `collection_path` (reusing `rules_file::parse_path_segments`/
    /// `path_routing::literal_skeleton`, the SAME functions Slice 01's own
    /// import path already proved), narrows via the indexed
    /// `list_access_rule_patterns_by_skeleton` query (typically 0-1
    /// candidate rows), then calls `path_routing::bind_ancestor` per
    /// candidate. Resolution 1's own import-time overlap-rejection
    /// guarantee means AT MOST ONE candidate can structurally match — if
    /// more than one ever does (a bug or a concurrent-import race), this
    /// fails closed (`PermissionDenied`) and logs
    /// `security_rules.routing_invariant_violated`, rather than guessing.
    /// `pub(crate)` (security-rules-cel-path-matching, Slice 03, US-03,
    /// ADR-063): `realtime::listen_handler::handle_add_target` also needs
    /// this same routing lookup for its own subscribe-time fallback
    /// (closing `OQ-CP-04`) — the identical function, never a second,
    /// independently-maintained copy.
    ///
    /// security-rules-cel-recursive-wildcards (Slice 02, US-02, ADR-064 §
    /// Decision — `resolve_access_rule_pattern` Extended): `document_id`
    /// gates a NEW step 3 (recursive-wildcard scan), reached ONLY on a step-2
    /// miss — "4b always wins over a containing recursive wildcard" (AC-17-
    /// 239) falls out of mere composition ORDER, zero runtime containment
    /// check. `document_id: Some(_)` — a concrete document is known, step 3
    /// runs (wired into `handle_get_document` this slice; a write handler or
    /// `handle_add_target` passes `None` until Slice 03 threads its own
    /// per-call value, preserving their exact pre-Slice-02 behavior
    /// unaffected in the meantime — a deliberate narrowing of ADR-064's own
    /// literal `document_id: &str` signature, since `handle_add_target`'s own
    /// subscribe-time call has no concrete document in scope at all).
    pub(crate) async fn resolve_access_rule_pattern(
        system_db: &SystemDb,
        project_id: &str,
        collection_path: &str,
        document_id: Option<&str>,
    ) -> Result<
        Option<(
            crate::adapters::system_db::AccessRulePatternRow,
            std::collections::BTreeMap<String, String>,
        )>,
        Status,
    > {
        let concrete_ancestor =
            embyr_core::access_control::rules_file::parse_path_segments(collection_path).map_err(
                |e| Status::internal(format!("failed to parse the request's own ancestor path: {e:?}")),
            )?;
        let ancestor_segment_count = concrete_ancestor.len() as i16;
        let literal_skeleton = embyr_core::access_control::path_routing::literal_skeleton(&concrete_ancestor);

        let candidates = system_db
            .list_access_rule_patterns_by_skeleton(project_id, ancestor_segment_count, &literal_skeleton)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let mut matched = None;
        for candidate in candidates {
            let pattern_ancestor = embyr_core::access_control::rules_file::parse_path_segments(
                &candidate.collection_path_pattern,
            )
            .map_err(|e| Status::internal(format!("stored pattern failed to re-parse: {e:?}")))?;
            let Some(bindings) =
                embyr_core::access_control::path_routing::bind_ancestor(&pattern_ancestor, &concrete_ancestor)
            else {
                continue;
            };
            if matched.is_some() {
                tracing::error!(
                    project_id,
                    collection_path,
                    "security_rules.routing_invariant_violated: more than one stored pattern \
                     structurally matched the same concrete path"
                );
                return Err(Status::permission_denied("access denied: routing invariant violated"));
            }
            matched = Some((candidate, bindings));
        }

        if matched.is_some() {
            return Ok(matched);
        }

        // Step 3 (NEW, security-rules-cel-recursive-wildcards, Slice 02,
        // ADR-064): recursive-wildcard scan — reached ONLY on a step-2 miss,
        // and ONLY when a concrete document is known.
        let Some(document_id) = document_id else {
            return Ok(None);
        };

        let mut concrete_full_path = concrete_ancestor.clone();
        concrete_full_path.push(embyr_core::access_control::rules_file::PathSegment::Literal(
            document_id.to_string(),
        ));

        let recursive_candidates = system_db
            .list_recursive_access_rule_patterns_up_to(project_id, concrete_full_path.len() as i16)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let mut best: Option<(
            crate::adapters::system_db::AccessRulePatternRow,
            std::collections::BTreeMap<String, String>,
            usize,
        )> = None;
        for candidate in recursive_candidates {
            // ADR-064 § Decision — New Pure Primitives: the empty-fixed
            // -prefix (project-wide catch-all, AC-17-237) case renders to
            // `""`, which `parse_path_segments` would reject as "empty match
            // path" — special-cased to the empty prefix directly.
            let prefix_segments = if candidate.collection_path_pattern.is_empty() {
                Vec::new()
            } else {
                embyr_core::access_control::rules_file::parse_path_segments(
                    &candidate.collection_path_pattern,
                )
                .map_err(|e| {
                    Status::internal(format!("stored recursive pattern failed to re-parse: {e:?}"))
                })?
            };
            let Some((bindings, _remainder)) = embyr_core::access_control::path_routing::bind_recursive_prefix(
                &prefix_segments,
                &concrete_full_path,
            ) else {
                continue;
            };
            match &best {
                Some((_, _, best_len)) if *best_len > prefix_segments.len() => {}
                Some((_, _, best_len)) if *best_len == prefix_segments.len() => {
                    tracing::error!(
                        project_id,
                        collection_path,
                        "security_rules.routing_invariant_violated: two recursive-wildcard \
                         patterns matched the same concrete path at the SAME depth"
                    );
                    return Err(Status::permission_denied("access denied: routing invariant violated"));
                }
                _ => best = Some((candidate, bindings, prefix_segments.len())),
            }
        }

        Ok(best.map(|(row, bindings, _)| (row, bindings)))
    }

    /// Evaluate one write's write-path security rule, shared by
    /// `handle_commit`'s per-write batch loop. Mirrors
    /// `handle_create_document`/`handle_update_document`/`handle_delete_document`'s
    /// own identical 5-step sequence exactly: lookup -> (no rule -> Allow
    /// short-circuit, unchanged behavior) -> pre-write fetch for
    /// `resource_fields` -> `evaluate()` -> `Deny` -> `PermissionDenied`.
    ///
    /// `request_resource_fields`: `Some(fields)` for Update (the proposed
    /// new document); `None` for Delete/Transform, meaning "mirror
    /// `resource_fields`" — Delete has no proposed document (same
    /// fail-closed-via-empty-map choice `handle_delete_document` already
    /// makes), and Transform's real proposed fields are not modeled in this
    /// codebase yet (`field_transforms` is discarded elsewhere, a
    /// separately-tracked gap) — mirroring the CURRENT document against
    /// itself is the closest defensible approximation to "no proposed
    /// change is known", the same choice an Update with unchanged fields
    /// would produce.
    async fn evaluate_write_rule_for_commit(
        system_db: &SystemDb,
        adapter: &SharedBackendAdapter,
        project_id_str: &str,
        path: &embyr_core::domain::document::DocumentPath,
        verified_identity: Option<&embyr_core::client_identity::VerifiedEndUserIdentity>,
        request_resource_fields: Option<&std::collections::BTreeMap<String, FieldValue>>,
    ) -> Result<(), Status> {
        let write_rule_row = system_db
            .get_write_access_rule(project_id_str, &path.collection_path)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let Some(write_rule_row) = write_rule_row else {
            return Ok(());
        };

        let condition = embyr_core::access_control::parse_condition(&write_rule_row.condition_source)
            .map_err(|e| Status::internal(format!("stored write rule failed to re-parse: {e:?}")))?;
        let auth_ctx = verified_identity.map(|v| embyr_core::access_control::AuthContext {
            uid: v.end_user_id.clone(),
            claims: v.claims.clone(),
        });

        let doc_opt = adapter.get_document(path, None).await.map_err(core_error_to_status)?;
        let empty_fields: std::collections::BTreeMap<String, FieldValue> =
            std::collections::BTreeMap::new();
        let resource_fields = doc_opt.as_ref().map(|d| &d.fields).unwrap_or(&empty_fields);
        let request_fields = request_resource_fields.unwrap_or(resource_fields);

        // security-rules-cel-expression-grammar (Slice 06, US-06, ADR-065):
        // the request's own server timestamp — a local clock read, zero
        // new I/O in the sense that matters (no new network/DB round
        // trip), mirroring `Utc::now()`'s own pre-existing use elsewhere
        // in this same file (e.g. Commit's own response formatting).
        let now = chrono::Utc::now();
        let now_field = FieldValue::Timestamp(now.timestamp(), now.timestamp_subsec_nanos() as i32);

        // security-rules-cel-parity (Slice 03, US-03, ADR-062): the
        // document's own already-known target ID (`path.document_id`) —
        // zero new I/O, the SAME `path` this function already received.
        // Mirrors `handle_get_document`'s own Slice 02 wiring exactly.
        match embyr_core::access_control::evaluate(
            &condition,
            auth_ctx.as_ref(),
            resource_fields,
            request_fields,
            Some(path.document_id.as_str()),
            // security-rules-cel-path-matching (Slice 02, ADR-063 §
            // Decision — evaluate() signature): mechanical empty-map
            // argument — write-path routing wiring is Slice 03's own job
            // (OUT of this slice's scope).
            &std::collections::BTreeMap::new(),
            Some(&now_field),
            // security-rules-cel-cross-document-reads (Slice 01, ADR-066):
            // mechanical empty-map — the shared Commit write-path helper's
            // own cross-document wiring is out of this slice's own locked
            // scope (GetDocument only); revisit alongside Slice 04's own
            // CreateDocument/UpdateDocument write-parity work if evidenced.
            &std::collections::BTreeMap::new(),
        ) {
            embyr_core::access_control::EvaluationOutcome::Deny => {
                Err(Status::permission_denied("access denied by write rule"))
            }
            embyr_core::access_control::EvaluationOutcome::Allow => Ok(()),
        }
    }

    /// security-rules-cel-cross-document-reads (Slice 01, ADR-066 §
    /// Decision — Fetch): for each distinct path
    /// `discover_cross_document_paths` returned, splits it into a
    /// `DocumentPath` (the SAME last-segment-is-document-id split
    /// `rules_file.rs`'s own ancestor/leaf discipline already uses) and
    /// calls the EXISTING `get_document` unchanged — zero new port
    /// method, zero new adapter capability. Sequential, not batched
    /// (ADR-066 § Decision Driver 5 — real Firestore's own 10/20-read
    /// ceiling and this feature's own single-level-only structural bound
    /// both confirm the practical count per evaluation is small).
    async fn fetch_cross_document_reads(
        adapter: &SharedBackendAdapter,
        project_id: &embyr_core::domain::project::ProjectId,
        paths: &std::collections::BTreeSet<String>,
    ) -> Result<std::collections::BTreeMap<String, Option<FirestoreDocument>>, Status> {
        let mut results = std::collections::BTreeMap::new();
        for path in paths {
            let (collection_path, document_id) = match path.rsplit_once('/') {
                Some((collection, doc_id)) => (collection.to_string(), doc_id.to_string()),
                None => {
                    // A single-segment path (no '/' at all) has no
                    // document-id position — never produced by a
                    // well-formed `PathTemplate` (its own required
                    // `/databases/$(database)/documents/` prefix, stripped
                    // before storage, guarantees at least one collection
                    // segment plus a document-id segment remain). Falls
                    // closed (no document found) rather than panicking —
                    // `evaluate()`'s own total-by-construction guarantee,
                    // reapplied here at the fetch boundary.
                    results.insert(path.clone(), None);
                    continue;
                }
            };
            let doc_path = DocumentPath {
                project_id: project_id.clone(),
                collection_path,
                document_id,
            };
            let doc = adapter
                .get_document(&doc_path, None)
                .await
                .map_err(core_error_to_status)?;
            results.insert(path.clone(), doc);
        }
        Ok(results)
    }

    /// firestore-batch-write (Slice 01, ADR-048 § Decision 4): the per-write
    /// body extracted verbatim from `translate_writes_for_commit`'s own
    /// pre-existing inline loop (including `evaluate_write_rule_for_commit`'s
    /// per-write call). Translates and access-rule-evaluates ONE write.
    /// Shared by `translate_writes_for_commit` (short-circuiting, via `?`)
    /// and `translate_writes_catching` (never short-circuits) — zero
    /// write-semantics logic duplicated between them.
    ///
    /// Returns `Ok(None)` for a write with no `operation` set — mirrors the
    /// pre-existing `None => {}` inline arm, which silently contributed
    /// nothing to `domain_writes` rather than erroring. Preserved exactly so
    /// `translate_writes_for_commit`'s own external behavior (used by
    /// `handle_commit`/`write_stream.rs`) is unchanged by this extraction.
    /// firestore-field-transforms (Slice 01, ADR-052 § Decision 4): the
    /// shared proto→domain `FieldTransform` translation helper, called from
    /// BOTH the `Update` arm (for `proto_write.update_transforms`) and the
    /// `Transform` arm (for `dt.field_transforms`) — zero write-semantics
    /// logic duplicated, mirroring `translate_one_write_for_commit`'s own
    /// established shared-helper discipline.
    ///
    /// Slice 01 gives `set_to_server_value` its real translation (AC-01-05:
    /// anything other than `REQUEST_TIME` is `InvalidArgument`, document
    /// unmodified). Slice 02 (ADR-052 § Decision 4) adds `increment`/
    /// `maximum`/`minimum`: operand decoded via `proto_value_to_field_value`
    /// (reused unchanged), rejected with `InvalidArgument` here — before any
    /// Postgres round trip — unless it is `Integer`/`Double`. Slice 03 adds
    /// `appendMissingElements`/`removeAllFromArray`: each `ArrayValue.values`
    /// entry decoded via the same `proto_value_to_field_value`; any decode
    /// failure -> `InvalidArgument` (matches `proto_fields_to_domain`'s own
    /// existing error message for the identical failure mode).
    fn translate_field_transforms(
        field_transforms: &[embyr_proto::firestore::document_transform::FieldTransform],
    ) -> Result<Vec<FieldTransform>, Status> {
        use embyr_proto::firestore::document_transform::{field_transform::TransformType, ServerValue};

        field_transforms
            .iter()
            .map(|ft| match &ft.transform_type {
                Some(TransformType::SetToServerValue(raw)) => {
                    if *raw == ServerValue::RequestTime as i32 {
                        Ok(FieldTransform::ServerTimestamp(ft.field_path.clone()))
                    } else {
                        Err(Status::invalid_argument(
                            "unsupported ServerValue in field transform",
                        ))
                    }
                }
                Some(TransformType::Increment(v)) => Self::translate_numeric_operand(v)
                    .map(|value| FieldTransform::Increment(ft.field_path.clone(), value))
                    .ok_or_else(|| Status::invalid_argument("increment delta must be numeric")),
                Some(TransformType::Maximum(v)) => Self::translate_numeric_operand(v)
                    .map(|value| FieldTransform::Maximum(ft.field_path.clone(), value))
                    .ok_or_else(|| Status::invalid_argument("maximum comparand must be numeric")),
                Some(TransformType::Minimum(v)) => Self::translate_numeric_operand(v)
                    .map(|value| FieldTransform::Minimum(ft.field_path.clone(), value))
                    .ok_or_else(|| Status::invalid_argument("minimum comparand must be numeric")),
                Some(TransformType::AppendMissingElements(arr)) => Self::translate_array_operand(arr)
                    .map(|values| FieldTransform::AppendMissingElements(ft.field_path.clone(), values))
                    .ok_or_else(|| Status::invalid_argument("invalid field value in write")),
                Some(TransformType::RemoveAllFromArray(arr)) => Self::translate_array_operand(arr)
                    .map(|values| FieldTransform::RemoveAllFromArray(ft.field_path.clone(), values))
                    .ok_or_else(|| Status::invalid_argument("invalid field value in write")),
                None => Err(Status::invalid_argument(
                    "field transform missing transform_type",
                )),
            })
            .collect()
    }

    /// Decodes an `increment`/`maximum`/`minimum` operand (ADR-052 §
    /// Decision 4) — reuses `proto_value_to_field_value` unchanged, then
    /// filters to `Integer`/`Double` (SPEC.md's own documented non-numeric-
    /// delta rule, extended by direct analogy to `maximum`/`minimum`).
    /// `None` on decode failure OR non-numeric result — the caller turns
    /// either case into the same `InvalidArgument`.
    fn translate_numeric_operand(v: &embyr_proto::firestore::Value) -> Option<FieldValue> {
        crate::encoding::firestore_proto::proto_value_to_field_value(v)
            .filter(|fv| matches!(fv, FieldValue::Integer(_) | FieldValue::Double(_)))
    }

    /// Decodes an `appendMissingElements`/`removeAllFromArray` operand
    /// (ADR-052 § Decision 4) — each `ArrayValue.values` entry via
    /// `proto_value_to_field_value` (reused unchanged). `None` if any
    /// element fails to decode.
    fn translate_array_operand(arr: &embyr_proto::firestore::ArrayValue) -> Option<Vec<FieldValue>> {
        arr.values
            .iter()
            .map(crate::encoding::firestore_proto::proto_value_to_field_value)
            .collect()
    }

    async fn translate_one_write_for_commit(
        system_db: &SystemDb,
        adapter: &SharedBackendAdapter,
        project_id_str: &str,
        verified_identity: Option<&embyr_core::client_identity::VerifiedEndUserIdentity>,
        proto_write: &embyr_proto::firestore::Write,
    ) -> Result<Option<DomainWrite>, Status> {
        let precondition = Self::convert_precondition(proto_write.current_document);
        match &proto_write.operation {
            Some(embyr_proto::firestore::write::Operation::Update(doc)) => {
                let path = Self::parse_document_path(&doc.name)?;
                let fields = proto_fields_to_domain(&doc.fields)
                    .ok_or_else(|| Status::invalid_argument("invalid field value in write"))?;
                // firestore-field-transforms (Slice 01, ADR-052 § Decision
                // 2): `update_transforms` (field 7) — "the transforms to
                // perform after update" — attached to the SAME `Write`
                // message as a regular `update`. Previously never read at
                // all; translates to ONE `DomainWrite::Update` with a
                // non-empty `transforms`, preserving the 1:1
                // `Vec<Write> -> Vec<WriteResult>` invariant `Commit`/`Write`/
                // `BatchWrite` all depend on.
                let transforms = Self::translate_field_transforms(&proto_write.update_transforms)?;

                Self::evaluate_write_rule_for_commit(
                    system_db,
                    adapter,
                    project_id_str,
                    &path,
                    verified_identity,
                    Some(&fields),
                )
                .await?;

                Ok(Some(DomainWrite::Update {
                    path,
                    fields,
                    version: None,
                    precondition,
                    transforms,
                }))
            }
            Some(embyr_proto::firestore::write::Operation::Delete(doc_name)) => {
                let path = Self::parse_document_path(doc_name)?;

                // Delete has no proposed new document (mirrors
                // `handle_delete_document`'s own always-empty
                // `request_resource_fields`).
                let empty_fields: std::collections::BTreeMap<String, FieldValue> =
                    std::collections::BTreeMap::new();
                Self::evaluate_write_rule_for_commit(
                    system_db,
                    adapter,
                    project_id_str,
                    &path,
                    verified_identity,
                    Some(&empty_fields),
                )
                .await?;

                Ok(Some(DomainWrite::Delete {
                    path,
                    version: None,
                    precondition,
                }))
            }
            Some(embyr_proto::firestore::write::Operation::Transform(dt)) => {
                let path = Self::parse_document_path(&dt.document)?;

                // firestore-field-transforms (Slice 01, ADR-052 § Decision
                // 4): `field_transforms` are now actually translated (were
                // unconditionally discarded into `vec![]` before this
                // feature). No proposed-fields shape is modeled for a
                // transform, so `request_resource_fields` mirrors the
                // CURRENT document (`None` — see
                // `evaluate_write_rule_for_commit`'s own doc comment). This
                // still evaluates the write rule for the transform's own
                // document path/collection rather than silently skipping
                // it.
                let transforms = Self::translate_field_transforms(&dt.field_transforms)?;

                Self::evaluate_write_rule_for_commit(
                    system_db,
                    adapter,
                    project_id_str,
                    &path,
                    verified_identity,
                    None,
                )
                .await?;

                Ok(Some(DomainWrite::Transform { path, transforms }))
            }
            None => Ok(None),
        }
    }

    /// firestore-write-streaming (Slice 01, ADR-046 § Decision 3, Reuse
    /// Analysis): the proto-`Write`-message → `DomainWrite` translation loop.
    /// A thin `?`-propagating loop over `translate_one_write_for_commit` —
    /// external signature and behavior UNCHANGED (ADR-048 § Decision 4), so
    /// `handle_commit` and `write_stream.rs` (both existing callers) require
    /// zero call-site changes.
    pub(crate) async fn translate_writes_for_commit(
        system_db: &SystemDb,
        adapter: &SharedBackendAdapter,
        project_id_str: &str,
        verified_identity: Option<&embyr_core::client_identity::VerifiedEndUserIdentity>,
        proto_writes: &[embyr_proto::firestore::Write],
    ) -> Result<Vec<DomainWrite>, Status> {
        let mut domain_writes = Vec::with_capacity(proto_writes.len());
        for proto_write in proto_writes {
            if let Some(domain_write) = Self::translate_one_write_for_commit(
                system_db,
                adapter,
                project_id_str,
                verified_identity,
                proto_write,
            )
            .await?
            {
                domain_writes.push(domain_write);
            }
        }
        Ok(domain_writes)
    }

    /// firestore-batch-write (Slice 01, ADR-048 § Decision 4): the
    /// non-short-circuiting sibling of `translate_writes_for_commit`. Never
    /// aborts the loop over the rest of the batch on a single write's own
    /// translation/rule-denial failure — every write gets its own
    /// `Result`, positionally aligned to `proto_writes` (a write with no
    /// `operation` set becomes that write's own `InvalidArgument` — unlike
    /// `translate_writes_for_commit`'s silent skip, `BatchWrite`'s own
    /// positional-alignment invariant requires one entry per input write,
    /// so a no-op write cannot be silently dropped here).
    pub(crate) async fn translate_writes_catching(
        system_db: &SystemDb,
        adapter: &SharedBackendAdapter,
        project_id_str: &str,
        verified_identity: Option<&embyr_core::client_identity::VerifiedEndUserIdentity>,
        proto_writes: &[embyr_proto::firestore::Write],
    ) -> Vec<Result<DomainWrite, Status>> {
        let mut results = Vec::with_capacity(proto_writes.len());
        for proto_write in proto_writes {
            let result = match Self::translate_one_write_for_commit(
                system_db,
                adapter,
                project_id_str,
                verified_identity,
                proto_write,
            )
            .await
            {
                Ok(Some(domain_write)) => Ok(domain_write),
                Ok(None) => Err(Status::invalid_argument("write has no operation set")),
                Err(status) => Err(status),
            };
            results.push(result);
        }
        results
    }
}

// ---------------------------------------------------------------------------
// Inner handler implementations (private methods on FirestoreService).
//
// Each `handle_*` method contains the original handler logic verbatim.
// The tonic trait impl below is a thin wrapper that:
//   1. Records the wall-clock start time (OBS-03).
//   2. Calls the corresponding `handle_*` method.
//   3. Records counter + histogram via `obs_helpers::record_grpc_call` (OBS-02, OBS-03).
//   4. Returns the result.
//
// This pattern ensures all exit paths (including `?`-propagated errors) are
// instrumented without duplicating code at every return site.
// ---------------------------------------------------------------------------

impl FirestoreService {
    async fn handle_get_document(
        &self,
        request: Request<GetDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        let name = request.get_ref().name.clone();
        let project_id = Self::extract_project_id(&name)?.to_string();
        let api_key = Self::extract_api_key(&request)?;

        let rate_info = match self.rate_limiter.check(&project_id).await {
            Ok(info) => info,
            Err(info) => return Err(Self::rate_limit_rejection(&info)),
        };

        let (adapter, status, _dsn) = self.authenticate(&project_id, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        // client-auth (ADR-026 step 4, additive-only): optional
        // client-identity resolution. Absent header -> zero calls into
        // embyr_core::client_identity (AC-16-08c); present-but-invalid ->
        // this call never rejects `getDoc` (AC-16-08b). Everything above
        // this line is the pre-existing, UNCHANGED authenticate() path
        // (AC-16-08a — regression guardrail).
        //
        // security-rules (ADR-029 § Identity reuse): renamed from
        // `_verified_identity` to `verified_identity` — this feature adds a
        // CONSUMER of the existing return value below, not a new code path
        // into `attach_client_identity_if_present` itself, which is
        // untouched by this feature.
        let verified_identity = self
            .attach_client_identity_if_present(&request, &project_id)
            .await;

        // Record read operation — best-effort, fire-and-forget.
        self.metrics_adapter.record_read(&project_id, 1).await;

        let path = Self::parse_document_path(&name)?;

        // firestore-transaction-read-consistency (US-01, AC-TRC-01/05): a
        // `transaction` consistency-selector registers this read in that
        // transaction's own read set, re-validated at `Commit`. `ReadTime`
        // is out of scope (§ Out of Scope) — unset, unread, unaffected.
        let txn_id = match request.get_ref().consistency_selector {
            Some(GetDocConsistencySelector::Transaction(ref bytes)) => {
                Some(embyr_core::domain::transaction::TransactionId(bytes.clone()))
            }
            _ => None,
        };

        // security-rules (ADR-029 § Structural no-rule-defined guardrail,
        // AC-17-14/15/16): a single indexed lookup on the composite primary
        // key `(project_id, collection_path)`, called BEFORE the document
        // fetch. `None` -> the match arm below is EXACTLY today's
        // pre-`security-rules` code, unmodified —
        // `embyr_core::access_control::evaluate()` is never called. This is
        // the same short-circuit shape as `attach_client_identity_if_present`'s
        // own AC-16-08(c) guarantee.
        let rule_row = self
            .system_db
            .get_access_rule(&project_id, &path.collection_path)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let doc_opt = adapter
            .get_document(&path, txn_id.as_ref())
            .await
            .map_err(core_error_to_status)?;

        match rule_row {
            // No EXACT-MATCH rule defined for this collection.
            //
            // security-rules-cel-path-matching (Slice 02, US-02, ADR-063 §
            // Decision — Routing Composition, step 2): before falling
            // through to the pre-existing unrestricted default, try the NEW
            // multi-segment pattern routing lookup — one additional indexed
            // lookup on this miss path (named Performance-vs-Simplicity
            // trade-off, ADR-063 § Consequences), never a scan. A
            // collection with NO patterns of any kind still falls through
            // to the EXACT SAME unrestricted behavior as before this slice
            // (US-05 zero-regression guardrail) — `resolve_access_rule_pattern`
            // returning `None` reaches the identical match arm the
            // no-rule-at-all case always has.
            None => {
                let routed = Self::resolve_access_rule_pattern(
                    &self.system_db,
                    &project_id,
                    &path.collection_path,
                    // security-rules-cel-recursive-wildcards (Slice 02,
                    // ADR-064): the document's own already-known ID — zero
                    // new I/O, the SAME `path` this handler already parsed
                    // above. Enables step 3's own recursive-wildcard scan
                    // for THIS call site (in scope this slice).
                    Some(path.document_id.as_str()),
                )
                .await?;
                match routed {
                    None => match doc_opt {
                        None => Err(Status::not_found(format!("{name} not found"))),
                        Some(doc) => {
                            let mut response = Response::new(document_to_proto(doc));
                            Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
                            Ok(response)
                        }
                    },
                    Some((pattern_row, ancestor_bindings)) => match pattern_row.read_condition {
                        // ADR-063 § Decision — Schema: `None` on the MATCHED
                        // pattern's own `read_condition` column means
                        // UNRESTRICTED for reads — the same composition rule
                        // an absent row means today, never a fail-closed
                        // deny.
                        None => match doc_opt {
                            None => Err(Status::not_found(format!("{name} not found"))),
                            Some(doc) => {
                                let mut response = Response::new(document_to_proto(doc));
                                Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
                                Ok(response)
                            }
                        },
                        Some(read_condition_source) => {
                            let condition =
                                embyr_core::access_control::parse_condition(&read_condition_source)
                                    .map_err(|e| {
                                        Status::internal(format!(
                                            "stored access rule pattern failed to re-parse: {e:?}"
                                        ))
                                    })?;
                            let auth_ctx = verified_identity.as_ref().map(|v| {
                                embyr_core::access_control::AuthContext {
                                    uid: v.end_user_id.clone(),
                                    claims: v.claims.clone(),
                                }
                            });
                            let empty_fields: std::collections::BTreeMap<String, FieldValue> =
                                std::collections::BTreeMap::new();
                            let resource_fields =
                                doc_opt.as_ref().map(|d| &d.fields).unwrap_or(&empty_fields);
                            // security-rules-cel-expression-grammar (Slice
                            // 07, US-07, ADR-065): the request's own server
                            // timestamp — a local clock read, zero new I/O
                            // in the sense that matters.
                            let now = chrono::Utc::now();
                            let now_field = FieldValue::Timestamp(
                                now.timestamp(),
                                now.timestamp_subsec_nanos() as i32,
                            );

                            // security-rules-cel-cross-document-reads
                            // (Slice 01, US-01, ADR-066): path-discovery
                            // (pure) then fetch (real I/O) — a condition
                            // with no cross-document operand produces an
                            // empty set here, zero extra fetch.
                            let cross_doc_paths = embyr_core::access_control::discover_cross_document_paths(
                                &condition,
                                auth_ctx.as_ref(),
                                Some(path.document_id.as_str()),
                                &ancestor_bindings,
                            );
                            let cross_document_reads = Self::fetch_cross_document_reads(
                                &adapter,
                                &path.project_id,
                                &cross_doc_paths,
                            )
                            .await?;

                            // security-rules-cel-path-matching (Slice 02,
                            // US-02, ADR-063 § Decision — Routing
                            // Composition, step 3): the leaf capture (if
                            // any) threads through the SAME, UNCHANGED
                            // `path_variable_value` slot ADR-062 already
                            // shipped — `path.document_id` unconditionally,
                            // exactly like the exact-match branch below (a
                            // condition referencing no leaf name simply
                            // never looks it up). The NEW ancestor bindings
                            // thread through the NEW 6th parameter.
                            match embyr_core::access_control::evaluate(
                                &condition,
                                auth_ctx.as_ref(),
                                resource_fields,
                                &empty_fields,
                                Some(path.document_id.as_str()),
                                &ancestor_bindings,
                                Some(&now_field),
                                &cross_document_reads,
                            ) {
                                embyr_core::access_control::EvaluationOutcome::Deny => {
                                    Err(Status::permission_denied("access denied by rule"))
                                }
                                embyr_core::access_control::EvaluationOutcome::Allow => match doc_opt {
                                    None => Err(Status::not_found(format!("{name} not found"))),
                                    Some(doc) => {
                                        let mut response = Response::new(document_to_proto(doc));
                                        Self::attach_rate_limit_headers(
                                            response.metadata_mut(),
                                            &rate_info,
                                        );
                                        Ok(response)
                                    }
                                },
                            }
                        }
                    },
                }
            }
            // A rule is defined — evaluate it (US-02/03/04, AC-17-06..13).
            Some(rule_row) => {
                let condition = embyr_core::access_control::parse_condition(&rule_row.condition_source)
                    .map_err(|e| {
                        Status::internal(format!("stored access rule failed to re-parse: {e:?}"))
                    })?;
                let auth_ctx = verified_identity
                    .as_ref()
                    .map(|v| embyr_core::access_control::AuthContext {
                        uid: v.end_user_id.clone(),
                        // custom-claims (US-02, ADR-034): the ONE call site
                        // in this slice's own scope — real claims propagate
                        // from the verified identity into the evaluator.
                        claims: v.claims.clone(),
                    });
                // AC-17-10 (existence non-leakage): a non-existent document
                // evaluates against an EMPTY field map — the same
                // fail-closed mechanism AC-17-09 already uses for a single
                // missing field, applied uniformly. `evaluate()` is called
                // UNCONDITIONALLY, whether or not the document exists.
                let empty_fields: std::collections::BTreeMap<String, FieldValue> =
                    std::collections::BTreeMap::new();
                let resource_fields = doc_opt.as_ref().map(|d| &d.fields).unwrap_or(&empty_fields);
                // security-rules-cel-expression-grammar (Slice 07, US-07,
                // ADR-065): the request's own server timestamp — a local
                // clock read, zero new I/O in the sense that matters.
                let now = chrono::Utc::now();
                let now_field = FieldValue::Timestamp(now.timestamp(), now.timestamp_subsec_nanos() as i32);
                // security-rules-cel-cross-document-reads (Slice 01, US-01,
                // ADR-066): path-discovery (pure) then fetch (real I/O) —
                // a condition with no cross-document operand produces an
                // empty set here, zero extra fetch.
                let cross_doc_paths = embyr_core::access_control::discover_cross_document_paths(
                    &condition,
                    auth_ctx.as_ref(),
                    Some(path.document_id.as_str()),
                    &std::collections::BTreeMap::new(),
                );
                let cross_document_reads =
                    Self::fetch_cross_document_reads(&adapter, &path.project_id, &cross_doc_paths)
                        .await?;

                // security-rules-write-path (ADR-030): one new argument at
                // this existing call site — an empty map for the new
                // `request_resource_fields` parameter. `GetDocument` has no
                // "proposed new document" concept; any read rule that
                // references `request.resource.data.<field>` (grammar-legal
                // but semantically nonsensical for a read) denies via the
                // same fail-closed mechanism, never a crash. Zero other
                // change to this function.
                //
                // security-rules-cel-parity (Slice 02, US-02, ADR-062 §
                // Decision — evaluate() signature): the document's own
                // already-known ID (`path.document_id`) — zero new I/O, the
                // SAME `path` this handler already parsed above. This is
                // the ONE real call site this slice wires with a real
                // value; every other `evaluate()` call site in this
                // codebase passes `None` until its own slice threads a
                // real value.
                match embyr_core::access_control::evaluate(
                    &condition,
                    auth_ctx.as_ref(),
                    resource_fields,
                    &empty_fields,
                    Some(path.document_id.as_str()),
                    // security-rules-cel-path-matching (Slice 02, ADR-063 §
                    // Decision — evaluate() signature): mechanical empty-map
                    // argument on the EXACT-MATCH branch — this rule row has
                    // no ancestor wildcard concept at all (US-05
                    // zero-regression guardrail).
                    &std::collections::BTreeMap::new(),
                    Some(&now_field),
                    &cross_document_reads,
                ) {
                    // AC-17-10: `Deny` ALWAYS produces the identical
                    // `PermissionDenied` response — never distinguishes
                    // "wrong owner" from "document does not exist" for a
                    // rule that references `resource.data` (scoped per
                    // OQ-SR-06, see feature-delta.md § DISTILL).
                    embyr_core::access_control::EvaluationOutcome::Deny => {
                        Err(Status::permission_denied("access denied by rule"))
                    }
                    // `Allow` only then branches on document existence —
                    // unchanged from today's `NotFound`/success shape.
                    embyr_core::access_control::EvaluationOutcome::Allow => match doc_opt {
                        None => Err(Status::not_found(format!("{name} not found"))),
                        Some(doc) => {
                            let mut response = Response::new(document_to_proto(doc));
                            Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
                            Ok(response)
                        }
                    },
                }
            }
        }
    }

    async fn handle_create_document(
        &self,
        request: Request<CreateDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        let req = request.get_ref();

        // Parse project_id from parent: "projects/{pid}/databases/(default)/documents"
        let project_id_str = Self::extract_project_id(&req.parent)?.to_string();
        let api_key = Self::extract_api_key(&request)?;

        let rate_info = match self.rate_limiter.check(&project_id_str).await {
            Ok(info) => info,
            Err(info) => return Err(Self::rate_limit_rejection(&info)),
        };

        let (adapter, status, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        let document_id = if req.document_id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            req.document_id.clone()
        };

        let project_id = embyr_core::domain::project::ProjectId::new(&project_id_str)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let path = embyr_core::domain::document::DocumentPath {
            project_id,
            collection_path: req.collection_id.clone(),
            document_id: document_id.clone(),
        };

        let fields = req
            .document
            .as_ref()
            .map(|d| {
                proto_fields_to_domain(&d.fields)
                    .ok_or_else(|| Status::invalid_argument("invalid field value"))
            })
            .transpose()?
            .unwrap_or_default();

        // security-rules-write-path (ADR-030 § Decision — Composition,
        // "Three new call sites, one shared pattern" — Create case, Slice
        // 02/US-02). Inserted after the existing authenticate()/rate-limit
        // checks and before the existing adapter.create_document(...) call,
        // mirroring `handle_get_document`'s own identity-attach +
        // rule-lookup shape exactly.
        let verified_identity = self
            .attach_client_identity_if_present(&request, &project_id_str)
            .await;

        // Write-rule lookup (queries `write_access_rules` ONLY — never
        // `access_rules`, the structural mechanism behind AC-17-43). `None`
        // -> proceed to the existing adapter.create_document(...) call,
        // completely unmodified (AC-17-42, this slice's own "no rule =
        // unaffected" regression guardrail).
        let write_rule_row = self
            .system_db
            .get_write_access_rule(&project_id_str, &req.collection_id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        match write_rule_row {
            Some(write_rule_row) => {
                let condition =
                    embyr_core::access_control::parse_condition(&write_rule_row.condition_source)
                        .map_err(|e| {
                            Status::internal(format!("stored write rule failed to re-parse: {e:?}"))
                        })?;
                let auth_ctx = verified_identity
                    .as_ref()
                    .map(|v| embyr_core::access_control::AuthContext {
                        uid: v.end_user_id.clone(),
                        // custom-claims (US-03, ADR-034): real claims propagate
                        // from the verified identity into the evaluator,
                        // mirroring `handle_get_document`'s own Slice 02 wiring
                        // exactly — the write-path reuse proof.
                        claims: v.claims.clone(),
                    });

                // Create: `resource_fields` is empty (no document exists yet —
                // AC-17-28's fail-closed mechanism reuse); `request_resource_fields`
                // is the proposed new document already parsed above, no new I/O.
                let empty_resource_fields: std::collections::BTreeMap<String, FieldValue> =
                    std::collections::BTreeMap::new();

                // security-rules-cel-expression-grammar (Slice 06, US-06,
                // ADR-065): the request's own server timestamp — a local
                // clock read, zero new I/O in the sense that matters.
                let now = chrono::Utc::now();
                let now_field =
                    FieldValue::Timestamp(now.timestamp(), now.timestamp_subsec_nanos() as i32);
                // security-rules-cel-cross-document-reads (Slice 04, US-04,
                // ADR-066): path-discovery (pure) then fetch (real I/O) —
                // a condition with no cross-document operand produces an
                // empty set here, zero extra fetch.
                let cross_doc_paths = embyr_core::access_control::discover_cross_document_paths(
                    &condition,
                    auth_ctx.as_ref(),
                    Some(path.document_id.as_str()),
                    &std::collections::BTreeMap::new(),
                );
                let cross_document_reads =
                    Self::fetch_cross_document_reads(&adapter, &path.project_id, &cross_doc_paths)
                        .await?;

                // security-rules-cel-parity (Slice 03, US-03, AC-17-184/186,
                // ADR-062): the document's own already-known target ID
                // (`path.document_id`) — for Create this is the TARGET path
                // being written to, not fetched content (nothing exists yet to
                // fetch), zero new I/O. Mirrors `handle_get_document`'s own
                // Slice 02 wiring exactly.
                match embyr_core::access_control::evaluate(
                    &condition,
                    auth_ctx.as_ref(),
                    &empty_resource_fields,
                    &fields,
                    Some(path.document_id.as_str()),
                    // security-rules-cel-path-matching (ADR-063): this rule
                    // row is an EXACT-MATCH row, no ancestor wildcard concept
                    // at all (mirrors `handle_get_document`'s own exact-match
                    // branch, US-05 zero-regression guardrail).
                    &std::collections::BTreeMap::new(),
                    Some(&now_field),
                    &cross_document_reads,
                ) {
                    embyr_core::access_control::EvaluationOutcome::Deny => {
                        return Err(Status::permission_denied("access denied by write rule"));
                    }
                    embyr_core::access_control::EvaluationOutcome::Allow => {}
                }
            }
            // security-rules-cel-path-matching (Slice 03, US-03,
            // AC-17-213/214/215, ADR-063 § Decision — Routing Composition):
            // no EXACT-MATCH write rule — try the multi-segment pattern
            // routing fallback, mirroring `handle_get_document`'s own Slice
            // 02 wiring exactly. A collection with no rule/pattern of any
            // kind pays zero additional cost beyond this one indexed lookup
            // and proceeds exactly as before this feature (US-05
            // zero-regression guardrail).
            None => {
                let routed = Self::resolve_access_rule_pattern(
                    &self.system_db,
                    &project_id_str,
                    &req.collection_id,
                    // security-rules-cel-recursive-wildcards (Slice 03,
                    // US-03, AC-17-245/246): `Some(path.document_id.as_str())`
                    // — the request's own TARGET path (`path`, already parsed
                    // above), never fetched content (nothing exists yet to
                    // fetch for Create), zero new I/O. Enables step 3's own
                    // recursive-wildcard scan for this call site, mirroring
                    // `handle_get_document`'s own Slice 02 wiring exactly.
                    Some(path.document_id.as_str()),
                )
                .await?;
                if let Some((pattern_row, ancestor_bindings)) = routed {
                    // `None` on the matched pattern's own `write_condition`
                    // column means UNRESTRICTED for writes — the same
                    // composition rule an absent row means today (mirrors
                    // `handle_get_document`'s own `read_condition` handling).
                    if let Some(write_condition_source) = pattern_row.write_condition {
                        let condition = embyr_core::access_control::parse_condition(
                            &write_condition_source,
                        )
                        .map_err(|e| {
                            Status::internal(format!(
                                "stored access rule pattern's write condition failed to re-parse: {e:?}"
                            ))
                        })?;
                        let auth_ctx = verified_identity.as_ref().map(|v| {
                            embyr_core::access_control::AuthContext {
                                uid: v.end_user_id.clone(),
                                claims: v.claims.clone(),
                            }
                        });
                        let empty_resource_fields: std::collections::BTreeMap<String, FieldValue> =
                            std::collections::BTreeMap::new();
                        // security-rules-cel-expression-grammar (Slice 06,
                        // US-06, ADR-065): the request's own server
                        // timestamp — a local clock read, zero new I/O in
                        // the sense that matters.
                        let now = chrono::Utc::now();
                        let now_field = FieldValue::Timestamp(
                            now.timestamp(),
                            now.timestamp_subsec_nanos() as i32,
                        );

                        // AC-17-215: `path.document_id` — the request's own
                        // TARGET path, zero new I/O — resolves identically
                        // whether or not a document already exists.
                        match embyr_core::access_control::evaluate(
                            &condition,
                            auth_ctx.as_ref(),
                            &empty_resource_fields,
                            &fields,
                            Some(path.document_id.as_str()),
                            &ancestor_bindings,
                            Some(&now_field),
                            &std::collections::BTreeMap::new(),
                        ) {
                            embyr_core::access_control::EvaluationOutcome::Deny => {
                                return Err(Status::permission_denied(
                                    "access denied by write rule",
                                ));
                            }
                            embyr_core::access_control::EvaluationOutcome::Allow => {}
                        }
                    }
                }
            }
        }

        let write_result = adapter
            .create_document(&path, fields.clone())
            .await
            .map_err(core_error_to_status)?;

        let create_time = write_result.create_time.unwrap_or(write_result.update_time);
        let update_time = write_result.update_time;

        let mut response = Response::new(Self::build_document_response(
            &path,
            &fields,
            create_time,
            update_time,
        ));
        Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
        Ok(response)
    }

    async fn handle_update_document(
        &self,
        request: Request<UpdateDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        let req = request.get_ref();

        let doc = req
            .document
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("document is required"))?;

        let path = Self::parse_document_path(&doc.name)?;
        let project_id_str = path.project_id.as_str().to_string();
        let api_key = Self::extract_api_key(&request)?;

        let rate_info = match self.rate_limiter.check(&project_id_str).await {
            Ok(info) => info,
            Err(info) => return Err(Self::rate_limit_rejection(&info)),
        };

        let (adapter, status, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        let fields = proto_fields_to_domain(&doc.fields)
            .ok_or_else(|| Status::invalid_argument("invalid field value"))?;

        let precondition = Self::convert_precondition(req.current_document);

        // security-rules-write-path (ADR-030 § Decision — Composition,
        // "Three new call sites, one shared pattern" — Update case, Slice
        // 03/US-03). Identical 5-step sequence to `handle_create_document`,
        // with ONE difference at step 3: the pre-write fetch populates
        // `resource_fields` from the REAL existing document (or an empty
        // map, existence non-leakage — AC-17-34), instead of Create's
        // always-empty map.
        let verified_identity = self
            .attach_client_identity_if_present(&request, &project_id_str)
            .await;

        // Write-rule lookup (queries `write_access_rules` ONLY). `None` ->
        // proceed to the existing adapter.update_document(...) call,
        // completely unmodified (regression guardrail — collections with no
        // write rule pay zero additional I/O, including the pre-write
        // fetch below).
        let write_rule_row = self
            .system_db
            .get_write_access_rule(&project_id_str, &path.collection_path)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        match write_rule_row {
            Some(write_rule_row) => {
                let condition =
                    embyr_core::access_control::parse_condition(&write_rule_row.condition_source)
                        .map_err(|e| {
                            Status::internal(format!("stored write rule failed to re-parse: {e:?}"))
                        })?;
                let auth_ctx = verified_identity
                    .as_ref()
                    .map(|v| embyr_core::access_control::AuthContext {
                        uid: v.end_user_id.clone(),
                        // custom-claims (US-03, ADR-034): real claims propagate
                        // from the verified identity into the evaluator,
                        // mirroring `handle_get_document`'s own Slice 02 wiring
                        // exactly — the write-path reuse proof.
                        claims: v.claims.clone(),
                    });

                // Pre-write state (DIFFERENT from Create): reuses the existing,
                // already-probed `BackendAdapter::get_document` — no new port.
                // Paid only when a write rule is defined for the target
                // collection (gated behind the cheap lookup above).
                //
                // AC-17-34 (existence non-leakage, mirrors ADR-029's own
                // mechanism verbatim): the fetch happens BEFORE the Allow/Deny
                // decision, `resource_fields` falls back to an empty map when
                // the document does not exist, and `evaluate()` is called
                // UNCONDITIONALLY — `Deny` always produces the identical
                // `PermissionDenied` response regardless of whether `doc_opt`
                // was `Some` or `None`.
                let doc_opt = adapter
                    .get_document(&path, None)
                    .await
                    .map_err(core_error_to_status)?;
                let empty_resource_fields: std::collections::BTreeMap<String, FieldValue> =
                    std::collections::BTreeMap::new();
                let resource_fields =
                    doc_opt.as_ref().map(|d| &d.fields).unwrap_or(&empty_resource_fields);

                // Proposed new state: the already-parsed update body fields, no
                // new I/O — the two-value old-vs-new comparison this slice
                // exists to prove.
                // security-rules-cel-expression-grammar (Slice 06, US-06,
                // ADR-065): the request's own server timestamp — a local
                // clock read, zero new I/O in the sense that matters.
                let now = chrono::Utc::now();
                let now_field =
                    FieldValue::Timestamp(now.timestamp(), now.timestamp_subsec_nanos() as i32);
                // security-rules-cel-cross-document-reads (Slice 04, US-04,
                // ADR-066): path-discovery (pure) then fetch (real I/O).
                let cross_doc_paths = embyr_core::access_control::discover_cross_document_paths(
                    &condition,
                    auth_ctx.as_ref(),
                    Some(path.document_id.as_str()),
                    &std::collections::BTreeMap::new(),
                );
                let cross_document_reads =
                    Self::fetch_cross_document_reads(&adapter, &path.project_id, &cross_doc_paths)
                        .await?;

                // security-rules-cel-parity (Slice 03, US-03, AC-17-184/186,
                // ADR-062): the document's own already-known target ID
                // (`path.document_id`) — zero new I/O. Mirrors
                // `handle_get_document`'s own Slice 02 wiring exactly.
                match embyr_core::access_control::evaluate(
                    &condition,
                    auth_ctx.as_ref(),
                    resource_fields,
                    &fields,
                    Some(path.document_id.as_str()),
                    // security-rules-cel-path-matching (ADR-063): this rule
                    // row is an EXACT-MATCH row, no ancestor wildcard concept
                    // at all (mirrors `handle_get_document`'s own exact-match
                    // branch, US-05 zero-regression guardrail).
                    &std::collections::BTreeMap::new(),
                    Some(&now_field),
                    &cross_document_reads,
                ) {
                    embyr_core::access_control::EvaluationOutcome::Deny => {
                        return Err(Status::permission_denied("access denied by write rule"));
                    }
                    embyr_core::access_control::EvaluationOutcome::Allow => {}
                }
            }
            // security-rules-cel-path-matching (Slice 03, US-03,
            // AC-17-213/214/215, ADR-063): no EXACT-MATCH write rule — try
            // the multi-segment pattern routing fallback, mirroring
            // `handle_get_document`'s own Slice 02 wiring exactly.
            None => {
                let routed = Self::resolve_access_rule_pattern(
                    &self.system_db,
                    &project_id_str,
                    &path.collection_path,
                    // security-rules-cel-recursive-wildcards (Slice 03,
                    // US-03, AC-17-245/246): `Some(path.document_id.as_str())`
                    // — the request's own TARGET path, zero new I/O. Enables
                    // step 3's own recursive-wildcard scan for this call
                    // site, mirroring `handle_get_document`'s own Slice 02
                    // wiring exactly.
                    Some(path.document_id.as_str()),
                )
                .await?;
                if let Some((pattern_row, ancestor_bindings)) = routed {
                    if let Some(write_condition_source) = pattern_row.write_condition {
                        let condition = embyr_core::access_control::parse_condition(
                            &write_condition_source,
                        )
                        .map_err(|e| {
                            Status::internal(format!(
                                "stored access rule pattern's write condition failed to re-parse: {e:?}"
                            ))
                        })?;
                        let auth_ctx = verified_identity.as_ref().map(|v| {
                            embyr_core::access_control::AuthContext {
                                uid: v.end_user_id.clone(),
                                claims: v.claims.clone(),
                            }
                        });

                        let doc_opt = adapter
                            .get_document(&path, None)
                            .await
                            .map_err(core_error_to_status)?;
                        let empty_resource_fields: std::collections::BTreeMap<String, FieldValue> =
                            std::collections::BTreeMap::new();
                        let resource_fields = doc_opt
                            .as_ref()
                            .map(|d| &d.fields)
                            .unwrap_or(&empty_resource_fields);
                        // security-rules-cel-expression-grammar (Slice 06,
                        // US-06, ADR-065): the request's own server
                        // timestamp — a local clock read, zero new I/O in
                        // the sense that matters.
                        let now = chrono::Utc::now();
                        let now_field = FieldValue::Timestamp(
                            now.timestamp(),
                            now.timestamp_subsec_nanos() as i32,
                        );

                        // AC-17-215: `path.document_id` — the request's own
                        // TARGET path, zero new I/O.
                        match embyr_core::access_control::evaluate(
                            &condition,
                            auth_ctx.as_ref(),
                            resource_fields,
                            &fields,
                            Some(path.document_id.as_str()),
                            &ancestor_bindings,
                            Some(&now_field),
                            &std::collections::BTreeMap::new(),
                        ) {
                            embyr_core::access_control::EvaluationOutcome::Deny => {
                                return Err(Status::permission_denied(
                                    "access denied by write rule",
                                ));
                            }
                            embyr_core::access_control::EvaluationOutcome::Allow => {}
                        }
                    }
                }
            }
        }

        let write_result = adapter
            .update_document(&path, fields.clone(), precondition)
            .await
            .map_err(core_error_to_status)?;

        let update_time = write_result.update_time;
        // For updates, create_time not returned by the write op; use update_time as fallback.
        let create_time = write_result.create_time.unwrap_or(update_time);

        let mut response = Response::new(Self::build_document_response(
            &path,
            &fields,
            create_time,
            update_time,
        ));
        Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
        Ok(response)
    }

    async fn handle_delete_document(
        &self,
        request: Request<DeleteDocumentRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.get_ref();

        let path = Self::parse_document_path(&req.name)?;
        let project_id_str = path.project_id.as_str().to_string();
        let api_key = Self::extract_api_key(&request)?;

        let rate_info = match self.rate_limiter.check(&project_id_str).await {
            Ok(info) => info,
            Err(info) => return Err(Self::rate_limit_rejection(&info)),
        };

        let (adapter, status, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        let precondition = Self::convert_precondition(req.current_document);

        // security-rules-write-path (ADR-030 § Decision — Composition,
        // "Three new call sites, one shared pattern" — Delete case, Slice
        // 04/US-04). Identical 5-step sequence to `handle_update_document`,
        // with ONE difference at step 4: `request_resource_fields` is ALWAYS
        // an empty map — a delete has no proposed new document, so any write
        // rule referencing `request.resource.data.<field>` fails closed via
        // the same fail-closed mechanism `evaluate()` already uses for a
        // missing field, never a crash and never a special case here.
        let verified_identity = self
            .attach_client_identity_if_present(&request, &project_id_str)
            .await;

        // Write-rule lookup (queries `write_access_rules` ONLY). `None` ->
        // proceed to the existing adapter.delete_document(...) call,
        // completely unmodified (regression guardrail — collections with no
        // write rule pay zero additional I/O, including the pre-write fetch
        // below).
        let write_rule_row = self
            .system_db
            .get_write_access_rule(&project_id_str, &path.collection_path)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        match write_rule_row {
            Some(write_rule_row) => {
                let condition =
                    embyr_core::access_control::parse_condition(&write_rule_row.condition_source)
                        .map_err(|e| {
                            Status::internal(format!("stored write rule failed to re-parse: {e:?}"))
                        })?;
                let auth_ctx = verified_identity
                    .as_ref()
                    .map(|v| embyr_core::access_control::AuthContext {
                        uid: v.end_user_id.clone(),
                        // custom-claims (US-03, ADR-034): real claims propagate
                        // from the verified identity into the evaluator,
                        // mirroring `handle_get_document`'s own Slice 02 wiring
                        // exactly — the write-path reuse proof.
                        claims: v.claims.clone(),
                    });

                // Pre-write state: reuses the existing, already-probed
                // `BackendAdapter::get_document` — no new port. Paid only when a
                // write rule is defined for the target collection (gated behind
                // the cheap lookup above).
                //
                // AC-17-38 (existence non-leakage, mirrors AC-17-34/ADR-029's
                // own mechanism verbatim): the fetch happens BEFORE the
                // Allow/Deny decision, `resource_fields` falls back to an empty
                // map when the document does not exist, and `evaluate()` is
                // called UNCONDITIONALLY — `Deny` always produces the identical
                // `PermissionDenied` response regardless of whether `doc_opt`
                // was `Some` or `None`.
                let doc_opt = adapter
                    .get_document(&path, None)
                    .await
                    .map_err(core_error_to_status)?;
                let empty_fields: std::collections::BTreeMap<String, FieldValue> =
                    std::collections::BTreeMap::new();
                let resource_fields = doc_opt.as_ref().map(|d| &d.fields).unwrap_or(&empty_fields);

                // Proposed new state: ALWAYS empty — a delete has no request
                // body to parse into fields (DIFFERENT from Create/Update).
                let request_resource_fields: std::collections::BTreeMap<String, FieldValue> =
                    std::collections::BTreeMap::new();

                // security-rules-cel-parity (Slice 03, US-03, AC-17-184,
                // ADR-062): the document's own already-known target ID
                // (`path.document_id`) — zero new I/O. Mirrors
                // `handle_get_document`'s own Slice 02 wiring exactly.
                match embyr_core::access_control::evaluate(
                    &condition,
                    auth_ctx.as_ref(),
                    resource_fields,
                    &request_resource_fields,
                    Some(path.document_id.as_str()),
                    // security-rules-cel-path-matching (ADR-063): this rule
                    // row is an EXACT-MATCH row, no ancestor wildcard concept
                    // at all (mirrors `handle_get_document`'s own exact-match
                    // branch, US-05 zero-regression guardrail).
                    &std::collections::BTreeMap::new(),
                    // security-rules-cel-expression-grammar (Slice 06,
                    // ADR-065): mechanical `None` — no domain example
                    // requires a time-window check on Delete; out of this
                    // feature's own locked scope entirely.
                    None,
                    // security-rules-cel-cross-document-reads (Slice 01,
                    // ADR-066): mechanical empty-map — Delete's own
                    // cross-document wiring is out of this feature's own
                    // locked scope (never evidenced, § Out of Scope).
                    &std::collections::BTreeMap::new(),
                ) {
                    embyr_core::access_control::EvaluationOutcome::Deny => {
                        return Err(Status::permission_denied("access denied by write rule"));
                    }
                    embyr_core::access_control::EvaluationOutcome::Allow => {}
                }
            }
            // security-rules-cel-path-matching (Slice 03, US-03,
            // AC-17-213/214, ADR-063): no EXACT-MATCH write rule — try the
            // multi-segment pattern routing fallback, mirroring
            // `handle_get_document`'s own Slice 02 wiring exactly.
            None => {
                let routed = Self::resolve_access_rule_pattern(
                    &self.system_db,
                    &project_id_str,
                    &path.collection_path,
                    // security-rules-cel-recursive-wildcards (Slice 03,
                    // US-03, AC-17-245/246): `Some(path.document_id.as_str())`
                    // — the request's own TARGET path, zero new I/O. Enables
                    // step 3's own recursive-wildcard scan for this call
                    // site, mirroring `handle_get_document`'s own Slice 02
                    // wiring exactly.
                    Some(path.document_id.as_str()),
                )
                .await?;
                if let Some((pattern_row, ancestor_bindings)) = routed {
                    if let Some(write_condition_source) = pattern_row.write_condition {
                        let condition = embyr_core::access_control::parse_condition(
                            &write_condition_source,
                        )
                        .map_err(|e| {
                            Status::internal(format!(
                                "stored access rule pattern's write condition failed to re-parse: {e:?}"
                            ))
                        })?;
                        let auth_ctx = verified_identity.as_ref().map(|v| {
                            embyr_core::access_control::AuthContext {
                                uid: v.end_user_id.clone(),
                                claims: v.claims.clone(),
                            }
                        });

                        let doc_opt = adapter
                            .get_document(&path, None)
                            .await
                            .map_err(core_error_to_status)?;
                        let empty_fields: std::collections::BTreeMap<String, FieldValue> =
                            std::collections::BTreeMap::new();
                        let resource_fields =
                            doc_opt.as_ref().map(|d| &d.fields).unwrap_or(&empty_fields);
                        let request_resource_fields: std::collections::BTreeMap<String, FieldValue> =
                            std::collections::BTreeMap::new();

                        match embyr_core::access_control::evaluate(
                            &condition,
                            auth_ctx.as_ref(),
                            resource_fields,
                            &request_resource_fields,
                            Some(path.document_id.as_str()),
                            &ancestor_bindings,
                            // security-rules-cel-expression-grammar (Slice
                            // 06, ADR-065): mechanical `None` — out of this
                            // feature's own locked scope for Delete.
                            None,
                            &std::collections::BTreeMap::new(),
                        ) {
                            embyr_core::access_control::EvaluationOutcome::Deny => {
                                return Err(Status::permission_denied(
                                    "access denied by write rule",
                                ));
                            }
                            embyr_core::access_control::EvaluationOutcome::Allow => {}
                        }
                    }
                }
            }
        }

        adapter
            .delete_document(&path, precondition)
            .await
            .map_err(core_error_to_status)?;

        let mut response = Response::new(());
        Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
        Ok(response)
    }

    /// firestore-list-rpcs (Slice 01, US-01, ADR-050/051): `ListDocuments`
    /// handler. Auth/rate-limit/suspension sequence mirrors
    /// `handle_get_document`'s own unary shape. Access-rule evaluation is
    /// PER-DOCUMENT — mirrors `handle_batch_get_documents`'s own pattern
    /// (the closer analog for a LIST of documents than `handle_run_query`'s
    /// query-SHAPE compliance check, since `ListDocuments` accepts no
    /// caller-supplied filter to validate the shape of): every candidate
    /// document is evaluated against its own collection's read rule, denied
    /// documents are silently excluded from the result (never abort the
    /// whole call, mirroring `handle_batch_get_documents`'s own
    /// never-abort-the-batch discipline).
    ///
    /// Uniform fetch-all-then-paginate-in-Rust for BOTH the `collection_id`
    /// set AND empty cases (ADR-050's own named, accepted simplification for
    /// the empty case, extended here to the set case too): `run_query` is
    /// called with no `limit`/`offset`, and the merged/filtered/sorted `Vec`
    /// is windowed in Rust via the same "fetch everything, then slice"
    /// technique. This is deliberately NOT SQL `LIMIT`/`OFFSET` push-down —
    /// `AgentBackendAdapter::run_query` does not forward `limit`/`offset` to
    /// the agent's own `RunQuery` RPC at all (confirmed by reading
    /// `crates/embyr-server/src/adapters/agent_backend.rs::run_query`, a
    /// pre-existing characteristic unrelated to this feature), so a
    /// SQL-push-down design would silently return page 1 for every
    /// `backend_mode=agent` page request. The uniform in-Rust-windowing
    /// design makes `backend_mode=agent` pagination correct by construction
    /// instead — `run_query`'s "return everything" agent-mode behavior is
    /// exactly what this handler already expects for every backend.
    async fn handle_list_documents(
        &self,
        request: Request<ListDocumentsRequest>,
    ) -> Result<Response<ListDocumentsResponse>, Status> {
        let req = request.get_ref();
        let (project_id_str, prefix) = Self::parse_parent_prefix(&req.parent)?;
        let api_key = Self::extract_api_key(&request)?;

        let rate_info = match self.rate_limiter.check(&project_id_str).await {
            Ok(info) => info,
            Err(info) => return Err(Self::rate_limit_rejection(&info)),
        };

        let (adapter, status_str, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status_str == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        let verified_identity = self
            .attach_client_identity_if_present(&request, &project_id_str)
            .await;
        let auth_ctx = verified_identity
            .as_ref()
            .map(|v| embyr_core::access_control::AuthContext {
                uid: v.end_user_id.clone(),
                claims: v.claims.clone(),
            });

        let project_id = embyr_core::domain::project::ProjectId::new(&project_id_str)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let page_size = if req.page_size <= 0 { 100i32 } else { req.page_size.min(100) };
        let offset = embyr_core::pagination::decode_page_token(&req.page_token)
            .map_err(core_error_to_status)?;

        // Every collection this call must fan out over: exactly one when
        // `collection_id` is set; every immediate child of `parent`
        // (ADR-050/051's shared "immediate children" primitive, called
        // internally/unpaginated) when it is empty (AC-01-03).
        let collection_paths: Vec<String> = if req.collection_id.is_empty() {
            let parent_collection = CollectionPath {
                project_id: project_id.clone(),
                collection_path: prefix.clone(),
            };
            let child_names = adapter
                .list_collection_ids(&parent_collection, i32::MAX, 0)
                .await
                .map_err(core_error_to_status)?;
            child_names
                .into_iter()
                .map(|name| {
                    if prefix.is_empty() {
                        name
                    } else {
                        format!("{prefix}/{name}")
                    }
                })
                .collect()
        } else if prefix.is_empty() {
            vec![req.collection_id.clone()]
        } else {
            vec![format!("{prefix}/{}", req.collection_id)]
        };

        // DDD-BGD-6-style in-request per-collection access-rule cache — one
        // lookup per distinct collection_path within this call.
        let mut rule_cache: HashMap<String, Option<crate::adapters::system_db::AccessRuleRow>> =
            HashMap::new();
        let mut all_docs: Vec<embyr_core::domain::document::FirestoreDocument> = Vec::new();

        for collection_path in &collection_paths {
            let collection = CollectionPath {
                project_id: project_id.clone(),
                collection_path: collection_path.clone(),
            };
            let query = StructuredQuery {
                collection_id: collection_path.clone(),
                all_descendants: false,
                filter: None,
                order_by: vec![],
                limit: None,
                offset: None,
                start_at: None,
                end_at: None,
                since_update_time: None,
            };
            let docs = adapter
                .run_query(&collection, &query, None)
                .await
                .map_err(core_error_to_status)?;

            let rule_row = match rule_cache.get(collection_path) {
                Some(cached) => cached.clone(),
                None => {
                    let fetched = self
                        .system_db
                        .get_access_rule(&project_id_str, collection_path)
                        .await
                        .map_err(|e| Status::internal(e.to_string()))?;
                    rule_cache.insert(collection_path.clone(), fetched.clone());
                    fetched
                }
            };

            match rule_row {
                // No rule defined — unrestricted, mirrors GetDocument's own
                // no-rule short-circuit.
                None => all_docs.extend(docs),
                Some(rule_row) => {
                    let condition =
                        embyr_core::access_control::parse_condition(&rule_row.condition_source)
                            .map_err(|e| {
                                Status::internal(format!(
                                    "stored access rule failed to re-parse: {e:?}"
                                ))
                            })?;
                    let empty_fields: std::collections::BTreeMap<String, FieldValue> =
                        std::collections::BTreeMap::new();
                    for doc in docs {
                        // security-rules-cel-parity (Slice 02, ADR-062):
                        // `evaluate()`'s new 5th parameter, mechanical
                        // `None` here — `ListDocuments` is out of this
                        // slice's own locked scope (`GetDocument` only).
                        match embyr_core::access_control::evaluate(
                            &condition,
                            auth_ctx.as_ref(),
                            &doc.fields,
                            &empty_fields,
                            None,
                            &std::collections::BTreeMap::new(),
                            // security-rules-cel-expression-grammar (Slice
                            // 06, ADR-065): mechanical `None` — out of this
                            // feature's own locked scope (`ListDocuments`).
                            None,
                            &std::collections::BTreeMap::new(),
                        ) {
                            embyr_core::access_control::EvaluationOutcome::Allow => {
                                all_docs.push(doc);
                            }
                            embyr_core::access_control::EvaluationOutcome::Deny => {}
                        }
                    }
                }
            }
        }

        // Deterministic merge order across collections (ADR-050).
        all_docs.sort_by(|a, b| {
            (a.path.collection_path.as_str(), a.path.document_id.as_str())
                .cmp(&(b.path.collection_path.as_str(), b.path.document_id.as_str()))
        });

        // Fetch-one-extra-to-detect-more-pages, applied in Rust over the
        // merged/filtered/sorted Vec (the same technique
        // `crates/embyr-agent/src/server.rs::list_documents` already proved
        // via SQL LIMIT/OFFSET, generalized here to work uniformly across
        // every backend_mode).
        let start = offset as usize;
        let (page, has_more) = if start >= all_docs.len() {
            (Vec::new(), false)
        } else {
            let end = start.saturating_add(page_size as usize + 1).min(all_docs.len());
            let mut slice = all_docs[start..end].to_vec();
            let more = slice.len() > page_size as usize;
            if more {
                slice.truncate(page_size as usize);
            }
            (slice, more)
        };

        let next_page_token = if has_more {
            embyr_core::pagination::encode_page_token(offset + page_size as u32)
        } else {
            String::new()
        };

        let documents = page.into_iter().map(document_to_proto).collect();

        let mut response = Response::new(ListDocumentsResponse { documents, next_page_token });
        Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
        Ok(response)
    }

    /// firestore-list-rpcs (Slice 02, US-02, ADR-051): `ListCollectionIds` —
    /// same auth/rate-limit/suspension sequence as `handle_list_documents`,
    /// no per-document access-rule evaluation (this RPC only exposes
    /// collection NAMES, never document contents, so there is nothing for
    /// `security-rules-query-path`'s own per-document rule engine to filter).
    /// Reuses the shared `list_collection_ids` primitive with the SAME
    /// fetch-one-extra-to-detect-more-pages pagination technique
    /// `handle_list_documents`/`run_query` already use.
    async fn handle_list_collection_ids(
        &self,
        request: Request<ListCollectionIdsRequest>,
    ) -> Result<Response<ListCollectionIdsResponse>, Status> {
        let req = request.get_ref();
        let (project_id_str, prefix) = Self::parse_parent_prefix(&req.parent)?;
        let api_key = Self::extract_api_key(&request)?;

        let rate_info = match self.rate_limiter.check(&project_id_str).await {
            Ok(info) => info,
            Err(info) => return Err(Self::rate_limit_rejection(&info)),
        };

        let (adapter, status_str, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status_str == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        let project_id = embyr_core::domain::project::ProjectId::new(&project_id_str)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let page_size = if req.page_size <= 0 { 100i32 } else { req.page_size.min(100) };
        let offset = embyr_core::pagination::decode_page_token(&req.page_token)
            .map_err(core_error_to_status)?;

        let parent = CollectionPath { project_id, collection_path: prefix };

        // Fetch-one-extra-to-detect-more-pages, same technique `run_query`'s
        // own LIMIT/OFFSET provides — `list_collection_ids`'s own SQL
        // (ADR-051 § Decision 2) already applies LIMIT/OFFSET server-side.
        let mut ids = adapter
            .list_collection_ids(&parent, page_size + 1, offset as i32)
            .await
            .map_err(core_error_to_status)?;

        let has_more = ids.len() > page_size as usize;
        if has_more {
            ids.truncate(page_size as usize);
        }
        let next_page_token = if has_more {
            embyr_core::pagination::encode_page_token(offset + page_size as u32)
        } else {
            String::new()
        };

        let mut response =
            Response::new(ListCollectionIdsResponse { collection_ids: ids, next_page_token });
        Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
        Ok(response)
    }

    async fn handle_batch_get_documents(
        &self,
        request: Request<BatchGetDocumentsRequest>,
    ) -> Result<Response<tonic::codegen::BoxStream<BatchGetDocumentsResponse>>, Status> {
        // Per-call composition (DDD-BGD-2/DDD-BGD-1): project identity comes
        // from `database`, NOT `documents[0]` — mirrors `handle_run_query`'s
        // own auth/rate-limit/identity-once-per-call granularity.
        let req = request.get_ref();
        let project_id_str = Self::extract_project_id(&req.database)?.to_string();
        let api_key = Self::extract_api_key(&request)?;

        let rate_info = match self.rate_limiter.check(&project_id_str).await {
            Ok(info) => info,
            Err(info) => return Err(Self::rate_limit_rejection(&info)),
        };

        let (adapter, status_str, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status_str == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        let verified_identity = self
            .attach_client_identity_if_present(&request, &project_id_str)
            .await;

        let documents = req.documents.clone();

        // DDD-BGD-3: non-empty + single-project validation, up front,
        // before any per-document work begins (AC-01-05/06).
        if documents.is_empty() {
            return Err(Status::invalid_argument("documents must not be empty"));
        }
        // DDD-BGD-14 (peer-review-surfaced, orchestrator-confirmed): bound
        // per-call resource consumption — a single rate-limit token must not
        // purchase an unbounded number of get_access_rule/get_document calls.
        if documents.len() > 1000 {
            return Err(Status::invalid_argument(
                "documents must not exceed 1000 per call",
            ));
        }
        for doc_name in &documents {
            if Self::extract_project_id(doc_name)? != project_id_str {
                return Err(Status::invalid_argument(
                    "all documents must belong to the same project as database",
                ));
            }
        }

        // DDD-BGD-9: usage metering counts N documents, once per call.
        self.metrics_adapter
            .record_read(&project_id_str, documents.len() as i64)
            .await;

        let auth_ctx = verified_identity
            .as_ref()
            .map(|v| embyr_core::access_control::AuthContext {
                uid: v.end_user_id.clone(),
                claims: v.claims.clone(),
            });

        let build_found = |doc: embyr_core::domain::document::FirestoreDocument| BatchGetDocumentsResponse {
            result: Some(embyr_proto::firestore::batch_get_documents_response::Result::Found(
                document_to_proto(doc),
            )),
            ..Default::default()
        };
        let build_missing = |name: &str| BatchGetDocumentsResponse {
            result: Some(embyr_proto::firestore::batch_get_documents_response::Result::Missing(
                name.to_string(),
            )),
            ..Default::default()
        };

        // DDD-BGD-6: in-request per-collection access-rule cache — lazily
        // populated on first reference to a given collection_path, read on
        // every subsequent reference within this same call.
        let mut rule_cache: HashMap<String, Option<crate::adapters::system_db::AccessRuleRow>> =
            HashMap::new();
        let mut responses: Vec<Result<BatchGetDocumentsResponse, Status>> =
            Vec::with_capacity(documents.len());

        for doc_name in &documents {
            let path = Self::parse_document_path(doc_name)?;

            // DDD-BGD-4/6: cached per-collection access-rule lookup,
            // mirrors handle_get_document's own single indexed lookup.
            let rule_row = match rule_cache.get(&path.collection_path) {
                Some(cached) => cached.clone(),
                None => {
                    let fetched = self
                        .system_db
                        .get_access_rule(&project_id_str, &path.collection_path)
                        .await
                        .map_err(|e| Status::internal(e.to_string()))?;
                    rule_cache.insert(path.collection_path.clone(), fetched.clone());
                    fetched
                }
            };

            // DDD-BGD-7: a genuine infra error aborts the whole call — never
            // conflated with a `Deny` (ADR-042 handles denial separately).
            let doc_opt = adapter
                .get_document(&path, None)
                .await
                .map_err(|e| Status::internal(e.to_string()))?;

            let response_item = match rule_row {
                // No rule defined — unrestricted, mirrors GetDocument's own
                // no-rule short-circuit (AC-01-04).
                None => match doc_opt {
                    None => build_missing(doc_name),
                    Some(doc) => build_found(doc),
                },
                Some(rule_row) => {
                    let condition = embyr_core::access_control::parse_condition(&rule_row.condition_source)
                        .map_err(|e| {
                            Status::internal(format!(
                                "stored access rule failed to re-parse: {e:?}"
                            ))
                        })?;
                    let empty_fields: std::collections::BTreeMap<String, FieldValue> =
                        std::collections::BTreeMap::new();
                    let resource_fields =
                        doc_opt.as_ref().map(|d| &d.fields).unwrap_or(&empty_fields);

                    // security-rules-cel-parity (Slice 02, ADR-062):
                    // `evaluate()`'s new 5th parameter, mechanical `None`
                    // here — `BatchGetDocuments` is out of this slice's own
                    // locked scope (`GetDocument` only).
                    match embyr_core::access_control::evaluate(
                        &condition,
                        auth_ctx.as_ref(),
                        resource_fields,
                        &empty_fields,
                        None,
                        &std::collections::BTreeMap::new(),
                        // security-rules-cel-expression-grammar (Slice 06,
                        // ADR-065): mechanical `None` — out of this
                        // feature's own locked scope (`BatchGetDocuments`).
                        None,
                        &std::collections::BTreeMap::new(),
                    ) {
                        // ADR-042/DDD-BGD-5: `Deny` maps to a per-document
                        // `missing` item — the batch is NEVER aborted
                        // (AC-01-03), unlike GetDocument's own PermissionDenied.
                        embyr_core::access_control::EvaluationOutcome::Deny => {
                            build_missing(doc_name)
                        }
                        embyr_core::access_control::EvaluationOutcome::Allow => match doc_opt {
                            None => build_missing(doc_name),
                            Some(doc) => build_found(doc),
                        },
                    }
                }
            };
            responses.push(Ok(response_item));
        }

        // DDD-BGD-8/DDD-BGD-11: eager Vec -> stream, mirroring
        // handle_run_query's own construction; streamed in request order.
        let stream: tonic::codegen::BoxStream<BatchGetDocumentsResponse> =
            Box::pin(tokio_stream::iter(responses));
        let mut response = Response::new(stream);
        Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
        Ok(response)
    }

    async fn handle_begin_transaction(
        &self,
        request: Request<BeginTransactionRequest>,
    ) -> Result<Response<BeginTransactionResponse>, Status> {
        let req = request.get_ref();
        let project_id_str = Self::extract_project_id(&req.database)?.to_string();
        let api_key = Self::extract_api_key(&request)?;

        let rate_info = match self.rate_limiter.check(&project_id_str).await {
            Ok(info) => info,
            Err(info) => return Err(Self::rate_limit_rejection(&info)),
        };

        let (adapter, status, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        let project_id = embyr_core::domain::project::ProjectId::new(&project_id_str)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let options = match req.options.as_ref().and_then(|o| o.mode.as_ref()) {
            Some(embyr_proto::firestore::transaction_options::Mode::ReadOnly(_)) => {
                TransactionOptions::ReadOnly
            }
            _ => TransactionOptions::ReadWrite,
        };

        let txn_id = adapter
            .begin_transaction(&project_id, options)
            .await
            .map_err(core_error_to_status)?;

        let mut response = Response::new(BeginTransactionResponse {
            transaction: txn_id.0,
        });
        Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
        Ok(response)
    }

    async fn handle_commit(
        &self,
        request: Request<CommitRequest>,
    ) -> Result<Response<CommitResponse>, Status> {
        let req = request.get_ref();
        let project_id_str = Self::extract_project_id(&req.database)?.to_string();
        let api_key = Self::extract_api_key(&request)?;

        let rate_info = match self.rate_limiter.check(&project_id_str).await {
            Ok(info) => info,
            Err(info) => return Err(Self::rate_limit_rejection(&info)),
        };

        let (adapter, status, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        let project_id = embyr_core::domain::project::ProjectId::new(&project_id_str)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let txn_id = embyr_core::domain::transaction::TransactionId(req.transaction.clone());

        // security-rules-write-path bug fix (2026-08-30): `Commit` batches
        // writes across possibly-many collections/documents in one call.
        // Every write's own collection write rule is evaluated HERE, before
        // any write is translated for `commit_transaction`. If ANY write in
        // the batch is denied, the whole `Commit` is rejected before
        // `commit_transaction` is ever called — mirroring `Commit`'s own
        // pre-existing all-or-nothing atomicity contract, and reusing the
        // exact evaluation shape `handle_create_document`/
        // `handle_update_document`/`handle_delete_document` already use.
        let verified_identity = self
            .attach_client_identity_if_present(&request, &project_id_str)
            .await;

        // Translate proto writes to domain writes — shared with the
        // `Write` bidi-stream's own per-message loop (firestore-write-streaming,
        // Slice 01, ADR-046 § Decision 3, Reuse Analysis).
        let domain_writes = Self::translate_writes_for_commit(
            &self.system_db,
            &adapter,
            &project_id_str,
            verified_identity.as_ref(),
            &req.writes,
        )
        .await?;

        let write_results = adapter
            .commit_transaction(&project_id, &txn_id, domain_writes)
            .await
            .map_err(core_error_to_status)?;

        let now = chrono::Utc::now();
        let proto_results: Vec<embyr_proto::firestore::WriteResult> = write_results
            .into_iter()
            .map(|wr| embyr_proto::firestore::WriteResult {
                update_time: Some(Timestamp {
                    seconds: wr.update_time.0,
                    nanos: wr.update_time.1,
                }),
                transform_results: wr.transform_results.iter().map(field_value_to_proto).collect(),
            })
            .collect();

        let mut response = Response::new(CommitResponse {
            write_results: proto_results,
            commit_time: Some(Timestamp {
                seconds: now.timestamp(),
                nanos: now.timestamp_subsec_nanos() as i32,
            }),
        });
        Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
        Ok(response)
    }

    /// firestore-batch-write (Slice 01, US-01, ADR-048 § Decision 2-3):
    /// applies a batch of writes, each in its OWN `begin_transaction`+
    /// `commit_transaction` pair — never once for the whole batch (§ Context
    /// finding 1). Auth/rate-limit/suspension sequence mirrors
    /// `handle_commit`'s own exactly (once per call, unary — no streaming
    /// scaffold). Reused unmodified across every `backend_mode`, including
    /// `agent` (ADR-049) — zero handler-level `backend_mode` branching.
    ///
    /// Once past pre-loop validation (auth/rate-limit/suspension/the 500-cap),
    /// this handler NEVER returns a top-level `Err` for a write-specific
    /// failure — every per-write failure becomes that write's own
    /// `status[i]`/`write_results[i]` entry, and the loop continues
    /// (ADR-048 § Decision Driver 3).
    async fn handle_batch_write(
        &self,
        request: Request<BatchWriteRequest>,
    ) -> Result<Response<BatchWriteResponse>, Status> {
        let req = request.get_ref();
        let project_id_str = Self::extract_project_id(&req.database)?.to_string();
        let api_key = Self::extract_api_key(&request)?;

        let rate_info = match self.rate_limiter.check(&project_id_str).await {
            Ok(info) => info,
            Err(info) => return Err(Self::rate_limit_rejection(&info)),
        };

        let (adapter, status_str, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status_str == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        // AC-01-04: empty batch short-circuits before any per-write
        // machinery, ahead of even the 500-cap check (ADR-048 § Decision 2).
        if req.writes.is_empty() {
            let mut response = Response::new(BatchWriteResponse {
                write_results: vec![],
                status: vec![],
            });
            Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
            return Ok(response);
        }

        // ADR-048 § Decision 2: matches real Firestore's own documented
        // per-call write limit for BatchWrite, mirroring
        // `handle_batch_get_documents`'s own DDD-BGD-14 precedent (a
        // different cap, same "unbounded per-call work" concern). Applied
        // uniformly to every backend_mode (ADR-049) — no branching here.
        if req.writes.len() > 500 {
            return Err(Status::invalid_argument(
                "writes must not exceed 500 per call",
            ));
        }

        let project_id = embyr_core::domain::project::ProjectId::new(&project_id_str)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let verified_identity = self
            .attach_client_identity_if_present(&request, &project_id_str)
            .await;

        // ADR-048 § Decision 3, step 1: translate the WHOLE batch up front,
        // catching every write's own failure — never short-circuiting.
        let translated = Self::translate_writes_catching(
            &self.system_db,
            &adapter,
            &project_id_str,
            verified_identity.as_ref(),
            &req.writes,
        )
        .await;

        let mut write_results = Vec::with_capacity(req.writes.len());
        let mut statuses = Vec::with_capacity(req.writes.len());

        for translation in translated {
            let domain_write = match translation {
                Ok(domain_write) => domain_write,
                Err(status) => {
                    write_results.push(embyr_proto::firestore::WriteResult {
                        update_time: None,
                        transform_results: vec![],
                    });
                    statuses.push(status_to_proto(status));
                    continue;
                }
            };

            // ADR-048 § Decision 3, step 2 / § Context finding 1: a fresh
            // `begin_transaction`+`commit_transaction` pair PER WRITE — never
            // once for the whole batch, since `commit_transaction` is
            // all-or-nothing per call.
            let txn_id = match adapter
                .begin_transaction(&project_id, TransactionOptions::ReadWrite)
                .await
            {
                Ok(id) => id,
                Err(e) => {
                    write_results.push(embyr_proto::firestore::WriteResult {
                        update_time: None,
                        transform_results: vec![],
                    });
                    statuses.push(status_to_proto(core_error_to_status(e)));
                    continue;
                }
            };

            match adapter
                .commit_transaction(&project_id, &txn_id, vec![domain_write])
                .await
            {
                Ok(mut results) => {
                    let wr = results.pop().unwrap_or(embyr_core::domain::document::WriteResult {
                        update_time: (0, 0),
                        create_time: None,
                        transform_results: vec![],
                    });
                    write_results.push(embyr_proto::firestore::WriteResult {
                        update_time: Some(Timestamp {
                            seconds: wr.update_time.0,
                            nanos: wr.update_time.1,
                        }),
                        transform_results: wr.transform_results.iter().map(field_value_to_proto).collect(),
                    });
                    statuses.push(embyr_proto::rpc::Status {
                        code: 0,
                        message: String::new(),
                        details: vec![],
                    });
                }
                Err(e) => {
                    write_results.push(embyr_proto::firestore::WriteResult {
                        update_time: None,
                        transform_results: vec![],
                    });
                    statuses.push(status_to_proto(core_error_to_status(e)));
                }
            }
        }

        let mut response = Response::new(BatchWriteResponse {
            write_results,
            status: statuses,
        });
        Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
        Ok(response)
    }

    async fn handle_rollback(
        &self,
        request: Request<RollbackRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.get_ref();
        let project_id_str = Self::extract_project_id(&req.database)?.to_string();
        let api_key = Self::extract_api_key(&request)?;

        let rate_info = match self.rate_limiter.check(&project_id_str).await {
            Ok(info) => info,
            Err(info) => return Err(Self::rate_limit_rejection(&info)),
        };

        let (adapter, status, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        let project_id = embyr_core::domain::project::ProjectId::new(&project_id_str)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let txn_id = embyr_core::domain::transaction::TransactionId(req.transaction.clone());

        adapter
            .rollback_transaction(&project_id, &txn_id)
            .await
            .map_err(core_error_to_status)?;

        let mut response = Response::new(());
        Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
        Ok(response)
    }

    async fn handle_run_query(
        &self,
        request: Request<RunQueryRequest>,
    ) -> Result<Response<tonic::codegen::BoxStream<RunQueryResponse>>, Status> {
        let req = request.get_ref();

        // Extract project_id from parent: "projects/{pid}/databases/(default)/documents"
        let project_id_str = Self::extract_project_id(&req.parent)?.to_string();
        let api_key = Self::extract_api_key(&request)?;

        let rate_info = match self.rate_limiter.check(&project_id_str).await {
            Ok(info) => info,
            Err(info) => return Err(Self::rate_limit_rejection(&info)),
        };

        let (adapter, status_str, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status_str == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        // security-rules-query-path (ADR-031 § Decision — Composition, step
        // 1): identical call shape to `handle_get_document`'s own
        // `attach_client_identity_if_present` placement — the function
        // itself is unchanged, this is a new consumer of its existing
        // return value.
        let verified_identity = self
            .attach_client_identity_if_present(&request, &project_id_str)
            .await;

        // Extract the structured query from the request
        let sq_proto = match &req.query_type {
            Some(QueryType::StructuredQuery(sq)) => sq,
            None => return Err(Status::invalid_argument("query_type is required")),
        };

        // Parse collection from from[0]
        let collection_id = sq_proto
            .from
            .first()
            .map(|cs| cs.collection_id.clone())
            .unwrap_or_default();
        let all_descendants = sq_proto
            .from
            .first()
            .map(|cs| cs.all_descendants)
            .unwrap_or(false);

        // Translate proto filter → domain QueryFilter
        let filter = sq_proto
            .r#where
            .as_ref()
            .and_then(translate_filter)
            .transpose()
            .map_err(Status::invalid_argument)?;

        // Translate proto order_by → domain OrderBy. Field paths are
        // validated here (SPEC.md Invariant 6) — this is the only place
        // order-by field paths enter the domain from proto; `Listen`'s own
        // order_by is always empty (realtime/listen_handler.rs), so this
        // single guard covers every order-by-carrying entry point.
        let order_by: Vec<OrderBy> = sq_proto
            .order_by
            .iter()
            .filter_map(|o| {
                let field_path = o.field.as_ref()?.field_path.clone();
                let direction = match Direction::try_from(o.direction).unwrap_or(Direction::Unspecified) {
                    Direction::Descending => OrderDirection::Descending,
                    _ => OrderDirection::Ascending,
                };
                Some((field_path, direction))
            })
            .map(|(field_path, direction)| {
                validate_field_path(&field_path)
                    .map(|()| OrderBy { field_path, direction })
                    .map_err(|e| Status::invalid_argument(e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Translate proto limit → domain
        let limit = sq_proto.limit;

        // Translate proto cursor → domain Cursor
        let start_at = sq_proto.start_at.as_ref().and_then(|c| {
            let values: Option<Vec<FieldValue>> = c
                .values
                .iter()
                .map(crate::encoding::firestore_proto::proto_value_to_field_value)
                .collect();
            values.map(|vals| Cursor { values: vals, before: c.before })
        });

        let project_id = embyr_core::domain::project::ProjectId::new(&project_id_str)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let collection = CollectionPath {
            project_id,
            collection_path: collection_id,
        };

        let domain_query = StructuredQuery {
            collection_id: collection.collection_path.clone(),
            all_descendants,
            filter,
            order_by,
            limit,
            offset: if sq_proto.offset > 0 { Some(sq_proto.offset) } else { None },
            start_at,
            end_at: None,
            since_update_time: None,
        };

        // security-rules-collection-group-rules (ADR-032 § Decision —
        // Composition): `all_descendants` is mutually exclusive by
        // construction (a StructuredQuery either targets a single
        // collection instance or the whole group, never both) — an
        // if/else is the structurally correct shape, not two independent
        // checks that could both fire or both be skipped.
        if all_descendants {
            // A WHOLLY NEW, mutually-exclusive arm (Slice 04, US-04). Reads
            // group_access_rules ONLY — never access_rules — the
            // structural (not conventional) mechanism behind AC-17-89/90/91.
            let group_rule_row = self
                .system_db
                .get_group_access_rule(&project_id_str, &collection.collection_path)
                .await
                .map_err(|e| Status::internal(e.to_string()))?;

            let Some(group_rule_row) = group_rule_row else {
                // US-04, Resolution 2 (universal fail-closed default): no
                // group rule defined -> reject outright, regardless of any
                // same-named exact-path rule's existence (AC-17-89). This
                // feature's single highest-consequence arm (designated
                // mutation-testing surface, CLAUDE.md).
                return Err(group_rule_not_defined_rejection());
            };

            let condition =
                embyr_core::access_control::parse_condition(&group_rule_row.condition_source)
                    .map_err(|e| {
                        Status::internal(format!(
                            "stored group access rule failed to re-parse: {e:?}"
                        ))
                    })?;
            let auth_ctx = verified_identity
                .as_ref()
                .map(|v| embyr_core::access_control::AuthContext {
                    uid: v.end_user_id.clone(),
                    // custom-claims (ADR-034): out of THIS slice's scope
                    // (write-path/RunQuery wiring is US-03/US-05's own job)
                    // — empty map keeps this call site compiling against
                    // AuthContext's new field with zero behavior change.
                    claims: std::collections::BTreeMap::new(),
                });

            // SAME check_query_compliance()/query_compliance_rejection()
            // real, non-group enforcement uses (ADR-031) — never a second,
            // independently-maintained shape-compliance path.
            match embyr_core::access_control::check_query_compliance(
                &condition,
                domain_query.filter.as_ref(),
                auth_ctx.as_ref(),
            ) {
                embyr_core::access_control::QueryComplianceOutcome::Admitted => {}
                outcome => return Err(query_compliance_rejection(&outcome)),
            }
        } else {
            // EXISTING ARM (security-rules-query-path, ADR-031), PRESERVED
            // UNCHANGED. Reads access_rules ONLY. `None` -> the `if let`
            // below simply does not execute — the composite-index check and
            // `adapter.run_query()` calls immediately below are reached
            // completely unmodified, the EXACT pre-security-rules-
            // collection-group-rules code path (AC-17-90 regression proof).
            let rule_row = self
                .system_db
                .get_access_rule(&project_id_str, &collection.collection_path)
                .await
                .map_err(|e| Status::internal(e.to_string()))?;

            if let Some(rule_row) = rule_row {
                let condition =
                    embyr_core::access_control::parse_condition(&rule_row.condition_source)
                        .map_err(|e| {
                            Status::internal(format!(
                                "stored access rule failed to re-parse: {e:?}"
                            ))
                        })?;
                let auth_ctx = verified_identity
                    .as_ref()
                    .map(|v| embyr_core::access_control::AuthContext {
                    uid: v.end_user_id.clone(),
                    // custom-claims (ADR-034): out of THIS slice's scope
                    // (write-path/RunQuery wiring is US-03/US-05's own job)
                    // — empty map keeps this call site compiling against
                    // AuthContext's new field with zero behavior change.
                    claims: std::collections::BTreeMap::new(),
                });

                // ADR-031 § OQ-SRQ-03 Resolution: compliance-checking runs
                // strictly BEFORE the composite-index check below — a
                // caller never entitled to query this collection at all
                // must never learn whether it also requires a composite
                // index.
                match embyr_core::access_control::check_query_compliance(
                    &condition,
                    domain_query.filter.as_ref(),
                    auth_ctx.as_ref(),
                ) {
                    embyr_core::access_control::QueryComplianceOutcome::Admitted => {}
                    outcome => return Err(query_compliance_rejection(&outcome)),
                }
            }
        }

        // Composite index check: filter on field X + orderBy field Y (where Y != X)
        // requires a READY composite index in the system DB.
        let requires_index = Self::requires_composite_index(&domain_query);
        if requires_index
            && !self
                .index_manager
                .is_index_ready(&project_id_str, &collection.collection_path)
                .await
        {
            // composite-index-requirement-rules (Slice 03, US-03, AC-CIR-07):
            // names the specific missing index (collection_path + fields)
            // instead of a generic message — reuses the SAME `fields` JSON
            // shape `POST /admin/v1/projects/:project_id/indexes`
            // (firestore-composite-indexes-admin-api) expects, so Alex can
            // copy it directly into a `CreateIndex` call.
            let missing_fields = Self::missing_index_fields(&domain_query);
            let fields_json = serde_json::to_string(&missing_fields)
                .unwrap_or_else(|_| "[]".to_string());
            return Err(Status::failed_precondition(format!(
                "query requires a composite index on collection '{}' with fields {}; create it \
                 via POST /admin/v1/projects/{{project_id}}/indexes",
                collection.collection_path, fields_json
            )));
        }

        let docs = adapter
            .run_query(&collection, &domain_query, None)
            .await
            .map_err(core_error_to_status)?;

        // Build response stream
        let mut responses: Vec<Result<RunQueryResponse, Status>> = docs
            .into_iter()
            .map(|doc| {
                Ok(RunQueryResponse {
                    document: Some(crate::encoding::firestore_proto::document_to_proto(doc)),
                    ..Default::default()
                })
            })
            .collect();

        // Final done=true message
        responses.push(Ok(RunQueryResponse {
            continuation_selector: Some(
                embyr_proto::firestore::run_query_response::ContinuationSelector::Done(true),
            ),
            ..Default::default()
        }));

        let stream: tonic::codegen::BoxStream<RunQueryResponse> =
            Box::pin(tokio_stream::iter(responses));
        let mut response = Response::new(stream);
        Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
        Ok(response)
    }

    /// aggregation-queries Slice 01 (US-01, ADR-038/039/040) — `RunAggregationQuery`
    /// handler. Mirrors `handle_run_query`'s own composition closely: auth,
    /// rate-limit, suspension check, `attach_client_identity_if_present`,
    /// the SAME dual-arm access-rule composition (`check_query_compliance()`
    /// on `all_descendants` via `get_access_rule`/`get_group_access_rule`),
    /// dispatch to the adapter, single-message response.
    ///
    /// Filter-identity invariant (ADR-039 § Decision 2): `domain_query` is
    /// parsed from the proto exactly ONCE and shared by reference into both
    /// `check_query_compliance()` and `adapter.run_aggregation_query()` —
    /// never a second, independent parse of the same proto filter.
    async fn handle_run_aggregation_query(
        &self,
        request: Request<RunAggregationQueryRequest>,
    ) -> Result<Response<tonic::codegen::BoxStream<RunAggregationQueryResponse>>, Status> {
        let req = request.get_ref();

        let project_id_str = Self::extract_project_id(&req.parent)?.to_string();
        let api_key = Self::extract_api_key(&request)?;

        let rate_info = match self.rate_limiter.check(&project_id_str).await {
            Ok(info) => info,
            Err(info) => return Err(Self::rate_limit_rejection(&info)),
        };

        let (adapter, status_str, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status_str == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        let verified_identity = self
            .attach_client_identity_if_present(&request, &project_id_str)
            .await;

        let saq = match &req.query_type {
            Some(AggregationQueryType::StructuredAggregationQuery(saq)) => saq,
            None => return Err(Status::invalid_argument("query_type is required")),
        };

        // v1 restriction (ADR-038): exactly one aggregation per request.
        if saq.aggregations.len() != 1 {
            return Err(Status::unimplemented(
                "multiple aggregations per request are not yet supported",
            ));
        }
        let agg_proto = &saq.aggregations[0];
        let alias = if agg_proto.alias.is_empty() {
            "field_0".to_string()
        } else {
            agg_proto.alias.clone()
        };

        let aggregation_kind = match &agg_proto.operator {
            Some(AggregationOperator::Count(c)) => {
                // v1 restriction (ADR-038): `Count.up_to` is not supported.
                if c.up_to.is_some() {
                    return Err(Status::unimplemented(
                        "count up_to limiting is not yet supported",
                    ));
                }
                AggregationKind::Count
            }
            Some(AggregationOperator::Sum(s)) => {
                let field_path = s
                    .field
                    .as_ref()
                    .map(|f| f.field_path.clone())
                    .ok_or_else(|| Status::invalid_argument("sum aggregation requires a field"))?;
                // Field-path validation wired now (ADR-040) so Slices 03/04
                // don't need to touch this handler again — not exercised by
                // this slice's own COUNT-only scope.
                validate_field_path(&field_path).map_err(|e| Status::invalid_argument(e.to_string()))?;
                AggregationKind::Sum(field_path)
            }
            Some(AggregationOperator::Avg(a)) => {
                let field_path = a
                    .field
                    .as_ref()
                    .map(|f| f.field_path.clone())
                    .ok_or_else(|| Status::invalid_argument("avg aggregation requires a field"))?;
                validate_field_path(&field_path).map_err(|e| Status::invalid_argument(e.to_string()))?;
                AggregationKind::Avg(field_path)
            }
            None => return Err(Status::invalid_argument("aggregation operator is required")),
        };

        let sq_proto = match &saq.query_type {
            Some(StructuredAggQueryType::StructuredQuery(sq)) => sq,
            None => return Err(Status::invalid_argument("structured_query is required")),
        };

        let collection_id = sq_proto
            .from
            .first()
            .map(|cs| cs.collection_id.clone())
            .unwrap_or_default();
        let all_descendants = sq_proto
            .from
            .first()
            .map(|cs| cs.all_descendants)
            .unwrap_or(false);

        let filter = sq_proto
            .r#where
            .as_ref()
            .and_then(translate_filter)
            .transpose()
            .map_err(Status::invalid_argument)?;

        let project_id = embyr_core::domain::project::ProjectId::new(&project_id_str)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let collection = CollectionPath {
            project_id,
            collection_path: collection_id,
        };

        let domain_query = StructuredQuery {
            collection_id: collection.collection_path.clone(),
            all_descendants,
            filter,
            order_by: vec![],
            limit: None,
            offset: None,
            start_at: None,
            end_at: None,
            since_update_time: None,
        };

        // Dual-arm access-rule composition — IDENTICAL to handle_run_query's
        // own (ADR-039 § Decision 1): reuses check_query_compliance()/
        // query_compliance_rejection()/group_rule_not_defined_rejection()
        // byte-for-byte unchanged, at the same composition point (strictly
        // before backend dispatch).
        if all_descendants {
            let group_rule_row = self
                .system_db
                .get_group_access_rule(&project_id_str, &collection.collection_path)
                .await
                .map_err(|e| Status::internal(e.to_string()))?;

            let Some(group_rule_row) = group_rule_row else {
                return Err(group_rule_not_defined_rejection());
            };

            let condition =
                embyr_core::access_control::parse_condition(&group_rule_row.condition_source)
                    .map_err(|e| {
                        Status::internal(format!(
                            "stored group access rule failed to re-parse: {e:?}"
                        ))
                    })?;
            let auth_ctx = verified_identity
                .as_ref()
                .map(|v| embyr_core::access_control::AuthContext {
                    uid: v.end_user_id.clone(),
                    claims: std::collections::BTreeMap::new(),
                });

            match embyr_core::access_control::check_query_compliance(
                &condition,
                domain_query.filter.as_ref(),
                auth_ctx.as_ref(),
            ) {
                embyr_core::access_control::QueryComplianceOutcome::Admitted => {}
                outcome => return Err(query_compliance_rejection(&outcome)),
            }
        } else {
            let rule_row = self
                .system_db
                .get_access_rule(&project_id_str, &collection.collection_path)
                .await
                .map_err(|e| Status::internal(e.to_string()))?;

            if let Some(rule_row) = rule_row {
                let condition =
                    embyr_core::access_control::parse_condition(&rule_row.condition_source)
                        .map_err(|e| {
                            Status::internal(format!(
                                "stored access rule failed to re-parse: {e:?}"
                            ))
                        })?;
                let auth_ctx = verified_identity
                    .as_ref()
                    .map(|v| embyr_core::access_control::AuthContext {
                        uid: v.end_user_id.clone(),
                        claims: std::collections::BTreeMap::new(),
                    });

                match embyr_core::access_control::check_query_compliance(
                    &condition,
                    domain_query.filter.as_ref(),
                    auth_ctx.as_ref(),
                ) {
                    embyr_core::access_control::QueryComplianceOutcome::Admitted => {}
                    outcome => return Err(query_compliance_rejection(&outcome)),
                }
            }
        }

        let agg_query = AggregationQuery {
            query: domain_query,
            aggregation: aggregation_kind,
            alias: alias.clone(),
        };

        let value = adapter
            .run_aggregation_query(&collection, &agg_query, None)
            .await
            .map_err(aggregation_error_to_status)?;

        let value_proto = {
            use embyr_proto::firestore::value::ValueType;
            let vt = match value {
                AggregateValue::Count(n) => ValueType::IntegerValue(n),
                AggregateValue::Sum(d) => ValueType::DoubleValue(d),
                AggregateValue::Avg(Some(d)) => ValueType::DoubleValue(d),
                AggregateValue::Avg(None) => ValueType::NullValue(0),
            };
            embyr_proto::firestore::Value { value_type: Some(vt) }
        };

        let mut aggregate_fields = HashMap::new();
        aggregate_fields.insert(alias, value_proto);

        // Response stream carries exactly one message, then closes — no
        // continuation/done-marker message (ADR-038: `RunAggregationQueryResponse`
        // has no `continuation_selector`, unlike `RunQueryResponse`).
        let response_msg = RunAggregationQueryResponse {
            result: Some(AggregationResult { aggregate_fields }),
            transaction: vec![],
            read_time: None,
        };

        let stream: tonic::codegen::BoxStream<RunAggregationQueryResponse> =
            Box::pin(tokio_stream::iter(vec![Ok(response_msg)]));
        let mut response = Response::new(stream);
        Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
        Ok(response)
    }

    async fn handle_listen(
        &self,
        mut request: Request<tonic::Streaming<ListenRequest>>,
    ) -> Result<Response<tonic::codegen::BoxStream<ListenResponse>>, Status> {
        let api_key = Self::extract_api_key(&request)?;

        // Read first message to get the AddTarget + project_id for auth.
        // `request` itself is kept alive (not yet consumed via `into_inner()`)
        // so its metadata is still available below for
        // `attach_client_identity_if_present` (security-rules-realtime,
        // ADR-033 § Decision — Subscribe-Time Composition).
        let first_msg = request
            .get_mut()
            .next()
            .await
            .ok_or_else(|| Status::invalid_argument("empty listen stream"))?
            .map_err(|e| Status::internal(e.to_string()))?;

        let project_id = extract_project_id_from_listen_request(&first_msg)?;

        let rate_info = match self.rate_limiter.check(&project_id).await {
            Ok(info) => info,
            Err(info) => return Err(Self::rate_limit_rejection(&info)),
        };

        let (adapter, status, dsn) = self.authenticate(&project_id, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        // Ensure a PostgresNotifyListener is running for this project.
        let channel = notify_channel(&project_id);
        {
            let mut listeners = self.active_listeners.lock().await;
            if !listeners.contains_key(&project_id) {
                // We need the pool from the adapter. The adapter is a SharedBackendAdapter
                // (dyn BackendAdapter) so we can't downcast. Instead, build a new pool
                // from the DSN specifically for the listener's fetch queries.
                let pool = sqlx::postgres::PgPoolOptions::new()
                    .max_connections(2)
                    .connect(&dsn)
                    .await
                    .map_err(|e| Status::internal(format!("notify listener pool: {e}")))?;
                let listener = PostgresNotifyListener::start(
                    &dsn,
                    &project_id,
                    Arc::clone(&self.listen_registry),
                    pool,
                )
                .await
                .map_err(|e| Status::internal(e.to_string()))?;
                listeners.insert(project_id.clone(), listener);
            }
        }

        // security-rules-realtime (ADR-033 § Decision — Subscribe-Time
        // Composition, US-03): identical call shape to `handle_run_query`'s
        // own `attach_client_identity_if_present` placement — the function
        // itself is unchanged, this is a new consumer of its existing
        // return value, so `request.auth` is available to
        // `handle_add_target`'s own subscribe-time compliance gate.
        //
        // `attach_client_identity_if_present<T>` only ever reads
        // `request.metadata()` — it never touches the streaming body. A
        // `Request<Streaming<ListenRequest>>` itself is not `Sync` (its
        // body is a `dyn Decoder` trait object), so holding `&request`
        // across this `.await` would make the enclosing future non-`Send`
        // (required by tonic's boxed handler future). A metadata-only
        // `Request<()>` carries the identical headers without that
        // constraint.
        let mut metadata_only_request = Request::new(());
        *metadata_only_request.metadata_mut() = request.metadata().clone();
        let verified_identity = self
            .attach_client_identity_if_present(&metadata_only_request, &project_id)
            .await;

        // Extract resume token from AddTarget if present.
        let resume_token: Option<Vec<u8>> = match &first_msg.target_change {
            Some(embyr_proto::firestore::listen_request::TargetChange::AddTarget(t)) => {
                use embyr_proto::firestore::target::ResumeType;
                match &t.resume_type {
                    Some(ResumeType::ResumeToken(bytes)) if !bytes.is_empty() => {
                        Some(bytes.clone())
                    }
                    _ => None,
                }
            }
            _ => None,
        };

        // Channel capacity 64 — matching the subscriber registry capacity.
        // When a slow consumer stops draining, try_send fails at 64 buffered
        // events → handler sends TargetChange(RESET).
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<ListenResponse, Status>>(64);
        let keepalive = self.keepalive_interval;
        let registry = Arc::clone(&self.listen_registry);
        let system_db = Arc::clone(&self.system_db);

        // `request`'s metadata is no longer needed beyond this point —
        // consume it into the owned `Streaming` body now, immediately
        // before the spawn that moves it in.
        let mut in_stream = request.into_inner();

        tokio::spawn(async move {
            if let Err(status) = crate::realtime::listen_handler::handle_add_target(
                &first_msg,
                &adapter,
                &system_db,
                verified_identity,
                &tx,
                keepalive,
                registry,
                &channel,
                resume_token,
            )
            .await
            {
                let _ = tx.send(Err(status)).await;
            }
            // Drain remaining client messages (RemoveTarget etc.) for future steps.
            while (in_stream.next().await).is_some() {}
        });

        let stream: tonic::codegen::BoxStream<ListenResponse> = Box::pin(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        );
        let mut response = Response::new(stream);
        Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
        Ok(response)
    }

    /// firestore-write-streaming (Slice 01, ADR-046 § Decision 2): mirrors
    /// `handle_listen`'s own scaffold exactly — peek the handshake message
    /// while `request`'s metadata is still available, reject a non-empty
    /// handshake (AC-01-04), run rate-limit/authenticate/identity exactly
    /// once (AC-01-05/06), then hand the owned stream + response sender to
    /// `write_stream::run_write_session`, which owns all further session
    /// state (stream_id/stream_token generation, the receive-and-reply loop).
    async fn handle_write(
        &self,
        mut request: Request<tonic::Streaming<WriteRequest>>,
    ) -> Result<Response<tonic::codegen::BoxStream<WriteResponse>>, Status> {
        let api_key = Self::extract_api_key(&request)?;

        let first_msg = request
            .get_mut()
            .next()
            .await
            .ok_or_else(|| Status::invalid_argument("empty write stream"))?
            .map_err(|e| Status::internal(e.to_string()))?;

        // AC-01-04: the handshake message must be empty (no writes, no
        // stream_id) — rejected before any write is attempted, before auth.
        if !first_msg.writes.is_empty() || !first_msg.stream_id.is_empty() {
            return Err(Status::invalid_argument(
                "handshake WriteRequest must have empty writes and empty stream_id",
            ));
        }

        let project_id_str = Self::extract_project_id(&first_msg.database)?.to_string();

        let rate_info = match self.rate_limiter.check(&project_id_str).await {
            Ok(info) => info,
            Err(info) => return Err(Self::rate_limit_rejection(&info)),
        };

        let (adapter, status, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        // Metadata-only request, mirroring `handle_listen`'s own reasoning:
        // `Request<Streaming<WriteRequest>>` is not `Sync` (its body is a
        // `dyn Decoder`), so holding `&request` across this `.await` would
        // make the handler future non-`Send`.
        let mut metadata_only_request = Request::new(());
        *metadata_only_request.metadata_mut() = request.metadata().clone();
        let verified_identity = self
            .attach_client_identity_if_present(&metadata_only_request, &project_id_str)
            .await;

        let system_db = Arc::clone(&self.system_db);
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<WriteResponse, Status>>(16);

        let in_stream = request.into_inner();

        tokio::spawn(crate::grpc::write_stream::run_write_session(
            in_stream,
            tx,
            adapter,
            system_db,
            project_id_str,
            verified_identity,
        ));

        let stream: tonic::codegen::BoxStream<WriteResponse> = Box::pin(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        );
        let mut response = Response::new(stream);
        Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
        Ok(response)
    }
}

// ---------------------------------------------------------------------------
// Firestore trait impl — thin wrappers that add OBS-02 / OBS-03 instrumentation.
// ---------------------------------------------------------------------------

#[tonic::async_trait]
impl Firestore for FirestoreService {
    async fn get_document(
        &self,
        request: Request<GetDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        let obs_start = std::time::Instant::now();
        let result = self.handle_get_document(request).await;
        obs_helpers::record_grpc_call(obs_helpers::METHOD_GET_DOCUMENT, &result, obs_start);
        result
    }

    async fn create_document(
        &self,
        request: Request<CreateDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        let obs_start = std::time::Instant::now();
        let result = self.handle_create_document(request).await;
        obs_helpers::record_grpc_call(obs_helpers::METHOD_CREATE_DOCUMENT, &result, obs_start);
        result
    }

    async fn update_document(
        &self,
        request: Request<UpdateDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        let obs_start = std::time::Instant::now();
        let result = self.handle_update_document(request).await;
        obs_helpers::record_grpc_call(obs_helpers::METHOD_UPDATE_DOCUMENT, &result, obs_start);
        result
    }

    async fn delete_document(
        &self,
        request: Request<DeleteDocumentRequest>,
    ) -> Result<Response<()>, Status> {
        let obs_start = std::time::Instant::now();
        let result = self.handle_delete_document(request).await;
        obs_helpers::record_grpc_call(obs_helpers::METHOD_DELETE_DOCUMENT, &result, obs_start);
        result
    }

    async fn list_documents(
        &self,
        request: Request<ListDocumentsRequest>,
    ) -> Result<Response<ListDocumentsResponse>, Status> {
        let obs_start = std::time::Instant::now();
        let result = self.handle_list_documents(request).await;
        obs_helpers::record_grpc_call(obs_helpers::METHOD_LIST_DOCUMENTS, &result, obs_start);
        result
    }

    async fn list_collection_ids(
        &self,
        request: Request<ListCollectionIdsRequest>,
    ) -> Result<Response<ListCollectionIdsResponse>, Status> {
        let obs_start = std::time::Instant::now();
        let result = self.handle_list_collection_ids(request).await;
        obs_helpers::record_grpc_call(obs_helpers::METHOD_LIST_COLLECTION_IDS, &result, obs_start);
        result
    }

    type BatchGetDocumentsStream = tonic::codegen::BoxStream<BatchGetDocumentsResponse>;

    async fn batch_get_documents(
        &self,
        request: Request<BatchGetDocumentsRequest>,
    ) -> Result<Response<Self::BatchGetDocumentsStream>, Status> {
        let obs_start = std::time::Instant::now();
        let result = self.handle_batch_get_documents(request).await;
        obs_helpers::record_grpc_call(
            obs_helpers::METHOD_BATCH_GET_DOCUMENTS,
            &result,
            obs_start,
        );
        result
    }

    async fn begin_transaction(
        &self,
        request: Request<BeginTransactionRequest>,
    ) -> Result<Response<BeginTransactionResponse>, Status> {
        let obs_start = std::time::Instant::now();
        let result = self.handle_begin_transaction(request).await;
        obs_helpers::record_grpc_call(
            obs_helpers::METHOD_BEGIN_TRANSACTION,
            &result,
            obs_start,
        );
        result
    }

    async fn commit(
        &self,
        request: Request<CommitRequest>,
    ) -> Result<Response<CommitResponse>, Status> {
        let obs_start = std::time::Instant::now();
        let result = self.handle_commit(request).await;
        obs_helpers::record_grpc_call(obs_helpers::METHOD_COMMIT, &result, obs_start);
        result
    }

    async fn batch_write(
        &self,
        request: Request<BatchWriteRequest>,
    ) -> Result<Response<BatchWriteResponse>, Status> {
        let obs_start = std::time::Instant::now();
        let result = self.handle_batch_write(request).await;
        obs_helpers::record_grpc_call(obs_helpers::METHOD_BATCH_WRITE, &result, obs_start);
        result
    }

    async fn rollback(
        &self,
        request: Request<RollbackRequest>,
    ) -> Result<Response<()>, Status> {
        let obs_start = std::time::Instant::now();
        let result = self.handle_rollback(request).await;
        obs_helpers::record_grpc_call(obs_helpers::METHOD_ROLLBACK, &result, obs_start);
        result
    }

    type RunQueryStream = tonic::codegen::BoxStream<RunQueryResponse>;

    async fn run_query(
        &self,
        request: Request<RunQueryRequest>,
    ) -> Result<Response<Self::RunQueryStream>, Status> {
        let obs_start = std::time::Instant::now();
        let result = self.handle_run_query(request).await;
        obs_helpers::record_grpc_call(obs_helpers::METHOD_RUN_QUERY, &result, obs_start);
        result
    }

    type RunAggregationQueryStream = tonic::codegen::BoxStream<RunAggregationQueryResponse>;

    async fn run_aggregation_query(
        &self,
        request: Request<RunAggregationQueryRequest>,
    ) -> Result<Response<Self::RunAggregationQueryStream>, Status> {
        let obs_start = std::time::Instant::now();
        let result = self.handle_run_aggregation_query(request).await;
        obs_helpers::record_grpc_call(
            obs_helpers::METHOD_RUN_AGGREGATION_QUERY,
            &result,
            obs_start,
        );
        result
    }

    type ListenStream = tonic::codegen::BoxStream<ListenResponse>;

    async fn listen(
        &self,
        request: Request<tonic::Streaming<ListenRequest>>,
    ) -> Result<Response<Self::ListenStream>, Status> {
        let obs_start = std::time::Instant::now();
        let result = self.handle_listen(request).await;
        obs_helpers::record_grpc_call(obs_helpers::METHOD_LISTEN, &result, obs_start);
        result
    }

    type WriteStream = tonic::codegen::BoxStream<WriteResponse>;

    async fn write(
        &self,
        request: Request<tonic::Streaming<WriteRequest>>,
    ) -> Result<Response<Self::WriteStream>, Status> {
        let obs_start = std::time::Instant::now();
        let result = self.handle_write(request).await;
        obs_helpers::record_grpc_call(obs_helpers::METHOD_WRITE, &result, obs_start);
        result
    }
}

/// Extract `project_id` from the `database` field of a `ListenRequest`.
///
/// Format: `projects/{pid}/databases/(default)`
fn extract_project_id_from_listen_request(msg: &ListenRequest) -> Result<String, Status> {
    let db = &msg.database;
    let mut parts = db.splitn(5, '/');
    match (parts.next(), parts.next()) {
        (Some("projects"), Some(pid)) if !pid.is_empty() => Ok(pid.to_string()),
        _ => Err(Status::invalid_argument(format!(
            "invalid database path in ListenRequest: {db}"
        ))),
    }
}

/// Rejection response for a non-compliant `RunQuery` (security-rules-query-path,
/// ADR-031), mirroring `handle_get_document`'s own `Status::permission_denied
/// ("access denied by rule")` precedent. Slice 01 scope: a minimal,
/// genuinely-observable "rejected, not executed" response — the exact
/// reason-code/message-wording taxonomy (AC-17-56/68) is a later slice's
/// job, this slice only needs SOME observable rejection distinguishable from
/// success (AC-17-51's own test asserts on the gRPC status code).
/// security-rules-query-path (Slice 02, ADR-031 § Decision — Rejection
/// Response Shape): the message now embeds a stable `[REASON_CODE]` token
/// per unsatisfied conjunct (`UnsatisfiedConjunct::reason_code()`) plus,
/// for `OwnershipFilterMissing`, the specific field name — so the
/// rejection names the specific missing constraint (AC-17-56) instead of
/// Slice 01's placeholder conjunct COUNT. Slice 01's own gRPC status CODE
/// choice (`Status::permission_denied`) is unchanged — distinguishability
/// from `authenticate()`'s `Status::unauthenticated` and the
/// composite-index check's `Status::failed_precondition` was already true
/// by status code alone; this only strengthens distinguishability WITHIN
/// the `PermissionDenied` family itself.
pub(crate) fn query_compliance_rejection(
    outcome: &embyr_core::access_control::QueryComplianceOutcome,
) -> Status {
    use embyr_core::access_control::{QueryComplianceOutcome, UnsatisfiedConjunct};
    let message = match outcome {
        QueryComplianceOutcome::RejectedUnsupportedRuleShape => {
            "query rejected [UNSUPPORTED_RULE_SHAPE]: this collection's access rule is not a \
             shape supported for query enforcement"
                .to_string()
        }
        QueryComplianceOutcome::Rejected { unsatisfied_conjuncts } => {
            let reasons: Vec<String> = unsatisfied_conjuncts
                .iter()
                .map(|c| match c {
                    UnsatisfiedConjunct::OwnershipFilterMissing { field_path } => format!(
                        "[{}] missing required equality filter on '{field_path}' bound to the \
                         caller's own identity",
                        c.reason_code()
                    ),
                    // Slice 03 (ADR-031): `DenyAll`/`AuthRequired` carry no
                    // extra data — the bare reason-code token is enough for
                    // distinguishability (AC-17-56's own convention).
                    other => format!("[{}]", other.reason_code()),
                })
                .collect();
            format!("query rejected by access rule: {}", reasons.join("; "))
        }
        QueryComplianceOutcome::Admitted => unreachable!("Admitted never reaches this function"),
    };
    Status::permission_denied(message)
}

/// security-rules-collection-group-rules (ADR-032 § Decision — Composition,
/// "GROUP_RULE_NOT_DEFINED is not a QueryComplianceOutcome variant"): the
/// US-04 "no group rule defined" default rejection. `check_query_compliance`
/// only ever receives an ALREADY-PARSED `Condition` — it has no way to
/// represent "there was no rule at all" — so this decision is made entirely
/// in `handle_run_query`'s own composition, before `parse_condition`/
/// `check_query_compliance` are ever reached, mirroring how
/// `get_access_rule`/`get_write_access_rule`'s own `None` short-circuits
/// already work (ADR-029), just with the opposite default. The bracketed
/// `[GROUP_RULE_NOT_DEFINED]` token is distinguishable from
/// `UNSUPPORTED_RULE_SHAPE`/`OWNERSHIP_FILTER_MISSING`/`AUTH_REQUIRED`/
/// `RULE_DENIES_ALL` (AC-17-92).
fn group_rule_not_defined_rejection() -> Status {
    Status::permission_denied(
        "query rejected [GROUP_RULE_NOT_DEFINED]: no collection-group rule \
         is defined for this collection id",
    )
}

/// Translate a proto `Filter` to a domain `QueryFilter`.
///
/// security-rules-realtime (ADR-033 § Decision — Subscribe-Time Composition,
/// US-02): widened from private `fn` to `pub(crate) fn` — visibility-only,
/// zero behavior change — so `realtime::listen_handler::handle_add_target`
/// can call the SAME function `handle_run_query` already uses to build
/// `domain_query.filter`, instead of a second, independently-maintained
/// filter-translation path.
pub(crate) fn translate_filter(
    f: &embyr_proto::firestore::structured_query::Filter,
) -> Option<Result<QueryFilter, String>> {
    match f.filter_type.as_ref()? {
        FilterType::FieldFilter(ff) => {
            let field_path = ff.field.as_ref()?.field_path.clone();
            if let Err(e) = validate_field_path(&field_path) {
                return Some(Err(e.to_string()));
            }
            let op = translate_field_op(FieldOp::try_from(ff.op).ok()?)?;
            let value =
                crate::encoding::firestore_proto::proto_value_to_field_value(ff.value.as_ref()?)?;
            // firestore-range-operator-value-type-support (Slice 02, US-02,
            // AC-RNG-05/06): a range comparison against an Array or Map
            // value has no real Firestore-accurate ordering to implement
            // (live-verified: real Firestore itself does not support range
            // queries on Array fields at all; Map's own support is
            // unconfirmed, treated conservatively) — rejected here, at
            // proto-translation time, via the SAME `Result<_, String>` ->
            // `Status::invalid_argument` mechanism `CompositeOp::
            // Unspecified` already uses below, rather than reaching
            // `push_scalar_comparison`'s own defensive panic.
            if matches!(
                op,
                FilterOp::LessThan | FilterOp::LessThanOrEqual | FilterOp::GreaterThan | FilterOp::GreaterThanOrEqual
            ) && matches!(value, FieldValue::Array(_) | FieldValue::Map(_))
            {
                let kind = if matches!(value, FieldValue::Array(_)) { "array" } else { "map" };
                return Some(Err(format!("range comparison operators are not supported on {kind} values")));
            }
            // firestore-malformed-filter-shape-validation (Slice 01, US-01,
            // AC-MFS-01/02): no real Firestore SDK's own query-builder API
            // can produce a non-Array value for `in`/`not-in`/`array
            // -contains-any` — only a caller who has already hand-crafted a
            // malformed raw gRPC request could trigger this. Rejected here
            // for operational clarity (a clean, named error in Sam's own
            // logs/traces instead of a raw panic), NOT because this is a
            // reachable real-client crash risk — the crash-elimination arc
            // this follows is already fully closed.
            if matches!(op, FilterOp::In | FilterOp::NotIn | FilterOp::ArrayContainsAny)
                && !matches!(value, FieldValue::Array(_))
            {
                let op_name = match op {
                    FilterOp::In => "in",
                    FilterOp::NotIn => "not-in",
                    _ => "array-contains-any",
                };
                return Some(Err(format!("{op_name} requires an array value")));
            }
            // firestore-malformed-filter-shape-validation (Slice 01, US-01,
            // AC-MFS-03): no real Firestore SDK exposes a way to pass
            // `null` to a range-comparison method — same reasoning as
            // above.
            if matches!(
                op,
                FilterOp::LessThan | FilterOp::LessThanOrEqual | FilterOp::GreaterThan | FilterOp::GreaterThanOrEqual
            ) && matches!(value, FieldValue::Null)
            {
                return Some(Err("range comparison operators do not support null values".to_string()));
            }
            Some(Ok(QueryFilter::Field(FieldFilter { field_path, op, value })))
        }
        FilterType::CompositeFilter(cf) => {
            match CompositeOp::try_from(cf.op).unwrap_or(CompositeOp::Unspecified) {
                CompositeOp::And | CompositeOp::Unspecified => {
                    let mut filters = Vec::new();
                    for sub in &cf.filters {
                        match translate_filter(sub) {
                            Some(Ok(qf)) => filters.push(qf),
                            Some(Err(e)) => return Some(Err(e)),
                            None => {}
                        }
                    }
                    Some(Ok(QueryFilter::Composite(filters)))
                }
                _ => Some(Err("unsupported composite operator".into())),
            }
        }
        FilterType::UnaryFilter(uf) => {
            let field_path = uf
                .operand_type
                .as_ref()
                .map(|op| match op {
                    embyr_proto::firestore::structured_query::unary_filter::OperandType::Field(
                        fr,
                    ) => fr.field_path.clone(),
                })?;
            if let Err(e) = validate_field_path(&field_path) {
                return Some(Err(e.to_string()));
            }
            let op = match UnaryOp::try_from(uf.op).ok()? {
                UnaryOp::IsNan => FilterOp::IsNan,
                UnaryOp::IsNotNan => FilterOp::IsNotNan,
                _ => return Some(Err(format!("unsupported unary filter op: {}", uf.op))),
            };
            // IS_NAN and IS_NOT_NAN use no value; provide a sentinel Null value.
            Some(Ok(QueryFilter::Field(FieldFilter {
                field_path,
                op,
                value: FieldValue::Null,
            })))
        }
    }
}

/// Translate a proto `FieldFilter.Operator` to a domain `FilterOp`.
fn translate_field_op(op: FieldOp) -> Option<FilterOp> {
    match op {
        FieldOp::LessThan => Some(FilterOp::LessThan),
        FieldOp::LessThanOrEqual => Some(FilterOp::LessThanOrEqual),
        FieldOp::GreaterThan => Some(FilterOp::GreaterThan),
        FieldOp::GreaterThanOrEqual => Some(FilterOp::GreaterThanOrEqual),
        FieldOp::Equal => Some(FilterOp::Equal),
        FieldOp::NotEqual => Some(FilterOp::NotEqual),
        FieldOp::ArrayContains => Some(FilterOp::ArrayContains),
        FieldOp::In => Some(FilterOp::In),
        FieldOp::ArrayContainsAny => Some(FilterOp::ArrayContainsAny),
        FieldOp::NotIn => Some(FilterOp::NotIn),
        FieldOp::Unspecified => None,
    }
}

/// aggregation-queries (ADR-041 § Decision 2): a LOCAL error-mapping
/// function used ONLY by `handle_run_aggregation_query`, distinct from the
/// shared `core_error_to_status` below. `FailedPrecondition` carries a
/// different client-facing meaning for THIS one RPC (SPEC.md's documented
/// `Unimplemented` for "this backend/operator combination isn't
/// implemented") than it does for every other handler in this file (e.g.
/// `RunQuery`'s composite-index rejection, still `failed_precondition`
/// there, unaffected — reused via the `other` fallthrough arm).
fn aggregation_error_to_status(e: CoreError) -> Status {
    match e {
        CoreError::FailedPrecondition(msg) => Status::unimplemented(msg),
        other => core_error_to_status(other),
    }
}

pub(crate) fn core_error_to_status(e: CoreError) -> Status {
    match e {
        CoreError::DocumentNotFound(_) => Status::not_found(e.to_string()),
        CoreError::AlreadyExists(_) => Status::already_exists(e.to_string()),
        CoreError::Unauthenticated => Status::unauthenticated(e.to_string()),
        CoreError::PermissionDenied(_) => Status::permission_denied(e.to_string()),
        CoreError::InvalidArgument(_) => Status::invalid_argument(e.to_string()),
        CoreError::OccConflict => Status::aborted(e.to_string()),
        CoreError::TransactionAborted => Status::aborted(e.to_string()),
        CoreError::TransactionNotFound => Status::not_found(e.to_string()),
        CoreError::ResourceExhausted(_) => Status::resource_exhausted(e.to_string()),
        CoreError::FailedPrecondition(_) => Status::failed_precondition(e.to_string()),
        _ => Status::internal(e.to_string()),
    }
}

/// firestore-batch-write (Slice 01, ADR-048 § Decision 5): converts a
/// `tonic::Status` to the wire type `BatchWriteResponse.status[i]` needs.
/// `code: 0` (`google.rpc.Code.OK`) is used for a successful write —
/// `docs/SPEC.md`'s own "`status[i] = null`" wording describes the SDK-level
/// projection after decode, not the wire encoding, which cannot represent a
/// sparse/absent entry in a `repeated message` field. First real producer of
/// a populated `embyr_proto::rpc::Status` in this codebase — every other
/// declared call site (`TargetChange.cause`) is never assigned.
fn status_to_proto(status: Status) -> embyr_proto::rpc::Status {
    embyr_proto::rpc::Status {
        code: status.code() as i32,
        message: status.message().to_string(),
        details: vec![],
    }
}

#[cfg(test)]
mod client_identity_extension_tests {
    //! client-auth (ADR-026 step 4) — pure, IO-free unit coverage for the
    //! metadata-extraction routing logic. Both this extractor and the
    //! verification computation downstream
    //! (`embyr_core::client_identity::verify_client_identity_token`, tested
    //! at layer 1 in `embyr-core`) are fully implemented (GREEN).
    //!
    //! AC-16-08(c)'s structural-unreachability claim starts here: proving
    //! the extraction function itself correctly distinguishes "header
    //! absent" from "header present" is the pure-function half of that
    //! proof — an absent header short-circuits via `?` in
    //! `attach_client_identity_if_present` before
    //! `embyr_core::client_identity` is ever called; the acceptance-level
    //! half (a real getDoc call succeeding with no client-identity header)
    //! lives in
    //! tests/client_auth/acceptance/ca02_signin_and_reject_invalid_tokens.rs.
    use super::FirestoreService;
    use tonic::Request;

    #[test]
    fn extract_client_identity_token_returns_none_when_header_absent() {
        let request = Request::new(());
        assert_eq!(FirestoreService::extract_client_identity_token(&request), None);
    }

    #[test]
    fn extract_client_identity_token_strips_bearer_prefix_when_present() {
        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("x-embyr-client-identity", "Bearer abc.def.ghi".parse().unwrap());
        assert_eq!(
            FirestoreService::extract_client_identity_token(&request),
            Some("abc.def.ghi".to_string())
        );
    }

    #[test]
    fn extract_client_identity_token_ignores_the_unrelated_authorization_header() {
        // Regression guard for ADR-026's "existing authorization metadata
        // key is untouched" claim — the new extractor must never read it.
        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("authorization", "Bearer some-api-key".parse().unwrap());
        assert_eq!(FirestoreService::extract_client_identity_token(&request), None);
    }
}

#[cfg(test)]
mod composite_index_requirement_tests {
    //! composite-index-requirement-rules (Slice 01, US-01) — pure, IO-free
    //! unit coverage for `requires_composite_index`. Mirrors `client_
    //! identity_extension_tests`'s own established pattern in this file
    //! (`use super::FirestoreService;`).
    use super::FirestoreService;
    use crate::admin::handlers::composite_indexes::{IndexFieldOrder, IndexFieldSpec};
    use embyr_core::domain::query::{
        FieldFilter, FilterOp, OrderBy, OrderDirection, QueryFilter, StructuredQuery,
    };

    fn base_query() -> StructuredQuery {
        StructuredQuery {
            collection_id: "products".to_string(),
            all_descendants: false,
            filter: None,
            order_by: vec![],
            limit: None,
            offset: None,
            start_at: None,
            end_at: None,
            since_update_time: None,
        }
    }

    fn order_by(field: &str) -> OrderBy {
        OrderBy { field_path: field.to_string(), direction: OrderDirection::Ascending }
    }

    fn equal_filter(field: &str) -> QueryFilter {
        QueryFilter::Field(FieldFilter {
            field_path: field.to_string(),
            op: FilterOp::Equal,
            value: embyr_core::domain::field_value::FieldValue::String("x".to_string()),
        })
    }

    /// AC-CIR-01: 2+ orderBy fields with NO filter at all still requires a
    /// composite index — real Firestore's own "regardless of filters" rule.
    #[test]
    fn two_order_by_fields_with_no_filter_requires_composite_index() {
        let mut q = base_query();
        q.order_by = vec![order_by("category"), order_by("score")];
        assert!(FirestoreService::requires_composite_index(&q));
    }

    /// AC-CIR-02: 2+ orderBy fields where EVERY orderBy field is already a
    /// filtered field STILL requires a composite index — proving the fix is
    /// not merely a field-membership widening of the old single-field check.
    #[test]
    fn two_order_by_fields_that_are_both_already_filtered_still_requires_composite_index() {
        let mut q = base_query();
        q.filter = Some(QueryFilter::Composite(vec![
            equal_filter("category"),
            equal_filter("score"),
        ]));
        q.order_by = vec![order_by("category"), order_by("score")];
        assert!(FirestoreService::requires_composite_index(&q));
    }

    /// AC-CIR-03 (regression guard): the pre-existing single-orderBy-field,
    /// different-field-from-filter shape (`category==`/`score`-orderBy, the
    /// codebase's own original evidenced example) is UNCHANGED — still
    /// requires composite (live-verified Firestore-accurate, § Resolution 2).
    #[test]
    fn single_order_by_field_different_from_an_equality_filter_still_requires_composite_index() {
        let mut q = base_query();
        q.filter = Some(equal_filter("category"));
        q.order_by = vec![order_by("score")];
        assert!(FirestoreService::requires_composite_index(&q));
    }

    /// AC-CIR-03 (regression guard, converse): a single orderBy field that
    /// MATCHES its own equality filter's field needs no composite index —
    /// unchanged.
    #[test]
    fn single_order_by_field_matching_the_filtered_field_does_not_require_composite_index() {
        let mut q = base_query();
        q.filter = Some(equal_filter("category"));
        q.order_by = vec![order_by("category")];
        assert!(!FirestoreService::requires_composite_index(&q));
    }

    /// A single orderBy field with NO filter at all needs no composite index
    /// — single-field index sufficient, unchanged.
    #[test]
    fn single_order_by_field_with_no_filter_does_not_require_composite_index() {
        let mut q = base_query();
        q.order_by = vec![order_by("score")];
        assert!(!FirestoreService::requires_composite_index(&q));
    }

    /// No orderBy at all, any filter shape: never requires composite via
    /// this rule family (the filter-only IN+range rule is Slice 02's own
    /// concern, not exercised here).
    #[test]
    fn no_order_by_at_all_does_not_require_composite_index() {
        let mut q = base_query();
        q.filter = Some(equal_filter("category"));
        assert!(!FirestoreService::requires_composite_index(&q));
    }

    fn in_filter(field: &str) -> QueryFilter {
        QueryFilter::Field(FieldFilter {
            field_path: field.to_string(),
            op: FilterOp::In,
            value: embyr_core::domain::field_value::FieldValue::String("x".to_string()),
        })
    }

    fn range_filter(field: &str, op: FilterOp) -> QueryFilter {
        QueryFilter::Field(FieldFilter {
            field_path: field.to_string(),
            op,
            value: embyr_core::domain::field_value::FieldValue::Integer(1),
        })
    }

    /// AC-CIR-04: a filter-only (no orderBy) `IN` + range-on-a-different
    /// -field query requires a composite index — the top-level `order_by.
    /// is_empty()` short-circuit must not swallow this.
    #[test]
    fn in_filter_plus_a_range_filter_on_a_different_field_requires_composite_index_with_no_order_by() {
        let mut q = base_query();
        q.filter = Some(QueryFilter::Composite(vec![
            in_filter("category"),
            range_filter("population", FilterOp::GreaterThan),
        ]));
        assert!(FirestoreService::requires_composite_index(&q));
    }

    /// AC-CIR-05 (false-positive regression guard): `IN` + a SEPARATE
    /// EQUALITY filter on a different field (compound-equality-only) does
    /// NOT require a composite index — live-verified real-Firestore
    /// behavior, proving Slice 02 doesn't over-widen the gate.
    #[test]
    fn in_filter_plus_a_separate_equality_filter_does_not_require_composite_index() {
        let mut q = base_query();
        q.filter = Some(QueryFilter::Composite(vec![
            in_filter("category"),
            equal_filter("status"),
        ]));
        assert!(!FirestoreService::requires_composite_index(&q));
    }

    /// AC-CIR-06 (regression guard): a lone `array-contains-any` (no other
    /// filter, no orderBy) does not require a composite index — unchanged.
    #[test]
    fn lone_array_contains_any_does_not_require_composite_index() {
        let mut q = base_query();
        q.filter = Some(QueryFilter::Field(FieldFilter {
            field_path: "tags".to_string(),
            op: FilterOp::ArrayContainsAny,
            value: embyr_core::domain::field_value::FieldValue::String("x".to_string()),
        }));
        assert!(!FirestoreService::requires_composite_index(&q));
    }

    /// AC-CIR-06 (regression guard): a lone `not-in` (no other filter, no
    /// orderBy) does not require a composite index — unchanged.
    #[test]
    fn lone_not_in_does_not_require_composite_index() {
        let mut q = base_query();
        q.filter = Some(QueryFilter::Field(FieldFilter {
            field_path: "category".to_string(),
            op: FilterOp::NotIn,
            value: embyr_core::domain::field_value::FieldValue::String("x".to_string()),
        }));
        assert!(!FirestoreService::requires_composite_index(&q));
    }

    /// `IN` on a field, with a RANGE filter on the SAME field (not a
    /// different one) — this is a degenerate/unusual shape (real Firestore
    /// itself disallows combining `in` with a range on the exact same
    /// field), but this function must not false-positive on it: the "range
    /// on a DIFFERENT field" check must correctly exclude the IN field
    /// itself.
    #[test]
    fn in_filter_plus_a_range_filter_on_the_same_field_does_not_trigger_the_in_plus_range_rule() {
        let mut q = base_query();
        q.filter = Some(QueryFilter::Composite(vec![
            in_filter("category"),
            range_filter("category", FilterOp::GreaterThan),
        ]));
        assert!(!FirestoreService::requires_composite_index(&q));
    }

    fn spec(field: &str, order: IndexFieldOrder) -> IndexFieldSpec {
        IndexFieldSpec { field: field.to_string(), order }
    }

    /// AC-CIR-07 (multi-orderBy shape): filtered fields first (Asc), then
    /// orderBy fields not already included, in their own declared direction.
    #[test]
    fn missing_index_fields_for_multi_order_by_lists_both_fields_in_their_own_directions() {
        let mut q = base_query();
        q.order_by = vec![
            OrderBy { field_path: "category".to_string(), direction: OrderDirection::Ascending },
            OrderBy { field_path: "score".to_string(), direction: OrderDirection::Descending },
        ];
        let fields = FirestoreService::missing_index_fields(&q);
        assert_eq!(
            fields,
            vec![spec("category", IndexFieldOrder::Asc), spec("score", IndexFieldOrder::Desc)]
        );
    }

    /// AC-CIR-07 (equality + different-orderBy shape): the filtered field
    /// comes first as Asc, then the orderBy field in its own direction.
    #[test]
    fn missing_index_fields_for_equality_plus_different_order_by_lists_filter_then_order_by() {
        let mut q = base_query();
        q.filter = Some(equal_filter("category"));
        q.order_by = vec![OrderBy { field_path: "score".to_string(), direction: OrderDirection::Descending }];
        let fields = FirestoreService::missing_index_fields(&q);
        assert_eq!(
            fields,
            vec![spec("category", IndexFieldOrder::Asc), spec("score", IndexFieldOrder::Desc)]
        );
    }

    /// AC-CIR-07 (IN+range shape, no orderBy): both filtered fields listed
    /// as Asc, no orderBy fields to append.
    #[test]
    fn missing_index_fields_for_in_plus_range_lists_both_filtered_fields() {
        let mut q = base_query();
        q.filter = Some(QueryFilter::Composite(vec![
            in_filter("category"),
            range_filter("population", FilterOp::GreaterThan),
        ]));
        let fields = FirestoreService::missing_index_fields(&q);
        assert_eq!(
            fields,
            vec![spec("category", IndexFieldOrder::Asc), spec("population", IndexFieldOrder::Asc)]
        );
    }
}

#[cfg(test)]
mod range_operator_value_type_tests {
    //! firestore-range-operator-value-type-support (Slice 02, US-02) —
    //! pure, IO-free unit coverage for `translate_filter`'s own new
    //! Array/Map + range-operator rejection.
    use super::translate_filter;
    use embyr_proto::firestore::{
        structured_query::{
            field_filter::Operator as FieldOp, filter::FilterType, FieldFilter, FieldReference,
            Filter,
        },
        value::ValueType,
        ArrayValue, Value,
    };

    fn field_filter_proto(field: &str, op: FieldOp, value: Value) -> Filter {
        Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(FieldReference { field_path: field.to_string() }),
                op: op as i32,
                value: Some(value),
            })),
        }
    }

    fn array_value(values: Vec<Value>) -> Value {
        Value { value_type: Some(ValueType::ArrayValue(ArrayValue { values })) }
    }

    fn map_value(fields: std::collections::HashMap<String, Value>) -> Value {
        Value {
            value_type: Some(ValueType::MapValue(embyr_proto::firestore::MapValue { fields })),
        }
    }

    fn string_value(s: &str) -> Value {
        Value { value_type: Some(ValueType::StringValue(s.to_string())) }
    }

    /// AC-RNG-05: a range operator against an Array value is rejected with
    /// a named error (surfaces as `Status::invalid_argument` at both of
    /// `translate_filter`'s own call sites), never reaching
    /// `push_scalar_comparison`'s own defensive panic.
    #[test]
    fn greater_than_on_array_is_rejected_with_a_named_error() {
        let f = field_filter_proto("tags", FieldOp::GreaterThan, array_value(vec![string_value("a")]));
        let result = translate_filter(&f).expect("must produce Some");
        let err = result.expect_err("must be rejected");
        assert!(err.contains("array"), "expected an array-naming error, got: {err}");
    }

    /// AC-RNG-06: same rejection for Map values.
    #[test]
    fn less_than_on_map_is_rejected_with_a_named_error() {
        let mut fields = std::collections::HashMap::new();
        fields.insert("k".to_string(), string_value("v"));
        let f = field_filter_proto("metadata", FieldOp::LessThan, map_value(fields));
        let result = translate_filter(&f).expect("must produce Some");
        let err = result.expect_err("must be rejected");
        assert!(err.contains("map"), "expected a map-naming error, got: {err}");
    }

    /// Regression guard: an Equal filter against an Array value is NOT
    /// rejected by this feature's own new check — the check is scoped to
    /// range operators only (`push_value_equality`, unaffected, already
    /// handles Array equality correctly).
    #[test]
    fn equal_on_array_is_not_rejected_by_the_range_operator_check() {
        let f = field_filter_proto("tags", FieldOp::Equal, array_value(vec![string_value("a")]));
        let result = translate_filter(&f).expect("must produce Some");
        assert!(result.is_ok(), "Equal on Array must not be rejected by this feature's own check");
    }
}

#[cfg(test)]
mod malformed_filter_shape_tests {
    //! firestore-malformed-filter-shape-validation (Slice 01, US-01) —
    //! pure, IO-free unit coverage for `translate_filter`'s own 2 new
    //! malformed-shape rejection checks.
    use super::translate_filter;
    use embyr_proto::firestore::{
        structured_query::{
            field_filter::Operator as FieldOp, filter::FilterType, FieldFilter, FieldReference,
            Filter,
        },
        value::ValueType,
        ArrayValue, Value,
    };

    fn field_filter_proto(field: &str, op: FieldOp, value: Value) -> Filter {
        Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(FieldReference { field_path: field.to_string() }),
                op: op as i32,
                value: Some(value),
            })),
        }
    }

    fn string_value(s: &str) -> Value {
        Value { value_type: Some(ValueType::StringValue(s.to_string())) }
    }

    fn array_value(values: Vec<Value>) -> Value {
        Value { value_type: Some(ValueType::ArrayValue(ArrayValue { values })) }
    }

    fn null_value() -> Value {
        Value { value_type: Some(ValueType::NullValue(0)) }
    }

    /// AC-MFS-01
    #[test]
    fn in_given_a_non_array_value_is_rejected() {
        let f = field_filter_proto("status", FieldOp::In, string_value("open"));
        let result = translate_filter(&f).expect("must produce Some");
        let err = result.expect_err("must be rejected");
        // Exact match, not `.contains("in")` — "array-contains-any" itself
        // contains the substring "in" (from "contains"), which would let a
        // mutant that maps `In` to the WRONG op-name string slip through a
        // looser assertion (confirmed via a real cargo-mutants miss).
        assert_eq!(err, "in requires an array value");
    }

    /// AC-MFS-02 (NotIn)
    #[test]
    fn not_in_given_a_non_array_value_is_rejected() {
        let f = field_filter_proto("status", FieldOp::NotIn, string_value("closed"));
        let result = translate_filter(&f).expect("must produce Some");
        let err = result.expect_err("must be rejected");
        assert_eq!(err, "not-in requires an array value");
    }

    /// AC-MFS-02 (ArrayContainsAny)
    #[test]
    fn array_contains_any_given_a_non_array_value_is_rejected() {
        let f = field_filter_proto("tags", FieldOp::ArrayContainsAny, string_value("urgent"));
        let result = translate_filter(&f).expect("must produce Some");
        let err = result.expect_err("must be rejected");
        assert_eq!(err, "array-contains-any requires an array value");
    }

    /// AC-MFS-03
    #[test]
    fn less_than_given_a_null_value_is_rejected() {
        let f = field_filter_proto("score", FieldOp::LessThan, null_value());
        let result = translate_filter(&f).expect("must produce Some");
        let err = result.expect_err("must be rejected");
        assert_eq!(err, "range comparison operators do not support null values");
    }

    /// AC-MFS-04 (regression guard): `In` given a well-formed `Array` value
    /// is NOT rejected by this feature's own new check.
    #[test]
    fn in_given_a_well_formed_array_value_is_not_rejected() {
        let f = field_filter_proto("status", FieldOp::In, array_value(vec![string_value("open")]));
        let result = translate_filter(&f).expect("must produce Some");
        assert!(result.is_ok(), "In with a well-formed Array value must not be rejected");
    }

    /// AC-MFS-04 (regression guard): `LessThan` given a well-formed
    /// non-Null value is NOT rejected by this feature's own new check.
    #[test]
    fn less_than_given_a_well_formed_value_is_not_rejected() {
        let f = field_filter_proto(
            "score",
            FieldOp::LessThan,
            Value { value_type: Some(ValueType::IntegerValue(100)) },
        );
        let result = translate_filter(&f).expect("must produce Some");
        assert!(result.is_ok(), "LessThan with a well-formed Integer value must not be rejected");
    }
}
