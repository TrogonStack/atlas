#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use async_trait::async_trait;
use pb::event_model_service_server::EventModelService;
use tokio::sync::{broadcast, Notify};
use tonic::{Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};
use trogon_atlas_store::{
    store::{
        BranchInfo, ChangeRecord, ChangeStreamStats, ListFilter, MutationOp, MutationOutcome,
        Store, Written,
    },
    ChangeKind, ChangesetPage, ChangesetRecord, ChangesetRef, NatsStore, StoreError, StoredEntity,
    WriteContext,
};

#[derive(Default)]
struct WriteGate {
    armed: AtomicBool,
    entered: Notify,
    release: Notify,
}

impl WriteGate {
    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    async fn hold(&self) {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
    }
}

struct GateStore {
    inner: Arc<NatsStore>,
    gates: Vec<(String, Arc<WriteGate>)>,
    guard_failure: Arc<AtomicBool>,
    scan_overflow: Arc<AtomicBool>,
}

impl GateStore {
    fn new(inner: Arc<NatsStore>) -> Self {
        Self {
            inner,
            gates: Vec::new(),
            guard_failure: Arc::default(),
            scan_overflow: Arc::default(),
        }
    }

    fn gated(mut self, slug: &str) -> (Self, Arc<WriteGate>) {
        let gate = Arc::new(WriteGate::default());
        self.gates.push((slug.into(), gate.clone()));
        (self, gate)
    }

    async fn hold(&self, entity: &pb::Entity) {
        let Some(slug) = slug_of(entity) else { return };
        for (target, gate) in &self.gates {
            if *target == slug {
                gate.hold().await;
            }
        }
    }

    fn guard(&self) -> trogon_atlas_store::StoreResult<()> {
        if self.guard_failure.load(Ordering::SeqCst) {
            return Err(StoreError::Backend("injected guard failure".into()));
        }
        Ok(())
    }
}

fn slug_of(entity: &pb::Entity) -> Option<String> {
    match entity.kind.as_ref()? {
        pb::entity::Kind::SystemGroup(value) => value.id.as_ref().map(|id| id.slug.clone()),
        pb::entity::Kind::Domain(value) => value.id.as_ref().map(|id| id.slug.clone()),
        pb::entity::Kind::Component(value) => value.id.as_ref().map(|id| id.slug.clone()),
        _ => None,
    }
}

