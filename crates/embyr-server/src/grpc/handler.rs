use std::sync::Arc;

use prost_types::Timestamp;
use tonic::{Request, Response, Status};

use embyr_core::{
    auth::{argon2, blake3, ecies},
    domain::project::CredentialCacheKey,
    error::CoreError,
    storage::backend_adapter::WritePrecondition,
};
use embyr_proto::firestore::{
    firestore_server::Firestore, precondition::ConditionType, BatchGetDocumentsRequest,
    BatchGetDocumentsResponse, BeginTransactionRequest, BeginTransactionResponse, CommitRequest,
    CommitResponse, CreateDocumentRequest, DeleteDocumentRequest, Document, GetDocumentRequest,
    ListenRequest, ListenResponse, RollbackRequest, RunQueryRequest, RunQueryResponse,
    UpdateDocumentRequest,
};

use crate::{
    adapters::{
        credential_cache::{CachedEntry, CredentialCache, SharedBackendAdapter},
        postgres_backend::PostgresBackendAdapter,
        system_db::SystemDb,
    },
    encoding::firestore_proto::{document_to_proto, fields_to_proto, proto_fields_to_domain},
};

pub struct FirestoreService {
    pub system_db: Arc<SystemDb>,
    pub credential_cache: Arc<CredentialCache>,
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
    async fn authenticate(
        &self,
        project_id_str: &str,
        api_key: &str,
    ) -> Result<(SharedBackendAdapter, String), Status> {
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
            return Ok(cached);
        }

        // Cache miss — load from system DB.
        let row = self
            .system_db
            .get_project_for_auth(project_id_str)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::unauthenticated("project not found"))?;

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

        if row.status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

        // Decrypt DSN via ECIES.
        let encrypted_dsn = row
            .ecies_encrypted_dsn
            .ok_or_else(|| Status::internal("project has no stored DSN"))?;
        let dsn_bytes = ecies::decrypt(api_key.as_bytes(), &encrypted_dsn)
            .map_err(|e| Status::internal(e.to_string()))?;
        let dsn =
            String::from_utf8(dsn_bytes).map_err(|_| Status::internal("DSN is not valid UTF-8"))?;

        // Build adapter and cache.
        let adapter = PostgresBackendAdapter::new(&dsn)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        let shared: SharedBackendAdapter = Arc::new(adapter);

        self.credential_cache
            .insert(
                cache_key,
                CachedEntry {
                    adapter: Arc::clone(&shared),
                    project_status: row.status.clone(),
                },
            )
            .await;

        Ok((shared, row.status))
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

        let (adapter, status) = self.authenticate(&project_id, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
        }

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

        let (adapter, status) = self.authenticate(&project_id_str, &api_key).await?;
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

        let (adapter, status) = self.authenticate(&project_id_str, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
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

        let (adapter, status) = self.authenticate(&project_id_str, &api_key).await?;
        if status == "suspended" {
            return Err(Status::permission_denied("project is suspended"));
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
        _: Request<BeginTransactionRequest>,
    ) -> Result<Response<BeginTransactionResponse>, Status> {
        Err(Status::unimplemented("implemented in step 06-01"))
    }

    async fn commit(
        &self,
        _: Request<CommitRequest>,
    ) -> Result<Response<CommitResponse>, Status> {
        Err(Status::unimplemented("implemented in step 06-01"))
    }

    async fn rollback(&self, _: Request<RollbackRequest>) -> Result<Response<()>, Status> {
        Err(Status::unimplemented("implemented in step 06-01"))
    }

    type RunQueryStream = tonic::codegen::BoxStream<RunQueryResponse>;

    async fn run_query(
        &self,
        _: Request<RunQueryRequest>,
    ) -> Result<Response<Self::RunQueryStream>, Status> {
        Err(Status::unimplemented("implemented in step 04-01"))
    }

    type ListenStream = tonic::codegen::BoxStream<ListenResponse>;

    async fn listen(
        &self,
        _: Request<tonic::Streaming<ListenRequest>>,
    ) -> Result<Response<Self::ListenStream>, Status> {
        Err(Status::unimplemented("implemented in step 05-01"))
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
        CoreError::ResourceExhausted(_) => Status::resource_exhausted(e.to_string()),
        CoreError::FailedPrecondition(_) => Status::failed_precondition(e.to_string()),
        _ => Status::internal(e.to_string()),
    }
}
