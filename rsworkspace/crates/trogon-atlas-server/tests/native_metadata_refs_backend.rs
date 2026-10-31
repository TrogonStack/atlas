#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use prost::Message;
use tonic::{Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

const GROUPS: &str = "type.googleapis.com/trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation";

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "native-refs".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn target(slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::Component as i32,
        id: Some(id(slug)),
    }
}

fn groups(targets: &[&str]) -> prost_types::Any {
    prost_types::Any {
        type_url: GROUPS.into(),
        value: pb::ResourceGroupsAnnotation {
            groups: vec![pb::ResourceGroup {
                title: Some(pb::MetadataTitle {
                    value: "related".into(),
                }),
                display_limit: Some(pb::PositiveDisplayLimit { value: 1 }),
                items: targets
                    .iter()
                    .map(|slug| pb::ResourceGroupItem {
                        target: Some(target(slug)),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
        }
        .encode_to_vec(),
    }
}

fn component(slug: &str, targets: &[&str]) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Component(pb::Component {
            id: Some(id(slug)),
            title: slug.into(),
            metadata: if targets.is_empty() {
                Vec::new()
            } else {
                vec![groups(targets)]
            },
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn event(slug: &str, targets: &[&str]) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(slug)),
            title: slug.into(),
            metadata: vec![groups(targets)],
            ..Default::default()
        })),
        ..Default::default()
    }
}

async fn service() -> EventModelServiceImpl {
    EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap()
    .with_snapshot_cap(0)
}

async fn put(
    service: &EventModelServiceImpl,
    entity: pb::Entity,
    if_match: &str,
) -> Result<pb::PutEntityResponse, tonic::Status> {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity),
            if_match: if_match.into(),
            ..Default::default()
        }))
        .await
        .map(tonic::Response::into_inner)
}

async fn get(service: &EventModelServiceImpl, slug: &str) -> Option<pb::GetEntityResponse> {
    service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Component as i32,
            id: Some(id(slug)),
        }))
        .await
        .ok()
        .map(tonic::Response::into_inner)
}

async fn incoming(service: &EventModelServiceImpl, slug: &str) -> Vec<pb::Reference> {
    service
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Component as i32,
            id: Some(id(slug)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references
}

fn delete(slug: &str, mode: pb::delete_entity_request::Mode) -> pb::DeleteEntityRequest {
    pb::DeleteEntityRequest {
        kind: pb::EntityKind::Component as i32,
        id: Some(id(slug)),
        mode: mode as i32,
        ..Default::default()
    }
}

#[tokio::test]
async fn group_selection_must_resolve_for_every_owner_kind() {
    let service = service().await;
    for entity in [component("agent", &["ghost"]), event("signal", &["ghost"])] {
        let rejected = put(&service, entity, "").await.unwrap_err();
        assert_eq!(rejected.code(), Code::InvalidArgument, "{rejected}");
        assert!(!rejected.message().contains("ghost"), "{rejected}");
    }
    assert!(get(&service, "agent").await.is_none());
    put(&service, component("present", &[]), "").await.unwrap();
    put(&service, component("agent", &["present"]), "")
        .await
        .unwrap();
    put(&service, event("signal", &["present"]), "")
        .await
        .unwrap();
}

#[tokio::test]
async fn group_selection_is_revalidated_for_unchanged_refs_and_batches_are_atomic() {
    let service = service().await;
    put(&service, component("present", &[]), "").await.unwrap();
    put(&service, component("bystander", &[]), "")
        .await
        .unwrap();
    let created = put(&service, component("agent", &["present"]), "")
        .await
        .unwrap();
    service
        .delete_entity(Request::new(delete(
            "present",
            pb::delete_entity_request::Mode::Force,
        )))
        .await
        .unwrap();

    let unrelated = get(&service, "bystander").await.unwrap();
    let mut retitled = unrelated.entity.clone().unwrap();
    if let Some(pb::entity::Kind::Component(value)) = retitled.kind.as_mut() {
        value.title = "still editable".into();
    }
    put(&service, retitled, &unrelated.etag).await.unwrap();

    let mut edited = created.entity.clone().unwrap();
    if let Some(pb::entity::Kind::Component(value)) = edited.kind.as_mut() {
        value.title = "edited".into();
    }
    let rejected = put(&service, edited, &created.etag).await.unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument, "{rejected}");
    assert_eq!(get(&service, "agent").await.unwrap().entity, created.entity);

    let batch = service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: vec![
                pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        entity: Some(component("fresh", &[])),
                        ..Default::default()
                    })),
                },
                pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        entity: Some(component("dangling", &["present"])),
                        ..Default::default()
                    })),
                },
            ],
            ..Default::default()
        }))
        .await;
    assert!(
        batch.is_err()
            || batch
                .as_ref()
                .is_ok_and(|reply| reply.get_ref().status
                    == pb::batch_mutate_response::Status::Failed as i32),
        "{batch:?}"
    );
    assert!(get(&service, "fresh").await.is_none());
    assert!(get(&service, "dangling").await.is_none());
}

