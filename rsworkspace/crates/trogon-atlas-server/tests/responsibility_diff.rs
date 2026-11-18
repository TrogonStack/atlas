#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::net::TcpListener;
use trogon_atlas_proto::{
    self as pb, event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

fn pointer(provider: &str, key: &str) -> pb::ExternalDirectoryPointer {
    pb::ExternalDirectoryPointer {
        provider: Some(pb::DirectoryProvider {
            value: provider.into(),
        }),
        key: Some(pb::OpaqueDirectoryKey { value: key.into() }),
    }
}

async fn assert_pointer_diff(
    before: Option<pb::ExternalDirectoryPointer>,
    after: Option<pb::ExternalDirectoryPointer>,
    expected_before: &str,
    expected_after: &str,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(EventModelServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let mut client = EventModelServiceClient::connect(format!("http://{addr}"))
        .await
        .unwrap();
    let id = |version| pb::Id {
        namespace: "directory".into(),
        slug: "alex".into(),
        version,
    };
    for (version, directory_pointer) in [(1, before), (2, after)] {
        client
            .put_entity(pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Person(pb::Person {
                        id: Some(id(version)),
                        title: Some(pb::DirectoryDisplayName {
                            value: "Alex".into(),
                        }),
                        directory_pointer,
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                create_only: true,
                ..Default::default()
            })
            .await
            .unwrap();
    }
    let reference = |version| pb::EntityRef {
        kind: pb::EntityKind::Person as i32,
        id: Some(id(version)),
    };
    let response = client
        .diff_entities(pb::DiffEntitiesRequest {
            a: Some(reference(1)),
            b: Some(reference(2)),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        response.ops,
        vec![pb::DiffOp {
            kind: pb::DiffOpKind::Changed as i32,
            path: "directory_pointer".into(),
            before: expected_before.into(),
            after: expected_after.into(),
            ..Default::default()
        }]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn diff_entities_detects_directory_pointer_addition() {
    assert_pointer_diff(
        None,
        Some(pointer("directory-a", "person-1")),
        "null",
        r#"{"key":{"value":"person-1"},"provider":{"value":"directory-a"}}"#,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn diff_entities_detects_directory_pointer_removal() {
    assert_pointer_diff(
        Some(pointer("directory-a", "person-1")),
        None,
        r#"{"key":{"value":"person-1"},"provider":{"value":"directory-a"}}"#,
        "null",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn diff_entities_detects_directory_pointer_provider_change() {
    assert_pointer_diff(
        Some(pointer("directory-a", "person-1")),
        Some(pointer("directory-b", "person-1")),
        r#"{"key":{"value":"person-1"},"provider":{"value":"directory-a"}}"#,
        r#"{"key":{"value":"person-1"},"provider":{"value":"directory-b"}}"#,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn diff_entities_detects_directory_pointer_key_change() {
    assert_pointer_diff(
        Some(pointer("directory-a", "person-1")),
        Some(pointer("directory-a", "person-2")),
        r#"{"key":{"value":"person-1"},"provider":{"value":"directory-a"}}"#,
        r#"{"key":{"value":"person-2"},"provider":{"value":"directory-a"}}"#,
    )
    .await;
}
