#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use async_trait::async_trait;
use futures::poll;
use tokio::sync::Notify;
use tonic::{Request, Status};
use trogon_atlas_core::{NamespaceId, OwnerId};
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelService};
use trogon_atlas_server::{
    artifacts::{storage::NatsBlobStorage, ArtifactService},
    auth::{Principal, PrincipalKind, Role},
    metrics::ServerMetrics,
    ownership::{Lens, NamespaceAuthorizer, RegistryAuthorizer, Visibility, WriteVerdict},
    scope::NamespaceScope,
    service::EventModelServiceImpl,
};
use trogon_atlas_store::{NamespaceRecord, NamespaceTenure, NatsStore, Store};

async fn service() -> (EventModelServiceImpl, Arc<NatsStore>) {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = Arc::new(nats.store().await);
    let context = async_nats::jetstream::new(async_nats::connect(&nats.url).await.unwrap());
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let blobs = NatsBlobStorage::open(
        context.clone(),
        async_nats::jetstream::object_store::Config {
            bucket: format!("namespace-authority-bytes-{suffix}"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let catalog = context
        .create_key_value(async_nats::jetstream::kv::Config {
            bucket: format!("namespace-authority-catalog-{suffix}"),
            history: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    let artifacts = ArtifactService::new(
        Arc::new(blobs),
        catalog,
        1024,
        Duration::from_secs(60),
        store.artifact_publication_authority().unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default())
        .unwrap()
        .with_artifacts(Arc::new(artifacts));
    (service, store)
}

fn upload(namespace: &str) -> pb::BeginArtifactUploadRequest {
    pb::BeginArtifactUploadRequest {
        namespace: namespace.into(),
        filename: "state.txt".into(),
        media_type: "text/plain".into(),
        size: 5,
        sha256: trogon_atlas_client::artifacts::sha256_hex(b"state"),
    }
}

fn writer<T>(body: T, branch: Option<&str>) -> Request<T> {
    let mut request = Request::new(body);
    request.extensions_mut().insert(Principal {
        name: Arc::from("namespace-authority-writer"),
        role: Role::Writer,
        kind: PrincipalKind::Agent,
        is_anonymous: false,
        namespaces: NamespaceScope::unrestricted(),
        parent: Some(OwnerId::parse("namespace-owner").unwrap()),
    });
    if let Some(branch) = branch {
        request
            .metadata_mut()
            .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    }
    request
}

#[tokio::test]
async fn direct_artifact_upload_reserves_missing_namespace_under_default_owner() {
    let (service, store) = service().await;
    let namespace = format!(
        "direct-artifact-{}",
        trogon_atlas_testsupport::unique_suffix_for_test()
    );
    let response = service
        .begin_artifact_upload(Request::new(upload(&namespace)))
        .await
        .expect("direct callers without a Principal must still reserve their namespace")
        .into_inner();
    assert_ne!(response.upload_id, "");
    let record = store
        .get_namespace(&NamespaceId::parse(&namespace).unwrap())
        .await
        .unwrap()
        .expect("artifact upload reserves a registry row");
    assert_eq!(record.parent.as_str(), "default");
    assert_eq!(record.tenure, NamespaceTenure::Permanent);
}

#[tokio::test]
async fn direct_artifact_upload_preserves_existing_namespace_owner() {
    let (service, store) = service().await;
    let namespace = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: format!(
                "existing-artifact-{}",
                trogon_atlas_testsupport::unique_suffix_for_test()
            ),
            parent: "existing-owner".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    service
        .begin_artifact_upload(Request::new(upload(&namespace)))
        .await
        .unwrap();
    let record = store
        .get_namespace(&NamespaceId::parse(&namespace).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.parent.as_str(), "existing-owner");
}

#[derive(Debug)]
struct GatedAuthorizer {
    registry: RegistryAuthorizer,
    grants: Mutex<BTreeSet<(NamespaceId, OwnerId)>>,
    write_checks: AtomicUsize,
    release_entered: Notify,
    release: Notify,
}

#[async_trait]
impl NamespaceAuthorizer for GatedAuthorizer {
    async fn admits(
        &self,
        visibility: &Visibility,
        namespace: &NamespaceId,
    ) -> Result<bool, Status> {
        if visibility.is_unrestricted() {
            return Ok(true);
        }
        Ok(visibility.owner().is_some_and(|owner| {
            self.grants
                .lock()
                .unwrap()
                .contains(&(namespace.clone(), owner.clone()))
        }))
    }

    async fn lens(&self, visibility: &Visibility) -> Result<Lens, Status> {
        self.registry.lens(visibility).await
    }

    async fn may_write(
        &self,
        visibility: &Visibility,
        namespace: &NamespaceId,
    ) -> Result<WriteVerdict, Status> {
        self.write_checks.fetch_add(1, Ordering::SeqCst);
        self.registry.may_write(visibility, namespace).await
    }

    async fn on_registered(&self, record: &NamespaceRecord) -> Result<(), Status> {
        self.grants
            .lock()
            .unwrap()
            .insert((record.id.clone(), record.parent.clone()));
        Ok(())
    }

    async fn on_moved(&self, record: &NamespaceRecord, from: &OwnerId) -> Result<(), Status> {
        let mut grants = self.grants.lock().unwrap();
        grants.remove(&(record.id.clone(), from.clone()));
        grants.insert((record.id.clone(), record.parent.clone()));
        Ok(())
    }

    async fn on_released(&self, record: &NamespaceRecord) -> Result<(), Status> {
        self.release_entered.notify_one();
        self.release.notified().await;
        self.grants
            .lock()
            .unwrap()
            .remove(&(record.id.clone(), record.parent.clone()));
        Ok(())
    }
}

#[tokio::test]
async fn artifact_reclaim_waits_for_the_previous_authorization_release() {
    let (service, store) = service().await;
    let authorizer = Arc::new(GatedAuthorizer {
        registry: RegistryAuthorizer::new(service.directory(), store.clone()),
        grants: Mutex::new(BTreeSet::new()),
        write_checks: AtomicUsize::new(0),
        release_entered: Notify::new(),
        release: Notify::new(),
    });
    let service = Arc::new(service.with_authorizer(authorizer.clone()));
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let namespace = format!("artifact-reclaim-{suffix}");
    let branch = format!("@namespace-owner:artifact-reclaim-branch-{suffix}");
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
        .put_entity(writer(
            pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Event(pb::Event {
                        id: Some(pb::Id {
                            namespace: namespace.clone(),
                            slug: "provisional".into(),
                            version: 1,
                        }),
                        title: "Provisional".into(),
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            },
            Some(&branch),
        ))
        .await
        .unwrap();
    let deleting_service = service.clone();
    let deletion = tokio::spawn(async move {
        deleting_service
            .delete_branch(writer(pb::DeleteBranchRequest { name: branch }, None))
            .await
    });
    tokio::time::timeout(
        Duration::from_secs(15),
        authorizer.release_entered.notified(),
    )
    .await
    .expect("branch deletion must enter the authorization release callback");
    let id = NamespaceId::parse(&namespace).unwrap();
    assert!(store.get_namespace(&id).await.unwrap().is_none());

    let checks_before_reclaim = authorizer.write_checks.load(Ordering::SeqCst);
    let mut reclaim = Box::pin(service.begin_artifact_upload(writer(upload(&namespace), None)));
    assert!(poll!(&mut reclaim).is_pending());
    assert_eq!(
        authorizer.write_checks.load(Ordering::SeqCst),
        checks_before_reclaim,
        "a reclaim must not start authorization before the old release callback settles"
    );

    authorizer.release.notify_one();
    deletion.await.unwrap().unwrap();
    reclaim.await.unwrap();
    let record = store.get_namespace(&id).await.unwrap().unwrap();
    assert_eq!(record.tenure, NamespaceTenure::Permanent);
    assert!(authorizer
        .grants
        .lock()
        .unwrap()
        .contains(&(id, record.parent)));
    service
        .begin_artifact_upload(writer(upload(&namespace), None))
        .await
        .expect("the newer grant must authorize subsequent artifact requests");
}
