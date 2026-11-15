//! Snapshot coverage through the public `GetSnapshotId` RPC: a load is
//! truncated only when the view holds more entities than the configured cap,
//! branch views honour the same cap, and narrowing a truncated load to a
//! caller's visibility never makes it look complete.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::net::TcpListener;
use tonic::{
    metadata::MetadataValue, service::interceptor::InterceptedService, transport::Channel,
};
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const CAP: usize = 3;
const ACME_TOKEN: &str = "coverage-acme-token";
const UNBOUND_TOKEN: &str = "coverage-unbound-token";

fn registry() -> TokenRegistry {
    let toml = format!(
        r#"
[principals.acme-writer]
role = "writer"
tokens = ["{ACME_TOKEN}"]
parent = "acme"

[principals.unbound-writer]
role = "writer"
tokens = ["{UNBOUND_TOKEN}"]
"#
    );
    let file: trogon_atlas_server::auth::RegistryFile = toml::from_str(&toml).unwrap();
    TokenRegistry::from_file_contents(file).unwrap()
}

async fn client() -> EventModelServiceClient<Channel> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default())
        .unwrap()
        .with_snapshot_cap(CAP);
    let auth = BearerAuth::new(Arc::new(registry()));
    let data_plane = InterceptedService::new(
        tower::ServiceBuilder::new()
            .layer(AuthzLayer)
            .service(EventModelServiceServer::new(svc)),
        auth,
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

fn authed<T>(body: T, token: &str, branch: Option<&str>) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).unwrap(),
    );
    if let Some(branch) = branch {
        req.metadata_mut().insert(
            "x-trogon-atlas-branch",
            MetadataValue::try_from(branch).unwrap(),
        );
    }
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

async fn put(
    c: &mut EventModelServiceClient<Channel>,
    token: &str,
    branch: Option<&str>,
    ns: &str,
    slug: &str,
) {
    c.put_entity(authed(
        pb::PutEntityRequest {
            entity: Some(event(ns, slug)),
            ..Default::default()
        },
        token,
        branch,
    ))
    .await
    .unwrap();
}

async fn snapshot(
    c: &mut EventModelServiceClient<Channel>,
    token: &str,
    branch: Option<&str>,
) -> pb::GetSnapshotIdResponse {
    c.get_snapshot_id(authed(
        pb::GetSnapshotIdRequest {
            namespaces: Vec::new(),
            include_entries: false,
        },
        token,
        branch,
    ))
    .await
    .unwrap()
    .into_inner()
}

#[tokio::test(flavor = "multi_thread")]
async fn baseline_is_truncated_only_above_the_cap() {
    let mut c = client().await;
    for slug in ["a", "b"] {
        put(&mut c, UNBOUND_TOKEN, None, "cov-base", slug).await;
    }
    let below = snapshot(&mut c, UNBOUND_TOKEN, None).await;
    assert_eq!(below.entity_count, 2);
    assert!(!below.truncated);

    put(&mut c, UNBOUND_TOKEN, None, "cov-base", "c").await;
    let at = snapshot(&mut c, UNBOUND_TOKEN, None).await;
    assert_eq!(at.entity_count, 3);
    assert!(!at.truncated, "exactly the cap is a complete view");

    put(&mut c, UNBOUND_TOKEN, None, "cov-base", "d").await;
    let above = snapshot(&mut c, UNBOUND_TOKEN, None).await;
    assert_eq!(above.entity_count, 3);
    assert!(above.truncated);
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_view_honours_the_configured_cap() {
    let mut c = client().await;
    for slug in ["a", "b"] {
        put(&mut c, UNBOUND_TOKEN, None, "cov-branch", slug).await;
    }
    c.create_branch(authed(
        pb::CreateBranchRequest {
            name: "cov-overlay".into(),
            doc: String::new(),
        },
        UNBOUND_TOKEN,
        None,
    ))
    .await
    .unwrap();

    put(
        &mut c,
        UNBOUND_TOKEN,
        Some("cov-overlay"),
        "cov-branch",
        "c",
    )
    .await;
    let at = snapshot(&mut c, UNBOUND_TOKEN, Some("cov-overlay")).await;
    assert_eq!(at.entity_count, 3);
    assert!(!at.truncated);

    put(
        &mut c,
        UNBOUND_TOKEN,
        Some("cov-overlay"),
        "cov-branch",
        "d",
    )
    .await;
    let above = snapshot(&mut c, UNBOUND_TOKEN, Some("cov-overlay")).await;
    assert_eq!(above.entity_count, 3);
    assert!(
        above.truncated,
        "the branch view exceeds the configured cap"
    );
    assert!(!snapshot(&mut c, UNBOUND_TOKEN, None).await.truncated);
}

#[tokio::test(flavor = "multi_thread")]
async fn narrowing_a_truncated_load_keeps_it_truncated() {
    let mut c = client().await;
    put(&mut c, ACME_TOKEN, None, "cov-acme", "mine").await;
    for slug in ["a", "b", "c"] {
        put(&mut c, UNBOUND_TOKEN, None, "cov-other", slug).await;
    }
    let scoped = snapshot(&mut c, ACME_TOKEN, None).await;
    assert!(scoped.entity_count <= 1);
    assert!(
        scoped.truncated,
        "a view cut from a partial load cannot be called complete"
    );
}
