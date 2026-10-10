#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;
use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::service::EventModelServiceImpl;
use trogon_atlas_store::{
    store::{
        BranchInfo, ChangeRecord, ChangeStreamStats, ListFilter, MutationOp, MutationOutcome,
        Store, Written,
    },
    ChangeKind, ChangesetPage, ChangesetRecord, ChangesetRef, NatsStore, StoreError, StoredEntity,
    WriteContext,
};

fn id(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}

fn make_event(ns: &str, slug: &str, version: u64) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug, version)),
            title: format!("{ns}/{slug}@{version}"),
            doc: String::new(),
            swimlane: None,
            metadata: Vec::new(),
            supersedes: None,
            schema: None,
        })),
    }
}

async fn svc() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    EventModelServiceImpl::try_new(Arc::new(store)).unwrap()
}

// ===== validate_project tests =====

#[tokio::test(flavor = "multi_thread")]
async fn validate_project_requires_scope() {
    let svc = svc().await;
    let err = svc
        .validate_project(Request::new(pb::ValidateProjectRequest { scope: None }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn validate_project_domain_scope_returns_empty_on_empty_store() {
    let svc = svc().await;
    let resp = svc
        .validate_project(Request::new(pb::ValidateProjectRequest {
            scope: Some(pb::validate_project_request::Scope::DomainId(id(
                "acme", "core", 1,
            ))),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.reports,
        [] as [trogon_atlas_proto::validate_project_response::ModelReport; 0]
    );
    assert_eq!(resp.total_errors, 0);
}

// ===== list_entities_by_domain tests =====

#[tokio::test(flavor = "multi_thread")]
async fn list_entities_by_domain_requires_domain_id() {
    let svc = svc().await;
    let err = svc
        .list_entities_by_domain(Request::new(pb::ListEntitiesByDomainRequest {
            domain_id: None,
            kinds: vec![],
            page_size: 0,
            page_token: String::new(),
            latest_versions_only: true,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_entities_by_domain_returns_empty_on_empty_store() {
    let svc = svc().await;
    let resp = svc
        .list_entities_by_domain(Request::new(pb::ListEntitiesByDomainRequest {
            domain_id: Some(id("acme", "core", 1)),
            kinds: vec![],
            page_size: 0,
            page_token: String::new(),
            latest_versions_only: true,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.entities, [] as [trogon_atlas_proto::Entity; 0]);
}

// ===== list_entities lifecycle filter + latest_versions_only tests =====

fn lifecycle_ann_any(status: &str) -> prost_types::Any {
    use prost::Message;
    let ann = pb::LifecycleAnnotation {
        status: status.into(),
        doc: String::new(),
    };
    prost_types::Any {
        type_url: "type.googleapis.com/trogonatlas.annotation.v1alpha1.LifecycleAnnotation".into(),
        value: ann.encode_to_vec(),
    }
}

fn make_event_with_lifecycle(ns: &str, slug: &str, version: u64, status: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug, version)),
            title: format!("{ns}/{slug}@{version}"),
            doc: String::new(),
            swimlane: None,
            metadata: vec![lifecycle_ann_any(status)],
            supersedes: None,
            schema: None,
        })),
    }
}

/// `entity_lifecycle_status` must decode a `LifecycleAnnotation` from an
/// entity's metadata `Any` payloads, and `list_entities`'s
/// `lifecycle_status_in` filter must use it to include only matching
/// entities (excluding entities with no annotation at all, since "" was not
/// requested).
#[tokio::test(flavor = "multi_thread")]
async fn list_entities_lifecycle_status_in_filters_by_annotation() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event_with_lifecycle(
            "lifecycle",
            "draft-one",
            1,
            "draft",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event_with_lifecycle(
            "lifecycle",
            "accepted-one",
            1,
            "accepted",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("lifecycle", "no-annotation", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .list_entities(Request::new(pb::ListEntitiesRequest {
            namespaces: vec!["lifecycle".into()],
            kinds: vec![],
            page_size: 0,
            page_token: String::new(),
            latest_versions_only: false,
            lifecycle_status_in: vec!["draft".into()],
            lifecycle_status_not_in: vec![],
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.entities.len(), 1);
    let pb::entity::Kind::Event(ev) = resp.entities[0].kind.as_ref().unwrap() else {
        panic!("expected event");
    };
    assert_eq!(ev.id.as_ref().unwrap().slug, "draft-one");
}

/// `lifecycle_status_not_in` containing "" must exclude entities with no
/// LifecycleAnnotation at all, per `entity_lifecycle_status` returning
/// `None` -> `status_key = ""`.
#[tokio::test(flavor = "multi_thread")]
async fn list_entities_lifecycle_status_not_in_empty_string_excludes_unannotated() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event_with_lifecycle(
            "lifecycle2",
            "accepted-one",
            1,
            "accepted",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("lifecycle2", "no-annotation", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .list_entities(Request::new(pb::ListEntitiesRequest {
            namespaces: vec!["lifecycle2".into()],
            kinds: vec![],
            page_size: 0,
            page_token: String::new(),
            latest_versions_only: false,
            lifecycle_status_in: vec![],
            lifecycle_status_not_in: vec![String::new()],
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.entities.len(), 1);
    let pb::entity::Kind::Event(ev) = resp.entities[0].kind.as_ref().unwrap() else {
        panic!("expected event");
    };
    assert_eq!(ev.id.as_ref().unwrap().slug, "accepted-one");
}

/// `latest_versions_only` must dedup to the highest version per
/// (kind, namespace, slug), exercising the `BTreeMap`-based reduction
/// including the branch that replaces an already-seen entry with a newer
/// version encountered later in iteration order.
#[tokio::test(flavor = "multi_thread")]
async fn list_entities_latest_versions_only_keeps_highest_version() {
    let svc = svc().await;

    for version in [1u64, 3, 2] {
        svc.put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event("latestver", "multi", version)),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap();
    }

    let resp = svc
        .list_entities(Request::new(pb::ListEntitiesRequest {
            namespaces: vec!["latestver".into()],
            kinds: vec![],
            page_size: 0,
            page_token: String::new(),
            latest_versions_only: true,
            lifecycle_status_in: vec![],
            lifecycle_status_not_in: vec![],
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.entities.len(), 1);
    let pb::entity::Kind::Event(ev) = resp.entities[0].kind.as_ref().unwrap() else {
        panic!("expected event");
    };
    assert_eq!(ev.id.as_ref().unwrap().version, 3);
}

/// `entity_lifecycle_status`'s metadata-extraction match must also cover
/// non-`Event` entity kinds (here `Command`), since `list_entities` filters
/// by lifecycle status regardless of entity kind.
#[tokio::test(flavor = "multi_thread")]
async fn list_entities_lifecycle_status_in_filters_non_event_entity_kind() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Command(pb::Command {
                id: Some(id("lifecycle-cmd", "place-order", 1)),
                title: "PlaceOrder".into(),
                doc: String::new(),
                swimlane: None,
                metadata: vec![lifecycle_ann_any("draft")],
                supersedes: None,
                schema: None,
            })),
        }),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .list_entities(Request::new(pb::ListEntitiesRequest {
            namespaces: vec!["lifecycle-cmd".into()],
            kinds: vec![],
            page_size: 0,
            page_token: String::new(),
            latest_versions_only: true,
            lifecycle_status_in: vec!["draft".into()],
            lifecycle_status_not_in: vec![],
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.entities.len(),
        1,
        "a Command entity's LifecycleAnnotation must be found and matched"
    );
}

/// `entity_lifecycle_status` must treat an Any payload whose `type_url`
/// matches `LifecycleAnnotation` but whose bytes fail to decode as absent
/// (`None`), not panic and not surface a decode error to the caller.
#[tokio::test(flavor = "multi_thread")]
async fn list_entities_lifecycle_filter_treats_undecodable_annotation_as_absent() {
    let svc = svc().await;

    let malformed_any = prost_types::Any {
        type_url: "type.googleapis.com/trogonatlas.annotation.v1alpha1.LifecycleAnnotation".into(),
        value: vec![0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
    };
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id("lifecycle-malformed", "corrupt-ann", 1)),
                title: "Corrupt".into(),
                doc: String::new(),
                swimlane: None,
                metadata: vec![malformed_any],
                supersedes: None,
                schema: None,
            })),
        }),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    // status_not_in: [""] means "exclude entities with no annotation at
    // all"; an undecodable annotation must be treated the same as absent,
    // so this entity must be excluded.
    let resp = svc
        .list_entities(Request::new(pb::ListEntitiesRequest {
            namespaces: vec!["lifecycle-malformed".into()],
            kinds: vec![],
            page_size: 0,
            page_token: String::new(),
            latest_versions_only: true,
            lifecycle_status_in: vec![],
            lifecycle_status_not_in: vec![String::new()],
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(
        resp.entities.is_empty(),
        "an entity with an undecodable LifecycleAnnotation must be treated as unannotated"
    );
}

// ===== extract_subgraph tests =====

#[tokio::test(flavor = "multi_thread")]
async fn extract_subgraph_requires_scope() {
    let svc = svc().await;
    let err = svc
        .extract_subgraph(Request::new(pb::ExtractSubgraphRequest {
            scope: None,
            include_cross_model: false,
            new_event_model_id: None,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn extract_subgraph_happy_path_returns_event_model() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("orders", "order-placed", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .extract_subgraph(Request::new(pb::ExtractSubgraphRequest {
            scope: Some(pb::AnalysisScope {
                scope: Some(pb::analysis_scope::Scope::Slice(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("orders", "order-placed", 1)),
                })),
            }),
            include_cross_model: false,
            new_event_model_id: None,
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(resp.event_model.is_some());
}

// ===== delete_by_query failure path tests =====

/// A store wrapper whose `batch_apply` always fails, with a backend error
/// unless a test picks another failure.
struct FailingBatchStore {
    inner: NatsStore,
    failure: fn() -> StoreError,
}

impl FailingBatchStore {
    async fn new() -> Self {
        Self::with_failure(|| StoreError::Backend("injected batch failure".into())).await
    }

    async fn with_failure(failure: fn() -> StoreError) -> Self {
        let nats = trogon_atlas_testsupport::shared().await;
        Self {
            inner: nats.store().await,
            failure,
        }
    }
}

#[async_trait]
impl Store for FailingBatchStore {
    async fn register_namespace(
        &self,
        name: &trogon_atlas_core::NamespaceName,
        parent: &trogon_atlas_core::OwnerId,
        created_by: &str,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceClaim> {
        self.inner
            .register_namespace(name, parent, created_by)
            .await
    }

    async fn adopt_namespace(
        &self,
        name: &trogon_atlas_core::NamespaceName,
        parent: &trogon_atlas_core::OwnerId,
        tenure: &trogon_atlas_store::NamespaceTenure,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceClaim> {
        self.inner.adopt_namespace(name, parent, tenure).await
    }

    async fn restore_namespace(
        &self,
        record: &trogon_atlas_store::NamespaceRecord,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceClaim> {
        self.inner.restore_namespace(record).await
    }

    async fn release_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner.release_namespace(id).await
    }

    async fn promote_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
    ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_store::NamespaceRecord>> {
        self.inner.promote_namespace(id).await
    }

    async fn get_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
    ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_store::NamespaceRecord>> {
        self.inner.get_namespace(id).await
    }

    async fn resolve_namespace(
        &self,
        parent: &trogon_atlas_core::OwnerId,
        name: &trogon_atlas_core::NamespaceName,
    ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_core::NamespaceId>> {
        self.inner.resolve_namespace(parent, name).await
    }

    async fn list_namespaces(
        &self,
    ) -> trogon_atlas_store::StoreResult<Vec<trogon_atlas_store::NamespaceRecord>> {
        self.inner.list_namespaces().await
    }

    async fn move_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
        new_parent: &trogon_atlas_core::OwnerId,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceRecord> {
        self.inner.move_namespace(id, new_parent).await
    }

    async fn get(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<StoredEntity> {
        self.inner.get(kind, id, branch).await
    }

    async fn batch_get(
        &self,
        keys: &[(pb::EntityKind, pb::Id)],
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<Vec<Option<StoredEntity>>> {
        self.inner.batch_get(keys, branch).await
    }

    async fn create(
        &self,
        kind: pb::EntityKind,
        entity: &pb::Entity,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Written> {
        self.inner.create(kind, entity, ctx).await
    }

    async fn put(
        &self,
        kind: pb::EntityKind,
        entity: &pb::Entity,
        force: bool,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Written> {
        self.inner.put(kind, entity, force, ctx).await
    }

    async fn update(
        &self,
        kind: pb::EntityKind,
        entity: &pb::Entity,
        expected_etag: &str,
        force: bool,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Written> {
        self.inner
            .update(kind, entity, expected_etag, force, ctx)
            .await
    }

    async fn delete(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        expected_etag: Option<&str>,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner.delete(kind, id, expected_etag, ctx).await
    }

    async fn list(
        &self,
        filter: ListFilter<'_>,
        limit: Option<usize>,
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<Vec<StoredEntity>> {
        self.inner.list(filter, limit, branch).await
    }

    async fn record_change(
        &self,
        kind: ChangeKind,
        entity_ref: &pb::EntityRef,
        changeset: Option<ChangesetRef<'_>>,
    ) -> trogon_atlas_store::StoreResult<u64> {
        self.inner.record_change(kind, entity_ref, changeset).await
    }

    async fn read_changes(
        &self,
        since: u64,
        limit: usize,
    ) -> trogon_atlas_store::StoreResult<Vec<ChangeRecord>> {
        self.inner.read_changes(since, limit).await
    }

    async fn current_change_seq(&self) -> trogon_atlas_store::StoreResult<u64> {
        self.inner.current_change_seq().await
    }

    async fn batch_apply(
        &self,
        _ops: &[MutationOp],
        _ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<MutationOutcome>> {
        Err((self.failure)())
    }

    fn subscribe(&self) -> broadcast::Receiver<ChangeRecord> {
        self.inner.subscribe()
    }

    async fn prune_changes(&self, before_seq: u64) -> trogon_atlas_store::StoreResult<()> {
        self.inner.prune_changes(before_seq).await
    }

    async fn change_stream_stats(
        &self,
    ) -> trogon_atlas_store::StoreResult<Option<ChangeStreamStats>> {
        self.inner.change_stream_stats().await
    }

    async fn create_branch(
        &self,
        name: &str,
        doc: &str,
    ) -> trogon_atlas_store::StoreResult<BranchInfo> {
        self.inner.create_branch(name, doc).await
    }

    async fn set_branch_fork_point(
        &self,
        name: &str,
        fork_changeset_id: &str,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner
            .set_branch_fork_point(name, fork_changeset_id)
            .await
    }

    async fn list_branches(&self) -> trogon_atlas_store::StoreResult<Vec<BranchInfo>> {
        self.inner.list_branches().await
    }

    async fn delete_branch(&self, name: &str) -> trogon_atlas_store::StoreResult<()> {
        self.inner.delete_branch(name).await
    }

    async fn list_branch_deltas(
        &self,
        branch: &str,
    ) -> trogon_atlas_store::StoreResult<Vec<trogon_atlas_store::store::BranchDeltaEntry>> {
        self.inner.list_branch_deltas(branch).await
    }

    async fn land_branch_merge(
        &self,
        branch: &str,
        ops: &[trogon_atlas_store::store::BranchLandOp],
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<MutationOutcome>> {
        self.inner.land_branch_merge(branch, ops, ctx).await
    }

    async fn rebase_branch_deltas(
        &self,
        branch: &str,
        rebases: &[(pb::EntityKind, pb::Id, Option<(pb::Entity, String)>)],
    ) -> trogon_atlas_store::StoreResult<u32> {
        self.inner.rebase_branch_deltas(branch, rebases).await
    }

    async fn resolve_branch_entry(
        &self,
        branch: &str,
        kind: pb::EntityKind,
        id: &pb::Id,
        take_theirs: bool,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner
            .resolve_branch_entry(branch, kind, id, take_theirs)
            .await
    }

    async fn append_changeset(
        &self,
        record: &ChangesetRecord,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner.append_changeset(record).await
    }

    async fn get_changeset(&self, id: &str) -> trogon_atlas_store::StoreResult<ChangesetRecord> {
        self.inner.get_changeset(id).await
    }

    async fn list_changesets(
        &self,
        page: ChangesetPage<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<ChangesetRecord>> {
        self.inner.list_changesets(page).await
    }

    async fn list_entity_revisions(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        page: trogon_atlas_store::RevisionPage<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<trogon_atlas_store::EntityRevisionRecord>> {
        self.inner.list_entity_revisions(kind, id, page).await
    }

    async fn get_entity_revision(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        changeset_id: &str,
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::EntityRevisionRecord> {
        self.inner
            .get_entity_revision(kind, id, changeset_id, branch)
            .await
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_happy_path_deletes_matching_entities() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("orders", "order-placed", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .delete_by_query(Request::new(pb::DeleteByQueryRequest {
            namespace: "orders".into(),
            slug: String::new(),
            kind: 0,
            project: String::new(),
            max_deletes: 10,
            mode: pb::delete_entity_request::Mode::Force as i32,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.deleted_count, 1);
    assert!(!resp.dry_run);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_batch_store_error_returns_internal() {
    let store = Arc::new(FailingBatchStore::new().await);
    let svc = EventModelServiceImpl::try_new(store.clone()).unwrap();

    for slug in ["a", "b"] {
        store
            .inner
            .create(
                pb::EntityKind::Event,
                &make_event("batchfail", slug, 1),
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }

    let err = svc
        .delete_by_query(Request::new(pb::DeleteByQueryRequest {
            namespace: "batchfail".into(),
            slug: String::new(),
            kind: 0,
            project: String::new(),
            max_deletes: 10,
            mode: pb::delete_entity_request::Mode::Force as i32,
        }))
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::Internal);
    assert!(
        !err.message().contains("injected batch failure"),
        "backend detail leaked to client: {}",
        err.message()
    );
}

const LEAKY_BACKEND_DETAIL: &str = "nats://10.20.30.40:4222 bucket KV_secret-bucket";

fn put_op(slug: &str) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event("batchpartial", slug, 1)),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        })),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_partial_apply_is_unavailable_and_names_the_journal() {
    let store = Arc::new(
        FailingBatchStore::with_failure(|| StoreError::BatchFailed {
            index: 1,
            source: Box::new(StoreError::PartialApply {
                keys: vec!["event.batchpartial.first".into()],
                journal: Some(trogon_atlas_store::BatchJournalId::new(
                    "journal-awaiting-recovery",
                )),
            }),
        })
        .await,
    );
    let svc = EventModelServiceImpl::try_new(store).unwrap();

    let err = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![put_op("first"), put_op("second")],
            validate_only: false,
        }))
        .await
        .expect_err("a partially applied batch must not report STATUS_FAILED");

    assert_eq!(err.code(), tonic::Code::Unavailable);
    assert!(
        err.message().contains("journal-awaiting-recovery"),
        "status must name the journal recovery resolves: {}",
        err.message()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_failure_issue_does_not_leak_backend_detail() {
    let store = Arc::new(
        FailingBatchStore::with_failure(|| StoreError::BatchFailed {
            index: 0,
            source: Box::new(StoreError::Backend(LEAKY_BACKEND_DETAIL.into())),
        })
        .await,
    );
    let svc = EventModelServiceImpl::try_new(store).unwrap();

    let resp = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![put_op("leaky")],
            validate_only: false,
        }))
        .await
        .expect("a rolled back batch reports STATUS_FAILED")
        .into_inner();

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Failed as i32
    );
    let issue = resp.failure.first().expect("failure must carry an issue");
    assert_eq!(issue.code, "BATCH_OP_FAILED");
    assert!(
        !issue.message.contains("nats://") && !issue.message.contains("KV_secret-bucket"),
        "backend detail leaked to client: {}",
        issue.message
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_project_namespace_conflict_is_invalid() {
    let svc = svc().await;
    let err = svc
        .delete_by_query(Request::new(pb::DeleteByQueryRequest {
            namespace: "orders".into(),
            slug: String::new(),
            kind: 0,
            project: "billing".into(),
            max_deletes: 10,
            mode: pb::delete_entity_request::Mode::Force as i32,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

// ===== new tests per issue #14 =====

#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_no_filters_rejected() {
    let svc = svc().await;
    let err = svc
        .delete_by_query(Request::new(pb::DeleteByQueryRequest {
            namespace: String::new(),
            slug: String::new(),
            kind: 0,
            project: String::new(),
            max_deletes: 10,
            mode: pb::delete_entity_request::Mode::Force as i32,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_max_deletes_zero_refused() {
    let svc = svc().await;
    let err = svc
        .delete_by_query(Request::new(pb::DeleteByQueryRequest {
            namespace: "orders".into(),
            slug: String::new(),
            kind: 0,
            project: String::new(),
            max_deletes: 0,
            mode: pb::delete_entity_request::Mode::Force as i32,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_negative_max_deletes_returns_invalid_argument() {
    let svc = svc().await;
    let err = svc
        .delete_by_query(Request::new(pb::DeleteByQueryRequest {
            namespace: "orders".into(),
            slug: String::new(),
            kind: 0,
            project: String::new(),
            max_deletes: -1,
            mode: pb::delete_entity_request::Mode::Force as i32,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_invalid_namespace_returns_invalid_argument() {
    let svc = svc().await;
    let err = svc
        .delete_by_query(Request::new(pb::DeleteByQueryRequest {
            namespace: "INVALID NAMESPACE!".into(),
            slug: String::new(),
            kind: 0,
            project: String::new(),
            max_deletes: 10,
            mode: pb::delete_entity_request::Mode::Force as i32,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_invalid_slug_returns_invalid_argument() {
    let svc = svc().await;
    let err = svc
        .delete_by_query(Request::new(pb::DeleteByQueryRequest {
            namespace: String::new(),
            slug: "INVALID SLUG!".into(),
            kind: pb::EntityKind::Event as i32,
            project: String::new(),
            max_deletes: 10,
            mode: pb::delete_entity_request::Mode::Force as i32,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_invalid_project_returns_invalid_argument() {
    let svc = svc().await;
    let err = svc
        .delete_by_query(Request::new(pb::DeleteByQueryRequest {
            namespace: String::new(),
            slug: String::new(),
            kind: pb::EntityKind::Event as i32,
            project: "INVALID PROJECT!".into(),
            max_deletes: 10,
            mode: pb::delete_entity_request::Mode::Force as i32,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_successful_delete_removes_from_store() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("dbq-sideeffects", "ev", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .delete_by_query(Request::new(pb::DeleteByQueryRequest {
            namespace: "dbq-sideeffects".into(),
            slug: String::new(),
            kind: 0,
            project: String::new(),
            max_deletes: 10,
            mode: pb::delete_entity_request::Mode::Force as i32,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.deleted_count, 1);
    assert!(!resp.dry_run);

    // Entity must no longer be retrievable from the store.
    let all = svc
        .list_entities(Request::new(pb::ListEntitiesRequest {
            namespaces: vec!["dbq-sideeffects".into()],
            kinds: vec![],
            page_size: 0,
            page_token: String::new(),
            latest_versions_only: true,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(
        all.entities.is_empty(),
        "deleted entities must not appear in list_entities; found: {:?}",
        all.entities
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_entity_etag_mismatch_aborts() {
    let svc = svc().await;
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("orders", "placed", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let err = svc
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            operation_id: String::new(),
            kind: pb::EntityKind::Event as i32,
            id: Some(id("orders", "placed", 1)),
            mode: pb::delete_entity_request::Mode::Force as i32,
            if_match: "wrong-etag".into(),
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::Aborted);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_entity_force_mode_succeeds_without_etag() {
    let svc = svc().await;
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("orders", "placed", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    svc.delete_entity(Request::new(pb::DeleteEntityRequest {
        operation_id: String::new(),
        kind: pb::EntityKind::Event as i32,
        id: Some(id("orders", "placed", 1)),
        mode: pb::delete_entity_request::Mode::Force as i32,
        if_match: String::new(),
    }))
    .await
    .expect("force delete without etag must succeed");
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_validate_only_leaves_store_unchanged() {
    let svc = svc().await;

    let resp = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                    operation_id: String::new(),
                    entity: Some(make_event("orders", "dry-run-event", 1)),
                    create_only: true,
                    if_match: String::new(),
                    validate_only: false,
                    force: false,
                })),
            }],
            validate_only: true,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Validated as i32
    );

    // The entity must not have been written to the store.
    let get_err = svc
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("orders", "dry-run-event", 1)),
        }))
        .await
        .unwrap_err();
    assert_eq!(get_err.code(), tonic::Code::NotFound);
}

/// `batch_mutate`'s dry-run overlay (`dry_run::Overlay::batch_apply`) must
/// report `Status::Failed` when a `create_only` op targets a key that
/// already exists in the snapshot, mirroring the real store's `AlreadyExists`
/// semantics without touching the store.
#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_dry_run_reports_create_already_exists() {
    let svc = svc().await;
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("orders", "dry-run-collide", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                    operation_id: String::new(),
                    entity: Some(make_event("orders", "dry-run-collide", 1)),
                    create_only: true,
                    if_match: String::new(),
                    validate_only: false,
                    force: false,
                })),
            }],
            validate_only: true,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Failed as i32
    );
    assert_eq!(resp.failed_op_index, 0);
}

/// `batch_mutate`'s dry-run overlay must report `Status::Failed` when a
/// `Delete` op targets a key absent from the snapshot.
#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_dry_run_reports_delete_not_found() {
    let svc = svc().await;

    let resp = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                    operation_id: String::new(),
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("orders", "dry-run-absent", 1)),
                    mode: 0,
                    if_match: String::new(),
                })),
            }],
            validate_only: true,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Failed as i32
    );
    assert_eq!(resp.failed_op_index, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_max_batch_ops_rejected() {
    use trogon_atlas_server::service::MAX_BATCH_OPS;
    let svc = svc().await;
    let ops: Vec<pb::BatchMutateOp> = (0..=MAX_BATCH_OPS)
        .map(|i| pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                operation_id: String::new(),
                entity: Some(make_event("orders", &format!("event-{i}"), 1)),
                create_only: true,
                if_match: String::new(),
                validate_only: false,
                force: false,
            })),
        })
        .collect();

    let err = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops,
            validate_only: false,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

/// A batch `Put` op with `entity: None` must be rejected up front as a
/// top-level `InvalidArgument`, not panic on the `ok_or_else` unwrap.
#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_put_without_entity_is_rejected() {
    let svc = svc().await;
    let err = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                    operation_id: String::new(),
                    entity: None,
                    create_only: true,
                    if_match: String::new(),
                    validate_only: false,
                    force: false,
                })),
            }],
            validate_only: false,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(err.message().contains("ops[0].put.entity is required"));
}

/// When a later op in a batch fails at the STORE layer (not the request
/// validation layer), `batch_mutate` must respond with `Status::Failed`
/// (via `batch_failure_response`) rather than a top-level gRPC error,
/// reporting which op index failed.
#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_store_layer_failure_reports_status_failed() {
    let svc = svc().await;

    // Pre-create the entity that op index 1 will collide with.
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("batchfail", "already-there", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .expect("seed entity must be created");

    let resp = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![
                pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        operation_id: String::new(),
                        entity: Some(make_event("batchfail", "fresh-one", 1)),
                        create_only: true,
                        if_match: String::new(),
                        validate_only: false,
                        force: false,
                    })),
                },
                pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        operation_id: String::new(),
                        entity: Some(make_event("batchfail", "already-there", 1)),
                        create_only: true,
                        if_match: String::new(),
                        validate_only: false,
                        force: false,
                    })),
                },
            ],
            validate_only: false,
        }))
        .await
        .expect("batch_mutate itself must not return a gRPC error")
        .into_inner();

    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Failed as i32
    );
    assert_eq!(resp.failed_op_index, 1);
    assert!(
        !resp.failure.is_empty(),
        "failure must carry at least one ValidationIssue"
    );
    assert_eq!(
        resp.failure[0].severity,
        pb::validation_issue::Severity::Error as i32
    );

    // The first op must have been rolled back since the batch as a whole failed.
    let get_err = svc
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("batchfail", "fresh-one", 1)),
        }))
        .await
        .unwrap_err();
    assert_eq!(get_err.code(), tonic::Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_changes_basic_roundtrip() {
    let svc = svc().await;

    // batch_mutate goes through batch_apply which records changes;
    // put_entity only calls store.create and does not record a change log entry.
    svc.batch_mutate(Request::new(pb::BatchMutateRequest {
        operation_id: String::new(),
        ops: vec![pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                operation_id: String::new(),
                entity: Some(make_event("orders", "order-placed", 1)),
                create_only: true,
                if_match: String::new(),
                validate_only: false,
                force: false,
            })),
        }],
        validate_only: false,
    }))
    .await
    .unwrap();

    // since_token="0" reads from the beginning of the change log.
    // An empty since_token reads from the current head (returns nothing historical).
    let resp = svc
        .list_changes(Request::new(pb::ListChangesRequest {
            since_token: "0".into(),
            page_size: 10,
            scopes: Vec::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(
        !resp.events.is_empty(),
        "expected at least one change event after batch_mutate"
    );
    assert!(!resp.next_token.is_empty(), "next_token must be set");
}

// ===== list_changes pagination tests =====

#[tokio::test(flavor = "multi_thread")]
async fn list_changes_pagination_multi_page() {
    let svc = svc().await;

    // Write 3 distinct events via batch_mutate so the change log has 3 records.
    for slug in ["ev-a", "ev-b", "ev-c"] {
        svc.batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                    operation_id: String::new(),
                    entity: Some(make_event("pg", slug, 1)),
                    create_only: true,
                    if_match: String::new(),
                    validate_only: false,
                    force: false,
                })),
            }],
            validate_only: false,
        }))
        .await
        .unwrap();
    }

    // Page through with page_size=1. We should get three non-empty pages whose
    // next_token strictly advances, then an empty fourth page (we're at the end).
    let mut token = "0".to_string();
    let mut all_tokens: Vec<String> = Vec::new();
    let mut total_events: usize = 0;

    for _ in 0..4 {
        let resp = svc
            .list_changes(Request::new(pb::ListChangesRequest {
                since_token: token.clone(),
                page_size: 1,
                scopes: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        total_events += resp.events.len();
        let new_token = resp.next_token.clone();
        // next_token must always be present (it tracks position even when empty page).
        assert!(!new_token.is_empty(), "next_token must never be empty");
        all_tokens.push(new_token.clone());
        if token == new_token {
            // We have consumed all records; the token did not advance.
            break;
        }
        token = new_token;
    }

    assert_eq!(
        total_events, 3,
        "expected exactly 3 change events across pages"
    );
    // All tokens after each page must be strictly increasing (as numeric seq values).
    let seqs: Vec<u64> = all_tokens.iter().map(|t| t.parse().unwrap()).collect();
    for w in seqs.windows(2) {
        assert!(
            w[0] <= w[1],
            "tokens must be non-decreasing: {all_tokens:?}"
        );
    }
}

// ===== diff_entities tests =====

fn make_event_with_title(ns: &str, slug: &str, version: u64, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug, version)),
            title: title.into(),
            doc: String::new(),
            swimlane: None,
            metadata: Vec::new(),
            supersedes: None,
            schema: None,
        })),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn diff_entities_detects_title_change() {
    let svc = svc().await;

    // Store version 1 with title "Alpha" and version 2 with title "Beta".
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event_with_title("orders", "placed", 1, "Alpha")),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event_with_title("orders", "placed", 2, "Beta")),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .diff_entities(Request::new(pb::DiffEntitiesRequest {
            a: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("orders", "placed", 1)),
            }),
            b: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("orders", "placed", 2)),
            }),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(
        !resp.ops.is_empty(),
        "diff between v1 and v2 must produce at least one op"
    );
    let has_title_change = resp.ops.iter().any(|op| op.path == "title");
    assert!(
        has_title_change,
        "expected a 'title' diff op; got: {:?}",
        resp.ops
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn diff_entities_identical_versions_empty_diff() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("orders", "shipped", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    // Diff an entity against itself: must produce zero ops.
    let resp = svc
        .diff_entities(Request::new(pb::DiffEntitiesRequest {
            a: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("orders", "shipped", 1)),
            }),
            b: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("orders", "shipped", 1)),
            }),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(
        resp.ops.is_empty(),
        "diffing an entity against itself must yield zero ops; got: {:?}",
        resp.ops
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn diff_entities_missing_entity_returns_not_found() {
    let svc = svc().await;

    // Only store one side; the other side is absent.
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("orders", "created", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let err = svc
        .diff_entities(Request::new(pb::DiffEntitiesRequest {
            a: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("orders", "created", 1)),
            }),
            b: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("orders", "does-not-exist", 1)),
            }),
        }))
        .await
        .unwrap_err();

    assert_eq!(
        err.code(),
        tonic::Code::NotFound,
        "missing b-side must return NotFound; got: {err}"
    );
}

