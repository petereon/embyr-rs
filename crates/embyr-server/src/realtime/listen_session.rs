//! One multiplexed `Listen` stream: any number of Query / Documents targets
//! added and removed at any point, one registry subscription, per-target
//! change delivery.
//!
//! Target ids: the client's own `target_id` is kept; `0` means the server
//! assigns one (see `embyr_core::realtime::listen_targets`). Every added
//! target is acknowledged with `TargetChange(ADD)`, then its snapshot, then
//! `CURRENT`. `RemoveTarget` answers `TargetChange(REMOVE)` and stops
//! delivery for that id. A target that fails to add AFTER the first message
//! is reported as `TargetChange(REMOVE, cause)` and the stream lives on; a
//! failure on the FIRST message ends the stream with that status (the
//! pre-multiplexing behaviour). The stream stays open until the client goes
//! away, including after the client half-closes its request side.
//!
//! Rules (ADR-033/063/064) are resolved per target at add time by the same
//! logic the single-target handler used, and re-checked per delivered change.

// Same allowance as grpc/handler.rs: `tonic::Status` is the error type throughout.
#![allow(clippy::result_large_err)]

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Duration,
};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use embyr_core::{
    access_control::{
        check_query_compliance, evaluate, parse_condition, AuthContext, Condition, EvalContext,
        EvaluationOutcome, QueryComplianceOutcome,
    },
    client_identity::VerifiedEndUserIdentity,
    domain::{
        document::{CollectionPath, DocumentPath, FirestoreDocument},
        field_value::FieldValue,
        project::ProjectId,
        query::{
            validate_field_path, OrderBy, OrderDirection, QueryFilter,
            StructuredQuery as DomainQuery,
        },
    },
    realtime::listen_targets::{diff_window, TargetIdError, TargetIds, TargetScope},
};
use embyr_proto::firestore::{
    listen_request, listen_response,
    structured_query::Direction,
    target::{query_target::QueryType, DocumentsTarget, QueryTarget, ResumeType, TargetType},
    target_change::TargetChangeType,
    DocumentChange, DocumentDelete, DocumentRemove, ListenRequest, ListenResponse, Target,
    TargetChange,
};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tonic::Status;

use crate::{
    adapters::{
        credential_cache::SharedBackendAdapter, index_manager::IndexManager, system_db::SystemDb,
    },
    encoding::firestore_proto::document_to_proto,
    grpc::handler::{query_compliance_rejection, sanitize_backend_error, FirestoreService},
    realtime::{
        listen_handler::{
            collection_id_from_query_target, filter_from_query_target, project_id_from_parent,
        },
        listen_registry::{ListenEvent, ListenRegistry},
        resume_token as rt,
    },
};

type ResponseTx = mpsc::Sender<Result<ListenResponse, Status>>;

/// Why a step stopped: the client is gone (nothing to report), or a status
/// to report.
enum Fail {
    Gone,
    Target(Status),
}

impl From<Status> for Fail {
    fn from(status: Status) -> Self {
        Self::Target(status)
    }
}

/// The access decision inputs for one collection, resolved at add time.
struct Rule {
    condition: Option<Condition>,
    bindings: BTreeMap<String, String>,
    /// Neither an exact rule nor a fixed-depth pattern matched at add time:
    /// a recursive-wildcard catch-all may still govern once a concrete
    /// document is known, so resolve it per change (AC-17-247).
    recheck: bool,
}

// A handful of targets per stream: boxing the one big variant buys nothing.
#[allow(clippy::large_enum_variant)]
enum Delivery {
    /// Whole-collection query (no filter/order/limit/offset): every change
    /// in the collection is a change of the result.
    Plain,
    /// Exactly the named documents.
    Documents,
    /// The result depends on more than collection membership. The server
    /// remembers what it sent (`members`: id -> version) and, on a relevant
    /// change, re-runs the query and sends the difference.
    Windowed {
        collection: CollectionPath,
        query: DomainQuery,
        members: BTreeMap<String, i64>,
    },
}