#[async_trait]
impl Store for GateStore {
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
        self.hold(entity).await;
        self.inner.create(kind, entity, ctx).await
    }

    async fn put(
        &self,
        kind: pb::EntityKind,
        entity: &pb::Entity,
        force: bool,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Written> {
        self.hold(entity).await;
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
        self.hold(entity).await;
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

    async fn referenced_targets(
        &self,
        targets: &[pb::EntityKey],
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<std::collections::HashSet<pb::EntityKey>> {
        self.guard()?;
        self.inner.referenced_targets(targets, branch).await
    }

    async fn referenced_targets_with_overlay(
        &self,
        targets: &[pb::EntityKey],
        branch: Option<&str>,
        overlay: &[(pb::EntityKey, Option<pb::Entity>)],
    ) -> trogon_atlas_store::StoreResult<std::collections::HashSet<pb::EntityKey>> {
        self.guard()?;
        self.inner
            .referenced_targets_with_overlay(targets, branch, overlay)
            .await
    }

    async fn has_incoming_reference(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<bool> {
        self.guard()?;
        self.inner.has_incoming_reference(kind, id, branch).await
    }

    async fn list(
        &self,
        filter: ListFilter<'_>,
        limit: Option<usize>,
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<Vec<StoredEntity>> {
        if self.scan_overflow.load(Ordering::SeqCst)
            && (filter.kinds.contains(&pb::EntityKind::Channel)
                || filter.kinds.contains(&pb::EntityKind::DataStore)
                || filter.kinds.contains(&pb::EntityKind::Specification))
        {
            return Ok((0..=5000)
                .map(|index| StoredEntity {
                    entity: pb::Entity {
                        kind: Some(pb::entity::Kind::Person(pb::Person {
                            id: Some(id(&format!("person-{index}"))),
                            ..Default::default()
                        })),
                        ..Default::default()
                    },
                    etag: index.to_string(),
                })
                .collect());
        }
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
        ops: &[MutationOp],
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<MutationOutcome>> {
        self.inner.batch_apply(ops, ctx).await
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

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "gates".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn component(slug: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Component(pb::Component {
            id: Some(id(slug)),
            title: slug.into(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn member(target: pb::system_group_member::Target) -> pb::SystemGroupMembership {
    pb::SystemGroupMembership {
        member: Some(pb::SystemGroupMember {
            target: Some(target),
        }),
        ..Default::default()
    }
}

fn member_group(slug: &str) -> pb::SystemGroupMembership {
    member(pb::system_group_member::Target::SystemGroup(
        pb::SystemGroupRef { id: Some(id(slug)) },
    ))
}

fn member_component(slug: &str) -> pb::SystemGroupMembership {
    member(pb::system_group_member::Target::Component(
        pb::ComponentRef { id: Some(id(slug)) },
    ))
}

fn group(slug: &str, members: Vec<pb::SystemGroupMembership>) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::SystemGroup(pb::SystemGroup {
            id: Some(id(slug)),
            title: slug.into(),
            scope: pb::SystemGroupScope::Internal as i32,
            members,
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn with_member(mut owner: pb::Entity, added: pb::SystemGroupMembership) -> pb::Entity {
    let Some(pb::entity::Kind::SystemGroup(value)) = owner.kind.as_mut() else {
        panic!("system group");
    };
    value.members.push(added);
    owner
}

fn members_of(owner: &pb::Entity) -> &[pb::SystemGroupMembership] {
    let Some(pb::entity::Kind::SystemGroup(value)) = owner.kind.as_ref() else {
        panic!("system group");
    };
    &value.members
}

fn service_over(store: Arc<dyn Store>) -> Arc<EventModelServiceImpl> {
    Arc::new(EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap())
}

async fn shared_store() -> Arc<NatsStore> {
    Arc::new(trogon_atlas_testsupport::shared().await.store().await)
}

async fn put(
    service: &EventModelServiceImpl,
    entity: pb::Entity,
    etag: &str,
) -> Result<pb::PutEntityResponse, tonic::Status> {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity),
            if_match: etag.into(),
            ..Default::default()
        }))
        .await
        .map(tonic::Response::into_inner)
}

async fn read(service: &EventModelServiceImpl, slug: &str) -> pb::GetEntityResponse {
    service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::SystemGroup as i32,
            id: Some(id(slug)),
        }))
        .await
        .unwrap()
        .into_inner()
}

async fn incoming_from(
    service: &EventModelServiceImpl,
    kind: pb::EntityKind,
    slug: &str,
) -> Vec<String> {
    service
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: kind as i32,
            id: Some(id(slug)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references
        .into_iter()
        .map(|reference| {
            let from = reference.from.unwrap().id.unwrap().slug;
            format!("{from}:{}", reference.from_field)
        })
        .collect()
}

async fn cycle_findings(store: Arc<NatsStore>) -> usize {
    service_over(store)
        .validate_project(Request::new(pb::ValidateProjectRequest {
            scope: Some(pb::validate_project_request::Scope::Namespace(
                "gates".into(),
            )),
        }))
        .await
        .unwrap()
        .into_inner()
        .reports
        .into_iter()
        .flat_map(|report| report.issues)
        .filter(|issue| issue.code == "SYSTEM_GROUP_MEMBERSHIP_CYCLE")
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn independent_writers_each_publish_one_half_of_a_cycle_without_a_partial_view() {
    let inner = shared_store().await;
    let (gated, gate) = GateStore::new(inner.clone()).gated("alpha");
    let first = service_over(Arc::new(gated));
    let second = service_over(Arc::new(GateStore::new(inner.clone())));
    let alpha = put(&second, group("alpha", vec![]), "").await.unwrap();
    let beta = put(&second, group("beta", vec![]), "").await.unwrap();

    gate.arm();
    let writer = tokio::spawn({
        let first = first.clone();
        let edit = with_member(alpha.entity.clone().unwrap(), member_group("beta"));
        let etag = alpha.etag.clone();
        async move { put(&first, edit, &etag).await }
    });
    gate.entered.notified().await;

    for service in [&first, &second] {
        let paused = read(service, "alpha").await;
        assert_eq!(paused.entity, alpha.entity);
        assert_eq!(paused.etag, alpha.etag);
    }
    let other_half = put(
        &second,
        with_member(beta.entity.clone().unwrap(), member_group("alpha")),
        &beta.etag,
    )
    .await
    .unwrap();
    assert!(!other_half.no_op);
    assert_eq!(read(&first, "alpha").await.entity, alpha.entity);
    assert_eq!(
        incoming_from(&second, pb::EntityKind::SystemGroup, "alpha").await,
        ["beta:members[0].system_group"]
    );
    assert_eq!(
        incoming_from(&second, pb::EntityKind::SystemGroup, "beta").await,
        [] as [String; 0]
    );
    assert_eq!(cycle_findings(inner.clone()).await, 0);

    gate.release.notify_one();
    let published = writer.await.unwrap().unwrap();
    assert!(!published.no_op);

    let final_alpha = read(&second, "alpha").await;
    let final_beta = read(&second, "beta").await;
    let reader = service_over(inner.clone());
    assert_eq!(final_alpha.entity, published.entity);
    assert_eq!(final_beta.entity, other_half.entity);
    for owner in [&final_alpha, &final_beta] {
        let links = members_of(owner.entity.as_ref().unwrap());
        assert_eq!(links.len(), 1);
        assert!(links[0].connection_id.is_some());
    }
    assert_eq!(
        incoming_from(&reader, pb::EntityKind::SystemGroup, "alpha").await,
        ["beta:members[0].system_group"]
    );
    assert_eq!(
        incoming_from(&reader, pb::EntityKind::SystemGroup, "beta").await,
        ["alpha:members[0].system_group"]
    );
    assert_eq!(cycle_findings(inner).await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn halves_queued_on_one_service_both_publish_after_the_gated_writer_releases() {
    let inner = shared_store().await;
    let (gated, gate) = GateStore::new(inner.clone()).gated("alpha");
    let service = service_over(Arc::new(gated));
    let alpha = put(&service, group("alpha", vec![]), "").await.unwrap();
    let beta = put(&service, group("beta", vec![]), "").await.unwrap();

    gate.arm();
    let first = tokio::spawn({
        let service = service.clone();
        let edit = with_member(alpha.entity.clone().unwrap(), member_group("beta"));
        let etag = alpha.etag.clone();
        async move { put(&service, edit, &etag).await }
    });
    gate.entered.notified().await;
    let second = tokio::spawn({
        let service = service.clone();
        let edit = with_member(beta.entity.clone().unwrap(), member_group("alpha"));
        let etag = beta.etag.clone();
        async move { put(&service, edit, &etag).await }
    });
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    assert!(!second.is_finished());
    let queued = inner
        .get(pb::EntityKind::SystemGroup, &id("beta"), None)
        .await
        .unwrap();
    assert_eq!(queued.etag, beta.etag);

    gate.release.notify_one();
    let first = first.await.unwrap().unwrap();
    let second = second.await.unwrap().unwrap();
    assert_eq!(read(&service, "alpha").await.entity, first.entity);
    assert_eq!(read(&service, "beta").await.entity, second.entity);
    assert_eq!(cycle_findings(inner).await, 1);
}

fn delete(slug: &str, mode: pb::delete_entity_request::Mode) -> pb::DeleteEntityRequest {
    pb::DeleteEntityRequest {
        kind: pb::EntityKind::Component as i32,
        id: Some(id(slug)),
        mode: mode as i32,
        ..Default::default()
    }
}

async fn component_exists(store: &NatsStore, slug: &str) -> bool {
    store
        .get(pb::EntityKind::Component, &id(slug), None)
        .await
        .is_ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_incomplete_report_never_loosens_the_guard_and_a_failed_guard_never_deletes() {
    use pb::delete_entity_request::Mode;
    let inner = shared_store().await;
    let store = GateStore::new(inner.clone());
    let overflow = store.scan_overflow.clone();
    let broken_guard = store.guard_failure.clone();
    let service = service_over(Arc::new(store));
    for slug in ["worker", "idle", "spare"] {
        put(&service, component(slug), "").await.unwrap();
    }
    put(
        &service,
        group("users", vec![member_component("worker")]),
        "",
    )
    .await
    .unwrap();

    overflow.store(true, Ordering::SeqCst);
    let preview = service
        .delete_entity(Request::new(delete("worker", Mode::DryRun)))
        .await
        .unwrap()
        .into_inner();
    assert!(preview.referrers_incomplete);
    assert_eq!(preview.referrers.len(), 1);
    let refused = service
        .delete_entity(Request::new(delete("worker", Mode::FailIfReferenced)))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert!(component_exists(&inner, "worker").await);
    let unreferenced = service
        .delete_entity(Request::new(delete("idle", Mode::FailIfReferenced)))
        .await
        .unwrap()
        .into_inner();
    assert!(unreferenced.referrers_incomplete);
    assert!(!component_exists(&inner, "idle").await);
    overflow.store(false, Ordering::SeqCst);

    broken_guard.store(true, Ordering::SeqCst);
    for slug in ["worker", "spare"] {
        let error = service
            .delete_entity(Request::new(delete(slug, Mode::FailIfReferenced)))
            .await
            .unwrap_err();
        assert_ne!(error.code(), Code::FailedPrecondition, "{error}");
        assert!(component_exists(&inner, slug).await, "{slug}");
    }
    assert_eq!(
        read(&service, "users")
            .await
            .entity
            .map(|owner| members_of(&owner).len()),
        Some(1)
    );
    broken_guard.store(false, Ordering::SeqCst);
    let released = service
        .delete_entity(Request::new(delete("spare", Mode::FailIfReferenced)))
        .await
        .unwrap()
        .into_inner();
    assert!(!released.referrers_incomplete);
    assert!(!component_exists(&inner, "spare").await);
}

#[tokio::test(flavor = "multi_thread")]
async fn referrers_past_the_report_output_budget_still_protect_the_target() {
    use pb::delete_entity_request::Mode;
    use trogon_atlas_store::store::MutationOp;
    let inner = shared_store().await;
    let service = service_over(inner.clone());
    put(&service, component("popular"), "").await.unwrap();
    let owners: Vec<_> = (0..4_097)
        .map(|index| group(&format!("owner-{index}"), vec![member_component("popular")]))
        .collect();
    for chunk in owners.chunks(128) {
        let ops: Vec<_> = chunk
            .iter()
            .cloned()
            .map(|entity| MutationOp::Create {
                kind: pb::EntityKind::SystemGroup,
                entity,
            })
            .collect();
        inner
            .batch_apply(&ops, WriteContext::baseline())
            .await
            .unwrap();
    }
    let preview = service
        .delete_entity(Request::new(delete("popular", Mode::DryRun)))
        .await
        .unwrap_err();
    assert_eq!(preview.code(), Code::ResourceExhausted);
    assert_eq!(preview.message(), "delete report output budget exceeded");
    let refused = service
        .delete_entity(Request::new(delete("popular", Mode::FailIfReferenced)))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    let forced = service
        .delete_entity(Request::new(delete("popular", Mode::Force)))
        .await
        .unwrap_err();
    assert_eq!(forced.code(), Code::ResourceExhausted);
    assert!(component_exists(&inner, "popular").await);
}
