#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::service::EventModelServiceImpl;

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

#[tokio::test(flavor = "multi_thread")]
async fn put_get_delete_roundtrip() {
    let svc = svc().await;

    let put = svc
        .put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event("orders", "order-placed", 1)),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(put.etag, "");

    let got = svc
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("orders", "order-placed", 1)),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(got.etag, put.etag);
    assert!(got.entity.is_some());

    let del = svc
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            operation_id: String::new(),
            kind: pb::EntityKind::Event as i32,
            id: Some(id("orders", "order-placed", 1)),
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            if_match: got.etag,
        }))
        .await;
    assert!(del.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn create_only_conflict_and_if_match() {
    let svc = svc().await;

    let first = svc
        .put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event("orders", "x", 1)),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap()
        .into_inner();

    let dup = svc
        .put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event("orders", "x", 1)),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await;
    assert_eq!(dup.unwrap_err().code(), tonic::Code::AlreadyExists);

    let stale = svc
        .put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event("orders", "x", 1)),
            create_only: false,
            if_match: "bogus".into(),
            validate_only: false,
            force: false,
        }))
        .await;
    assert_eq!(stale.unwrap_err().code(), tonic::Code::Aborted);

    let ok = svc
        .put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event("orders", "x", 1)),
            create_only: false,
            if_match: first.etag.clone(),
            validate_only: false,
            force: true,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(ok.etag, first.etag);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_versions_and_latest() {
    let svc = svc().await;
    for v in [1u64, 2, 3] {
        svc.put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event("orders", "x", v)),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap();
    }
    let versions = svc
        .list_versions(Request::new(pb::ListVersionsRequest {
            kind: pb::EntityKind::Event as i32,
            namespace: "orders".into(),
            slug: "x".into(),
            page_size: 0,
            page_token: String::new(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(versions.versions.len(), 3);

    let latest = svc
        .get_latest_version(Request::new(pb::GetLatestVersionRequest {
            kind: pb::EntityKind::Event as i32,
            namespace: "orders".into(),
            slug: "x".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    let entity = latest.entity.unwrap();
    let id = match entity.kind.unwrap() {
        pb::entity::Kind::Event(e) => e.id.unwrap(),
        _ => panic!("expected event"),
    };
    assert_eq!(id.version, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn list_entities_paginates() {
    let svc = svc().await;
    for i in 0..7 {
        svc.put_entity(Request::new(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(make_event("orders", &format!("slug-{i}"), 1)),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
        }))
        .await
        .unwrap();
    }
    let page1 = svc
        .list_entities(Request::new(pb::ListEntitiesRequest {
            kinds: vec![],
            namespaces: vec![],
            latest_versions_only: false,
            page_size: 3,
            page_token: String::new(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(page1.entities.len(), 3);
    assert_ne!(page1.next_page_token, "");

    let page2 = svc
        .list_entities(Request::new(pb::ListEntitiesRequest {
            kinds: vec![],
            namespaces: vec![],
            latest_versions_only: false,
            page_size: 3,
            page_token: page1.next_page_token,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(page2.entities.len(), 3);

    let page3 = svc
        .list_entities(Request::new(pb::ListEntitiesRequest {
            kinds: vec![],
            namespaces: vec![],
            latest_versions_only: false,
            page_size: 3,
            page_token: page2.next_page_token,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(page3.entities.len(), 1);
    assert_eq!(page3.next_page_token, "");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_namespaces_counts() {
    let svc = svc().await;
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("orders", "a", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("orders", "b", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();
    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("billing", "c", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();
    let resp = svc
        .list_namespaces(Request::new(pb::ListNamespacesRequest {}))
        .await
        .unwrap()
        .into_inner();
    let map: std::collections::HashMap<_, _> = resp
        .namespaces
        .into_iter()
        .map(|n| (n.name, n.entity_count))
        .collect();
    assert_eq!(map.get("orders").copied(), Some(2));
    assert_eq!(map.get("billing").copied(), Some(1));
}
