//! mTLS gRPC server for embyr-agent.
//!
//! Implements StorageAgent backed by PostgresBackendAdapter.
//! Exposes `serve()` for in-process test starts and `run()` for the binary.

// tonic::Status (~176 bytes: code + message + metadata map + source) is the
// idiomatic error type for gRPC handler functions across this file — boxing
// it at every one of these call sites would add noise without a real
// correctness or performance benefit at this request volume.
#![allow(clippy::result_large_err)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use embyr_core::domain::{document::DocumentPath, field_value::FieldValue, project::ProjectId};
use embyr_core::domain::transaction::{TransactionId, TransactionOptions};
use embyr_core::error::CoreError;
use embyr_core::storage::backend_adapter::{BackendAdapter, Write as DomainWrite, WritePrecondition};
use embyr_pg_storage::backend_adapter::PostgresBackendAdapter;
use embyr_core::domain::{document::CollectionPath, query::{FieldFilter, FilterOp, QueryFilter, StructuredQuery as DomainStructuredQuery}};
use embyr_proto::agent::{
    filter::FilterType as ProtoFilterType,
    precondition::ConditionType,
    run_query_request::QueryType as RunQueryQueryType,
    run_query_response::ContinuationSelector,
    storage_agent_server::{StorageAgent, StorageAgentServer},
    write::Operation,
    BeginTransactionRequest, BeginTransactionResponse, CommitRequest, CommitResponse,
    CreateDocumentRequest, DeleteDocumentRequest, DocChange, DocChangeKind, Document, FieldFilterOp,
    Filter as ProtoFilter, GetDocumentRequest,
    ListDocumentsRequest, ListDocumentsResponse,
    PingRequest, PingResponse, Precondition, RollbackRequest,
    RunAggregationQueryRequest, RunAggregationQueryResponse,
    RunQueryRequest, RunQueryResponse,
    SubscribeRequest,
    UpdateDocumentRequest,
};
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::{
    Request, Response, Status,
    transport::{Certificate, Identity, ServerTlsConfig},
};
use tracing::info;

use crate::config::AgentConfig;
use crate::encoding::{domain_doc_to_proto, proto_value_to_field_value};
use crate::notify_bridge::AgentNotifyBridge;
use crate::sweeper::AgentTransactionSweeper;

/// Encode an offset as a hex string page token (no external deps).
fn encode_page_token(offset: u32) -> String {
    format!("{:08x}", offset)
}

/// Decode a hex page token back to an offset.
fn decode_page_token(token: &str) -> Result<u32, Status> {
    if token.is_empty() {
        return Ok(0);
    }
    u32::from_str_radix(token, 16)
        .map_err(|_| Status::invalid_argument(format!("invalid page_token: {token}")))
}

/// StorageAgent gRPC service backed by PostgresBackendAdapter.
pub struct StorageAgentService {
    project_id: String,
    storage: Arc<PostgresBackendAdapter>,
    notify_bridge: Arc<AgentNotifyBridge>,
}

impl StorageAgentService {
    /// Construct the service.
    pub fn new(
        project_id: String,
        storage: Arc<PostgresBackendAdapter>,
        notify_bridge: Arc<AgentNotifyBridge>,
    ) -> Self {
        Self { project_id, storage, notify_bridge }
    }
}

/// Map a `CoreError` to a gRPC `Status`.
fn core_error_to_status(e: CoreError) -> Status {
    match e {
        CoreError::DocumentNotFound(msg) => Status::not_found(msg),
        CoreError::AlreadyExists(msg) => Status::already_exists(msg),
        CoreError::OccConflict => Status::failed_precondition("optimistic concurrency conflict"),
        CoreError::FailedPrecondition(msg) => Status::failed_precondition(msg),
        CoreError::InvalidArgument(msg) => Status::invalid_argument(msg),
        CoreError::BackendUnavailable(msg) => Status::internal(msg),
        CoreError::TransactionNotFound => Status::not_found("transaction not found or expired"),
        CoreError::TransactionAborted => Status::aborted("transaction aborted"),
        CoreError::ProjectNotFound(msg) => Status::not_found(msg),
        CoreError::PermissionDenied(msg) => Status::permission_denied(msg),
        CoreError::Unauthenticated => Status::unauthenticated("unauthenticated"),
        CoreError::ResourceExhausted(msg) => Status::resource_exhausted(msg),
    }
}

