//! Two caches on the ownership read path, and how long each may keep serving
//! a decision that has already changed.
//!
//! Ownership is the tenancy boundary: it is what makes an API key safe to
//! hand to a client. Both caches here were keyed in a way that made a
//! revocation take effect only when the *serving process* happened to do the
//! revoking, which is not a bound at all in either of the deployments
//! `docs/explanation/authorization.md` describes.
//!
//! * `NamespaceDirectory` is dropped on a registry write. With replicas, the
//!   replica that did not serve the `MoveNamespace` never dropped it, so it
//!   kept the namespace with its previous owner until it restarted.
//! * `owner_snapshot_cache` holds the lens-filtered entity list per owner and
//!   never re-consulted the lens on a hit, so a grant revoked directly in
//!   SpiceDB -- which that document says is how sharing is expressed and
//!   which this server never observes -- left the entities being served
//!   anyway.
//!
//! Both are now bounded: the directory view expires, and the filtered
//! snapshot is keyed by the lens that produced it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use async_trait::async_trait;
use tokio::net::TcpListener;
use tonic::{
    metadata::MetadataValue, service::interceptor::InterceptedService, transport::Channel, Code,
    Status,
};
use trogon_atlas_core::{NamespaceId, OwnerId};
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    ownership::{
        Lens, NamespaceAuthorizer, NamespaceDirectory, Visibility, WriteVerdict, DEFAULT_OWNER,
    },
    service::EventModelServiceImpl,
    token_reload::ReloadInterval,
};
use trogon_atlas_store::{NamespaceRecord, Store};

const ACME_TOKEN: &str = "fresh-acme-token";

fn registry() -> TokenRegistry {
    let toml = format!(
        r#"
[principals.acme-writer]
role = "writer"
tokens = ["{ACME_TOKEN}"]
parent = "acme"
"#
    );
    let file: trogon_atlas_server::auth::RegistryFile =
        toml::from_str(&toml).expect("valid registry TOML");
    TokenRegistry::from_file_contents(file).expect("valid registry")
}

async fn serve(svc: EventModelServiceImpl) -> EventModelServiceClient<Channel> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let data_plane = InterceptedService::new(
        tower::ServiceBuilder::new()
            .layer(AuthzLayer)
            .service(EventModelServiceServer::new(svc)),
        BearerAuth::new(Arc::new(registry())),
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(data_plane)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    EventModelServiceClient::connect(format!("http://{addr}"))
        .await
        .unwrap()
}

fn authed<T>(body: T, token: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).unwrap(),
    );
    req
}

fn event(ns: &str, slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version: 1,
            }),
            title: slug.into(),
            ..Default::default()
        })),
    }
}

fn put_req(entity: pb::Entity) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(entity),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }
}

fn slice_emitting(ns: &str, slug: &str, target: pb::EventRef) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version: 1,
            }),
            title: slug.into(),
            emitted_events: vec![pb::EventEdge {
                event: Some(target),
                ..Default::default()
            }],
            ..Default::default()
        })),
    }
}

async fn incoming_references(
    c: &mut EventModelServiceClient<Channel>,
    target: pb::Id,
) -> Result<Vec<pb::Reference>, Status> {
    Ok(c.get_incoming_references(authed(
        pb::GetReferencesRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(target),
            filter_kinds: Vec::new(),
            page_size: 0,
            page_token: String::new(),
        },
        ACME_TOKEN,
    ))
    .await?
    .into_inner()
    .references)
}

async fn visible_namespaces(c: &mut EventModelServiceClient<Channel>) -> Vec<String> {
    let mut seen: Vec<String> = c
        .list_namespaces(authed(pb::ListNamespacesRequest {}, ACME_TOKEN))
        .await
        .unwrap()
        .into_inner()
        .namespaces
        .into_iter()
        .map(|ns| ns.id)
        .collect();
    seen.sort();
    seen
}

