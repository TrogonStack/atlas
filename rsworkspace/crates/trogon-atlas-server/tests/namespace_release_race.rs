//! A write authorized against a namespace row must not land after that row
//! was removed while the write waited for the mutation lock.
//!
//! Writes authorize before they take the lock, so a branch deletion can
//! release a provisional row in between. An entity that lands anyway sits in
//! a namespace with no owner, and the next writer from any owner claims it.

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
    let mut request = Request::new(body);
    request.extensions_mut().insert(Principal {
        name: Arc::from("release-race-writer"),
        role: Role::Writer,
        kind: PrincipalKind::Agent,
        is_anonymous: false,
        namespaces: NamespaceScope::unrestricted(),
        parent: Some(OwnerId::parse("acme").unwrap()),
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
