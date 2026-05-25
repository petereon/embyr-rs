use std::sync::Arc;

use tonic::{Request, Response, Status};

use embyr_core::{
    auth::{argon2, blake3, ecies},
    domain::project::CredentialCacheKey,
    error::CoreError,
};
use embyr_proto::firestore::{
    firestore_server::Firestore, BatchGetDocumentsRequest, BatchGetDocumentsResponse,
    BeginTransactionRequest, BeginTransactionResponse, CommitRequest, CommitResponse,
    CreateDocumentRequest, DeleteDocumentRequest, Document, GetDocumentRequest, ListenRequest,
    ListenResponse, RollbackRequest, RunQueryRequest, RunQueryResponse, UpdateDocumentRequest,
};

use crate::{
    adapters::{
        credential_cache::{CachedEntry, CredentialCache, SharedBackendAdapter},
        postgres_backend::PostgresBackendAdapter,
        system_db::SystemDb,
    },
    encoding::firestore_proto::document_to_proto,
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
        _: Request<CreateDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        Err(Status::unimplemented("implemented in step 03-01"))
    }

    async fn update_document(
        &self,
        _: Request<UpdateDocumentRequest>,
    ) -> Result<Response<Document>, Status> {
        Err(Status::unimplemented("implemented in step 03-01"))
    }

    async fn delete_document(
        &self,
        _: Request<DeleteDocumentRequest>,
    ) -> Result<Response<()>, Status> {
        Err(Status::unimplemented("implemented in step 03-01"))
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
        CoreError::Unauthenticated => Status::unauthenticated(e.to_string()),
        CoreError::PermissionDenied(_) => Status::permission_denied(e.to_string()),
        CoreError::InvalidArgument(_) => Status::invalid_argument(e.to_string()),
        CoreError::OccConflict => Status::aborted(e.to_string()),
        CoreError::ResourceExhausted(_) => Status::resource_exhausted(e.to_string()),
        CoreError::FailedPrecondition(_) => Status::failed_precondition(e.to_string()),
        _ => Status::internal(e.to_string()),
    }
}