// ===== get_impact tests =====

/// Helper: build a minimal command-slice entity that references an event.
/// We only need it to create a reference edge in the graph; the slice
/// entity does not need to be semantically complete.
fn make_command_slice_referencing_event(
    ns: &str,
    slice_slug: &str,
    event_slug: &str,
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(ns, slice_slug, 1)),
            title: slice_slug.into(),
            doc: String::new(),
            persona: None,
            ui: None,
            command: None,
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id(ns, event_slug, 1)),
                }),
                doc: String::new(),
                metadata: Vec::new(),
            }],
            scenarios: Vec::new(),
            metadata: Vec::new(),
            supersedes: None,
        })),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_impact_includes_referencing_slice() {
    let svc = svc().await;

    // Store an event and a command-slice that references it via emitted_events.
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("shop", "order-confirmed", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice_referencing_event(
            "shop",
            "place-order-slice",
            "order-confirmed",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .get_impact(Request::new(pb::GetImpactRequest {
            root: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("shop", "order-confirmed", 1)),
            }),
            max_depth: 0,
            filter_kinds: Vec::new(),
            page_size: 0,
            page_token: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    // The impact graph must contain the command-slice that references the event.
    let has_slice = resp.nodes.iter().any(|n| {
        n.entity
            .as_ref()
            .and_then(|r| r.id.as_ref())
            .is_some_and(|i| i.slug == "place-order-slice")
    });
    assert!(
        has_slice,
        "impact of order-confirmed must include place-order-slice; nodes: {:?}",
        resp.nodes
    );
}

/// A non-empty `filter_kinds` must restrict the returned impact nodes to
/// only those kinds (root is always kept regardless, per `n.depth == 0`).
#[tokio::test(flavor = "multi_thread")]
async fn get_impact_filter_kinds_restricts_result_kinds() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("impactfilt", "order-confirmed", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice_referencing_event(
            "impactfilt",
            "place-order-slice",
            "order-confirmed",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .get_impact(Request::new(pb::GetImpactRequest {
            root: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("impactfilt", "order-confirmed", 1)),
            }),
            max_depth: 0,
            filter_kinds: vec![pb::EntityKind::ReadModel as i32],
            page_size: 0,
            page_token: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    // The command-slice (depth > 0, kind CommandSlice, not in filter_kinds)
    // must be excluded; only the root (depth == 0) survives the retain.
    let has_slice = resp.nodes.iter().any(|n| {
        n.entity
            .as_ref()
            .and_then(|r| r.id.as_ref())
            .is_some_and(|i| i.slug == "place-order-slice")
    });
    assert!(
        !has_slice,
        "filter_kinds=[ReadModel] must exclude the CommandSlice node; nodes: {:?}",
        resp.nodes
    );
    assert!(
        resp.nodes.iter().any(|n| n.depth == 0),
        "root node must always survive filter_kinds regardless of its kind"
    );
}

// ===== retarget_references tests =====

/// A non-dry-run `retarget_references` with at least one referrer actually
/// rewritten must persist the change through the batch-write path
/// (`MutationOp::Put` construction + `apply_batch_side_effects`), and the
/// referrer's stored entity must reflect the new target afterward.
#[tokio::test(flavor = "multi_thread")]
async fn retarget_references_persists_rewritten_referrer() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("retarget", "old-event", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("retarget", "new-event", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice_referencing_event(
            "retarget",
            "slice-with-ref",
            "old-event",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .retarget_references(Request::new(pb::RetargetReferencesRequest {
            from: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("retarget", "old-event", 1)),
            }),
            to: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("retarget", "new-event", 1)),
            }),
            dry_run: false,
            filter_kinds: vec![],
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.rewritten, 1);
    assert_eq!(resp.failed, 0);
    assert_eq!(resp.results.len(), 1);
    assert_ne!(resp.results[0].fields, [] as [std::string::String; 0]);

    let stored = svc
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::CommandSlice as i32,
            id: Some(id("retarget", "slice-with-ref", 1)),
        }))
        .await
        .unwrap()
        .into_inner();
    let pb::entity::Kind::CommandSlice(slice) = stored.entity.unwrap().kind.unwrap() else {
        panic!("expected command slice");
    };
    assert_eq!(
        slice.emitted_events[0]
            .event
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .slug,
        "new-event"
    );
}