#[tokio::test]
async fn group_occurrences_are_indexed_below_display_limit_and_block_safe_delete() {
    let service = service().await;
    put(&service, component("target", &[]), "").await.unwrap();
    let owner = put(&service, component("agent", &["target", "target"]), "")
        .await
        .unwrap();
    let references = incoming(&service, "target").await;
    assert_eq!(references.len(), 2, "{references:?}");
    let ids: Vec<_> = references
        .iter()
        .map(|reference| reference.connection_id.clone())
        .collect();
    assert!(ids.iter().all(Option::is_some), "{references:?}");
    assert_ne!(ids[0], ids[1]);
    let uid = owner
        .entity
        .as_ref()
        .unwrap()
        .system
        .as_ref()
        .unwrap()
        .uid
        .clone();
    assert!(references.iter().all(|reference| reference.from_uid == uid));
    assert!(references
        .iter()
        .all(|reference| reference.from_field.contains(".items[")));

    let blocked = service
        .delete_entity(Request::new(delete(
            "target",
            pb::delete_entity_request::Mode::FailIfReferenced,
        )))
        .await
        .unwrap_err();
    assert_eq!(blocked.code(), Code::FailedPrecondition, "{blocked}");
    assert!(get(&service, "target").await.is_some());

    let impact = service
        .get_impact(Request::new(pb::GetImpactRequest {
            root: Some(target("target")),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(impact
        .nodes
        .iter()
        .any(|node| node.entity.as_ref().and_then(|e| e.id.as_ref()) == Some(&id("agent"))));
}

#[tokio::test]
async fn group_selection_batch_deleting_target_with_final_overlay_is_atomic() {
    let service = service().await;
    put(&service, component("target", &[]), "").await.unwrap();
    let owner = put(&service, component("agent", &["target"]), "")
        .await
        .unwrap();
    let mut cleared = owner.entity.clone().unwrap();
    if let Some(pb::entity::Kind::Component(value)) = cleared.kind.as_mut() {
        value.metadata.clear();
    }
    let delete_op = |expected: &str| pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Component as i32,
            id: Some(id("target")),
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            if_match: expected.into(),
            ..Default::default()
        })),
    };
    let rejected = service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: vec![delete_op("")],
            ..Default::default()
        }))
        .await;
    assert!(
        rejected.is_err()
            || rejected
                .as_ref()
                .is_ok_and(|reply| reply.get_ref().status
                    == pb::batch_mutate_response::Status::Failed as i32),
        "{rejected:?}"
    );
    assert!(get(&service, "target").await.is_some());

    let applied = service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: vec![
                pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        entity: Some(cleared),
                        if_match: owner.etag.clone(),
                        ..Default::default()
                    })),
                },
                delete_op(""),
            ],
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        applied.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "{applied:?}"
    );
    assert!(get(&service, "target").await.is_none());
}

#[tokio::test]
async fn generic_retarget_renews_group_item_ids_and_keeps_the_index_consistent() {
    let service = service().await;
    put(&service, component("old", &[]), "").await.unwrap();
    put(&service, component("new", &[]), "").await.unwrap();
    put(&service, component("keep", &[]), "").await.unwrap();
    let owner = put(&service, component("agent", &["old", "keep"]), "")
        .await
        .unwrap();
    let before = incoming(&service, "old").await;
    let reply = service
        .retarget_references(Request::new(pb::RetargetReferencesRequest {
            from: Some(target("old")),
            to: Some(target("new")),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(reply.rewritten, 1, "{reply:?}");
    let stale = incoming(&service, "old").await;
    assert_eq!(stale.len(), 0, "{stale:?}");
    let after = incoming(&service, "new").await;
    assert_eq!(after.len(), 1);
    assert_ne!(after[0].connection_id, before[0].connection_id);
    let kept = incoming(&service, "keep").await;
    let current = get(&service, "agent").await.unwrap();
    assert_ne!(current.etag, owner.etag);
    assert_eq!(kept.len(), 1);
}

#[tokio::test]
async fn group_item_provenance_is_identical_on_outgoing_incoming_and_impact_paths() {
    let service = service().await;
    put(&service, component("target", &[]), "").await.unwrap();
    let owner = put(&service, component("agent", &["target", "target"]), "")
        .await
        .unwrap();
    let uid = owner
        .entity
        .as_ref()
        .unwrap()
        .system
        .as_ref()
        .unwrap()
        .uid
        .clone();
    let outgoing = service
        .get_outgoing_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Component as i32,
            id: Some(id("agent")),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references;
    let mut incoming = incoming(&service, "target").await;
    let impact = service
        .get_impact(Request::new(pb::GetImpactRequest {
            root: Some(target("target")),
            max_depth: 1,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    let mut pathed: Vec<pb::Reference> = impact
        .nodes
        .iter()
        .filter(|node| node.depth == 1)
        .flat_map(|node| node.path.clone())
        .collect();
    assert_eq!(outgoing.len(), 2, "{outgoing:?}");
    for set in [&outgoing, &incoming, &pathed] {
        assert!(set.iter().all(|r| r.connection_id.is_some()), "{set:?}");
        assert!(set.iter().all(|r| r.from_uid == uid), "{set:?}");
    }
    let key = |r: &pb::Reference| {
        (
            r.from_field.clone(),
            r.connection_id.as_ref().map(|c| c.value.clone()),
        )
    };
    let mut outgoing: Vec<_> = outgoing.iter().map(key).collect();
    outgoing.sort();
    incoming.sort_by_key(key);
    pathed.sort_by_key(key);
    assert_eq!(outgoing, incoming.iter().map(key).collect::<Vec<_>>());
    assert!(pathed.iter().map(key).all(|hop| outgoing.contains(&hop)));
}

#[tokio::test]
async fn group_occurrences_share_the_discovery_budget_regardless_of_display_limit() {
    let service = service().await;
    put(&service, component("target", &[]), "").await.unwrap();
    let targets = vec!["target"; 256];
    for owner in 0..40 {
        put(&service, component(&format!("agent-{owner}"), &targets), "")
            .await
            .unwrap();
    }
    let exhausted = service
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Component as i32,
            id: Some(id("target")),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(exhausted.code(), Code::ResourceExhausted, "{exhausted}");
}
