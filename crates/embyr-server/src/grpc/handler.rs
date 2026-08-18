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
        document::CollectionPath,
        field_value::FieldValue,
        project::CredentialCacheKey,
        query::{
            Cursor, FieldFilter, FilterOp, OrderBy, OrderDirection, QueryFilter, StructuredQuery,
        },
        transaction::TransactionOptions,
    },
    error::CoreError,
    storage::backend_adapter::{Write as DomainWrite, WritePrecondition},
};
use embyr_proto::firestore::{
    firestore_server::Firestore, precondition::ConditionType, BatchGetDocumentsRequest,
    BatchGetDocumentsResponse, BeginTransactionRequest, BeginTransactionResponse, CommitRequest,
    CommitResponse, CreateDocumentRequest, DeleteDocumentRequest, Document, GetDocumentRequest,
    ListenRequest, ListenResponse, RollbackRequest, RunQueryRequest, RunQueryResponse,
    UpdateDocumentRequest,
    run_query_request::QueryType,
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
    encoding::firestore_proto::{document_to_proto, fields_to_proto, proto_fields_to_domain},
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
    async fn attach_client_identity_if_present<T>(
        &self,
        request: &Request<T>,
        project_id_str: &str,
    ) -> Option<embyr_core::client_identity::VerifiedEndUserIdentity> {
        let token = Self::extract_client_identity_token(request)?;

        let row = self
            .system_db
            .get_client_identity_credential(project_id_str)
            .await
            .ok()??;
        let credential = embyr_core::client_identity::ClientIdentityCredential {
            public_key_current: row.public_key_current.try_into().ok()?,
            public_key_previous: row
                .public_key_previous
                .and_then(|v| v.try_into().ok()),
        };

        embyr_core::client_identity::verify_client_identity_token(
            Some(&token),
            project_id_str,
            &credential,
        )
        .ok()
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
        let Some(filter) = &query.filter else { return false };
        if query.order_by.is_empty() {
            return false;
        }
        // Collect filtered field paths.
        let filter_fields = Self::collect_filter_fields(filter);
        // If any orderBy field is NOT in the filter fields, composite index required.
        query
            .order_by
            .iter()
            .any(|ob| !filter_fields.contains(&ob.field_path.as_str()))
    }

    /// Collect all field paths referenced by a filter (recursively).
    fn collect_filter_fields(filter: &QueryFilter) -> Vec<&str> {
        match filter {
            QueryFilter::Field(ff) => vec![ff.field_path.as_str()],
            QueryFilter::Composite(sub) => {
                sub.iter().flat_map(Self::collect_filter_fields).collect()
            }
        }
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
            .get_document(&path)
            .await
            .map_err(core_error_to_status)?;

        match rule_row {
            // No rule defined for this collection — UNCHANGED, unmodified
            // pre-`security-rules` code path (AC-17-14/15/16, structural
            // regression guardrail).
            None => match doc_opt {
                None => Err(Status::not_found(format!("{name} not found"))),
                Some(doc) => {
                    let mut response = Response::new(document_to_proto(doc));
                    Self::attach_rate_limit_headers(response.metadata_mut(), &rate_info);
                    Ok(response)
                }
            },
            // A rule is defined — evaluate it (US-02/03/04, AC-17-06..13).
            Some(rule_row) => {
                let condition = embyr_core::access_control::parse_condition(&rule_row.condition_source)
                    .map_err(|e| {
                        Status::internal(format!("stored access rule failed to re-parse: {e:?}"))
                    })?;
                let auth_ctx = verified_identity
                    .as_ref()
                    .map(|v| embyr_core::access_control::AuthContext { uid: v.end_user_id.clone() });
                // AC-17-10 (existence non-leakage): a non-existent document
                // evaluates against an EMPTY field map — the same
                // fail-closed mechanism AC-17-09 already uses for a single
                // missing field, applied uniformly. `evaluate()` is called
                // UNCONDITIONALLY, whether or not the document exists.
                let empty_fields: std::collections::BTreeMap<String, FieldValue> =
                    std::collections::BTreeMap::new();
                let resource_fields = doc_opt.as_ref().map(|d| &d.fields).unwrap_or(&empty_fields);

                // security-rules-write-path (ADR-030): one new argument at
                // this existing call site — an empty map for the new
                // `request_resource_fields` parameter. `GetDocument` has no
                // "proposed new document" concept; any read rule that
                // references `request.resource.data.<field>` (grammar-legal
                // but semantically nonsensical for a read) denies via the
                // same fail-closed mechanism, never a crash. Zero other
                // change to this function.
                match embyr_core::access_control::evaluate(
                    &condition,
                    auth_ctx.as_ref(),
                    resource_fields,
                    &empty_fields,
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

        if let Some(write_rule_row) = write_rule_row {
            let condition =
                embyr_core::access_control::parse_condition(&write_rule_row.condition_source)
                    .map_err(|e| {
                        Status::internal(format!("stored write rule failed to re-parse: {e:?}"))
                    })?;
            let auth_ctx = verified_identity
                .as_ref()
                .map(|v| embyr_core::access_control::AuthContext { uid: v.end_user_id.clone() });

            // Create: `resource_fields` is empty (no document exists yet —
            // AC-17-28's fail-closed mechanism reuse); `request_resource_fields`
            // is the proposed new document already parsed above, no new I/O.
            let empty_resource_fields: std::collections::BTreeMap<String, FieldValue> =
                std::collections::BTreeMap::new();

            match embyr_core::access_control::evaluate(
                &condition,
                auth_ctx.as_ref(),
                &empty_resource_fields,
                &fields,
            ) {
                embyr_core::access_control::EvaluationOutcome::Deny => {
                    return Err(Status::permission_denied("access denied by write rule"));
                }
                embyr_core::access_control::EvaluationOutcome::Allow => {}
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

        if let Some(write_rule_row) = write_rule_row {
            let condition =
                embyr_core::access_control::parse_condition(&write_rule_row.condition_source)
                    .map_err(|e| {
                        Status::internal(format!("stored write rule failed to re-parse: {e:?}"))
                    })?;
            let auth_ctx = verified_identity
                .as_ref()
                .map(|v| embyr_core::access_control::AuthContext { uid: v.end_user_id.clone() });

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
                .get_document(&path)
                .await
                .map_err(core_error_to_status)?;
            let empty_resource_fields: std::collections::BTreeMap<String, FieldValue> =
                std::collections::BTreeMap::new();
            let resource_fields =
                doc_opt.as_ref().map(|d| &d.fields).unwrap_or(&empty_resource_fields);

            // Proposed new state: the already-parsed update body fields, no
            // new I/O — the two-value old-vs-new comparison this slice
            // exists to prove.
            match embyr_core::access_control::evaluate(
                &condition,
                auth_ctx.as_ref(),
                resource_fields,
                &fields,
            ) {
                embyr_core::access_control::EvaluationOutcome::Deny => {
                    return Err(Status::permission_denied("access denied by write rule"));
                }
                embyr_core::access_control::EvaluationOutcome::Allow => {}
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

        if let Some(write_rule_row) = write_rule_row {
            let condition =
                embyr_core::access_control::parse_condition(&write_rule_row.condition_source)
                    .map_err(|e| {
                        Status::internal(format!("stored write rule failed to re-parse: {e:?}"))
                    })?;
            let auth_ctx = verified_identity
                .as_ref()
                .map(|v| embyr_core::access_control::AuthContext { uid: v.end_user_id.clone() });

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
                .get_document(&path)
                .await
                .map_err(core_error_to_status)?;
            let empty_fields: std::collections::BTreeMap<String, FieldValue> =
                std::collections::BTreeMap::new();
            let resource_fields = doc_opt.as_ref().map(|d| &d.fields).unwrap_or(&empty_fields);

            // Proposed new state: ALWAYS empty — a delete has no request
            // body to parse into fields (DIFFERENT from Create/Update).
            let request_resource_fields: std::collections::BTreeMap<String, FieldValue> =
                std::collections::BTreeMap::new();

            match embyr_core::access_control::evaluate(
                &condition,
                auth_ctx.as_ref(),
                resource_fields,
                &request_resource_fields,
            ) {
                embyr_core::access_control::EvaluationOutcome::Deny => {
                    return Err(Status::permission_denied("access denied by write rule"));
                }
                embyr_core::access_control::EvaluationOutcome::Allow => {}
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

    async fn handle_batch_get_documents(
        &self,
        _: Request<BatchGetDocumentsRequest>,
    ) -> Result<Response<tonic::codegen::BoxStream<BatchGetDocumentsResponse>>, Status> {
        Err(Status::unimplemented("not implemented"))
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

        // Translate proto writes to domain writes
        let mut domain_writes = Vec::with_capacity(req.writes.len());
        for proto_write in &req.writes {
            let precondition = Self::convert_precondition(proto_write.current_document);
            match &proto_write.operation {
                Some(embyr_proto::firestore::write::Operation::Update(doc)) => {
                    let path = Self::parse_document_path(&doc.name)?;
                    let fields = proto_fields_to_domain(&doc.fields)
                        .ok_or_else(|| Status::invalid_argument("invalid field value in write"))?;
                    domain_writes.push(DomainWrite::Update {
                        path,
                        fields,
                        version: None,
                        precondition,
                    });
                }
                Some(embyr_proto::firestore::write::Operation::Delete(doc_name)) => {
                    let path = Self::parse_document_path(doc_name)?;
                    domain_writes.push(DomainWrite::Delete {
                        path,
                        version: None,
                        precondition,
                    });
                }
                Some(embyr_proto::firestore::write::Operation::Transform(dt)) => {
                    let path = Self::parse_document_path(&dt.document)?;
                    domain_writes.push(DomainWrite::Transform {
                        path,
                        transforms: vec![],
                    });
                }
                None => {}
            }
        }

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
                transform_results: vec![],
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

        // Translate proto order_by → domain OrderBy
        let order_by: Vec<OrderBy> = sq_proto
            .order_by
            .iter()
            .filter_map(|o| {
                let field_path = o.field.as_ref()?.field_path.clone();
                let direction = match Direction::try_from(o.direction).unwrap_or(Direction::Unspecified) {
                    Direction::Descending => OrderDirection::Descending,
                    _ => OrderDirection::Ascending,
                };
                Some(OrderBy { field_path, direction })
            })
            .collect();

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

        // security-rules-query-path (ADR-031 § Decision — Composition, step
        // 3): a single indexed lookup on `(project_id, collection_path)`,
        // identical shape/cost to `handle_get_document`'s own
        // `get_access_rule` call, reading the SAME `access_rules` table
        // (never `write_access_rules`). `None` -> the `if let` below simply
        // does not execute — the composite-index check and
        // `adapter.run_query()` calls immediately below are reached
        // completely unmodified, the EXACT pre-feature code path
        // (regression guardrail).
        let rule_row = self
            .system_db
            .get_access_rule(&project_id_str, &collection.collection_path)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        if let Some(rule_row) = rule_row {
            let condition = embyr_core::access_control::parse_condition(&rule_row.condition_source)
                .map_err(|e| {
                    Status::internal(format!("stored access rule failed to re-parse: {e:?}"))
                })?;
            let auth_ctx = verified_identity
                .as_ref()
                .map(|v| embyr_core::access_control::AuthContext { uid: v.end_user_id.clone() });

            // ADR-031 § OQ-SRQ-03 Resolution: compliance-checking runs
            // strictly BEFORE the composite-index check below — a caller
            // never entitled to query this collection at all must never
            // learn whether it also requires a composite index.
            match embyr_core::access_control::check_query_compliance(
                &condition,
                domain_query.filter.as_ref(),
                auth_ctx.as_ref(),
            ) {
                embyr_core::access_control::QueryComplianceOutcome::Admitted => {}
                outcome => return Err(query_compliance_rejection(&outcome)),
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
            return Err(Status::failed_precondition(
                "query requires a composite index; create the index before running this query",
            ));
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

    async fn handle_listen(
        &self,
        request: Request<tonic::Streaming<ListenRequest>>,
    ) -> Result<Response<tonic::codegen::BoxStream<ListenResponse>>, Status> {
        let api_key = Self::extract_api_key(&request)?;
        let mut in_stream = request.into_inner();

        // Read first message to get the AddTarget + project_id for auth.
        let first_msg = in_stream
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

        tokio::spawn(async move {
            if let Err(e) = crate::realtime::listen_handler::handle_add_target(
                &first_msg,
                &adapter,
                &tx,
                keepalive,
                registry,
                &channel,
                resume_token,
            )
            .await
            {
                let _ = tx.send(Err(Status::internal(e))).await;
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
fn query_compliance_rejection(
    outcome: &embyr_core::access_control::QueryComplianceOutcome,
) -> Status {
    use embyr_core::access_control::QueryComplianceOutcome;
    let message = match outcome {
        QueryComplianceOutcome::RejectedUnsupportedRuleShape => {
            "query rejected: this collection's access rule is not a shape supported for query \
             enforcement"
                .to_string()
        }
        QueryComplianceOutcome::Rejected { unsatisfied_conjuncts } => format!(
            "query rejected by access rule: {} unsatisfied filter requirement(s)",
            unsatisfied_conjuncts.len()
        ),
        QueryComplianceOutcome::Admitted => unreachable!("Admitted never reaches this function"),
    };
    Status::permission_denied(message)
}

/// Translate a proto `Filter` to a domain `QueryFilter`.
fn translate_filter(
    f: &embyr_proto::firestore::structured_query::Filter,
) -> Option<Result<QueryFilter, String>> {
    match f.filter_type.as_ref()? {
        FilterType::FieldFilter(ff) => {
            let field_path = ff.field.as_ref()?.field_path.clone();
            let op = translate_field_op(FieldOp::try_from(ff.op).ok()?)?;
            let value =
                crate::encoding::firestore_proto::proto_value_to_field_value(ff.value.as_ref()?)?;
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

fn core_error_to_status(e: CoreError) -> Status {
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
