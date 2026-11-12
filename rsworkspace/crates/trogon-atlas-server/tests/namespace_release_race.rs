//! A write authorized against a namespace row must not land after that row
//! was removed while the write waited for the mutation lock.
//!
//! Writes authorize before they take the lock, so a branch deletion can
//! release a provisional row in between. An entity that lands anyway sits in
//! a namespace with no owner, and the next writer from any owner claims it.
//! A row another owner adopted in between is not the row the write was
//! authorized against either.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use async_trait::async_trait;
use tokio::sync::Notify;
use tonic::{Code, Request, Status};
use trogon_atlas_core::{NamespaceId, OwnerId};
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelService};
use trogon_atlas_server::{
    auth::{Principal, PrincipalKind, Role},
    metrics::ServerMetrics,
    ownership::{Lens, NamespaceAuthorizer, RegistryAuthorizer, Visibility, WriteVerdict},
    scope::NamespaceScope,
    service::EventModelServiceImpl,
};
use trogon_atlas_store::{NamespaceRecord, Store};

/// The registry authorizer, with one write check that pauses after it
/// answers, so the test can remove the row before the write goes on.
#[derive(Debug)]
struct PausingAuthorizer {
    registry: RegistryAuthorizer,
    armed: AtomicBool,
    authorized: Notify,
    resume: Notify,
}

#[async_trait]
impl NamespaceAuthorizer for PausingAuthorizer {
    async fn admits(
        &self,
        visibility: &Visibility,
        namespace: &NamespaceId,
    ) -> Result<bool, Status> {
        self.registry.admits(visibility, namespace).await
    }

    async fn lens(&self, visibility: &Visibility) -> Result<Lens, Status> {
        self.registry.lens(visibility).await
    }

    async fn may_write(
        &self,
        visibility: &Visibility,
        namespace: &NamespaceId,
    ) -> Result<WriteVerdict, Status> {
        let verdict = self.registry.may_write(visibility, namespace).await?;
        if self.armed.swap(false, Ordering::SeqCst) {
            self.authorized.notify_one();
            self.resume.notified().await;
        }
        Ok(verdict)
    }

    async fn on_registered(&self, record: &NamespaceRecord) -> Result<(), Status> {
        self.registry.on_registered(record).await
    }

    async fn on_moved(&self, record: &NamespaceRecord, from: &OwnerId) -> Result<(), Status> {
        self.registry.on_moved(record, from).await
    }

    async fn on_released(&self, record: &NamespaceRecord) -> Result<(), Status> {
        self.registry.on_released(record).await
    }
}

fn writer<T>(body: T, branch: Option<&str>) -> Request<T> {
    writer_of(Some("acme"), body, branch)
}

/// `owner` of `None` is an unbound caller, the only kind that may revert.
fn writer_of<T>(owner: Option<&str>, body: T, branch: Option<&str>) -> Request<T> {
    let mut request = Request::new(body);
    request.extensions_mut().insert(Principal {
        name: Arc::from(format!(
            "release-race-writer-{}",
            owner.unwrap_or("unbound")
        )),
        role: Role::Writer,
        kind: PrincipalKind::Agent,
        is_anonymous: false,
        namespaces: NamespaceScope::unrestricted(),
        parent: owner.map(|owner| OwnerId::parse(owner).unwrap()),
    });
    if let Some(branch) = branch {
        request
            .metadata_mut()
            .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    }
    request
}

fn put(namespace: &str, slug: &str) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        entity: Some(pb::Entity {
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: namespace.into(),
                    slug: slug.into(),
                    version: 1,
                }),
                title: "Order Placed".into(),
                ..Default::default()
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}

struct Race {
    store: Arc<dyn Store>,
    authorizer: Arc<PausingAuthorizer>,
    service: Arc<EventModelServiceImpl>,
    namespace: String,
    branch: String,
}

/// A namespace whose only registry row is provisional for a live branch, so
/// deleting the branch releases it while baseline holds nothing there.
async fn race(prefix: &str) -> Race {
    let store: Arc<dyn Store> = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
    let authorizer = Arc::new(PausingAuthorizer {
        registry: RegistryAuthorizer::new(service.directory(), store.clone()),
        armed: AtomicBool::new(false),
        authorized: Notify::new(),
        resume: Notify::new(),
    });
    let service = Arc::new(service.with_authorizer(authorizer.clone()));
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let namespace = format!("{prefix}-{suffix}");
    let branch = format!("@acme:draft-{suffix}");
    service
        .create_branch(writer(
            pb::CreateBranchRequest {
                name: branch.clone(),
                doc: String::new(),
            },
            None,
        ))
        .await
        .unwrap();
    service
        .put_entity(writer(put(&namespace, "order-drafted"), Some(&branch)))
        .await
        .unwrap();
    Race {
        store,
        authorizer,
        service,
        namespace,
        branch,
    }
}

impl Race {
    async fn release(&self) {
        self.service
            .delete_branch(writer(
                pb::DeleteBranchRequest {
                    name: self.branch.clone(),
                },
                None,
            ))
            .await
            .unwrap();
        let id = NamespaceId::parse(&self.namespace).unwrap();
        assert!(self.store.get_namespace(&id).await.unwrap().is_none());
    }
}

