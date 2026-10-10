//! What a branch write does to the namespace registry, and what undoes it.
//!
//! A namespace enters the registry on its first write. When that write is on
//! a branch the row is provisional: it exists so the branch's own author can
//! see the work through the registry-filtered lens, and it exists only for as
//! long as the branch does. Deleting the branch has to take the row with it,
//! or abandoning an experiment would reserve a namespace name globally and
//! forever, which is the one thing branches promise not to do.
//!
//! The rows are inspected through the store rather than through
//! `ListNamespaces`, because the question here is what was persisted, not
//! what a caller is shown.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::net::TcpListener;
use tonic::{
    metadata::MetadataValue, service::interceptor::InterceptedService, transport::Channel,
};
use trogon_atlas_core::NamespaceId;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    service::EventModelServiceImpl,
};
use trogon_atlas_store::{NamespaceTenure, NatsStore, Store};

const TOKEN: &str = "bnt-writer-token";

fn make_registry() -> TokenRegistry {
    let toml = format!(
        r#"
[principals.bnt-writer]
role = "writer"
tokens = ["{TOKEN}"]
"#
    );
    let file: trogon_atlas_server::auth::RegistryFile =
        toml::from_str(&toml).expect("valid registry TOML");
    TokenRegistry::from_file_contents(file).expect("valid registry")
}

/// The client and the store behind it. The tests assert on the store.
struct World {
    client: EventModelServiceClient<Channel>,
    store: Arc<NatsStore>,
    /// Distinguishes this test's namespace and branch from every other
    /// test's, because both live in one shared store.
    tag: String,
}

impl World {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
        let svc = EventModelServiceImpl::try_new(store.clone()).unwrap();
        let grpc = EventModelServiceServer::new(svc);

        let auth = BearerAuth::new(Arc::new(make_registry()));
        let data_plane = InterceptedService::new(
            tower::ServiceBuilder::new().layer(AuthzLayer).service(grpc),
            auth,
        );

        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(data_plane)
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });

        Self {
            client: EventModelServiceClient::connect(format!("http://{addr}"))
                .await
                .unwrap(),
            store,
            tag: format!("bnt{}", trogon_atlas_testsupport::unique_suffix_for_test()),
        }
    }

    fn namespace(&self) -> String {
        format!("{}-ns", self.tag)
    }

    fn branch(&self) -> String {
        format!("{}-branch", self.tag)
    }

    /// The registry row for this test's namespace, if there is one. Adopted
    /// ids equal the name, which is what makes this lookup possible.
    async fn row(&self) -> Option<trogon_atlas_store::NamespaceRecord> {
        self.store
            .get_namespace(&NamespaceId::parse(&self.namespace()).unwrap())
            .await
            .unwrap()
    }

    async fn create_branch(&mut self) {
        self.client
            .create_branch(authed(
                pb::CreateBranchRequest {
                    name: self.branch(),
                    doc: "tenure test".into(),
                },
                None,
            ))
            .await
            .unwrap();
    }

    /// Write one event into this test's namespace, on `branch` or, with
    /// `None`, on baseline.
    async fn put(&mut self, slug: &str, branch: Option<&str>) {
        self.client
            .put_entity(authed(
                pb::PutEntityRequest {
                    entity: Some(pb::Entity {
                        system: None,
                        kind: Some(pb::entity::Kind::Event(pb::Event {
                            id: Some(pb::Id {
                                namespace: self.namespace(),
                                slug: slug.into(),
                                version: 1,
                            }),
                            title: slug.into(),
                            ..Default::default()
                        })),
                    }),
                    ..Default::default()
                },
                branch,
            ))
            .await
            .unwrap();
    }
}

fn authed<T>(body: T, branch: Option<&str>) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {TOKEN}")).unwrap(),
    );
    if let Some(branch) = branch {
        req.metadata_mut().insert(
            "x-trogon-atlas-branch",
            MetadataValue::try_from(branch).unwrap(),
        );
    }
    req
}

#[tokio::test(flavor = "multi_thread")]
async fn a_baseline_write_claims_the_namespace_permanently() {
    let mut w = World::new().await;
    w.put("placed", None).await;

    let row = w
        .row()
        .await
        .expect("baseline write registers its namespace");
    assert_eq!(
        row.tenure,
        NamespaceTenure::Permanent,
        "a baseline write is public the moment it lands, so its registry row is too",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_write_claims_the_namespace_only_for_that_branch() {
    let mut w = World::new().await;
    w.create_branch().await;
    w.put("placed", Some(&w.branch())).await;

    let row = w.row().await.expect(
        "the row has to exist while the branch lives, or the author's own \
         registry-filtered reads cannot see the work",
    );
    assert_eq!(
        row.tenure,
        NamespaceTenure::Provisional { branch: w.branch() },
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_the_branch_gives_the_namespace_back() {
    let mut w = World::new().await;
    w.create_branch().await;
    w.put("placed", Some(&w.branch())).await;
    assert!(w.row().await.is_some());

    w.client
        .delete_branch(authed(pb::DeleteBranchRequest { name: w.branch() }, None))
        .await
        .unwrap();

    assert!(
        w.row().await.is_none(),
        "an abandoned branch must not leave a namespace name reserved forever",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn merging_the_branch_makes_the_namespace_permanent() {
    let mut w = World::new().await;
    w.create_branch().await;
    w.put("placed", Some(&w.branch())).await;

    w.client
        .merge_branch(authed(
            pb::MergeBranchRequest {
                name: w.branch(),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap();

    let row = w
        .row()
        .await
        .expect("merged work is baseline work, so its namespace stays registered");
    assert_eq!(row.tenure, NamespaceTenure::Permanent);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_namespace_baseline_also_uses_survives_the_branch_that_claimed_it() {
    let mut w = World::new().await;
    w.create_branch().await;
    w.put("branch-only", Some(&w.branch())).await;
    // Baseline reaches the same namespace while the branch is still open. The
    // row was provisional, but the namespace is no longer only the branch's.
    w.put("on-baseline", None).await;

    w.client
        .delete_branch(authed(pb::DeleteBranchRequest { name: w.branch() }, None))
        .await
        .unwrap();

    let row = w.row().await.expect(
        "releasing this row would orphan the baseline entity from the registry \
         that makes it visible",
    );
    assert_eq!(row.tenure, NamespaceTenure::Permanent);
}