/// Parse a `Precondition` proto into a `WritePrecondition`.
fn parse_precondition(p: Option<Precondition>) -> Option<WritePrecondition> {
    match p.and_then(|p| p.condition_type) {
        None => None,
        Some(ConditionType::Exists(true)) => Some(WritePrecondition::MustExist),
        Some(ConditionType::Exists(false)) => Some(WritePrecondition::MustNotExist),
        Some(ConditionType::UpdateTime(ts)) => {
            Some(WritePrecondition::UpdateTime(ts.seconds, ts.nanos))
        }
    }
}

/// Validate that a field path does not contain consecutive dots.
fn validate_field_path(path: &str) -> Result<(), Status> {
    if path.contains("..") {
        Err(Status::invalid_argument(format!(
            "invalid field path '{}': consecutive dots not allowed",
            path
        )))
    } else {
        Ok(())
    }
}

/// Convert a proto FieldFilterOp i32 to a domain FilterOp.
fn proto_filter_op_to_domain(op_i32: i32) -> Result<FilterOp, Status> {
    match FieldFilterOp::try_from(op_i32).unwrap_or(FieldFilterOp::Unspecified) {
        FieldFilterOp::LessThan => Ok(FilterOp::LessThan),
        FieldFilterOp::LessThanOrEqual => Ok(FilterOp::LessThanOrEqual),
        FieldFilterOp::GreaterThan => Ok(FilterOp::GreaterThan),
        FieldFilterOp::GreaterThanOrEqual => Ok(FilterOp::GreaterThanOrEqual),
        FieldFilterOp::Equal => Ok(FilterOp::Equal),
        FieldFilterOp::NotEqual => Ok(FilterOp::NotEqual),
        FieldFilterOp::ArrayContains => Ok(FilterOp::ArrayContains),
        FieldFilterOp::In => Ok(FilterOp::In),
        FieldFilterOp::NotIn => Ok(FilterOp::NotIn),
        FieldFilterOp::ArrayContainsAny => Ok(FilterOp::ArrayContainsAny),
        FieldFilterOp::Unspecified => Err(Status::invalid_argument("filter op unspecified")),
    }
}

/// Convert a proto `Filter` to a domain `QueryFilter`, validating field paths.
fn proto_filter_to_domain(filter: ProtoFilter) -> Result<QueryFilter, Status> {
    match filter.filter_type {
        Some(ProtoFilterType::FieldFilter(ff)) => {
            validate_field_path(&ff.field_path)?;
            let op = proto_filter_op_to_domain(ff.op)?;
            let value = ff
                .value
                .map(|v| proto_value_to_field_value(&v))
                .unwrap_or(FieldValue::Null);
            Ok(QueryFilter::Field(FieldFilter {
                field_path: ff.field_path,
                op,
                value,
            }))
        }
        Some(ProtoFilterType::CompositeFilter(cf)) => {
            let filters = cf
                .filters
                .into_iter()
                .map(proto_filter_to_domain)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(QueryFilter::Composite(filters))
        }
        None => Err(Status::invalid_argument("filter_type required in Filter")),
    }
}

/// Generate a 20-character Firestore-compatible alphanumeric document ID.
fn generate_document_id() -> String {
    use rand_core::{OsRng, RngCore};
    let chars: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = OsRng;
    let mut id = String::with_capacity(20);
    for _ in 0..20 {
        let idx = (rng.next_u32() as usize) % chars.len();
        id.push(chars[idx] as char);
    }
    id
}

/// Convert proto fields map to domain `BTreeMap<String, FieldValue>`.
fn proto_fields_to_domain(
    fields: std::collections::HashMap<String, embyr_proto::agent::Value>,
) -> BTreeMap<String, FieldValue> {
    fields
        .into_iter()
        .map(|(k, v)| (k, proto_value_to_field_value(&v)))
        .collect()
}