fn event_id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}

/// The refusal asks for a retry, so it must not leave the retry's
/// `operation_id` claimed by an attempt that wrote nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_revert_leaves_its_operation_id_free_to_retry() {
    let race = race("acme-revert").await;
    let namespace = race.namespace.clone();
    race.service
        .put_entity(writer(put(&namespace, "order-placed"), None))
        .await
        .unwrap();
    let deleted = race
        .service
        .delete_entity(writer(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(event_id(&namespace, "order-placed")),
                mode: pb::delete_entity_request::Mode::Force as i32,
                if_match: String::new(),
                operation_id: format!("delete-{namespace}"),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let changeset = deleted.operation_receipt.unwrap().changeset_id;
    let revert = pb::RevertChangesetRequest {
        id: changeset,
        dry_run: false,
        operation_id: format!("revert-{namespace}"),
    };

    race.authorizer.armed.store(true, Ordering::SeqCst);
    let reverting = race.service.clone();
    let first = {
        let revert = revert.clone();
        tokio::spawn(async move {
            reverting
                .revert_changeset(writer_of(None, revert, None))
                .await
        })
    };
    race.authorizer.authorized.notified().await;
    race.release().await;
    race.authorizer.resume.notify_one();
    let status = first
        .await
        .unwrap()
        .expect_err("a revert into a namespace released while it waited must not land");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");

    let retried = race
        .service
        .revert_changeset(writer_of(None, revert, None))
        .await
        .expect("the retry the refusal asked for claims the id again and lands")
        .into_inner();
    assert!(!retried.changeset_id.is_empty(), "{retried:?}");
    assert!(!retried.operation_receipt.unwrap().replayed);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_that_waited_while_another_owner_adopted_its_namespace_is_refused() {
    let race = race("acme-adopted").await;
    let namespace = race.namespace.clone();

    race.authorizer.armed.store(true, Ordering::SeqCst);
    let writing = race.service.clone();
    let acme_write = {
        let namespace = namespace.clone();
        tokio::spawn(async move {
            writing
                .put_entity(writer(put(&namespace, "order-placed"), None))
                .await
        })
    };
    race.authorizer.authorized.notified().await;
    race.release().await;

    // The legacy id equals the name, so the new owner's row has the same id
    // as the one acme was authorized against.
    race.service
        .put_entity(writer_of(
            Some("beta"),
            put(&namespace, "order-shipped"),
            None,
        ))
        .await
        .unwrap();
    let id = NamespaceId::parse(&namespace).unwrap();
    let row = race.store.get_namespace(&id).await.unwrap().unwrap();
    assert_eq!(row.parent, OwnerId::parse("beta").unwrap());

    race.authorizer.resume.notify_one();
    let status = acme_write
        .await
        .unwrap()
        .expect_err("a write authorized for acme must not land in beta's namespace");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");

    let stored = race
        .store
        .get(
            pb::EntityKind::Event,
            &event_id(&namespace, "order-placed"),
            None,
        )
        .await;
    assert!(stored.is_err(), "nothing landed in beta's namespace");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_that_waited_while_its_namespace_was_released_is_refused() {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
    let authorizer = Arc::new(PausingAuthorizer {
        registry: RegistryAuthorizer::new(service.directory(), store.clone()),
        armed: AtomicBool::new(false),
        authorized: Notify::new(),
        resume: Notify::new(),
    });
    let service = Arc::new(service.with_authorizer(authorizer.clone()));
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let namespace = format!("acme-orders-{suffix}");
    let branch = format!("@acme:draft-{suffix}");
    let id = NamespaceId::parse(&namespace).unwrap();

    service
        .create_branch(writer(
            pb::CreateBranchRequest {
                name: branch.clone(),
                doc: String::new(),
            },
            None,
        ))
        .await
        .unwrap();
    service
        .put_entity(writer(put(&namespace, "order-drafted"), Some(&branch)))
        .await
        .unwrap();
    assert!(
        store.get_namespace(&id).await.unwrap().is_some(),
        "the branch write claims a provisional row"
    );

    authorizer.armed.store(true, Ordering::SeqCst);
    let writing = service.clone();
    let baseline_write = {
        let namespace = namespace.clone();
        tokio::spawn(async move {
            writing
                .put_entity(writer(put(&namespace, "order-placed"), None))
                .await
        })
    };
    authorizer.authorized.notified().await;

    // Baseline holds nothing in the namespace yet, so deleting the branch
    // releases its row while the authorized write is still waiting.
    service
        .delete_branch(writer(pb::DeleteBranchRequest { name: branch }, None))
        .await
        .unwrap();
    assert!(store.get_namespace(&id).await.unwrap().is_none());

    authorizer.resume.notify_one();
    let status = baseline_write
        .await
        .unwrap()
        .expect_err("a write into a namespace released while it waited must not land");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");

    let stored = service
        .get_entity(writer(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(pb::Id {
                    namespace: namespace.clone(),
                    slug: "order-placed".into(),
                    version: 0,
                }),
            },
            None,
        ))
        .await;
    assert!(stored.is_err(), "nothing landed: {stored:?}");
}
