#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use support::two_tenants::{authed, make_registry, serve_with_registry, ROOT_TOKEN};
use tonic::transport::Channel;
use trogon_atlas_proto::{self as pb, event_model_service_client::EventModelServiceClient};
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

type Client = EventModelServiceClient<Channel>;

fn entity(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}

fn reference(value: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(value).unwrap()
}

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}

async fn put(client: &mut Client, entity: pb::Entity, etag: &str) -> pb::PutEntityResponse {
    client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(entity),
                if_match: etag.into(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
}

fn relationships(value: &pb::Entity) -> &[pb::ArchitecturalRelationship] {
    let Some(pb::entity::Kind::Component(component)) = &value.kind else {
        panic!("expected component");
    };
    &component.relationships
}

async fn assert_subject_counts(client: &mut Client, selected: &pb::ConnectionRef, count: usize) {
    let documents = client
        .list_subject_documents(authed(
            pb::ListSubjectDocumentsRequest {
                subject: Some(pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Connection(selected.clone())),
                }),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!documents.incomplete);
    assert_eq!(documents.declarations.len(), count);
    let assignments = client
        .list_subject_assignments(authed(
            pb::ListSubjectAssignmentsRequest {
                subject: Some(pb::ResponsibilitySubject {
                    selection: Some(pb::responsibility_subject::Selection::Connection(
                        selected.clone(),
                    )),
                }),
                kind: None,
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!assignments.incomplete);
    assert_eq!(assignments.assignments.len(), count);
}

#[tokio::test(flavor = "multi_thread")]
async fn parallel_declarations_keep_provenance_and_exact_subjects_after_reorder() {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    let mut client = serve_with_registry(service, make_registry()).await;
    let namespace = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "relationship-acceptance".into(),
                parent: "commerce".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let target = put(
        &mut client,
        entity(pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
            id: Some(id(&namespace, "provider")),
            title: "Provider".into(),
            ..Default::default()
        })),
        "",
    )
    .await
    .entity
    .unwrap();
    let declaration = pb::ArchitecturalRelationship {
        target: Some(pb::ArchitecturalEndpoint {
            target: Some(pb::architectural_endpoint::Target::ExternalSystem(
                pb::ExternalSystemRef {
                    id: reference(&target).id,
                },
            )),
        }),
        label: Some(pb::CatalogTitle {
            value: "submits orders".into(),
        }),
        ..Default::default()
    };
    let authored = entity(pb::entity::Kind::Component(pb::Component {
        id: Some(id(&namespace, "checkout")),
        title: "Checkout".into(),
        members: vec![reference(&target)],
        relationships: vec![declaration.clone(), declaration],
        ..Default::default()
    }));
    let mut malformed = authored.clone();
    let Some(pb::entity::Kind::Component(component)) = &mut malformed.kind else {
        panic!("expected component");
    };
    component.relationships[0].connection_id = Some(pb::ConnectionId {
        value: String::new(),
    });
    let rejected = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(malformed),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), tonic::Code::InvalidArgument);
    let absent = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Component as i32,
                id: Some(id(&namespace, "checkout")),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(absent.code(), tonic::Code::NotFound);
    let created = put(&mut client, authored, "").await;
    let mut owner = created.entity.unwrap();
    let ids: Vec<_> = relationships(&owner)
        .iter()
        .map(|value| value.connection_id.clone().unwrap())
        .collect();
    assert_ne!(ids[0], ids[1]);
    let selectors: Vec<_> = ids
        .iter()
        .map(|id| pb::ConnectionRef {
            owner: Some(reference(&owner)),
            owner_uid: owner.system.as_ref().unwrap().uid.clone(),
            id: Some(id.clone()),
        })
        .collect();
    let query = |target: &pb::Entity| pb::GetReferencesRequest {
        kind: reference(target).kind,
        id: reference(target).id,
        ..Default::default()
    };
    let outgoing = client
        .get_outgoing_references(authed(query(&owner), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(outgoing.references.len(), 3);
    assert_eq!(
        outgoing
            .references
            .iter()
            .filter(|hit| hit.connection_id.is_none())
            .count(),
        1
    );
    for (index, connection_id) in ids.iter().enumerate() {
        let hit = outgoing
            .references
            .iter()
            .find(|hit| hit.connection_id.as_ref() == Some(connection_id))
            .unwrap();
        assert_eq!(hit.from, Some(reference(&owner)));
        assert_eq!(hit.from_uid, selectors[index].owner_uid);
        assert_eq!(hit.to, Some(reference(&target)));
        assert_eq!(
            hit.from_field,
            format!("relationships[{index}].target.external_system")
        );
    }
    let incoming = client
        .get_incoming_references(authed(query(&target), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(incoming.references.len(), outgoing.references.len());
    for hit in &outgoing.references {
        assert!(incoming.references.contains(hit));
    }
    let note = "These are authored declarations.";
    put(
        &mut client,
        entity(pb::entity::Kind::Document(pb::Document {
            id: Some(id(&namespace, "rationale")),
            title: Some(pb::CatalogTitle {
                value: "Rationale".into(),
            }),
            subjects: vec![pb::SubjectAssociation {
                subject: Some(pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Connection(
                        selectors[0].clone(),
                    )),
                }),
                ..Default::default()
            }],
            body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
                media_type: Some(pb::MediaType {
                    value: "text/plain".into(),
                }),
                content: note.into(),
                digest: Some(trogon_atlas_core::specification::content_digest(note)),
            })),
            ..Default::default()
        })),
        "",
    )
    .await;
    put(
        &mut client,
        entity(pb::entity::Kind::Person(pb::Person {
            id: Some(id(&namespace, "maintainer")),
            title: Some(pb::DirectoryDisplayName {
                value: "Maintainer".into(),
            }),
            responsibilities: vec![pb::Responsibility {
                kind: pb::ResponsibilityKind::Maintainer as i32,
                subject: Some(pb::ResponsibilitySubject {
                    selection: Some(pb::responsibility_subject::Selection::Connection(
                        selectors[0].clone(),
                    )),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        "",
    )
    .await;
    assert_subject_counts(&mut client, &selectors[0], 1).await;
    assert_subject_counts(&mut client, &selectors[1], 0).await;
    let Some(pb::entity::Kind::Component(component)) = &mut owner.kind else {
        panic!("expected component");
    };
    component.relationships.reverse();
    component.relationships[1].label.as_mut().unwrap().value = "declared order submission".into();
    let edited = put(&mut client, owner, &created.etag).await;
    let edited_owner = edited.entity.unwrap();
    assert_eq!(
        relationships(&edited_owner)[0].connection_id.as_ref(),
        Some(&ids[1])
    );
    assert_eq!(
        relationships(&edited_owner)[1].connection_id.as_ref(),
        Some(&ids[0])
    );
    assert_eq!(
        edited_owner.system.as_ref().unwrap().uid,
        selectors[0].owner_uid
    );
    assert_subject_counts(&mut client, &selectors[0], 1).await;
    assert_subject_counts(&mut client, &selectors[1], 0).await;
    let replay = put(&mut client, edited_owner, &edited.etag).await;
    assert!(replay.no_op);
    assert_eq!(replay.etag, edited.etag);
}