/// Translate a proto `Write` message to a domain `Write` variant.
fn proto_write_to_domain(w: embyr_proto::agent::Write, project_id_str: &str) -> Result<DomainWrite, Status> {
    let precondition = parse_precondition(w.current_document);
    match w.operation {
        Some(Operation::Update(doc)) => {
            let path = parse_document_name(&doc.name, project_id_str)?;
            let fields = proto_fields_to_domain(doc.fields);
            Ok(DomainWrite::Update { path, fields, version: None, precondition })
        }
        Some(Operation::Delete(name)) => {
            let path = parse_document_name(&name, project_id_str)?;
            Ok(DomainWrite::Delete { path, version: None, precondition })
        }
        None => Err(Status::invalid_argument("write operation required")),
    }
}

#[tonic::async_trait]
impl StorageAgent for StorageAgentService {
    async fn get_document(
        &self,
        request: Request<GetDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        let start = std::time::Instant::now();
        let name = request.into_inner().name;

        if name.is_empty() {
            return Err(Status::invalid_argument("name is required"));
        }

        let path = parse_document_name(&name, &self.project_id)?;

        match self.storage.get_document(&path).await {
            Ok(Some(doc)) => {
                let duration_ms = start.elapsed().as_millis();
                tracing::info!(
                    project_id = %path.project_id.as_str(),
                    path = %name,
                    duration_ms = duration_ms,
                    "GetDocument"
                );
                Ok(Response::new(domain_doc_to_proto(doc)))
            }
            Ok(None) => Err(Status::not_found(format!("document not found: {name}"))),
            Err(e) => Err(Status::internal(format!("{e}"))),
        }
    }

    async fn create_document(
        &self,
        request: Request<CreateDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        let req = request.into_inner();

        // Validate parent
        if req.parent.is_empty() {
            return Err(Status::invalid_argument("parent is required"));
        }
        if req.collection_id.is_empty() {
            return Err(Status::invalid_argument("collection_id is required"));
        }

        // Parse project_id from parent: "projects/{project_id}/databases/(default)/documents[/...]"
        let project_id_str = parse_project_id_from_parent(&req.parent)?;

        // Build collection_path: parent suffix (after "documents") + "/" + collection_id
        // Parent may be "projects/{pid}/databases/(default)/documents" (root)
        // or "projects/{pid}/databases/(default)/documents/subcoll" (nested).
        let collection_path = build_collection_path(&req.parent, &req.collection_id)?;

        // Auto-generate document_id if empty
        let document_id = if req.document_id.is_empty() {
            generate_document_id()
        } else {
            req.document_id.clone()
        };

        // Extract fields from the request document
        let fields = req
            .document
            .map(|d| proto_fields_to_domain(d.fields))
            .unwrap_or_default();

        let pid = ProjectId::new(&project_id_str)
            .map_err(|e| Status::invalid_argument(format!("invalid project_id: {e}")))?;

        let path = DocumentPath {
            project_id: pid,
            collection_path,
            document_id,
        };

        self.storage
            .create_document(&path, fields)
            .await
            .map_err(core_error_to_status)?;

        // Fetch the created document to return with timestamps and generation
        match self.storage.get_document(&path).await {
            Ok(Some(doc)) => Ok(Response::new(domain_doc_to_proto(doc))),
            Ok(None) => Err(Status::internal("document not found after creation")),
            Err(e) => Err(core_error_to_status(e)),
        }
    }

