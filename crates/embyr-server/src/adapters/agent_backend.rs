//! AgentBackendAdapter — proxies BackendAdapter calls over mTLS StorageAgent gRPC.
//!
//! Security invariant: For agent-mode projects, the customer DB credentials never
//! appear in the system DB. The SaaS connects to the agent via mTLS gRPC and the
//! agent manages its own Postgres connection in the customer VPC.

use std::collections::BTreeMap;

use async_trait::async_trait;
use embyr_core::{
    domain::{
        document::{CollectionPath, DocumentPath, FirestoreDocument, WriteResult},
        field_value::FieldValue,
        project::ProjectId,
        query::StructuredQuery,
        transaction::{TransactionId, TransactionOptions},
    },
    error::CoreError,
    storage::backend_adapter::{BackendAdapter, Write, WritePrecondition},
};
use embyr_proto::agent::{
    storage_agent_client::StorageAgentClient,
    BeginTransactionRequest, CommitRequest, CreateDocumentRequest, DeleteDocumentRequest,
    Document as AgentDocument, GetDocumentRequest, RollbackRequest,
    UpdateDocumentRequest, Value as AgentValue, Write as AgentWrite,
    Precondition as AgentPrecondition,
};
use prost_types::Timestamp;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};

/// Adapter that proxies all BackendAdapter calls to a remote agent via mTLS gRPC.
pub struct AgentBackendAdapter {
    client: StorageAgentClient<Channel>,
    endpoint: String,
}

impl AgentBackendAdapter {
    /// Build an mTLS gRPC channel to `endpoint` (format: "host:port").
    ///
    /// `ca_pem`          — PEM bytes of the CA that signed the agent's server cert.
    /// `client_cert_pem` — PEM bytes of the SaaS client certificate.
    /// `client_key_pem`  — PEM bytes of the SaaS client private key.
    pub async fn new(
        endpoint: &str,
        ca_pem: &[u8],
        client_cert_pem: &[u8],
        client_key_pem: &[u8],
    ) -> Result<Self, CoreError> {
        let tls = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(ca_pem))
            .identity(Identity::from_pem(client_cert_pem, client_key_pem))
            .domain_name("localhost");

        let channel = Channel::from_shared(format!("https://{}", endpoint))
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?
            .tls_config(tls)
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?
            .connect()
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        Ok(Self {
            client: StorageAgentClient::new(channel),
            endpoint: endpoint.to_string(),
        })
    }

    /// Return the agent endpoint address (host:port).
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

// ---------------------------------------------------------------------------
// Domain ↔ proto conversion helpers
// ---------------------------------------------------------------------------

fn field_value_to_agent_value(fv: &FieldValue) -> AgentValue {
    use embyr_proto::agent::{ArrayValue, MapValue, NullValue, value::ValueType};
    let value_type = match fv {
        FieldValue::Null => ValueType::NullValue(NullValue::NullValue as i32),
        FieldValue::Boolean(b) => ValueType::BooleanValue(*b),
        FieldValue::Integer(i) => ValueType::IntegerValue(*i),
        FieldValue::Double(d) => ValueType::DoubleValue(*d),
        FieldValue::Timestamp(s, n) => ValueType::TimestampValue(Timestamp {
            seconds: *s,
            nanos: *n,
        }),
        FieldValue::String(s) => ValueType::StringValue(s.clone()),
        FieldValue::Bytes(b) => ValueType::BytesValue(b.clone()),
        FieldValue::Reference(r) => ValueType::ReferenceValue(r.clone()),
        FieldValue::Array(arr) => ValueType::ArrayValue(ArrayValue {
            values: arr.iter().map(field_value_to_agent_value).collect(),
        }),
        FieldValue::Map(m) => ValueType::MapValue(MapValue {
            fields: m
                .iter()
                .map(|(k, v)| (k.clone(), field_value_to_agent_value(v)))
                .collect(),
        }),
    };
    AgentValue {
        value_type: Some(value_type),
    }
}