struct ActiveTarget {
    scope: TargetScope,
    delivery: Delivery,
    /// Keyed by collection path (a Documents target can span collections).
    rules: BTreeMap<String, Rule>,
}

struct Snapshot {
    target: ActiveTarget,
    docs: Vec<FirestoreDocument>,
    /// Resume-token anchor: `now + 1s`, taken before the query ran so every
    /// snapshot document is at or before it.
    token_ts: DateTime<Utc>,
}

/// Per-stream dependencies, separate from the target map so a target can be
/// borrowed mutably while these are used.
struct Ctx<'a> {
    adapter: &'a SharedBackendAdapter,
    system_db: &'a Arc<SystemDb>,
    index_manager: &'a IndexManager,
    project_id: String,
    auth_ctx: Option<AuthContext>,
    tx: &'a ResponseTx,
}

struct Session<'a> {
    ctx: Ctx<'a>,
    ids: TargetIds,
    targets: BTreeMap<i32, ActiveTarget>,
}

/// Run a Listen stream to completion. `first_msg` is the already-read first
/// request (it carried the project id used for authentication).
// One call site (grpc/handler.rs); each parameter is a distinct dependency of
// the stream — see the same note on the single-target handler this replaces.
#[allow(clippy::too_many_arguments)]
pub async fn run_listen_session<S>(
    first_msg: ListenRequest,
    mut in_stream: S,
    adapter: &SharedBackendAdapter,
    system_db: &Arc<SystemDb>,
    index_manager: &IndexManager,
    project_id: String,
    verified_identity: Option<VerifiedEndUserIdentity>,
    tx: &ResponseTx,
    keepalive: Duration,
    registry: Arc<ListenRegistry>,
    channel: &str,
) -> Result<(), Status>
where
    S: tokio_stream::Stream<Item = Result<ListenRequest, Status>> + Unpin,
{
    if !matches!(first_msg.target_change, Some(listen_request::TargetChange::AddTarget(_))) {
        return Err(Status::invalid_argument("first message must be AddTarget"));
    }

    let auth_ctx = verified_identity.as_ref().map(|v| AuthContext {
        uid: v.end_user_id.clone(),
        // custom-claims (ADR-034): out of THIS slice's scope (Listen
        // wiring is a later, named follow-up) — empty map keeps this
        // call site compiling against AuthContext's new field.
        claims: BTreeMap::new(),
    });
    let mut session = Session {
        ctx: Ctx { adapter, system_db, index_manager, project_id, auth_ctx, tx },
        ids: TargetIds::default(),
        targets: BTreeMap::new(),
    };

    // Subscribe before any snapshot runs so no concurrent write is missed.
    let handle = registry.register(channel).await;
    let mut event_rx = handle.event_rx;
    let reset_notify = handle.reset_notify;

    match session.handle_request(first_msg, true).await {
        Ok(()) => {}
        Err(Fail::Gone) => return Ok(()),
        Err(Fail::Target(status)) => return Err(status),
    }

    let mut input_open = true;
    loop {
        // Ok(true): keep going. Ok(false): stream is over.
        let step: Result<bool, Fail> = tokio::select! {
            _ = tokio::time::sleep(keepalive) => {
                // Keepalive token is anchored 1s in the future for the same precision reason.
                let token = rt::encode(Utc::now() + ChronoDuration::seconds(1));
                session.ctx.send(target_change(TargetChangeType::NoChange, vec![], token)).await.map(|()| true)
            }
            request = in_stream.next(), if input_open => match request {
                Some(Ok(request)) => session.handle_request(request, false).await.map(|()| true),
                // Client half-closed (or its request side errored): the
                // stream stays open for the targets already added.
                _ => { input_open = false; Ok(true) }
            },
            event = event_rx.recv() => match event {
                Some(event) => session.on_event(&event).await.map(|()| true),
                // Registry channel closed unexpectedly.
                None => Ok(false),
            },
            _ = reset_notify.notified() => {
                // Slow consumer detected by fan_out — send RESET and end.
                let ids: Vec<i32> = session.targets.keys().copied().collect();
                // The client's channel may also be full; the stream closes momentarily anyway.
                let _ = session.ctx.send(target_change(TargetChangeType::Reset, ids, vec![])).await;
                Ok(false)
            }
            _ = tx.closed() => Ok(false),
        };
        match step {
            Ok(true) => {}
            Ok(false) | Err(Fail::Gone) => return Ok(()),
            Err(Fail::Target(status)) => return Err(status),
        }
    }
}