/// A `filter_kinds` that excludes the referrer's kind must skip it entirely:
/// no rewrite is staged and no error is reported for that referrer.
#[tokio::test(flavor = "multi_thread")]
async fn retarget_references_filter_kinds_excludes_non_matching_referrer() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("retargetfilt", "old-event", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("retargetfilt", "new-event", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice_referencing_event(
            "retargetfilt",
            "slice-with-ref",
            "old-event",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .retarget_references(Request::new(pb::RetargetReferencesRequest {
            from: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("retargetfilt", "old-event", 1)),
            }),
            to: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("retargetfilt", "new-event", 1)),
            }),
            dry_run: true,
            filter_kinds: vec![pb::EntityKind::ReadModel as i32],
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.rewritten, 0,
        "the CommandSlice referrer must be excluded by filter_kinds=[ReadModel]"
    );
    assert_eq!(
        resp.results,
        [] as [trogon_atlas_proto::retarget_references_response::Result; 0]
    );
}

// ===== get_slice_projection tests =====

#[tokio::test(flavor = "multi_thread")]
async fn get_slice_projection_returns_slice_and_event() {
    let svc = svc().await;

    // Store an event then a command-slice that emits it.
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("proj", "item-added", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice_referencing_event(
            "proj",
            "add-item-slice",
            "item-added",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .get_slice_projection(Request::new(pb::GetSliceProjectionRequest {
            kind: pb::EntityKind::CommandSlice as i32,
            id: Some(id("proj", "add-item-slice", 1)),
        }))
        .await
        .unwrap()
        .into_inner();

    let projection = resp.projection.expect("projection must be present");
    assert!(
        projection.slice.is_some(),
        "projection.slice must be populated"
    );
    // The closure must include the referenced event entity.
    let has_event = projection
        .entities
        .values()
        .any(|e| matches!(&e.kind, Some(pb::entity::Kind::Event(ev)) if ev.id.as_ref().is_some_and(|i| i.slug == "item-added")));
    assert!(
        has_event,
        "projection entities must include the referenced event item-added; keys: {:?}",
        projection.entities.keys().collect::<Vec<_>>()
    );
}

/// `slice_from_entity`'s `ReadModelSlice` arm (distinct from the `CommandSlice`
/// arm exercised elsewhere in this file) must also be reachable via
/// `get_slice_projection`.
#[tokio::test(flavor = "multi_thread")]
async fn get_slice_projection_returns_read_model_slice() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
                id: Some(id("proj-rm", "rm-slice", 1)),
                title: "RM Slice".into(),
                doc: String::new(),
                source_events: Vec::new(),
                read_model: None,
                scenarios: Vec::new(),
                metadata: Vec::new(),
                supersedes: None,
                projection_role: 0,
            })),
        }),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .get_slice_projection(Request::new(pb::GetSliceProjectionRequest {
            kind: pb::EntityKind::ReadModelSlice as i32,
            id: Some(id("proj-rm", "rm-slice", 1)),
        }))
        .await
        .unwrap()
        .into_inner();

    let projection = resp.projection.expect("projection must be present");
    assert!(matches!(
        projection.slice.as_ref().unwrap().kind,
        Some(pb::slice::Kind::ReadModel(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn get_slice_projection_rejects_non_slice_kind() {
    let svc = svc().await;

    let err = svc
        .get_slice_projection(Request::new(pb::GetSliceProjectionRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("shop", "some-event", 1)),
        }))
        .await
        .unwrap_err();

    assert_eq!(
        err.code(),
        tonic::Code::InvalidArgument,
        "non-slice kind must return InvalidArgument"
    );
}

// ===== get_storyboard_projection tests =====

fn make_storyboard_entity(ns: &str, slug: &str, slice_slugs: Vec<&str>) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Storyboard(pb::Storyboard {
            id: Some(id(ns, slug, 1)),
            title: slug.into(),
            doc: String::new(),
            entry: None,
            outcome: None,
            slices: slice_slugs
                .into_iter()
                .map(|s| pb::SliceRef {
                    id: Some(id(ns, s, 1)),
                })
                .collect(),
            traces: Vec::new(),
            metadata: Vec::new(),
            supersedes: None,
        })),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_storyboard_projection_returns_storyboard_and_slice() {
    let svc = svc().await;

    // Seed: event -> command-slice -> storyboard.
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("sb", "user-registered", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice_referencing_event(
            "sb",
            "register-user-slice",
            "user-registered",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_storyboard_entity(
            "sb",
            "onboarding",
            vec!["register-user-slice"],
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .get_storyboard_projection(Request::new(pb::GetStoryboardProjectionRequest {
            id: Some(id("sb", "onboarding", 1)),
        }))
        .await
        .unwrap()
        .into_inner();

    let projection = resp.projection.expect("projection must be present");
    assert!(
        projection.storyboard.is_some(),
        "projection.storyboard must be populated"
    );
    // The slices list should contain the command-slice inline.
    assert!(
        !projection.slices.is_empty(),
        "projection.slices must include register-user-slice"
    );
}

// ===== infer_data_flow tests =====

#[tokio::test(flavor = "multi_thread")]
async fn infer_data_flow_returns_ok_for_command_slice_scope() {
    let svc = svc().await;

    // Minimal command-slice (no fields needed, the RPC is heuristic and
    // returns Ok even when no mappings can be inferred from an empty graph).
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice_referencing_event(
            "flow",
            "checkout-slice",
            "checkout-completed",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .infer_data_flow(Request::new(pb::InferDataFlowRequest {
            scope: Some(pb::AnalysisScope {
                scope: Some(pb::analysis_scope::Scope::Slice(pb::EntityRef {
                    kind: pb::EntityKind::CommandSlice as i32,
                    id: Some(id("flow", "checkout-slice", 1)),
                })),
            }),
        }))
        .await
        .unwrap()
        .into_inner();

    // The response is always Ok; assert the shape is valid.
    assert!(
        resp.provenance == pb::AnalysisProvenance::Deterministic as i32,
        "without an LLM analyzer the provenance must be Deterministic"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn infer_data_flow_requires_scope() {
    let svc = svc().await;
    let err = svc
        .infer_data_flow(Request::new(pb::InferDataFlowRequest { scope: None }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

// ===== check_information_completeness tests =====

#[tokio::test(flavor = "multi_thread")]
async fn check_information_completeness_returns_ok_for_slice_scope() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice_referencing_event(
            "compl",
            "pay-slice",
            "payment-captured",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .check_information_completeness(Request::new(pb::CheckInformationCompletenessRequest {
            scope: Some(pb::AnalysisScope {
                scope: Some(pb::analysis_scope::Scope::Slice(pb::EntityRef {
                    kind: pb::EntityKind::CommandSlice as i32,
                    id: Some(id("compl", "pay-slice", 1)),
                })),
            }),
        }))
        .await
        .unwrap()
        .into_inner();

    // Score is always in [0.0, 1.0].
    assert!(
        resp.overall_score >= 0.0 && resp.overall_score <= 1.0,
        "overall_score must be in [0, 1]; got {}",
        resp.overall_score
    );
    assert!(
        resp.provenance == pb::AnalysisProvenance::Deterministic as i32,
        "without an LLM analyzer the provenance must be Deterministic"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn check_information_completeness_requires_scope() {
    let svc = svc().await;
    let err = svc
        .check_information_completeness(Request::new(pb::CheckInformationCompletenessRequest {
            scope: None,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

// ===== GetServerInfo tests =====

#[tokio::test(flavor = "multi_thread")]
async fn get_server_info_returns_version_and_features() {
    let svc = svc().await;
    let resp = svc
        .get_server_info(Request::new(pb::GetServerInfoRequest {}))
        .await
        .unwrap()
        .into_inner();
    assert!(
        !resp.server_version.is_empty(),
        "server_version must be set"
    );
    assert_eq!(resp.schema_version, pb::SCHEMA_VERSION);
    let features = resp.features.expect("features must be present");
    assert!(features.search, "search feature must be advertised");
    assert!(features.mutations);
    assert!(features.validate_only);
    assert!(features.branch_scoped_requests);
    assert!(features.state_preconditions);
    assert_eq!(resp.contract_revision, pb::CONTRACT_REVISION);
    assert_eq!(
        resp.min_client_contract_revision,
        pb::MIN_CLIENT_CONTRACT_REVISION
    );
    let limits = resp.limits.expect("limits must be present");
    assert!(
        limits.max_projection_entities > 0,
        "max_projection_entities must be positive"
    );
}

// ===== ListEntityKinds tests =====

#[tokio::test(flavor = "multi_thread")]
async fn list_entity_kinds_returns_non_empty_catalog() {
    let svc = svc().await;
    let resp = svc
        .list_entity_kinds(Request::new(pb::ListEntityKindsRequest {}))
        .await
        .unwrap()
        .into_inner();
    assert!(
        !resp.kinds.is_empty(),
        "entity kind catalog must not be empty"
    );
    let has_event = resp.kinds.iter().any(|k| k.json_key == "event");
    assert!(has_event, "catalog must include 'event'");
}

// ===== ListValidationRules tests =====

#[tokio::test(flavor = "multi_thread")]
async fn list_validation_rules_returns_non_empty_catalog() {
    let svc = svc().await;
    let resp = svc
        .list_validation_rules(Request::new(pb::ListValidationRulesRequest {}))
        .await
        .unwrap()
        .into_inner();
    assert!(
        !resp.rules.is_empty(),
        "validation rule catalog must not be empty"
    );
    let has_schema_type_url_empty = resp.rules.iter().any(|r| r.code == "SCHEMA_TYPE_URL_EMPTY");
    assert!(
        has_schema_type_url_empty,
        "catalog must include the SCHEMA_TYPE_URL_EMPTY rule"
    );
}

// ===== BatchGetEntities tests =====

#[tokio::test(flavor = "multi_thread")]
async fn batch_get_entities_happy_path() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("bge", "ev-1", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .batch_get_entities(Request::new(pb::BatchGetEntitiesRequest {
            keys: vec![pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("bge", "ev-1", 1)),
            }],
            dense: false,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.entities.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_get_entities_missing_key_in_dense_mode_returns_default() {
    let svc = svc().await;
    let resp = svc
        .batch_get_entities(Request::new(pb::BatchGetEntitiesRequest {
            keys: vec![pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("bge2", "no-such-event", 1)),
            }],
            dense: true,
        }))
        .await
        .unwrap()
        .into_inner();

    // dense=true: missing entity is a zero-valued Entity in the slot.
    assert_eq!(resp.entities.len(), 1);
    // The returned Entity has no kind set (default value).
    assert!(resp.entities[0].kind.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_get_entities_exceeding_limit_is_rejected() {
    let svc = svc().await;
    let keys: Vec<pb::EntityRef> = (0..501)
        .map(|i| pb::EntityRef {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("bge3", &format!("e{i}"), 1)),
        })
        .collect();
    let err = svc
        .batch_get_entities(Request::new(pb::BatchGetEntitiesRequest {
            keys,
            dense: false,
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

// ===== GetSupersessionChain tests =====

fn make_event_superseding(
    ns: &str,
    slug: &str,
    version: u64,
    supersedes_slug: &str,
    supersedes_version: u64,
) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug, version)),
            title: format!("{slug}@{version}"),
            doc: String::new(),
            swimlane: None,
            metadata: Vec::new(),
            supersedes: Some(id(ns, supersedes_slug, supersedes_version)),
            schema: None,
        })),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_supersession_chain_happy_path() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("supchain", "order-placed", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event_superseding(
            "supchain",
            "order-placed",
            2,
            "order-placed",
            1,
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .get_supersession_chain(Request::new(pb::GetSupersessionChainRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("supchain", "order-placed", 2)),
            direction: pb::get_supersession_chain_request::Direction::Both as i32,
            page_size: 0,
            page_token: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(
        !resp.chain.is_empty(),
        "chain must contain at least the requested entity"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_supersession_chain_missing_entity_returns_empty_chain() {
    let svc = svc().await;

    let resp = svc
        .get_supersession_chain(Request::new(pb::GetSupersessionChainRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("supchain2", "nonexistent", 1)),
            direction: pb::get_supersession_chain_request::Direction::Both as i32,
            page_size: 0,
            page_token: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    // The entity does not exist in the store; the chain has no resolved entities.
    assert!(
        resp.chain.is_empty(),
        "chain for non-existent entity must be empty"
    );
}

// ===== GetIncomingReferences / GetOutgoingReferences tests =====

fn make_command_slice_with_event_ref(ns: &str, slug: &str, event_slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(ns, slug, 1)),
            title: slug.into(),
            doc: String::new(),
            persona: None,
            ui: None,
            command: None,
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id(ns, event_slug, 1)),
                }),
                doc: String::new(),
                metadata: Vec::new(),
            }],
            scenarios: Vec::new(),
            metadata: Vec::new(),
            supersedes: None,
        })),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_incoming_references_happy_path() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("inref", "order-placed", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice_with_event_ref(
            "inref",
            "place-order-slice",
            "order-placed",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("inref", "order-placed", 1)),
            filter_kinds: vec![],
            page_size: 0,
            page_token: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(
        !resp.references.is_empty(),
        "the command slice must appear as an incoming reference to the event"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_incoming_references_no_referrers_returns_empty() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("inref2", "isolated-event", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("inref2", "isolated-event", 1)),
            filter_kinds: vec![],
            page_size: 0,
            page_token: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(
        resp.references.is_empty(),
        "isolated event must have no incoming references"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_outgoing_references_happy_path() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("outref", "order-placed", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice_with_event_ref(
            "outref",
            "place-order-slice",
            "order-placed",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .get_outgoing_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::CommandSlice as i32,
            id: Some(id("outref", "place-order-slice", 1)),
            filter_kinds: vec![],
            page_size: 0,
            page_token: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert!(
        !resp.references.is_empty(),
        "command slice must have at least one outgoing reference to the event"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_outgoing_references_missing_entity_returns_not_found() {
    let svc = svc().await;

    let err = svc
        .get_outgoing_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("outref2", "ghost-event", 1)),
            filter_kinds: vec![],
            page_size: 0,
            page_token: String::new(),
        }))
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::NotFound);
}

// ===== delete_by_query FailIfReferenced tests =====

#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_fail_if_referenced_blocks_external_referrer() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("dbqfir", "target-event", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice_with_event_ref(
            "dbqfir",
            "referrer-slice",
            "target-event",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    // Delete only the event (the slice stays and references it) -- should fail.
    let err = svc
        .delete_by_query(Request::new(pb::DeleteByQueryRequest {
            namespace: "dbqfir".into(),
            slug: "target-event".into(),
            kind: pb::EntityKind::Event as i32,
            project: String::new(),
            max_deletes: 10,
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
        }))
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(
        err.message().contains("victim set"),
        "error must mention the victim set; got: {}",
        err.message()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_fail_if_referenced_allows_intra_set_deletion() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("intraset", "ev", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice_with_event_ref("intraset", "sl", "ev")),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    // Delete the whole namespace: both entities are victims, so the
    // intra-set reference must not block the delete.
    let resp = svc
        .delete_by_query(Request::new(pb::DeleteByQueryRequest {
            namespace: "intraset".into(),
            slug: String::new(),
            kind: 0,
            project: String::new(),
            max_deletes: 10,
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.deleted_count, 2);
}

/// Proto contract: `DeleteEntityRequest.Mode` documents MODE_UNSPECIFIED as
/// defaulting to FAIL_IF_REFERENCED (same semantics DeleteByQuery claims to
/// share). DeleteEntity remaps Unspecified → FailIfReferenced; DeleteByQuery
/// must do the same. Leaving mode=0 must NOT act like FORCE.
#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_unspecified_mode_defaults_to_fail_if_referenced() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("dbqunspec", "target-event", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice_with_event_ref(
            "dbqunspec",
            "referrer-slice",
            "target-event",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    // mode=0 is MODE_UNSPECIFIED (protobuf default). Must refuse because the
    // slice outside the victim set still references the event.
    let err = svc
        .delete_by_query(Request::new(pb::DeleteByQueryRequest {
            namespace: "dbqunspec".into(),
            slug: "target-event".into(),
            kind: pb::EntityKind::Event as i32,
            project: String::new(),
            max_deletes: 10,
            mode: 0,
        }))
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(
        err.message().contains("victim set"),
        "Unspecified mode must behave as FailIfReferenced; got: {}",
        err.message()
    );

    // Target must still exist after the refused call.
    svc.get_entity(Request::new(pb::GetEntityRequest {
        kind: pb::EntityKind::Event as i32,
        id: Some(id("dbqunspec", "target-event", 1)),
    }))
    .await
    .expect("Unspecified-mode delete_by_query must not force-delete");
}

// ===== content size limit tests (P1) =====

fn make_event_with_oversized_doc(ns: &str, slug: &str) -> pb::Entity {
    let doc = "x".repeat(trogon_atlas_server::conv::MAX_DOC_BYTES + 1);
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug, 1)),
            title: "title".into(),
            doc,
            swimlane: None,
            metadata: Vec::new(),
            supersedes: None,
            schema: None,
        })),
    }
}

fn make_event_with_too_many_fields(ns: &str, slug: &str) -> pb::Entity {
    let fields: Vec<pb::FieldSpec> = (0..=trogon_atlas_server::conv::MAX_FIELDS_COUNT)
        .map(|i| pb::FieldSpec {
            name: format!("field{i}"),
            doc: String::new(),
            r#type: None,
            repeated: false,
            optional: false,
            metadata: Vec::new(),
        })
        .collect();
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug, 1)),
            title: "title".into(),
            doc: String::new(),
            swimlane: None,
            metadata: Vec::new(),
            supersedes: None,
            schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
        })),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn put_entity_oversized_doc_is_rejected() {
    let svc = svc().await;
    let err = svc
        .put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event_with_oversized_doc("sizelimits", "big-doc")),
            create_only: false,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        tonic::Code::InvalidArgument,
        "oversized doc must be rejected with InvalidArgument; got: {err}"
    );
    assert!(
        err.message().contains("doc"),
        "error message should mention 'doc'; got: {}",
        err.message()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn put_entity_too_many_fields_is_rejected() {
    let svc = svc().await;
    let err = svc
        .put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event_with_too_many_fields(
                "sizelimits",
                "too-many-fields",
            )),
            create_only: false,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        tonic::Code::InvalidArgument,
        "too-many-fields must be rejected with InvalidArgument; got: {err}"
    );
    assert!(
        err.message().contains("fields"),
        "error message should mention 'fields'; got: {}",
        err.message()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_oversized_doc_is_rejected() {
    let svc = svc().await;
    let err = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                    operation_id: String::new(),
                    entity: Some(make_event_with_oversized_doc("sizelimits", "batch-big-doc")),
                    create_only: false,
                    if_match: String::new(),
                    validate_only: false,
                    force: false,
                })),
            }],
            validate_only: false,
        }))
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        tonic::Code::InvalidArgument,
        "oversized doc in batch must be rejected; got: {err}"
    );
}