fn agent_value_to_field_value(v: &AgentValue) -> Option<FieldValue> {
    use embyr_proto::agent::value::ValueType;
    match v.value_type.as_ref()? {
        ValueType::NullValue(_) => Some(FieldValue::Null),
        ValueType::BooleanValue(b) => Some(FieldValue::Boolean(*b)),
        ValueType::IntegerValue(i) => Some(FieldValue::Integer(*i)),
        ValueType::DoubleValue(d) => Some(FieldValue::Double(*d)),
        ValueType::TimestampValue(ts) => {
            Some(FieldValue::Timestamp(ts.seconds, ts.nanos))
        }
        ValueType::StringValue(s) => Some(FieldValue::String(s.clone())),
        ValueType::BytesValue(b) => Some(FieldValue::Bytes(b.clone())),
        ValueType::ReferenceValue(r) => Some(FieldValue::Reference(r.clone())),
        ValueType::ArrayValue(arr) => {
            let values: Option<Vec<FieldValue>> =
                arr.values.iter().map(agent_value_to_field_value).collect();
            Some(FieldValue::Array(values?))
        }
        ValueType::MapValue(m) => {
            let fields: Option<BTreeMap<String, FieldValue>> = m
                .fields
                .iter()
                .map(|(k, v)| agent_value_to_field_value(v).map(|fv| (k.clone(), fv)))
                .collect();
            Some(FieldValue::Map(fields?))
        }
    }
}

/// Parse a Firestore resource name into (project_id, collection_path, document_id).
/// Format: "projects/{pid}/databases/(default)/documents/{collection}/{doc_id}"
fn parse_agent_doc_name(name: &str) -> Option<(String, String, String)> {
    // Split off "projects/{pid}/databases/(default)/documents/"
    let after_docs = name.split("/documents/").nth(1)?;
    let mut segments: Vec<&str> = after_docs.split('/').collect();
    if segments.is_empty() {
        return None;
    }
    let document_id = segments.pop()?.to_string();
    let collection_path = segments.join("/");

    // Extract project_id
    let mut parts = name.splitn(5, '/');
    let _projects = parts.next()?;
    let project_id = parts.next()?.to_string();
    Some((project_id, collection_path, document_id))
}

fn agent_doc_to_domain(doc: AgentDocument) -> Option<FirestoreDocument> {
    let fields: Option<BTreeMap<String, FieldValue>> = doc
        .fields
        .iter()
        .map(|(k, v)| agent_value_to_field_value(v).map(|fv| (k.clone(), fv)))
        .collect();
    let create_time = doc
        .create_time
        .map(|ts| (ts.seconds, ts.nanos))
        .unwrap_or((0, 0));
    let update_time = doc
        .update_time
        .map(|ts| (ts.seconds, ts.nanos))
        .unwrap_or((0, 0));

    let (project_id_str, collection_path, document_id) = parse_agent_doc_name(&doc.name)?;
    let project_id =
        embyr_core::domain::project::ProjectId::new(&project_id_str).ok()?;
    let path = DocumentPath {
        project_id,
        collection_path,
        document_id,
    };

    Some(FirestoreDocument {
        path,
        fields: fields?,
        create_time,
        update_time,
        version: 0,
    })
}

fn fields_to_agent_map(fields: &BTreeMap<String, FieldValue>) -> std::collections::HashMap<String, AgentValue> {
    fields
        .iter()
        .map(|(k, v)| (k.clone(), field_value_to_agent_value(v)))
        .collect()
}

fn domain_path_to_agent_name(path: &DocumentPath) -> String {
    format!(
        "projects/{}/databases/(default)/documents/{}/{}",
        path.project_id.as_str(),
        path.collection_path,
        path.document_id,
    )
}

fn domain_path_to_agent_parent(path: &DocumentPath) -> String {
    format!(
        "projects/{}/databases/(default)/documents",
        path.project_id.as_str(),
    )
}

fn precondition_to_agent(p: &WritePrecondition) -> AgentPrecondition {
    use embyr_proto::agent::precondition::ConditionType;
    match p {
        WritePrecondition::MustExist => AgentPrecondition {
            condition_type: Some(ConditionType::Exists(true)),
        },
        WritePrecondition::MustNotExist => AgentPrecondition {
            condition_type: Some(ConditionType::Exists(false)),
        },
        WritePrecondition::UpdateTime(s, n) => AgentPrecondition {
            condition_type: Some(ConditionType::UpdateTime(Timestamp {
                seconds: *s,
                nanos: *n,
            })),
        },
    }
}