impl Session<'_> {
    async fn handle_request(&mut self, request: ListenRequest, first: bool) -> Result<(), Fail> {
        match request.target_change {
            Some(listen_request::TargetChange::AddTarget(target)) => {
                self.add_target(target, first).await
            }
            Some(listen_request::TargetChange::RemoveTarget(id)) => self.remove_target(id).await,
            None => Ok(()),
        }
    }

    async fn add_target(&mut self, target: Target, first: bool) -> Result<(), Fail> {
        // A bad/duplicate id is a protocol violation, not a per-target failure:
        // answering REMOVE for a duplicate would remove the client's live target.
        let id = self.ids.activate(target.target_id).map_err(|e| match e {
            TargetIdError::Negative(id) => {
                Status::invalid_argument(format!("negative target_id {id}"))
            }
            TargetIdError::AlreadyActive(id) => {
                Status::already_exists(format!("target_id {id} is already active"))
            }
        })?;
        match self.ctx.snapshot(&target).await {
            Ok(Snapshot { target, docs, token_ts }) => {
                self.ctx.announce(id, docs, token_ts).await?;
                self.targets.insert(id, target);
                Ok(())
            }
            Err(status) if first => {
                self.ids.release(id);
                Err(status.into())
            }
            Err(status) => {
                self.ids.release(id);
                let cause = embyr_proto::rpc::Status {
                    code: status.code() as i32,
                    message: status.message().to_string(),
                    details: vec![],
                };
                let mut change = target_change(TargetChangeType::Remove, vec![id], vec![]);
                if let Some(listen_response::ResponseType::TargetChange(tc)) = &mut change.response_type {
                    tc.cause = Some(cause);
                }
                self.ctx.send(change).await
            }
        }
    }

    async fn remove_target(&mut self, id: i32) -> Result<(), Fail> {
        if self.targets.remove(&id).is_none() {
            return Ok(()); // unknown or already removed: idempotent
        }
        self.ids.release(id);
        self.ctx.send(target_change(TargetChangeType::Remove, vec![id], vec![])).await
    }

    async fn on_event(&mut self, event: &ListenEvent) -> Result<(), Fail> {
        for (id, target) in self.targets.iter_mut() {
            self.ctx.deliver(*id, target, event).await?;
        }
        Ok(())
    }
}

