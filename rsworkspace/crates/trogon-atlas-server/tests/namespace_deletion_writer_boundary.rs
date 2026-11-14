#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use async_trait::async_trait;
use prost::Message as _;
use tonic::{Code, Request, Status};
use trogon_atlas_core::{NamespaceId, OwnerId, WriterRole};
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelService};
use trogon_atlas_server::{
    metrics::ServerMetrics,
    ownership::{Lens, NamespaceAuthorizer, RegistryAuthorizer, Visibility, WriteVerdict},
    service::EventModelServiceImpl,
};
use trogon_atlas_store::{NamespaceRecord, NatsStore, Store};

#[derive(Debug)]
struct ObservedRelease {
    registry: RegistryAuthorizer,
    releases: AtomicUsize,
}

#[async_trait]
impl NamespaceAuthorizer for ObservedRelease {
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
        self.registry.may_write(visibility, namespace).await
    }

    async fn on_registered(&self, record: &NamespaceRecord) -> Result<(), Status> {
        self.registry.on_registered(record).await
    }

    async fn on_moved(&self, record: &NamespaceRecord, previous: &OwnerId) -> Result<(), Status> {
        self.registry.on_moved(record, previous).await
    }

    async fn on_released(&self, _record: &NamespaceRecord) -> Result<(), Status> {
        self.releases.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn observed_service(store: Arc<NatsStore>) -> (EventModelServiceImpl, Arc<ObservedRelease>) {
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
    let authorizer = Arc::new(ObservedRelease {
        registry: RegistryAuthorizer::new(service.directory(), store),
        releases: AtomicUsize::new(0),
    });
    (service.with_authorizer(authorizer.clone()), authorizer)
}

async fn register(service: &EventModelServiceImpl, name: &str) -> String {
    service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: name.into(),
            parent: "acme".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id
}

#[derive(prost::Message)]
struct LeaseFaultRecord {
    #[prost(string, tag = "1")]
    holder_id: String,
    #[prost(uint64, tag = "2")]
    epoch: u64,
    #[prost(int64, tag = "3")]
    expires_at_micros: i64,
}

#[tokio::test(flavor = "multi_thread")]
async fn a_non_writer_refuses_deletion_before_revoking_grants() {
    let nats = trogon_atlas_testsupport::shared().await;
    let config = nats.store_config();
    let writer = Arc::new(NatsStore::connect_with(config.clone()).await.unwrap());
    let service = EventModelServiceImpl::try_new(writer.clone(), ServerMetrics::default()).unwrap();
    let namespace = register(&service, "reader-delete").await;
    let id = NamespaceId::parse(&namespace).unwrap();
    let original = writer.get_namespace(&id).await.unwrap().unwrap();

    let mut reader_config = config;
    reader_config.role = WriterRole::Reader;
    let reader = Arc::new(NatsStore::connect_with(reader_config).await.unwrap());
    assert!(!reader.is_writer());
    let (reader, authorizer) = observed_service(reader);
    let status = reader
        .delete_namespace(Request::new(pb::DeleteNamespaceRequest { id: namespace }))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::Unavailable, "{status:?}");
    let failure = trogon_atlas_client::failure::Failure::from_status(&status);
    assert_eq!(failure.code.unwrap().as_str(), "NOT_WRITER");
    assert_eq!(
        authorizer.releases.load(Ordering::SeqCst),
        0,
        "a non-writer must not revoke a registered namespace's grants"
    );
    assert_eq!(writer.get_namespace(&id).await.unwrap(), Some(original));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_promoted_writer_refreshes_cached_emptiness_before_deletion() {
    let nats = trogon_atlas_testsupport::shared().await;
    let mut config = nats.store_config();
    config.lease_duration = Duration::from_secs(120);
    config.lease_renew_interval = Duration::from_secs(60);
    let writer = Arc::new(NatsStore::connect_with(config.clone()).await.unwrap());
    let original_epoch = writer.writer_status().epoch;
    let writer_service =
        EventModelServiceImpl::try_new(writer.clone(), ServerMetrics::default()).unwrap();
    let namespace = register(&writer_service, "promoted-delete").await;
    let id = NamespaceId::parse(&namespace).unwrap();
    let original = writer.get_namespace(&id).await.unwrap().unwrap();

    let mut standby_config = config.clone();
    standby_config.role = WriterRole::Standby;
    standby_config.lease_renew_interval = Duration::from_millis(20);
    let standby = Arc::new(NatsStore::connect_with(standby_config).await.unwrap());
    assert!(!standby.is_writer());
    let (standby_service, authorizer) = observed_service(standby.clone());
    let primed = standby_service
        .get_snapshot_id(Request::new(pb::GetSnapshotIdRequest::default()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(primed.entity_count, 0);

    let entity_id = pb::Id {
        namespace: namespace.clone(),
        slug: "placed".into(),
        version: 1,
    };
    writer_service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::Event(pb::Event {
                    id: Some(entity_id.clone()),
                    title: "Order placed".into(),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap();
    let before = writer
        .get(pb::EntityKind::Event, &entity_id, None)
        .await
        .unwrap();

    let js = async_nats::jetstream::new(async_nats::connect(&nats.url).await.unwrap());
    let lease = js.get_key_value(&config.lease_bucket).await.unwrap();
    let entry = lease.entry("writer-lease").await.unwrap().unwrap();
    let mut row = LeaseFaultRecord::decode(entry.value).unwrap();
    row.expires_at_micros = 0;
    lease
        .update("writer-lease", row.encode_to_vec().into(), entry.revision)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !standby.is_writer() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the same standby process must acquire the expired lease");
    assert!(standby.writer_status().epoch > original_epoch);

    let status = standby_service
        .delete_namespace(Request::new(pb::DeleteNamespaceRequest { id: namespace }))
        .await
        .expect_err("the promoted writer must observe the entity absent from its standby cache");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    assert_eq!(authorizer.releases.load(Ordering::SeqCst), 0);
    assert_eq!(standby.get_namespace(&id).await.unwrap(), Some(original));
    let after = standby
        .get(pb::EntityKind::Event, &entity_id, None)
        .await
        .unwrap();
    assert_eq!(after.etag, before.etag);
    assert_eq!(after.entity, before.entity);
}