    async fn update_document(
        &self,
        request: Request<UpdateDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        let req = request.into_inner();

        let doc = req.document.ok_or_else(|| Status::invalid_argument("document is required"))?;

        if doc.name.is_empty() {
            return Err(Status::invalid_argument("document.name is required"));
        }

        let path = parse_document_name(&doc.name, &self.project_id)?;
        let precondition = parse_precondition(req.current_document);

        // Build the final field set — apply field mask if present
        let final_fields = if let Some(mask) = req.update_mask {
            if !mask.field_paths.is_empty() {
                // Read-modify-write: fetch current doc, merge masked fields
                let current_doc = self
                    .storage
                    .get_document(&path)
                    .await
                    .map_err(core_error_to_status)?
                    .ok_or_else(|| {
                        Status::not_found(format!("document not found: {}", doc.name))
                    })?;

                let update_fields = proto_fields_to_domain(doc.fields);
                let mut merged = current_doc.fields.clone();

                for field_path in &mask.field_paths {
                    match update_fields.get(field_path) {
                        Some(v) => {
                            merged.insert(field_path.clone(), v.clone());
                        }
                        None => {
                            merged.remove(field_path);
                        }
                    }
                }
                merged
            } else {
                proto_fields_to_domain(doc.fields)
            }
        } else {
            proto_fields_to_domain(doc.fields)
        };

        self.storage
            .update_document(&path, final_fields, precondition)
            .await
            .map_err(core_error_to_status)?;

        // Fetch the updated document to return with timestamps and generation
        match self.storage.get_document(&path).await {
            Ok(Some(updated_doc)) => Ok(Response::new(domain_doc_to_proto(updated_doc))),
            Ok(None) => Err(Status::internal("document not found after update")),
            Err(e) => Err(core_error_to_status(e)),
        }
    }

    async fn delete_document(
        &self,
        request: Request<DeleteDocumentRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        if req.name.is_empty() {
            return Err(Status::invalid_argument("name is required"));
        }
        let path = parse_document_name(&req.name, &self.project_id)?;
        let precondition = parse_precondition(req.current_document);

        let must_exist = matches!(precondition, Some(WritePrecondition::MustExist));

        match self.storage.delete_document(&path, precondition).await {
            Ok(()) => Ok(Response::new(())),
            // Firestore no-op semantics: absent doc delete succeeds unless MustExist precondition
            Err(CoreError::DocumentNotFound(msg)) => {
                if must_exist {
                    Err(Status::not_found(format!("document not found: {msg}")))
                } else {
                    Ok(Response::new(()))
                }
            }
            Err(e) => Err(core_error_to_status(e)),
        }
    }

    type RunQueryStream = ReceiverStream<Result<RunQueryResponse, Status>>;

    async fn run_query(
        &self,
        request: Request<RunQueryRequest>,
    ) -> Result<Response<Self::RunQueryStream>, Status> {
        let req = request.into_inner();

        // Extract structured query
        let sq = match req.query_type {
            Some(RunQueryQueryType::StructuredQuery(sq)) => sq,
            None => return Err(Status::invalid_argument("query_type is required")),
        };

        // Validate and extract from clause
        let from = sq
            .from
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("from clause is required"))?;

        // Convert filter BEFORE touching storage (validates field paths eagerly)
        let domain_filter = sq.filter.map(proto_filter_to_domain).transpose()?;

        // Parse project_id from parent
        let project_id_str = parse_project_id_from_parent(&req.parent)?;

        // Build domain query
        let pid = ProjectId::new(&project_id_str)
            .map_err(|e| Status::invalid_argument(format!("invalid project_id: {e}")))?;
        let collection = CollectionPath {
            project_id: pid,
            collection_path: from.collection_id.clone(),
        };
        let query = DomainStructuredQuery {
            collection_id: from.collection_id,
            all_descendants: from.all_descendants,
            filter: domain_filter,
            order_by: vec![],
            limit: None,
            offset: None,
            start_at: None,
            end_at: None,
            since_update_time: None,
        };

        // Execute query against storage
        let docs = self
            .storage
            .run_query(&collection, &query, None)
            .await
            .map_err(core_error_to_status)?;