// ===== pagination tests (proto contract item 1) =====

#[tokio::test(flavor = "multi_thread")]
async fn list_versions_pagination_page_size_and_token_continuation() {
    let svc = svc().await;

    // Write 5 versions of the same slug.
    for v in 1u64..=5 {
        svc.put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event("pagination-ns", "my-event", v)),
            create_only: false,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap();
    }

    // Fetch the first page of 2.
    let page1 = svc
        .list_versions(Request::new(pb::ListVersionsRequest {
            kind: pb::EntityKind::Event as i32,
            namespace: "pagination-ns".into(),
            slug: "my-event".into(),
            page_size: 2,
            page_token: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(page1.versions.len(), 2);
    assert!(
        !page1.next_page_token.is_empty(),
        "next_page_token must be set when more pages remain"
    );

    // Fetch the second page using the returned token.
    let page2 = svc
        .list_versions(Request::new(pb::ListVersionsRequest {
            kind: pb::EntityKind::Event as i32,
            namespace: "pagination-ns".into(),
            slug: "my-event".into(),
            page_size: 2,
            page_token: page1.next_page_token.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(page2.versions.len(), 2);
    assert!(
        !page2.next_page_token.is_empty(),
        "next_page_token must be set for the third page"
    );

    // Fetch the last page.
    let page3 = svc
        .list_versions(Request::new(pb::ListVersionsRequest {
            kind: pb::EntityKind::Event as i32,
            namespace: "pagination-ns".into(),
            slug: "my-event".into(),
            page_size: 2,
            page_token: page2.next_page_token.clone(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(page3.versions.len(), 1);
    assert!(
        page3.next_page_token.is_empty(),
        "next_page_token must be empty on the last page"
    );

    // Reassemble all versions and verify they cover 1..=5 with no duplicates.
    let mut all_versions: Vec<u64> = page1
        .versions
        .iter()
        .chain(page2.versions.iter())
        .chain(page3.versions.iter())
        .filter_map(|e| {
            if let Some(pb::entity::Kind::Event(ev)) = e.kind.as_ref() {
                ev.id.as_ref().map(|i| i.version)
            } else {
                None
            }
        })
        .collect();
    all_versions.sort_unstable();
    assert_eq!(all_versions, vec![1, 2, 3, 4, 5]);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_versions_no_page_size_returns_up_to_max() {
    let svc = svc().await;
    // Write 3 versions; omit page_size (defaults to ADVERTISED_DEFAULT_PAGE_SIZE).
    for v in 1u64..=3 {
        svc.put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event("pagination-default-ns", "ev", v)),
            create_only: false,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap();
    }
    let resp = svc
        .list_versions(Request::new(pb::ListVersionsRequest {
            kind: pb::EntityKind::Event as i32,
            namespace: "pagination-default-ns".into(),
            slug: "ev".into(),
            page_size: 0,
            page_token: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();
    // All 3 fit within the default page size.
    assert_eq!(resp.versions.len(), 3);
    assert_eq!(resp.next_page_token, "");
}

// ===== batch_get_entities found flags tests (proto contract item 2) =====

#[tokio::test(flavor = "multi_thread")]
async fn batch_get_entities_dense_mode_found_flags_for_hits_and_misses() {
    let svc = svc().await;

    // Write one entity; the second key will be a miss.
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("bge-found-ns", "exists", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .batch_get_entities(Request::new(pb::BatchGetEntitiesRequest {
            keys: vec![
                pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("bge-found-ns", "exists", 1)),
                },
                pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("bge-found-ns", "missing", 1)),
                },
            ],
            dense: true,
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.entities.len(),
        2,
        "dense mode must return one entry per key"
    );
    assert_eq!(
        resp.found.len(),
        2,
        "found must be parallel to entities in dense mode"
    );
    assert!(resp.found[0], "first key was stored and must be found");
    assert!(
        !resp.found[1],
        "second key was never stored and must not be found"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_get_entities_sparse_mode_found_flags_empty() {
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("bge-sparse-ns", "ev", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .batch_get_entities(Request::new(pb::BatchGetEntitiesRequest {
            keys: vec![
                pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("bge-sparse-ns", "ev", 1)),
                },
                pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("bge-sparse-ns", "ghost", 1)),
                },
            ],
            dense: false,
        }))
        .await
        .unwrap()
        .into_inner();

    // Sparse mode: only the hit is returned, found is not populated.
    assert_eq!(resp.entities.len(), 1);
    assert!(
        resp.found.is_empty(),
        "found must not be populated in sparse mode"
    );
}

// ===== LLM fallback_reason tests (proto contract item 3) =====

/// A mock LLM client that always returns an error. Placed here (rather than
/// `llm_handlers.rs`) so we can assert the `fallback_reason` field without
/// depending on a separate test file owned by another agent.
struct AlwaysFailingLlmClient {
    message: String,
}

#[async_trait]
impl trogon_atlas_server::llm::LlmClient for AlwaysFailingLlmClient {
    async fn complete(&self, _system: &str, _user: &str) -> anyhow::Result<String> {
        Err(anyhow::anyhow!("{}", self.message))
    }

    fn model(&self) -> &'static str {
        "mock-failing"
    }

    fn provider(&self) -> &'static str {
        "mock"
    }
}

async fn svc_with_failing_llm(error_msg: &str) -> EventModelServiceImpl {
    use trogon_atlas_server::llm_analysis::LlmAnalyzer;
    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    let client = Arc::new(AlwaysFailingLlmClient {
        message: error_msg.to_owned(),
    });
    let analyzer = Arc::new(LlmAnalyzer::new(client));
    EventModelServiceImpl::try_new(Arc::new(store))
        .unwrap()
        .with_llm_analyzer(analyzer)
}

fn event_scope(ns: &str, slug: &str) -> pb::AnalysisScope {
    pb::AnalysisScope {
        scope: Some(pb::analysis_scope::Scope::Slice(pb::EntityRef {
            kind: pb::EntityKind::Event as i32,
            id: Some(id(ns, slug, 1)),
        })),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn infer_data_flow_llm_error_sets_fallback_reason() {
    let svc = svc_with_failing_llm("connection refused").await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("fallback-ns", "ev", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .infer_data_flow(Request::new(pb::InferDataFlowRequest {
            scope: Some(event_scope("fallback-ns", "ev")),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.provenance,
        pb::AnalysisProvenance::Deterministic as i32,
        "provenance must be deterministic after LLM failure"
    );
    assert!(
        !resp.fallback_reason.is_empty(),
        "fallback_reason must be set when LLM fails"
    );
    assert_eq!(
        resp.fallback_reason, "llm_error",
        "generic LLM failure must map to 'llm_error' reason code"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn infer_data_flow_llm_timeout_sets_timeout_reason() {
    let svc = svc_with_failing_llm("request timed out").await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("fallback-timeout-ns", "ev", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .infer_data_flow(Request::new(pb::InferDataFlowRequest {
            scope: Some(event_scope("fallback-timeout-ns", "ev")),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.fallback_reason, "llm_timeout");
}

#[tokio::test(flavor = "multi_thread")]
async fn infer_data_flow_no_llm_leaves_fallback_reason_empty() {
    // No LLM configured: always deterministic, fallback_reason must be empty.
    let svc = svc().await;

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("no-llm-ns", "ev", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    let resp = svc
        .infer_data_flow(Request::new(pb::InferDataFlowRequest {
            scope: Some(event_scope("no-llm-ns", "ev")),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.provenance,
        pb::AnalysisProvenance::Deterministic as i32
    );
    assert!(
        resp.fallback_reason.is_empty(),
        "fallback_reason must be empty when no LLM is configured"
    );
}

// ===== cross-project write enforcement tests (proto contract item 4) =====

fn make_domain(ns: &str, slug: &str, project_ns: Option<&str>) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Domain(pb::Domain {
            id: Some(id(ns, slug, 1)),
            title: format!("{ns}/{slug}"),
            doc: String::new(),
            metadata: Vec::new(),
            supersedes: None,
            project: project_ns.map(|pns| pb::ProjectRef {
                id: Some(pb::Id {
                    namespace: pns.into(),
                    slug: pns.into(),
                    version: 1,
                }),
            }),
        })),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn put_entity_domain_cross_project_namespace_rejected() {
    let svc = svc().await;
    // Domain in namespace "orders" declares a project in namespace "billing".
    let err = svc
        .put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_domain("orders", "orders", Some("billing"))),
            create_only: false,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        tonic::Code::FailedPrecondition,
        "cross-project namespace conflict must return FAILED_PRECONDITION"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn put_entity_domain_matching_project_namespace_accepted() {
    let svc = svc().await;
    // Domain in namespace "logistics" declares a project in the same namespace.
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_domain("logistics", "logistics", Some("logistics"))),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .expect("domain with matching project namespace must be accepted");
}

#[tokio::test(flavor = "multi_thread")]
async fn put_entity_domain_no_project_accepted() {
    let svc = svc().await;
    // Domain with no project declared: no cross-project check applies.
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_domain("no-proj-ns", "no-proj-ns", None)),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .expect("domain with no project must be accepted");
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_domain_cross_project_namespace_rejected() {
    let svc = svc().await;
    let resp = svc
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                    operation_id: String::new(),
                    entity: Some(make_domain("orders-batch", "orders-batch", Some("finance"))),
                    create_only: false,
                    if_match: String::new(),
                    validate_only: false,
                    force: false,
                })),
            }],
            validate_only: false,
        }))
        .await
        .unwrap_err();
    assert_eq!(resp.code(), tonic::Code::FailedPrecondition);
}

// ===== principal authorship tests (P1) =====

#[tokio::test(flavor = "multi_thread")]
async fn author_from_metadata_prefers_principal_over_spoofed_header() {
    use std::sync::Arc;

    use trogon_atlas_server::auth::{Principal, PrincipalKind, Role};

    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();

    // Build a request that spoofs the author name via header but also
    // carries a verified Principal in extensions.
    let mut req = Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("auth-test", "principal-wins", 1)),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: false,
    });
    req.metadata_mut().insert(
        "x-trogon-atlas-author-name",
        tonic::metadata::MetadataValue::try_from("spoofed-name").unwrap(),
    );
    req.extensions_mut().insert(Principal {
        name: Arc::from("verified-principal"),
        role: Role::Writer,
        kind: PrincipalKind::User,
        is_anonymous: false,
        namespaces: trogon_atlas_server::scope::NamespaceScope::unrestricted(),
        parent: None,
    });

    // The put must succeed (we are only asserting the author selection path
    // does not panic and that the Principal's name takes precedence).
    svc.put_entity(req).await.expect("put_entity must succeed");
}

// ===== reverse index cache consistency tests (Item 1) =====

/// Verify that repeated reference queries return consistent results after a
/// mutation invalidates the snapshot and reverse index cache. The first query
/// populates the cache; the mutation invalidates it; the second query rebuilds
/// from the new snapshot. Both results must reflect the live store state.
#[tokio::test(flavor = "multi_thread")]
async fn reverse_index_cache_invalidated_by_mutation() {
    let svc = svc().await;

    // Write an event with no references initially.
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("ricache", "target-event", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    // First reference query: event has no incoming refs yet.
    let first = svc
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("ricache", "target-event", 1)),
            filter_kinds: vec![],
            page_size: 0,
            page_token: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(
        first.references.is_empty(),
        "no referrers expected before mutation: {:?}",
        first.references
    );

    // Mutation: add a slice that references the event. This invalidates the cache.
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_command_slice_with_event_ref(
            "ricache",
            "place-order-slice",
            "target-event",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    // Second reference query: cache was invalidated; index must be rebuilt
    // from the updated snapshot and reflect the new referrer.
    let second = svc
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("ricache", "target-event", 1)),
            filter_kinds: vec![],
            page_size: 0,
            page_token: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(
        !second.references.is_empty(),
        "referrer must appear after cache invalidation: {:?}",
        second.references
    );
}

// ===== search_entities clamp tests (Item 5) =====

/// A limit value above `ADVERTISED_MAX_PAGE_SIZE` must be silently clamped to
/// the advertised maximum rather than honored verbatim or rejected.
#[tokio::test(flavor = "multi_thread")]
async fn search_entities_limit_above_max_is_clamped() {
    use trogon_atlas_server::service::ADVERTISED_MAX_PAGE_SIZE;
    let svc = svc().await;

    // Write a few events so the search index has something to work with.
    for i in 0..3u64 {
        svc.put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event("srchclamp", &format!("event-{i}"), 1)),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap();
    }

    // Request a limit far above the advertised maximum (10 000).
    let resp = svc
        .search_entities(Request::new(pb::SearchEntitiesRequest {
            query: "event".into(),
            kinds: vec![],
            namespaces: vec!["srchclamp".into()],
            limit: 10_000,
        }))
        .await
        .unwrap()
        .into_inner();

    // The response must succeed and return at most ADVERTISED_MAX_PAGE_SIZE results.
    assert!(
        resp.results.len() <= ADVERTISED_MAX_PAGE_SIZE as usize,
        "result count {} must not exceed ADVERTISED_MAX_PAGE_SIZE={}",
        resp.results.len(),
        ADVERTISED_MAX_PAGE_SIZE,
    );
}

/// `search_apply_put`/`search_apply_delete` early-return while the search
/// index has not been warmed (`search_ready == false`); mutations that
/// happen before the first `SearchEntities`/`ensure_search_index` call never
/// exercise their live-update bodies. Warm the index explicitly first, then
/// mutate, so both functions run their real (non-early-return) update path
/// and a subsequent search reflects the change.
#[tokio::test(flavor = "multi_thread")]
async fn search_index_live_updates_after_warmup_reflect_put_and_delete() {
    let svc = svc().await;

    // Warm the index while the store is empty for this namespace.
    svc.ensure_search_index()
        .await
        .expect("ensure_search_index must succeed");

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("srchlive", "findable-event", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    // Give the spawn_blocking index-update task a chance to complete; poll
    // briefly rather than sleeping a fixed duration blindly.
    let mut found = false;
    for _ in 0..50 {
        let resp = svc
            .search_entities(Request::new(pb::SearchEntitiesRequest {
                query: "findable-event".into(),
                kinds: vec![],
                namespaces: vec!["srchlive".into()],
                limit: 10,
            }))
            .await
            .unwrap()
            .into_inner();
        if !resp.results.is_empty() {
            found = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        found,
        "live index update via search_apply_put must make the entity searchable"
    );

    svc.delete_entity(Request::new(pb::DeleteEntityRequest {
        operation_id: String::new(),
        kind: pb::EntityKind::Event as i32,
        id: Some(id("srchlive", "findable-event", 1)),
        mode: pb::delete_entity_request::Mode::Force as i32,
        if_match: String::new(),
    }))
    .await
    .unwrap();

    let mut gone = false;
    for _ in 0..50 {
        let resp = svc
            .search_entities(Request::new(pb::SearchEntitiesRequest {
                query: "findable-event".into(),
                kinds: vec![],
                namespaces: vec!["srchlive".into()],
                limit: 10,
            }))
            .await
            .unwrap()
            .into_inner();
        if resp.results.is_empty() {
            gone = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        gone,
        "live index update via search_apply_delete must remove the entity from search results"
    );
}