impl Ctx<'_> {
    async fn send(&self, response: ListenResponse) -> Result<(), Fail> {
        self.tx.send(Ok(response)).await.map_err(|_| Fail::Gone)
    }

    /// ADD, one DocumentChange per snapshot document, CURRENT, then NO_CHANGE
    /// carrying a fresh resume token. The token encodes `token_ts`, so on
    /// reconnect the delta filter `update_time > token_ts` excludes every
    /// initial-snapshot document.
    async fn announce(
        &self,
        id: i32,
        docs: Vec<FirestoreDocument>,
        token_ts: DateTime<Utc>,
    ) -> Result<(), Fail> {
        self.send(target_change(TargetChangeType::Add, vec![id], vec![])).await?;
        for doc in docs {
            self.send(document_change(doc, id)).await?;
        }
        self.send(target_change(TargetChangeType::Current, vec![id], vec![])).await?;
        self.send(target_change(TargetChangeType::NoChange, vec![], rt::encode(token_ts))).await
    }

    async fn snapshot(&self, target: &Target) -> Result<Snapshot, Status> {
        match &target.target_type {
            Some(TargetType::Query(qt)) => self.query_snapshot(target, qt).await,
            Some(TargetType::Documents(dt)) => self.documents_snapshot(dt).await,
            None => Err(Status::invalid_argument("target must have a Query or Documents target type")),
        }
    }

    async fn query_snapshot(&self, target: &Target, qt: &QueryTarget) -> Result<Snapshot, Status> {
        // Scope to the parent's own subcollection (`users/u1` + `posts`
        // -> `users/u1/posts`), the shape documents are stored and
        // NOTIFY events are published under.
        let collection_path = FirestoreService::collection_path_under_parent(
            &qt.parent,
            &collection_id_from_query_target(qt).map_err(Status::internal)?,
        )?;
        if project_id_from_parent(&qt.parent).map_err(Status::internal)? != self.project_id {
            return Err(Status::invalid_argument("target parent belongs to a different project"));
        }
        // A malformed filter is the client's error, not an internal one.
        let filter = filter_from_query_target(qt).map_err(Status::invalid_argument)?;
        let Some(QueryType::StructuredQuery(sq)) = &qt.query_type else {
            return Err(Status::internal("QueryTarget has no query_type"));
        };
        let order_by = order_by_from_query(sq)?;
        let limit = sq.limit;
        let offset = (sq.offset > 0).then_some(sq.offset);

        let collection = CollectionPath {
            project_id: ProjectId::new(&self.project_id)
                .map_err(|e| Status::internal(e.to_string()))?,
            collection_path,
        };
        // security-rules-realtime (ADR-033, US-03): the one-time subscribe
        // compliance gate runs BEFORE any row is read for the snapshot.
        let rule = self.resolve_rule(&collection.collection_path, filter.as_ref(), true).await?;

        let windowed =
            filter.is_some() || !order_by.is_empty() || limit.is_some() || offset.is_some();
        // Resume-token delta delivery only for whole-collection targets.
        // ponytail: a windowed target ignores its resume token and re-sends
        // the full window — a delta of a limit window can't tell which
        // documents left it while the client was away. Add if reconnect cost matters.
        let since_update_time = if windowed {
            None
        } else {
            resume_token_ts(target)
        };
        let query = DomainQuery {
            collection_id: collection.collection_path.clone(),
            all_descendants: false,
            filter,
            order_by,
            limit,
            offset,
            start_at: None,
            end_at: None,
            since_update_time,
        };

        // Same composite-index gate as RunQuery (audit #68), after the
        // compliance gate above so a denied caller never learns whether the
        // query also needs an index.
        FirestoreService::require_composite_index_ready(
            self.index_manager,
            &self.project_id,
            &collection_id_from_query_target(qt).map_err(Status::internal)?,
            &query,
        )
        .await?;

        // See `Snapshot::token_ts`.
        let token_ts = Utc::now() + ChronoDuration::seconds(1);
        let docs = self
            .adapter
            .run_query(&collection, &query, None)
            .await
            .map_err(|e| sanitize_backend_error(e, "handle_listen: initial snapshot run_query"))?;

        let scope = TargetScope::Collection(collection.collection_path.clone());
        let delivery = if windowed {
            let members = docs.iter().map(|d| (d.path.document_id.clone(), d.version)).collect();
            Delivery::Windowed { collection: collection.clone(), query, members }
        } else {
            Delivery::Plain
        };
        let rules = BTreeMap::from([(collection.collection_path, rule)]);
        Ok(Snapshot { target: ActiveTarget { scope, delivery, rules }, docs, token_ts })
    }

    async fn documents_snapshot(&self, dt: &DocumentsTarget) -> Result<Snapshot, Status> {
        if dt.documents.is_empty() {
            return Err(Status::invalid_argument("Documents target names no documents"));
        }
        let mut paths: Vec<DocumentPath> = Vec::new();
        for name in &dt.documents {
            let path = FirestoreService::parse_document_path(name)?;
            if path.project_id.as_str() != self.project_id {
                return Err(Status::invalid_argument("document belongs to a different project"));
            }
            if !paths.contains(&path) {
                paths.push(path);
            }
        }

        let mut rules = BTreeMap::new();
        for path in &paths {
            if !rules.contains_key(&path.collection_path) {
                // A single document is evaluated, not proven for a whole
                // query: no compliance gate, the per-document check below.
                let rule = self.resolve_rule(&path.collection_path, None, false).await?;
                rules.insert(path.collection_path.clone(), rule);
            }
        }

        let token_ts = Utc::now() + ChronoDuration::seconds(1);
        let mut docs = Vec::new();
        for path in &paths {
            let found = self
                .adapter
                .get_document(path, None)
                .await
                .map_err(|e| sanitize_backend_error(e, "handle_listen: documents target get_document"))?;
            // A missing document is simply absent from the snapshot.
            let Some(doc) = found else { continue };
            let allowed = self
                .allows(&rules[&path.collection_path], &path.collection_path, &path.document_id, &doc.fields)
                .await?;
            if !allowed {
                return Err(Status::permission_denied("access denied by rule"));
            }
            docs.push(doc);
        }

        let scope = TargetScope::Documents(
            paths.into_iter().map(|p| (p.collection_path, p.document_id)).collect(),
        );
        Ok(Snapshot {
            target: ActiveTarget { scope, delivery: Delivery::Documents, rules },
            docs,
            token_ts,
        })
    }

    /// Resolve the read rule for `collection_path` — exact rule, else the
    /// multi-segment pattern fallback (`resolve_access_rule_pattern`, steps
    /// 1-2; step 3 needs a concrete document, so it is deferred per change via
    /// `Rule::recheck`). `gate`: apply the subscribe-time query-compliance
    /// check (query targets only; it is deliberately not extended to patterns
    /// — OQ-PM-07, so a pattern-only collection's snapshot stays unfiltered).
    async fn resolve_rule(
        &self,
        collection_path: &str,
        filter: Option<&QueryFilter>,
        gate: bool,
    ) -> Result<Rule, Status> {
        let rule_row = self
            .system_db
            .get_access_rule(&self.project_id, collection_path)
            .await
            .map_err(|e| sanitize_backend_error(e, "handle_listen: get_access_rule"))?;
        let Some(row) = rule_row else {
            let routed = FirestoreService::resolve_access_rule_pattern(
                self.system_db,
                &self.project_id,
                collection_path,
                None,
            )
            .await?;
            return match routed {
                None => Ok(Rule { condition: None, bindings: BTreeMap::new(), recheck: true }),
                Some((pattern_row, bindings)) => match pattern_row.read_condition {
                    None => Ok(Rule { condition: None, bindings: BTreeMap::new(), recheck: false }),
                    Some(source) => Ok(Rule {
                        condition: Some(parse_pattern_condition(&source)?),
                        bindings,
                        recheck: false,
                    }),
                },
            };
        };
        let condition = parse_condition(&row.condition_source).map_err(|e| {
            Status::internal(format!("stored access rule failed to re-parse: {e:?}"))
        })?;
        if gate {
            match check_query_compliance(&condition, filter, self.auth_ctx.as_ref()) {
                QueryComplianceOutcome::Admitted => {}
                // AC-17-116: same rejection RunQuery uses.
                outcome => return Err(query_compliance_rejection(&outcome)),
            }
        }
        Ok(Rule { condition: Some(condition), bindings: BTreeMap::new(), recheck: false })
    }

    /// Evaluate `rule` for one concrete document. ADR-030's empty-map
    /// convention for the proposed-write side: Listen has none.
    async fn allows(
        &self,
        rule: &Rule,
        collection_path: &str,
        document_id: &str,
        fields: &BTreeMap<String, FieldValue>,
    ) -> Result<bool, Status> {
        let (condition, bindings) = if rule.recheck {
            self.resolve_recursive_wildcard_condition(collection_path, document_id).await?
        } else {
            (rule.condition.clone(), rule.bindings.clone())
        };
        let Some(condition) = condition else { return Ok(true) };
        let empty_fields: BTreeMap<String, FieldValue> = BTreeMap::new();
        Ok(evaluate(
            &condition,
            &EvalContext {
                auth: self.auth_ctx.as_ref(),
                resource_fields: fields,
                request_resource_fields: &empty_fields,
                path_variable_value: Some(document_id),
                ancestor_path_variable_values: &bindings,
                request_time: None,
                cross_document_reads: &BTreeMap::new(),
            },
        ) != EvaluationOutcome::Deny)
    }

    /// security-rules-cel-recursive-wildcards (Slice 03, AC-17-247): per-change
    /// fallback — re-runs `resolve_access_rule_pattern` with the concrete
    /// document id so step 3 (the recursive-wildcard scan) can run. Only
    /// reached when add time's steps 1-2 both missed (`Rule::recheck`).
    async fn resolve_recursive_wildcard_condition(
        &self,
        collection_path: &str,
        document_id: &str,
    ) -> Result<(Option<Condition>, BTreeMap<String, String>), Status> {
        let routed = FirestoreService::resolve_access_rule_pattern(
            self.system_db,
            &self.project_id,
            collection_path,
            Some(document_id),
        )
        .await?;
        match routed {
            Some((pattern_row, bindings)) => match pattern_row.read_condition {
                Some(source) => Ok((Some(parse_pattern_condition(&source)?), bindings)),
                None => Ok((None, BTreeMap::new())),
            },
            None => Ok((None, BTreeMap::new())),
        }
    }

    /// Deliver one NOTIFY event to one target (or nothing, if it is not
    /// relevant or the rule withholds it).
    async fn deliver(&self, id: i32, target: &mut ActiveTarget, event: &ListenEvent) -> Result<(), Fail> {
        let (path, fields) = match event {
            ListenEvent::Changed(doc) => (&doc.path, &doc.fields),
            ListenEvent::Removed { path, fields } => (path, fields),
        };
        if !target.scope.covers(&path.collection_path, &path.document_id) {
            return Ok(());
        }
        let ActiveTarget { delivery, rules, .. } = target;
        let Some(rule) = rules.get(&path.collection_path) else { return Ok(()) };
        match (delivery, event) {
            (Delivery::Plain | Delivery::Documents, ListenEvent::Changed(doc)) => {
                // US-04: a denied change is withheld, never sent, never a crash.
                if self.allows(rule, &path.collection_path, &path.document_id, fields).await? {
                    self.send(document_change(doc.clone(), id)).await?;
                }
            }
            (Delivery::Plain | Delivery::Documents, ListenEvent::Removed { .. }) => {
                // US-05: the pre-deletion fields are only the rule's input,
                // never serialized.
                if self.allows(rule, &path.collection_path, &path.document_id, fields).await? {
                    self.send(document_delete(path, id)).await?;
                }
            }
            (Delivery::Windowed { collection, query, members }, ListenEvent::Changed(_)) => {
                self.refresh_window(id, collection, query, members, rule).await?;
            }
            (Delivery::Windowed { collection, query, members }, ListenEvent::Removed { .. }) => {
                let was_member = members.remove(&path.document_id).is_some();
                if was_member
                    && self.allows(rule, &path.collection_path, &path.document_id, fields).await?
                {
                    self.send(document_delete(path, id)).await?;
                }
                // A deleted member can pull a document into a limit window; any
                // deletion ahead of an offset shifts the window.
                if query.offset.is_some() || (was_member && query.limit.is_some()) {
                    self.refresh_window(id, collection, query, members, rule).await?;
                }
            }
        }
        Ok(())
    }

    /// Re-run a windowed target's query and send what changed since the last
    /// delivery: documents that entered or changed as DocumentChange, documents
    /// that left as DocumentRemove.
    // ponytail: O(window) query per relevant change, even when the changed
    // document cannot affect the result. Add an in-memory filter evaluator to
    // skip irrelevant changes if hot filtered/limit listeners show up in profiles.
    async fn refresh_window(
        &self,
        id: i32,
        collection: &CollectionPath,
        query: &DomainQuery,
        members: &mut BTreeMap<String, i64>,
        rule: &Rule,
    ) -> Result<(), Fail> {
        let rows = self
            .adapter
            .run_query(collection, query, None)
            .await
            .map_err(|e| sanitize_backend_error(e, "handle_listen: window refresh run_query"))?;
        let current: Vec<(String, i64)> =
            rows.iter().map(|d| (d.path.document_id.clone(), d.version)).collect();
        let diff = diff_window(members, &current);
        let changed: HashSet<&str> = diff.entered_or_changed.iter().map(String::as_str).collect();

        for doc in rows.iter().filter(|d| changed.contains(d.path.document_id.as_str())) {
            // Withheld when denied: neither sent nor remembered.
            if !self
                .allows(rule, &collection.collection_path, &doc.path.document_id, &doc.fields)
                .await?
            {
                continue;
            }
            members.insert(doc.path.document_id.clone(), doc.version);
            self.send(document_change(doc.clone(), id)).await?;
        }
        for left in &diff.left {
            members.remove(left);
            let path = DocumentPath {
                project_id: collection.project_id.clone(),
                collection_path: collection.collection_path.clone(),
                document_id: left.clone(),
            };
            self.send(document_remove(&path, id)).await?;
        }
        Ok(())
    }
}