async fn visible_entities(c: &mut EventModelServiceClient<Channel>) -> usize {
    c.list_entities(authed(
        pb::ListEntitiesRequest {
            kinds: vec![pb::EntityKind::Event as i32],
            namespaces: Vec::new(),
            latest_versions_only: false,
            page_size: 0,
            page_token: String::new(),
            lifecycle_status_in: Vec::new(),
            lifecycle_status_not_in: Vec::new(),
        },
        ACME_TOKEN,
    ))
    .await
    .unwrap()
    .into_inner()
    .entities
    .len()
}

// ---------------------------------------------------------------------------
// The registry cache expires
// ---------------------------------------------------------------------------

/// Moving `mine` away from `acme` through a handle this server never sees is
/// what a second replica serving `MoveNamespace` looks like from here.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_made_by_another_replica_takes_effect_within_one_interval() {
    let store: Arc<dyn Store> = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let directory = Arc::new(NamespaceDirectory::with_freshness(
        ReloadInterval::from_secs(1),
    ));
    let svc = EventModelServiceImpl::try_new_with_directory(
        store.clone(),
        directory,
        ServerMetrics::default(),
    )
    .unwrap();
    let mut c = serve(svc).await;

    c.put_entity(authed(put_req(event("mine", "placed")), ACME_TOKEN))
        .await
        .expect("the first write claims the namespace");
    assert_eq!(visible_namespaces(&mut c).await, vec!["mine".to_string()]);

    store
        .move_namespace(
            &NamespaceId::parse("mine").unwrap(),
            &OwnerId::parse("beta").unwrap(),
        )
        .await
        .expect("the other replica reassigns the namespace");

    // Held first so the wait below is measuring expiry rather than a read
    // that was never cached in the first place.
    assert_eq!(
        visible_namespaces(&mut c).await,
        vec!["mine".to_string()],
        "the cached view is what makes this a bound worth having",
    );

    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;

    assert!(
        visible_namespaces(&mut c).await.is_empty(),
        "once the view expires this replica must agree that acme no longer owns `mine`",
    );
    assert_eq!(
        visible_entities(&mut c).await,
        0,
        "and the entity read path must agree with it",
    );
}

// ---------------------------------------------------------------------------
// The filtered snapshot follows the lens
// ---------------------------------------------------------------------------

/// An authorizer whose answer changes underneath the server, the way one
/// backed by a policy store somebody else writes to does.
#[derive(Debug)]
struct RevocableAuthorizer {
    namespace: NamespaceId,
    permanent_namespace: Option<NamespaceId>,
    granted: AtomicBool,
}

impl RevocableAuthorizer {
    fn new(namespace: &str) -> Self {
        Self {
            namespace: NamespaceId::parse(namespace).unwrap(),
            permanent_namespace: None,
            granted: AtomicBool::new(true),
        }
    }

    fn with_permanent_namespace(mut self, namespace: &str) -> Self {
        self.permanent_namespace = Some(NamespaceId::parse(namespace).unwrap());
        self
    }

    fn revoke(&self) {
        self.granted.store(false, Ordering::SeqCst);
    }

