//! AgentBackendAdapter — proxies BackendAdapter calls over mTLS StorageAgent gRPC.
//!
//! Security invariant: For agent-mode projects, the customer DB credentials never
//! appear in the system DB. The SaaS connects to the agent via mTLS gRPC and the
//! agent manages its own Postgres connection in the customer VPC.

use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use embyr_core::{
    domain::{
        document::{CollectionPath, DocumentPath, FirestoreDocument, WriteResult},
        field_value::FieldValue,
        project::ProjectId,
        query::{AggregateValue, Aggregation, AggregationKind, AggregationQuery, FilterOp, QueryFilter, StructuredQuery},
        transaction::{TransactionId, TransactionOptions},
    },
    error::CoreError,
    storage::{
        backend_adapter::{
            BackendAdapter, FieldTransform as CoreFieldTransform, Write, WritePrecondition,
        },
        change_feed::{ChangeEvent, ChangeFeed},
    },
};
use embyr_proto::agent::{
    filter::FilterType as AgentFilterType,
    storage_agent_client::StorageAgentClient,
    BeginTransactionRequest, CommitRequest, CompositeFilterOp as AgentCompositeFilterOp,
    CompositeFilterProto as AgentCompositeFilterProto, CreateDocumentRequest,
    DeleteDocumentRequest, Document as AgentDocument, FieldFilterOp as AgentFieldFilterOp,
    FieldFilterProto as AgentFieldFilterProto, FieldTransform as AgentFieldTransform,
    Filter as AgentFilter, GetDocumentRequest, RollbackRequest, ServerValue as AgentServerValue,
    Transform as AgentTransform, UpdateDocumentRequest, Value as AgentValue, Write as AgentWrite,
    Precondition as AgentPrecondition, DocChange, DocChangeKind, SubscribeRequest,
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

/// Translate a domain `FieldTransform` into the agent proto's own
/// `FieldTransform` — the encode-side symmetric counterpart to
/// `embyr-agent`'s own `translate_field_transforms` (ADR-057 § Decision 3).
/// Reuses `field_value_to_agent_value` unchanged for every operand.
fn field_transform_to_agent(t: &CoreFieldTransform) -> AgentFieldTransform {
    use embyr_proto::agent::{field_transform::TransformType, ArrayValue as AgentArrayValue};
    let (field_path, transform_type) = match t {
        CoreFieldTransform::ServerTimestamp(p) => (
            p.clone(),
            TransformType::SetToServerValue(AgentServerValue::RequestTime as i32),
        ),
        CoreFieldTransform::Increment(p, v) => {
            (p.clone(), TransformType::Increment(field_value_to_agent_value(v)))
        }
        CoreFieldTransform::Maximum(p, v) => {
            (p.clone(), TransformType::Maximum(field_value_to_agent_value(v)))
        }
        CoreFieldTransform::Minimum(p, v) => {
            (p.clone(), TransformType::Minimum(field_value_to_agent_value(v)))
        }
        CoreFieldTransform::AppendMissingElements(p, values) => (
            p.clone(),
            TransformType::AppendMissingElements(AgentArrayValue {
                values: values.iter().map(field_value_to_agent_value).collect(),
            }),
        ),
        CoreFieldTransform::RemoveAllFromArray(p, values) => (
            p.clone(),
            TransformType::RemoveAllFromArray(AgentArrayValue {
                values: values.iter().map(field_value_to_agent_value).collect(),
            }),
        ),
    };
    AgentFieldTransform { field_path, transform_type: Some(transform_type) }
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

/// Translate one agent `DocChange` into a backend-neutral `ChangeEvent`.
/// Anything unintelligible becomes `Reset` (resync) rather than being dropped.
fn doc_change_to_event(change: &DocChange) -> ChangeEvent {
    let kind = DocChangeKind::try_from(change.kind).unwrap_or(DocChangeKind::Unspecified);
    let path = || {
        let (pid, collection_path, document_id) = parse_agent_doc_name(&change.document_name)?;
        Some(DocumentPath { project_id: ProjectId::new(&pid).ok()?, collection_path, document_id })
    };
    match (kind, path()) {
        (DocChangeKind::Upsert, Some(p)) => ChangeEvent::Changed(p),
        (DocChangeKind::Delete, Some(p)) => ChangeEvent::Deleted(p),
        _ => ChangeEvent::Reset,
    }
}

/// `ChangeFeed` over the agent's `Subscribe` server-stream.
struct AgentChangeFeed {
    stream: tonic::Streaming<DocChange>,
}

#[async_trait]
impl ChangeFeed for AgentChangeFeed {
    async fn next(&mut self) -> Result<ChangeEvent, CoreError> {
        match self.stream.message().await {
            Ok(Some(change)) => Ok(doc_change_to_event(&change)),
            Ok(None) => Err(CoreError::BackendUnavailable("agent subscribe stream closed".into())),
            Err(s) => Err(grpc_err(s)),
        }
    }
}

fn fields_to_agent_map(fields: &BTreeMap<String, FieldValue>) -> HashMap<String, AgentValue> {
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

/// Build the agent RPC's own `parent` string from a `CollectionPath` used as
/// a PARENT PREFIX (ADR-051's own semantic overload for
/// `BackendAdapter::list_collection_ids` — a `CollectionPath` whose
/// `collection_path` is empty means database root) — distinct from
/// `domain_path_to_agent_parent` (&DocumentPath), which never carries a path
/// suffix (ADR-059 § Decision 3).
fn domain_collection_prefix_to_agent_parent(collection: &CollectionPath) -> String {
    let base = format!(
        "projects/{}/databases/(default)/documents",
        collection.project_id.as_str()
    );
    if collection.collection_path.is_empty() {
        base
    } else {
        format!("{base}/{}", collection.collection_path)
    }
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
    CoreError::BackendUnavailable(format!("agent gRPC error: {e}"))
}

/// Translate a domain `FilterOp` to the agent proto's `FieldFilterOp`.
///
/// The agent proto has no NaN comparison operators (see
/// `proto/embyr/agent/v1/storage_agent.proto`'s `FieldFilterOp` enum) —
/// reject rather than silently drop or misrepresent the filter.
fn domain_filter_op_to_agent(op: &FilterOp) -> Result<AgentFieldFilterOp, CoreError> {
    Ok(match op {
        FilterOp::LessThan => AgentFieldFilterOp::LessThan,
        FilterOp::LessThanOrEqual => AgentFieldFilterOp::LessThanOrEqual,
        FilterOp::GreaterThan => AgentFieldFilterOp::GreaterThan,
        FilterOp::GreaterThanOrEqual => AgentFieldFilterOp::GreaterThanOrEqual,
        FilterOp::Equal => AgentFieldFilterOp::Equal,
        FilterOp::NotEqual => AgentFieldFilterOp::NotEqual,
        FilterOp::ArrayContains => AgentFieldFilterOp::ArrayContains,
        FilterOp::In => AgentFieldFilterOp::In,
        FilterOp::NotIn => AgentFieldFilterOp::NotIn,
        FilterOp::ArrayContainsAny => AgentFieldFilterOp::ArrayContainsAny,
        FilterOp::IsNan | FilterOp::IsNotNan | FilterOp::IsNull | FilterOp::IsNotNull => {
            return Err(CoreError::InvalidArgument(
                "IS_NAN/IS_NOT_NAN/IS_NULL/IS_NOT_NULL filters are not supported in backend_mode=agent"
                    .into(),
            ));
        }
    })
}

/// Translate a domain `QueryFilter` — already validated and approved by
/// `check_query_compliance()` upstream — into the agent's own proto
/// `Filter` shape. This is the inverse of `embyr-agent`'s own
/// `proto_filter_to_domain` (crates/embyr-agent/src/server.rs), running the
/// other direction and living in embyr-server: without it, `run_query`
/// forwarded no filter at all to the agent, so an approved ownership-scoped
/// query returned every document in the collection (cross-user leak).
fn domain_filter_to_agent_filter(filter: &QueryFilter) -> Result<AgentFilter, CoreError> {
    match filter {
        QueryFilter::Field(ff) => {
            let op = domain_filter_op_to_agent(&ff.op)? as i32;
            Ok(AgentFilter {
                filter_type: Some(AgentFilterType::FieldFilter(AgentFieldFilterProto {
                    field_path: ff.field_path.clone(),
                    op,
                    value: Some(field_value_to_agent_value(&ff.value)),
                })),
            })
        }
        QueryFilter::Composite(filters) => {
            let translated = filters
                .iter()
                .map(domain_filter_to_agent_filter)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(AgentFilter {
                filter_type: Some(AgentFilterType::CompositeFilter(AgentCompositeFilterProto {
                    op: AgentCompositeFilterOp::And as i32,
                    filters: translated,
                })),
            })
        }
        // firestore-or-filter-support: the agent's own internal
        // `embyr.agent.v1.CompositeFilterOp` proto has no `Or` variant —
        // backend_mode=agent is explicitly deferred (feature-delta.md §
        // Out of Scope). A clean rejection here, not a silent
        // mistranslation to AND (which would return the intersection
        // instead of the union — this SAME function's own doc comment
        // above already documents a prior cross-user-leak incident from a
        // different silent-mistranslation bug).
        QueryFilter::CompositeOr(_) => Err(CoreError::FailedPrecondition(
            "OR filters are not supported when backend_mode=agent".into(),
        )),
    }
}

// ---------------------------------------------------------------------------
// BackendAdapter implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl BackendAdapter for AgentBackendAdapter {
    async fn get_document(
        &self,
        path: &DocumentPath,
        // firestore-transaction-read-consistency: backend_mode=agent is
        // explicitly deferred (feature-delta.md § Out of Scope) — the
        // internal `embyr.agent.v1.GetDocumentRequest` proto has no
        // transaction-carrying field yet. Mirrors this adapter's own
        // already-existing `run_query`/`run_aggregation_query`
        // `_transaction_id`-ignored pattern.
        _transaction_id: Option<&TransactionId>,
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
            transform_results: vec![],
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
            transform_results: vec![],
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
        query: &StructuredQuery,
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
        // Forward the caller's already-compliance-checked filter to the
        // agent (security fix: previously hardcoded to None, so agent-mode
        // ownership-scoped queries leaked every user's documents).
        let filter = query
            .filter
            .as_ref()
            .map(domain_filter_to_agent_filter)
            .transpose()?;
        let sq = AgentStructuredQuery {
            from: vec![CollectionSelector {
                collection_id: collection.collection_path.clone(),
                // Pre-existing hardcoded false, unchanged — out of scope for
                // this fix (cross-user filter-forwarding leak only).
                all_descendants: false,
            }],
            filter,
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

    /// Proxies `embyr-agent`'s `RunAggregationQuery` RPC. COUNT-only requests
    /// use the original `count` wire shape (works against any agent version);
    /// anything with SUM/AVG sends `aggregations` and reads `results`. An
    /// agent that predates those fields ignores them and answers with an empty
    /// `results`, mapped to a "not supported" FailedPrecondition (-> client
    /// `Unimplemented`, ADR-041) as before. Filter translation reuses
    /// `domain_filter_to_agent_filter`, same as `run_query`.
    async fn run_aggregation_query(
        &self,
        collection: &CollectionPath,
        query: &AggregationQuery,
        _transaction_id: Option<&TransactionId>,
    ) -> Result<Vec<AggregateValue>, CoreError> {
        let count_only = query
            .aggregations
            .iter()
            .all(|a| matches!(a.kind, AggregationKind::Count(_)));

        use embyr_proto::agent::{
            run_aggregation_query_request::QueryType,
            structured_query::CollectionSelector,
            RunAggregationQueryRequest as AgentAggregationRequest,
            StructuredQuery as AgentStructuredQuery,
        };
        let parent = format!(
            "projects/{}/databases/(default)/documents",
            collection.project_id.as_str()
        );
        let filter = query
            .query
            .filter
            .as_ref()
            .map(domain_filter_to_agent_filter)
            .transpose()?;
        let sq = AgentStructuredQuery {
            from: vec![CollectionSelector {
                collection_id: collection.collection_path.clone(),
                all_descendants: query.query.all_descendants,
            }],
            filter,
        };
        let req = AgentAggregationRequest {
            parent,
            query_type: Some(QueryType::StructuredQuery(sq)),
            aggregations: if count_only { vec![] } else { aggregations_to_proto(&query.aggregations) },
        };
        let mut client = self.client.clone();
        let resp = client
            .run_aggregation_query(req)
            .await
            .map_err(grpc_err)?
            .into_inner();
        if count_only {
            // One agent round trip serves every COUNT; each applies its own `up_to`.
            return Ok(query
                .aggregations
                .iter()
                .map(|a| match a.kind {
                    AggregationKind::Count(Some(up_to)) => AggregateValue::Count(resp.count.min(up_to)),
                    _ => AggregateValue::Count(resp.count),
                })
                .collect());
        }
        aggregation_results_from_proto(&query.aggregations, resp.results)
    }

    /// agent-mode-list-collection-ids (Slice 01, US-01, ADR-059 § Decision
    /// 3): the driven-port contract is a single bounded call (`limit`,
    /// `offset`), but the agent's own new `ListCollectionIds` RPC is itself
    /// paginated (≤100 per call, matching `list_documents`'s existing
    /// defensive clamp). This loop flattens the agent's own pagination into
    /// a single `Vec<String>` satisfying an arbitrary `limit`, transparently
    /// to the caller — verified correct at the exact 100/101-item boundary
    /// worked through in the ADR (forwarding `limit` directly as the wire
    /// `page_size` in one round trip is demonstrated incorrect there).
    async fn list_collection_ids(
        &self,
        parent: &CollectionPath,
        limit: i32,
        offset: i32,
    ) -> Result<Vec<String>, CoreError> {
        use embyr_proto::agent::{ListCollectionIdsRequest, ListCollectionIdsResponse};

        let parent_str = domain_collection_prefix_to_agent_parent(parent);
        let mut collected: Vec<String> = Vec::new();
        let mut page_token = embyr_core::pagination::encode_page_token(offset.max(0) as u32);
        let mut client = self.client.clone();

        loop {
            let req = ListCollectionIdsRequest {
                parent: parent_str.clone(),
                page_size: 100, // agent's own per-call ceiling
                page_token,
            };
            let resp: ListCollectionIdsResponse =
                client.list_collection_ids(req).await.map_err(grpc_err)?.into_inner();
            collected.extend(resp.collection_ids);

            if resp.next_page_token.is_empty() || collected.len() as i64 >= limit as i64 {
                break;
            }
            page_token = resp.next_page_token;
        }

        collected.truncate(limit.max(0) as usize);
        Ok(collected)
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
            .map(|w| match w {
                Write::Update { path, fields, precondition, transforms, .. } => {
                    let name = domain_path_to_agent_name(&path);
                    let doc = AgentDocument {
                        name,
                        fields: fields_to_agent_map(&fields),
                        ..Default::default()
                    };
                    AgentWrite {
                        operation: Some(embyr_proto::agent::write::Operation::Update(doc)),
                        current_document: precondition.as_ref().map(precondition_to_agent),
                        update_transforms: transforms.iter().map(field_transform_to_agent).collect(),
                        ..Default::default()
                    }
                }
                Write::Delete { path, precondition, .. } => {
                    let name = domain_path_to_agent_name(&path);
                    AgentWrite {
                        operation: Some(embyr_proto::agent::write::Operation::Delete(name)),
                        current_document: precondition.as_ref().map(precondition_to_agent),
                        ..Default::default()
                    }
                }
                Write::Transform { path, transforms } => {
                    let name = domain_path_to_agent_name(&path);
                    AgentWrite {
                        operation: Some(embyr_proto::agent::write::Operation::Transform(AgentTransform {
                            document: name,
                            field_transforms: transforms.iter().map(field_transform_to_agent).collect(),
                        })),
                        // Write::Transform has no precondition field of its own —
                        // pre-existing gap, ADR-052 § Consequences, not this
                        // feature's scope.
                        current_document: None,
                        ..Default::default()
                    }
                }
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
                // Agent-mode has no transform wire representation at all
                // (backend_mode=agent is out of v1 scope, ADR-052 §
                // Alternatives Considered) — permanently empty, not a
                // temporary placeholder.
                transform_results: vec![],
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
            name: "projects/__probe__/databases/(default)/documents/__probe__/__probe__"
                .to_string(),
            ..Default::default()
        };
        let mut client = self.client.clone();
        match client.get_document(req).await {
            Ok(_) => Ok(()),
            Err(s) if s.code() == tonic::Code::NotFound => Ok(()),
            Err(s) if s.code() == tonic::Code::Unimplemented => Ok(()),
            Err(s) => Err(CoreError::BackendUnavailable(format!("agent probe failed: {s}"))),
        }
    }

    async fn open_change_feed(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<Box<dyn ChangeFeed>>, CoreError> {
        // The agent's `Subscribe` filters by SUBSTRING match of
        // `collection_path` against each change's full document name and
        // rejects an empty value. A project-wide feed (one registry channel
        // per project) therefore subscribes with the project's documents
        // prefix, which every document name of the project contains. Zero
        // proto/agent change; works with already-deployed agents.
        let req = SubscribeRequest {
            collection_path: format!(
                "projects/{}/databases/(default)/documents/",
                project_id.as_str()
            ),
        };
        let mut client = self.client.clone();
        let stream = client.subscribe(req).await.map_err(grpc_err)?.into_inner();
        Ok(Some(Box::new(AgentChangeFeed { stream })))
    }
}

/// Domain aggregations -> wire ops (aliases stay server-side; the agent
/// answers by position). `up_to` 0 on the wire means "no cap".
fn aggregations_to_proto(aggregations: &[Aggregation]) -> Vec<embyr_proto::agent::AggregationOp> {
    use embyr_proto::agent::{aggregation_op::Op, AggregationCount, AggregationField, AggregationOp};
    let field = |fp: &String| AggregationField { field_path: fp.clone() };
    aggregations
        .iter()
        .map(|a| AggregationOp {
            op: Some(match &a.kind {
                AggregationKind::Count(up_to) => Op::Count(AggregationCount { up_to: up_to.unwrap_or(0) }),
                AggregationKind::Sum(fp) => Op::Sum(field(fp)),
                AggregationKind::Avg(fp) => Op::Avg(field(fp)),
            }),
        })
        .collect()
}

/// Wire results -> domain values, one per requested aggregation. Empty
/// `results` means the agent predates SUM/AVG support (it ignored
/// `aggregations`): report the same "not supported" error as before. Any other
/// shape mismatch is a protocol violation, never a silently wrong value.
fn aggregation_results_from_proto(
    aggregations: &[Aggregation],
    results: Vec<embyr_proto::agent::AggregationResult>,
) -> Result<Vec<AggregateValue>, CoreError> {
    use embyr_proto::agent::aggregation_result::Value;
    if results.is_empty() {
        return Err(CoreError::FailedPrecondition(
            "SUM/AVG aggregation is not supported by this embyr-agent version; upgrade the agent".into(),
        ));
    }
    let malformed = || CoreError::BackendUnavailable("agent returned malformed aggregation results".into());
    if results.len() != aggregations.len() {
        return Err(malformed());
    }
    aggregations
        .iter()
        .zip(results)
        .map(|(a, r)| match (&a.kind, r.value) {
            (AggregationKind::Count(_), Some(Value::Count(n))) => Ok(AggregateValue::Count(n)),
            (AggregationKind::Sum(_), Some(Value::Sum(d))) => Ok(AggregateValue::Sum(d)),
            (AggregationKind::Avg(_), Some(Value::Avg(d))) => Ok(AggregateValue::Avg(Some(d))),
            (AggregationKind::Avg(_), None) => Ok(AggregateValue::Avg(None)),
            _ => Err(malformed()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! firestore-or-filter-support (Slice 01, US-01, AC-OR-07): a direct
    //! unit test of `domain_filter_to_agent_filter`'s own pure translation
    //! logic — a full end-to-end agent-binary harness would be
    //! disproportionate for testing one function's own `Err` arm.
    use super::*;
    use embyr_core::domain::query::FieldFilter;

    fn field_filter(field_path: &str) -> QueryFilter {
        QueryFilter::Field(FieldFilter {
            field_path: field_path.to_string(),
            op: FilterOp::Equal,
            value: FieldValue::String("x".to_string()),
        })
    }

    #[test]
    fn composite_or_is_cleanly_rejected_not_silently_mistranslated_to_and() {
        let filter = QueryFilter::CompositeOr(vec![field_filter("a"), field_filter("b")]);
        let result = domain_filter_to_agent_filter(&filter);
        assert!(
            result.is_err(),
            "OR filters must be cleanly rejected for backend_mode=agent, not silently \
             translated to AND (which would return the intersection instead of the union)"
        );
    }

    #[test]
    fn composite_and_still_translates_successfully() {
        let filter = QueryFilter::Composite(vec![field_filter("a"), field_filter("b")]);
        let result = domain_filter_to_agent_filter(&filter);
        assert!(
            result.is_ok(),
            "AND filters must continue to translate successfully — regression guard"
        );
    }

    fn unary_filter(op: FilterOp) -> QueryFilter {
        QueryFilter::Field(FieldFilter {
            field_path: "status".to_string(),
            op,
            value: FieldValue::Null,
        })
    }

    #[test]
    fn is_null_is_cleanly_rejected_for_backend_mode_agent() {
        let result = domain_filter_to_agent_filter(&unary_filter(FilterOp::IsNull));
        assert!(result.is_err(), "IS_NULL must be cleanly rejected for backend_mode=agent");
    }

    #[test]
    fn is_not_null_is_cleanly_rejected_for_backend_mode_agent() {
        let result = domain_filter_to_agent_filter(&unary_filter(FilterOp::IsNotNull));
        assert!(result.is_err(), "IS_NOT_NULL must be cleanly rejected for backend_mode=agent");
    }

    fn agg(kind: AggregationKind) -> Aggregation {
        Aggregation { alias: "a".into(), kind }
    }

    #[test]
    fn aggregations_round_trip_in_order_with_uncapped_count_as_zero() {
        use embyr_proto::agent::{aggregation_op::Op, aggregation_result::Value, AggregationResult};
        let aggs = [
            agg(AggregationKind::Count(None)),
            agg(AggregationKind::Count(Some(4))),
            agg(AggregationKind::Sum("s".into())),
            agg(AggregationKind::Avg("v".into())),
            agg(AggregationKind::Avg("v".into())),
        ];
        let ops: Vec<_> = aggregations_to_proto(&aggs).into_iter().map(|o| o.op.unwrap()).collect();
        assert!(matches!(&ops[0], Op::Count(c) if c.up_to == 0));
        assert!(matches!(&ops[1], Op::Count(c) if c.up_to == 4));
        assert!(matches!(&ops[2], Op::Sum(f) if f.field_path == "s"));
        assert!(matches!(&ops[3], Op::Avg(f) if f.field_path == "v"));
        let r = |v| AggregationResult { value: v };
        let got = aggregation_results_from_proto(
            &aggs,
            vec![
                r(Some(Value::Count(9))),
                r(Some(Value::Count(4))),
                r(Some(Value::Sum(1.5))),
                r(Some(Value::Avg(0.0))),
                r(None),
            ],
        )
        .unwrap();
        assert_eq!(
            got,
            [
                AggregateValue::Count(9),
                AggregateValue::Count(4),
                AggregateValue::Sum(1.5),
                AggregateValue::Avg(Some(0.0)),
                AggregateValue::Avg(None),
            ]
        );
    }

    #[test]
    fn empty_results_mean_old_agent_and_map_to_not_supported() {
        let aggs = [agg(AggregationKind::Sum("s".into()))];
        assert!(matches!(
            aggregation_results_from_proto(&aggs, vec![]),
            Err(CoreError::FailedPrecondition(_))
        ));
    }

    #[test]
    fn mismatched_or_mistyped_results_are_a_backend_error_not_a_wrong_value() {
        use embyr_proto::agent::{aggregation_result::Value, AggregationResult};
        let aggs = [agg(AggregationKind::Sum("s".into()))];
        let r = |v| AggregationResult { value: v };
        for bad in [
            vec![r(Some(Value::Count(1))), r(Some(Value::Count(1)))],
            vec![r(Some(Value::Count(1)))],
            vec![r(None)],
        ] {
            assert!(matches!(
                aggregation_results_from_proto(&aggs, bad),
                Err(CoreError::BackendUnavailable(_))
            ));
        }
    }

    fn change(kind: DocChangeKind, name: &str) -> DocChange {
        DocChange { kind: kind as i32, document_name: name.to_string(), ..Default::default() }
    }

    #[test]
    fn doc_change_translates_to_neutral_event_and_degrades_to_reset() {
        let name = "projects/p1/databases/(default)/documents/orders/o1";
        let path = DocumentPath {
            project_id: ProjectId::new("p1").unwrap(),
            collection_path: "orders".into(),
            document_id: "o1".into(),
        };
        assert_eq!(doc_change_to_event(&change(DocChangeKind::Upsert, name)), ChangeEvent::Changed(path.clone()));
        assert_eq!(doc_change_to_event(&change(DocChangeKind::Delete, name)), ChangeEvent::Deleted(path));
        assert_eq!(doc_change_to_event(&change(DocChangeKind::Reset, "")), ChangeEvent::Reset);
        // Unintelligible input must resync, never be dropped.
        assert_eq!(doc_change_to_event(&change(DocChangeKind::Upsert, "garbage")), ChangeEvent::Reset);
        assert_eq!(doc_change_to_event(&change(DocChangeKind::Unspecified, name)), ChangeEvent::Reset);
    }
}