fn parse_pattern_condition(source: &str) -> Result<Condition, Status> {
    parse_condition(source).map_err(|e| {
        Status::internal(format!("stored access rule pattern failed to re-parse: {e:?}"))
    })
}

/// Fresh (<=24h) resume tokens enable delta delivery.
fn resume_token_ts(target: &Target) -> Option<DateTime<Utc>> {
    match &target.resume_type {
        Some(ResumeType::ResumeToken(bytes)) => Some(bytes.as_slice()),
        _ => None,
    }
    .filter(|t| !t.is_empty() && !rt::is_stale(t))
    .and_then(rt::decode_ts)
}

/// Order-by field paths are validated (SPEC.md Invariant 6): they are
/// interpolated into SQL by the storage layer.
fn order_by_from_query(
    sq: &embyr_proto::firestore::StructuredQuery,
) -> Result<Vec<OrderBy>, Status> {
    sq.order_by
        .iter()
        .filter_map(|o| {
            let field_path = o.field.as_ref()?.field_path.clone();
            let direction = match Direction::try_from(o.direction).unwrap_or(Direction::Unspecified) {
                Direction::Descending => OrderDirection::Descending,
                _ => OrderDirection::Ascending,
            };
            Some(
                validate_field_path(&field_path)
                    .map(|()| OrderBy { field_path, direction })
                    .map_err(|e| Status::invalid_argument(e.to_string())),
            )
        })
        .collect()
}