    fn grants(&self) -> bool {
        self.granted.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl NamespaceAuthorizer for RevocableAuthorizer {
    async fn admits(&self, _vis: &Visibility, namespace: &NamespaceId) -> Result<bool, Status> {
        Ok(self.permanent_namespace.as_ref() == Some(namespace)
            || (self.grants() && namespace == &self.namespace))
    }

    async fn lens(&self, _vis: &Visibility) -> Result<Lens, Status> {
        let mut ids: std::collections::BTreeSet<_> =
            self.permanent_namespace.iter().cloned().collect();
        if self.grants() {
            ids.insert(self.namespace.clone());
        }
        Ok(Lens::Only(Arc::new(ids)))
    }

    async fn may_write(
        &self,
        _vis: &Visibility,
        _namespace: &NamespaceId,
    ) -> Result<WriteVerdict, Status> {
        Ok(WriteVerdict::Allowed)
    }

    async fn on_registered(&self, _record: &NamespaceRecord) -> Result<(), Status> {
        Ok(())
    }

    async fn on_moved(&self, _record: &NamespaceRecord, _from: &OwnerId) -> Result<(), Status> {
        Ok(())
    }

    async fn on_released(&self, _record: &NamespaceRecord) -> Result<(), Status> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_grant_outside_this_server_stops_the_entities_being_served() {
    let store: Arc<dyn Store> = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    store
        .adopt_namespace(
            &trogon_atlas_core::NamespaceName::parse("shared").unwrap(),
            &OwnerId::parse(DEFAULT_OWNER).unwrap(),
            &trogon_atlas_store::NamespaceTenure::Permanent,
        )
        .await
        .unwrap();
    let authorizer = Arc::new(RevocableAuthorizer::new("shared"));
    let svc = EventModelServiceImpl::try_new(store, ServerMetrics::default())
        .unwrap()
        .with_authorizer(authorizer.clone());
    let mut c = serve(svc).await;

    c.put_entity(authed(put_req(event("shared", "placed")), ACME_TOKEN))
        .await
        .unwrap();
    assert_eq!(
        visible_entities(&mut c).await,
        1,
        "the grant is in force, so the entity is visible",
    );

    authorizer.revoke();

    assert_eq!(
        visible_entities(&mut c).await,
        0,
        "the lens no longer admits the namespace, and nothing this server \
         did would have invalidated the per-owner snapshot",
    );
}

/// The same revocation seen through the reference graph, which is served
/// from a second per-owner cache.
#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_grant_outside_this_server_empties_the_reverse_index_too() {
    let store: Arc<dyn Store> = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    for namespace in ["indexed", "root"] {
        store
            .adopt_namespace(
                &trogon_atlas_core::NamespaceName::parse(namespace).unwrap(),
                &OwnerId::parse(DEFAULT_OWNER).unwrap(),
                &trogon_atlas_store::NamespaceTenure::Permanent,
            )
            .await
            .unwrap();
    }
    let authorizer = Arc::new(RevocableAuthorizer::new("indexed").with_permanent_namespace("root"));
    let svc = EventModelServiceImpl::try_new(store, ServerMetrics::default())
        .unwrap()
        .with_authorizer(authorizer.clone());
    let mut c = serve(svc).await;
    let root = pb::Id {
        namespace: "root".into(),
        slug: "placed".into(),
        version: 1,
    };
    let revoked = pb::Id {
        namespace: "indexed".into(),
        slug: "placed".into(),
        version: 1,
    };
    for namespace in ["root", "indexed"] {
        c.put_entity(authed(put_req(event(namespace, "placed")), ACME_TOKEN))
            .await
            .unwrap();
    }
    c.put_entity(authed(
        put_req(slice_emitting(
            "indexed",
            "place",
            pb::EventRef {
                id: Some(root.clone()),
            },
        )),
        ACME_TOKEN,
    ))
    .await
    .unwrap();

    for _ in 0..2 {
        let references = incoming_references(&mut c, root.clone()).await.unwrap();
        assert_eq!(
            references.len(),
            1,
            "the cached visible index contains the granted referrer"
        );
        assert_eq!(
            references[0]
                .from
                .as_ref()
                .unwrap()
                .id
                .as_ref()
                .unwrap()
                .namespace,
            "indexed"
        );
    }
    assert_eq!(
        incoming_references(&mut c, revoked.clone()).await.unwrap(),
        []
    );

    authorizer.revoke();

    assert_eq!(
        incoming_references(&mut c, root.clone()).await.unwrap(),
        [],
        "the visible root remains queryable while the changed lens removes its cached referrer",
    );
    assert!(c
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(root)
            },
            ACME_TOKEN
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .is_some());
    assert_eq!(
        incoming_references(&mut c, revoked)
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}