fn grpc_err(e: tonic::Status) -> CoreError {
    CoreError::BackendUnavailable(format!("agent gRPC error: {}", e))
}

// ---------------------------------------------------------------------------
// BackendAdapter implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl BackendAdapter for AgentBackendAdapter {
    async fn get_document(
        &self,
        path: &DocumentPath,
    ) -> Result<Option<FirestoreDocument>, CoreError> {
        let name = domain_path_to_agent_name(path);
        let req = GetDocumentRequest {
            name,
            ..Default::default()
        };
        let mut client = self.client.clone();
        match client.get_document(req).await {
            Ok(resp) => Ok(agent_doc_to_domain(resp.into_inner())),
            Err(s) if s.code() == tonic::Code::NotFound => Ok(None),
            Err(s) => Err(grpc_err(s)),
        }
    }

    async fn create_document(
        &self,
        path: &DocumentPath,
        fields: BTreeMap<String, FieldValue>,
    ) -> Result<WriteResult, CoreError> {
        let parent = domain_path_to_agent_parent(path);
        let doc = AgentDocument {
            name: String::new(),
            fields: fields_to_agent_map(&fields),
            ..Default::default()
        };
        let req = CreateDocumentRequest {
            parent,
            collection_id: path.collection_path.clone(),
            document_id: path.document_id.clone(),
            document: Some(doc),
            ..Default::default()
        };
        let mut client = self.client.clone();
        let resp = client.create_document(req).await.map_err(grpc_err)?;
        let doc = resp.into_inner();
        let update_time = doc
            .update_time
            .map(|ts| (ts.seconds, ts.nanos))
            .unwrap_or((0, 0));
        let create_time = doc.create_time.map(|ts| (ts.seconds, ts.nanos));
        Ok(WriteResult {
            update_time,
            create_time,
        })
    }

    async fn update_document(
        &self,
        path: &DocumentPath,
        fields: BTreeMap<String, FieldValue>,
        precondition: Option<WritePrecondition>,
    ) -> Result<WriteResult, CoreError> {
        let name = domain_path_to_agent_name(path);
        let doc = AgentDocument {
            name,
            fields: fields_to_agent_map(&fields),
            ..Default::default()
        };
        let req = UpdateDocumentRequest {
            document: Some(doc),
            current_document: precondition.as_ref().map(precondition_to_agent),
            ..Default::default()
        };
        let mut client = self.client.clone();
        let resp = client.update_document(req).await.map_err(grpc_err)?;
        let doc = resp.into_inner();
        let update_time = doc
            .update_time
            .map(|ts| (ts.seconds, ts.nanos))
            .unwrap_or((0, 0));
        let create_time = doc.create_time.map(|ts| (ts.seconds, ts.nanos));
        Ok(WriteResult {
            update_time,
            create_time,
        })
    }

    async fn delete_document(
        &self,
        path: &DocumentPath,
        precondition: Option<WritePrecondition>,
    ) -> Result<(), CoreError> {
        let name = domain_path_to_agent_name(path);
        let req = DeleteDocumentRequest {
            name,
            current_document: precondition.as_ref().map(precondition_to_agent),
        };
        let mut client = self.client.clone();
        client.delete_document(req).await.map_err(grpc_err)?;
        Ok(())
    }

    async fn run_query(
        &self,
        collection: &CollectionPath,
        _query: &StructuredQuery,
        _transaction_id: Option<&TransactionId>,
    ) -> Result<Vec<FirestoreDocument>, CoreError> {
        use embyr_proto::agent::{
            RunQueryRequest, StructuredQuery as AgentStructuredQuery,
            structured_query::CollectionSelector,
            run_query_request::QueryType,
        };
        let parent = format!(
            "projects/{}/databases/(default)/documents",
            collection.project_id.as_str()
        );
        let sq = AgentStructuredQuery {
            from: vec![CollectionSelector {
                collection_id: collection.collection_path.clone(),
                all_descendants: false,
            }],
        };
        let req = RunQueryRequest {
            parent,
            query_type: Some(QueryType::StructuredQuery(sq)),
            ..Default::default()
        };
        let mut client = self.client.clone();
        let mut stream = client
            .run_query(req)
            .await
            .map_err(grpc_err)?
            .into_inner();
        let mut docs = Vec::new();
        while let Some(resp) = {
            use tokio_stream::StreamExt;
            stream.next().await
        } {
            let resp = resp.map_err(grpc_err)?;
            if let Some(doc) = resp.document {
                if let Some(domain_doc) = agent_doc_to_domain(doc) {
                    docs.push(domain_doc);
                }
            }
        }
        Ok(docs)
    }

    async fn begin_transaction(
        &self,
        project_id: &ProjectId,
        _options: TransactionOptions,
    ) -> Result<TransactionId, CoreError> {
        let req = BeginTransactionRequest {
            database: format!(
                "projects/{}/databases/(default)",
                project_id.as_str()
            ),
            ..Default::default()
        };
        let mut client = self.client.clone();
        let resp = client.begin_transaction(req).await.map_err(grpc_err)?;
        Ok(TransactionId(resp.into_inner().transaction))
    }

    async fn commit_transaction(
        &self,
        project_id: &ProjectId,
        transaction_id: &TransactionId,
        writes: Vec<Write>,
    ) -> Result<Vec<WriteResult>, CoreError> {
        let agent_writes: Vec<AgentWrite> = writes
            .into_iter()
            .filter_map(|w| match w {
                Write::Update { path, fields, precondition, .. } => {
                    let name = domain_path_to_agent_name(&path);
                    let doc = AgentDocument {
                        name,
                        fields: fields_to_agent_map(&fields),
                        ..Default::default()
                    };
                    Some(AgentWrite {
                        operation: Some(embyr_proto::agent::write::Operation::Update(doc)),
                        current_document: precondition.as_ref().map(precondition_to_agent),
                        ..Default::default()
                    })
                }
                Write::Delete { path, precondition, .. } => {
                    let name = domain_path_to_agent_name(&path);
                    Some(AgentWrite {
                        operation: Some(embyr_proto::agent::write::Operation::Delete(name)),
                        current_document: precondition.as_ref().map(precondition_to_agent),
                        ..Default::default()
                    })
                }
                Write::Transform { .. } => None,
            })
            .collect();

        let req = CommitRequest {
            database: format!(
                "projects/{}/databases/(default)",
                project_id.as_str()
            ),
            writes: agent_writes,
            transaction: transaction_id.0.clone(),
        };
        let mut client = self.client.clone();
        let resp = client.commit(req).await.map_err(grpc_err)?;
        let results: Vec<WriteResult> = resp
            .into_inner()
            .write_results
            .into_iter()
            .map(|wr| WriteResult {
                update_time: wr
                    .update_time
                    .map(|ts| (ts.seconds, ts.nanos))
                    .unwrap_or((0, 0)),
                create_time: None,
            })
            .collect();
        Ok(results)
    }

    async fn rollback_transaction(
        &self,
        project_id: &ProjectId,
        transaction_id: &TransactionId,
    ) -> Result<(), CoreError> {
        let req = RollbackRequest {
            database: format!(
                "projects/{}/databases/(default)",
                project_id.as_str()
            ),
            transaction: transaction_id.0.clone(),
        };
        let mut client = self.client.clone();
        client.rollback(req).await.map_err(grpc_err)?;
        Ok(())
    }

    async fn probe(&self) -> Result<(), CoreError> {
        // Probe: attempt a GetDocument on a sentinel path. The agent must be reachable.
        // NotFound is acceptable (the path may not exist) — only transport errors fail.
        let req = GetDocumentRequest {
            name: format!(
                "projects/__probe__/databases/(default)/documents/__probe__/__probe__"
            ),
            ..Default::default()
        };
        let mut client = self.client.clone();
        match client.get_document(req).await {
            Ok(_) => Ok(()),
            Err(s) if s.code() == tonic::Code::NotFound => Ok(()),
            Err(s) if s.code() == tonic::Code::Unimplemented => Ok(()),
            Err(s) => Err(CoreError::BackendUnavailable(format!(
                "agent probe failed: {}",
                s
            ))),
        }
    }
}