        // Stream results via tokio mpsc channel
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move {
            for doc in docs {
                let proto = domain_doc_to_proto(doc);
                if tx
                    .send(Ok(RunQueryResponse {
                        document: Some(proto),
                        ..Default::default()
                    }))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            // Send completion signal
            let _ = tx
                .send(Ok(RunQueryResponse {
                    continuation_selector: Some(ContinuationSelector::Done(true)),
                    ..Default::default()
                }))
                .await;
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn run_aggregation_query(
        &self,
        request: Request<RunAggregationQueryRequest>,
    ) -> Result<Response<RunAggregationQueryResponse>, Status> {
        use embyr_proto::agent::run_aggregation_query_request::QueryType;
        let req = request.into_inner();
        let sq = match req.query_type {
            Some(QueryType::StructuredQuery(sq)) => sq,
            None => return Err(Status::invalid_argument("query_type is required")),
        };
        let from = sq
            .from
            .into_iter()
            .next()
            .ok_or_else(|| Status::invalid_argument("from clause required"))?;
        let project_id_str = parse_project_id_from_parent(&req.parent)?;
        let domain_filter = sq.filter.map(proto_filter_to_domain).transpose()?;
        let pid = ProjectId::new(&project_id_str)
            .map_err(|e| Status::invalid_argument(format!("{e}")))?;
        let collection = CollectionPath {
            project_id: pid,
            collection_path: from.collection_id.clone(),
        };
        let query = DomainStructuredQuery {
            collection_id: from.collection_id,
            all_descendants: from.all_descendants,
            filter: domain_filter,
            order_by: vec![],
            limit: None,
            offset: None,
            start_at: None,
            end_at: None,
            since_update_time: None,
        };
        let docs = self
            .storage
            .run_query(&collection, &query, None)
            .await
            .map_err(core_error_to_status)?;
        Ok(Response::new(RunAggregationQueryResponse {
            count: docs.len() as i64,
        }))
    }

    async fn list_documents(
        &self,
        request: Request<ListDocumentsRequest>,
    ) -> Result<Response<ListDocumentsResponse>, Status> {
        let req = request.into_inner();
        let project_id_str = parse_project_id_from_parent(&req.parent)?;
        let page_size = if req.page_size <= 0 { 100i32 } else { req.page_size.min(100) };
        let offset = decode_page_token(&req.page_token)?;
        let pid = ProjectId::new(&project_id_str)
            .map_err(|e| Status::invalid_argument(format!("{e}")))?;
        let collection = CollectionPath {
            project_id: pid,
            collection_path: req.collection_id.clone(),
        };
        let query = DomainStructuredQuery {
            collection_id: req.collection_id,
            all_descendants: false,
            filter: None,
            order_by: vec![],
            limit: Some(page_size + 1), // fetch one extra to detect if more pages exist
            offset: Some(offset as i32),
            start_at: None,
            end_at: None,
            since_update_time: None,
        };
        let mut docs = self
            .storage
            .run_query(&collection, &query, None)
            .await
            .map_err(core_error_to_status)?;
        let has_more = docs.len() > page_size as usize;
        if has_more {
            docs.truncate(page_size as usize);
        }
        let next_page_token = if has_more {
            encode_page_token(offset + page_size as u32)
        } else {
            String::new()
        };
        let proto_docs = docs.into_iter().map(domain_doc_to_proto).collect();
        Ok(Response::new(ListDocumentsResponse {
            documents: proto_docs,
            next_page_token,
        }))
    }

    async fn begin_transaction(
        &self,
        _request: Request<BeginTransactionRequest>,
    ) -> Result<Response<BeginTransactionResponse>, Status> {
        let pid = ProjectId::new(&self.project_id)
            .map_err(|e| Status::internal(format!("{e}")))?;
        match self.storage.begin_transaction(&pid, TransactionOptions::ReadWrite).await {
            Ok(txn_id) => Ok(Response::new(BeginTransactionResponse { transaction: txn_id.0 })),
            Err(e) => Err(core_error_to_status(e)),
        }
    }

    async fn commit(
        &self,
        request: Request<CommitRequest>,
    ) -> Result<Response<CommitResponse>, Status> {
        let req = request.into_inner();
        let pid = ProjectId::new(&self.project_id)
            .map_err(|e| Status::internal(format!("{e}")))?;
        let txn_id = TransactionId(req.transaction);

        let writes: Vec<DomainWrite> = req.writes
            .into_iter()
            .map(|w| proto_write_to_domain(w, &self.project_id))
            .collect::<Result<Vec<_>, Status>>()?;

        let now = chrono::Utc::now();
        match self.storage.commit_transaction(&pid, &txn_id, writes).await {
            Ok(_results) => Ok(Response::new(CommitResponse {
                commit_time: Some(prost_types::Timestamp {
                    seconds: now.timestamp(),
                    nanos: now.timestamp_subsec_nanos() as i32,
                }),
                ..Default::default()
            })),
            Err(CoreError::TransactionNotFound) => {
                Err(Status::not_found("transaction not found or expired"))
            }
            Err(CoreError::TransactionAborted) => {
                Err(Status::aborted("transaction aborted: OCC conflict"))
            }
            Err(e) => Err(core_error_to_status(e)),
        }
    }

    async fn rollback(
        &self,
        request: Request<RollbackRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        let pid = ProjectId::new(&self.project_id)
            .map_err(|e| Status::internal(format!("{e}")))?;
        let txn_id = TransactionId(req.transaction);

        match self.storage.rollback_transaction(&pid, &txn_id).await {
            Ok(()) => Ok(Response::new(())),
            Err(CoreError::TransactionNotFound) => {
                Err(Status::not_found("transaction not found or already committed"))
            }
            Err(e) => Err(core_error_to_status(e)),
        }
    }

    async fn ping(
        &self,
        _request: Request<PingRequest>,
    ) -> Result<Response<PingResponse>, Status> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Status::internal(format!("system time error: {e}")))?;
        Ok(Response::new(PingResponse {
            server_time: Some(prost_types::Timestamp {
                seconds: now.as_secs() as i64,
                nanos: now.subsec_nanos() as i32,
            }),
        }))
    }