fn document_name(path: &DocumentPath) -> String {
    format!(
        "projects/{}/databases/(default)/documents/{}/{}",
        path.project_id.as_str(),
        path.collection_path,
        path.document_id,
    )
}

fn target_change(kind: TargetChangeType, target_ids: Vec<i32>, resume_token: Vec<u8>) -> ListenResponse {
    ListenResponse {
        response_type: Some(listen_response::ResponseType::TargetChange(TargetChange {
            target_change_type: kind as i32,
            target_ids,
            resume_token,
            ..Default::default()
        })),
    }
}

fn document_change(doc: FirestoreDocument, target_id: i32) -> ListenResponse {
    ListenResponse {
        response_type: Some(listen_response::ResponseType::DocumentChange(DocumentChange {
            document: Some(document_to_proto(doc)),
            target_ids: vec![target_id],
            removed_target_ids: vec![],
        })),
    }
}

/// The document was deleted.
fn document_delete(path: &DocumentPath, target_id: i32) -> ListenResponse {
    ListenResponse {
        response_type: Some(listen_response::ResponseType::DocumentDelete(DocumentDelete {
            document: document_name(path),
            removed_target_ids: vec![target_id],
            read_time: None,
        })),
    }
}

/// The document still exists but no longer belongs to the target's result.
fn document_remove(path: &DocumentPath, target_id: i32) -> ListenResponse {
    ListenResponse {
        response_type: Some(listen_response::ResponseType::DocumentRemove(DocumentRemove {
            document: document_name(path),
            removed_target_ids: vec![target_id],
            read_time: None,
        })),
    }
}
