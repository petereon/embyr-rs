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
    middleware::rate_limit::RateLimiter,
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
    /// Per-project token bucket rate limiter. Applied after authentication.
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
    fn collect_filter_fields<'a>(filter: &'a QueryFilter) -> Vec<&'a str> {
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

#[tonic::async_trait]
impl Firestore for FirestoreService {
    async fn get_document(
        &self,
        request: Request<GetDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        let name = request.get_ref().name.clone();
        let project_id = Self::extract_project_id(&name)?.to_string();
        let api_key = Self::extract_api_key(&request)?;

        let (adapter, status, _dsn) = self.authenticate(&project_id, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }
        if self.rate_limiter.check(&project_id).await.is_err() {
            return Err(Status::resource_exhausted("rate limit exceeded"));
        }

        // Record read operation — best-effort, fire-and-forget.
        self.metrics_adapter.record_read(&project_id, 1).await;

        let path = Self::parse_document_path(&name)?;
        let doc_opt = adapter
            .get_document(&path)
            .await
            .map_err(core_error_to_status)?;

        match doc_opt {
            None => Err(Status::not_found(format!("{name} not found"))),
            Some(doc) => Ok(Response::new(document_to_proto(doc))),
        }
    }

    async fn create_document(
        &self,
        request: Request<CreateDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        let req = request.get_ref();

        // Parse project_id from parent: "projects/{pid}/databases/(default)/documents"
        let project_id_str = Self::extract_project_id(&req.parent)?.to_string();
        let api_key = Self::extract_api_key(&request)?;

        let (adapter, status, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }
        if self.rate_limiter.check(&project_id_str).await.is_err() {
            return Err(Status::resource_exhausted("rate limit exceeded"));
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

        let write_result = adapter
            .create_document(&path, fields.clone())
            .await
            .map_err(core_error_to_status)?;

        let create_time = write_result.create_time.unwrap_or(write_result.update_time);
        let update_time = write_result.update_time;

        Ok(Response::new(Self::build_document_response(
            &path,
            &fields,
            create_time,
            update_time,
        )))
    }

    async fn update_document(
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

        let (adapter, status, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }
        if self.rate_limiter.check(&project_id_str).await.is_err() {
            return Err(Status::resource_exhausted("rate limit exceeded"));
        }

        let fields = proto_fields_to_domain(&doc.fields)
            .ok_or_else(|| Status::invalid_argument("invalid field value"))?;

        let precondition = Self::convert_precondition(req.current_document.clone());

        let write_result = adapter
            .update_document(&path, fields.clone(), precondition)
            .await
            .map_err(core_error_to_status)?;

        let update_time = write_result.update_time;
        // For updates, create_time not returned by the write op; use update_time as fallback.
        let create_time = write_result.create_time.unwrap_or(update_time);

        Ok(Response::new(Self::build_document_response(
            &path,
            &fields,
            create_time,
            update_time,
        )))
    }

    async fn delete_document(
        &self,
        request: Request<DeleteDocumentRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.get_ref();

        let path = Self::parse_document_path(&req.name)?;
        let project_id_str = path.project_id.as_str().to_string();
        let api_key = Self::extract_api_key(&request)?;

        let (adapter, status, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }
        if self.rate_limiter.check(&project_id_str).await.is_err() {
            return Err(Status::resource_exhausted("rate limit exceeded"));
        }

        let precondition = Self::convert_precondition(req.current_document.clone());

        adapter
            .delete_document(&path, precondition)
            .await
            .map_err(core_error_to_status)?;

        Ok(Response::new(()))
    }

    type BatchGetDocumentsStream =
        tonic::codegen::BoxStream<BatchGetDocumentsResponse>;

    async fn batch_get_documents(
        &self,
        _: Request<BatchGetDocumentsRequest>,
    ) -> Result<Response<Self::BatchGetDocumentsStream>, Status> {
        Err(Status::unimplemented("not implemented"))
    }

    async fn begin_transaction(
        &self,
        request: Request<BeginTransactionRequest>,
    ) -> Result<Response<BeginTransactionResponse>, Status> {
        let req = request.get_ref();
        let project_id_str = Self::extract_project_id(&req.database)?.to_string();
        let api_key = Self::extract_api_key(&request)?;

        let (adapter, status, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }
        if self.rate_limiter.check(&project_id_str).await.is_err() {
            return Err(Status::resource_exhausted("rate limit exceeded"));
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

        Ok(Response::new(BeginTransactionResponse {
            transaction: txn_id.0,
        }))
    }

    async fn commit(
        &self,
        request: Request<CommitRequest>,
    ) -> Result<Response<CommitResponse>, Status> {
        let req = request.get_ref();
        let project_id_str = Self::extract_project_id(&req.database)?.to_string();
        let api_key = Self::extract_api_key(&request)?;

        let (adapter, status, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }
        if self.rate_limiter.check(&project_id_str).await.is_err() {
            return Err(Status::resource_exhausted("rate limit exceeded"));
        }

        let project_id = embyr_core::domain::project::ProjectId::new(&project_id_str)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let txn_id = embyr_core::domain::transaction::TransactionId(req.transaction.clone());

        // Translate proto writes to domain writes
        let mut domain_writes = Vec::with_capacity(req.writes.len());
        for proto_write in &req.writes {
            let precondition = Self::convert_precondition(proto_write.current_document.clone());
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

        Ok(Response::new(CommitResponse {
            write_results: proto_results,
            commit_time: Some(Timestamp {
                seconds: now.timestamp(),
                nanos: now.timestamp_subsec_nanos() as i32,
            }),
        }))
    }

    async fn rollback(&self, request: Request<RollbackRequest>) -> Result<Response<()>, Status> {
        let req = request.get_ref();
        let project_id_str = Self::extract_project_id(&req.database)?.to_string();
        let api_key = Self::extract_api_key(&request)?;

        let (adapter, status, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }
        if self.rate_limiter.check(&project_id_str).await.is_err() {
            return Err(Status::resource_exhausted("rate limit exceeded"));
        }

        let project_id = embyr_core::domain::project::ProjectId::new(&project_id_str)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let txn_id = embyr_core::domain::transaction::TransactionId(req.transaction.clone());

        adapter
            .rollback_transaction(&project_id, &txn_id)
            .await
            .map_err(core_error_to_status)?;

        Ok(Response::new(()))
    }

    type RunQueryStream = tonic::codegen::BoxStream<RunQueryResponse>;

    async fn run_query(
        &self,
        request: Request<RunQueryRequest>,
    ) -> Result<Response<Self::RunQueryStream>, Status> {
        let req = request.get_ref();

        // Extract project_id from parent: "projects/{pid}/databases/(default)/documents"
        let project_id_str = Self::extract_project_id(&req.parent)?.to_string();
        let api_key = Self::extract_api_key(&request)?;

        let (adapter, status_str, _dsn) = self.authenticate(&project_id_str, &api_key).await?;
        if status_str == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }
        if self.rate_limiter.check(&project_id_str).await.is_err() {
            return Err(Status::resource_exhausted("rate limit exceeded"));
        }

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
            .and_then(|f| translate_filter(f))
            .transpose()
            .map_err(|e| Status::invalid_argument(e))?;

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
                .map(|v| crate::encoding::firestore_proto::proto_value_to_field_value(v))
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

        Ok(Response::new(Box::pin(tokio_stream::iter(responses))))
    }

    type ListenStream = tonic::codegen::BoxStream<ListenResponse>;

    async fn listen(
        &self,
        request: Request<tonic::Streaming<ListenRequest>>,
    ) -> Result<Response<Self::ListenStream>, Status> {
        let api_key = Self::extract_api_key(&request)?;
        let mut in_stream = request.into_inner();

        // Read first message to get the AddTarget + project_id for auth.
        let first_msg = in_stream
            .next()
            .await
            .ok_or_else(|| Status::invalid_argument("empty listen stream"))?
            .map_err(|e| Status::internal(e.to_string()))?;

        let project_id = extract_project_id_from_listen_request(&first_msg)?;
        let (adapter, status, dsn) = self.authenticate(&project_id, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }
        if self.rate_limiter.check(&project_id).await.is_err() {
            return Err(Status::resource_exhausted("rate limit exceeded"));
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

        Ok(Response::new(Box::pin(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        )))
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
                .and_then(|op| match op {
                    embyr_proto::firestore::structured_query::unary_filter::OperandType::Field(
                        fr,
                    ) => Some(fr.field_path.clone()),
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