    type SubscribeStream = ReceiverStream<Result<DocChange, Status>>;

    async fn subscribe(
        &self,
        request: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let collection_path = request.into_inner().collection_path;
        if collection_path.is_empty() {
            return Err(Status::invalid_argument("collection_path is required"));
        }

        // Obtain a per-subscription receiver from the bridge.
        // Each call creates an independent PgListener connection.
        let receiver = self
            .notify_bridge
            .subscribe()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        // Filter bridge events: pass RESET always; pass others only when the
        // document_name contains the subscribed collection_path.
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<DocChange, Status>>(64);
        tokio::spawn(async move {
            let mut receiver = receiver;
            while let Some(change) = receiver.recv().await {
                let pass = change.kind == DocChangeKind::Reset as i32
                    || change.document_name.contains(&collection_path);
                if pass && tx.send(Ok(change)).await.is_err() {
                    break;
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

/// Parse a Firestore resource name into a `DocumentPath`.
///
/// Expected format: `projects/{project_id}/databases/(default)/documents/{collection_path}/{document_id}`
fn parse_document_name(name: &str, _expected_project_id: &str) -> Result<DocumentPath, Status> {
    let prefix = "projects/";
    if !name.starts_with(prefix) {
        return Err(Status::invalid_argument(format!("invalid document name: {name}")));
    }
    let rest = &name[prefix.len()..];
    let slash_pos = rest
        .find('/')
        .ok_or_else(|| Status::invalid_argument("missing project_id separator"))?;
    let project_id_str = &rest[..slash_pos];
    let rest = &rest[slash_pos + 1..];

    let doc_prefix = "databases/(default)/documents/";
    if !rest.starts_with(doc_prefix) {
        return Err(Status::invalid_argument("invalid database/documents path"));
    }
    let doc_path = &rest[doc_prefix.len()..];

    let last_slash = doc_path
        .rfind('/')
        .ok_or_else(|| Status::invalid_argument("missing document_id"))?;
    let collection_path = &doc_path[..last_slash];
    let document_id = &doc_path[last_slash + 1..];

    let pid = ProjectId::new(project_id_str)
        .map_err(|e| Status::invalid_argument(format!("invalid project_id: {e}")))?;

    Ok(DocumentPath {
        project_id: pid,
        collection_path: collection_path.to_string(),
        document_id: document_id.to_string(),
    })
}

/// Extract project_id from a parent path like:
/// `projects/{project_id}/databases/(default)/documents[/...]`
fn parse_project_id_from_parent(parent: &str) -> Result<String, Status> {
    let prefix = "projects/";
    if !parent.starts_with(prefix) {
        return Err(Status::invalid_argument(format!("invalid parent: {parent}")));
    }
    let rest = &parent[prefix.len()..];
    let slash_pos = rest
        .find('/')
        .ok_or_else(|| Status::invalid_argument("missing project_id in parent"))?;
    Ok(rest[..slash_pos].to_string())
}

/// Build the collection path from a parent and collection_id.
///
/// Parent formats:
///   `projects/{pid}/databases/(default)/documents` → collection_path = collection_id
///   `projects/{pid}/databases/(default)/documents/a/b` → collection_path = "a/b/{collection_id}"
fn build_collection_path(parent: &str, collection_id: &str) -> Result<String, Status> {
    let doc_marker = "databases/(default)/documents";
    let marker_pos = parent
        .find(doc_marker)
        .ok_or_else(|| Status::invalid_argument("parent missing databases/(default)/documents"))?;
    let after_documents = &parent[marker_pos + doc_marker.len()..];

    if after_documents.is_empty() {
        // Root: collection_path = collection_id
        Ok(collection_id.to_string())
    } else {
        // Nested path: strip leading slash, append collection_id
        let nested = after_documents.trim_start_matches('/');
        Ok(format!("{nested}/{collection_id}"))
    }
}

/// Start the agent server in-process, binding to `listen_addr`.
///
/// Spawns the tonic serve loop in a background task and returns the bound
/// `SocketAddr`. Used by the test harness to start an agent without a separate
/// process.
pub async fn serve(
    project_id: String,
    storage: Arc<PostgresBackendAdapter>,
    notify_bridge: Arc<AgentNotifyBridge>,
    tls: ServerTlsConfig,
    listen_addr: &str,
) -> Result<SocketAddr, Box<dyn std::error::Error + Send + Sync>> {
    let service = StorageAgentService::new(project_id, storage, notify_bridge);
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .tls_config(tls)
            .unwrap()
            .add_service(StorageAgentServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    Ok(addr)
}

/// Start the mTLS gRPC server from environment config.
///
/// 1. Connects to Postgres (probe logs "connected to Postgres" before this is called).
/// 2. Reads TLS cert/key/CA from the paths in config.
/// 3. Starts tonic with `ServerTlsConfig` requiring client certificates.
/// 4. Listens for SIGTERM; on receipt, drains in-flight RPCs, logs "shutdown complete",
///    and exits with code 0.
pub async fn run(config: AgentConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(config.max_conns)
        .connect(&config.db_dsn)
        .await?;

    let cert_pem = std::fs::read_to_string(&config.cert_path)?;
    let key_pem = std::fs::read_to_string(&config.key_path)?;
    let ca_pem = std::fs::read_to_string(&config.ca_path)?;

    let identity = Identity::from_pem(cert_pem, key_pem);
    let ca_cert = Certificate::from_pem(ca_pem);

    let tls = ServerTlsConfig::new().identity(identity).client_ca_root(ca_cert);

    let storage = Arc::new(PostgresBackendAdapter::new_from_pool(pool.clone()));
    let bridge = Arc::new(AgentNotifyBridge::new(pool.clone(), config.project_id.clone()));
    let service = StorageAgentService::new(config.project_id, storage, bridge);

    // Spawn background transaction sweeper — deletes expired active transactions
    // every 30 seconds using the same 60-second TTL as commit_transaction.
    let sweeper = AgentTransactionSweeper::new(
        pool,
        60,
        std::time::Duration::from_secs(30),
    );
    let _sweep_handle = sweeper.spawn();

    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    let addr = listener.local_addr()?;
    info!("listening on {addr}");

    let mut sigterm = tokio::signal::unix::signal(
        tokio::signal::unix::SignalKind::terminate(),
    )?;

    tonic::transport::Server::builder()
        .tls_config(tls)?
        .add_service(StorageAgentServer::new(service))
        .serve_with_incoming_shutdown(
            TcpListenerStream::new(listener),
            async move {
                sigterm.recv().await;
                tracing::info!("received SIGTERM — initiating graceful shutdown");
            },
        )
        .await?;

    info!("shutdown complete");
    Ok(())
}
