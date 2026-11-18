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

fn declarations(value: &pb::Entity) -> &[pb::CommunicationDeclaration] {
    trogon_atlas_core::communication_intent::declarations(value)
}

fn query(value: &pb::Entity) -> pb::GetReferencesRequest {
    let reference = reference(value);
    pb::GetReferencesRequest {
        kind: reference.kind,
        id: reference.id,
        ..Default::default()
    }
}

async fn assert_subject(
    client: &mut Client,
    selected: &pb::ConnectionRef,
    count: usize,
    state: pb::ResponsibilityResolutionState,
) {
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
    let catalog_state = if state == pb::ResponsibilityResolutionState::Resolved {
        pb::CatalogResolutionState::Resolved
    } else {
        pb::CatalogResolutionState::Unavailable
    };
    assert!(documents
        .declarations
        .iter()
        .all(|declaration| declaration.state == catalog_state as i32));
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
    assert_eq!(assignments.subject_state, state as i32);
}

#[tokio::test(flavor = "multi_thread")]
async fn communication_subjects_keep_exact_message_and_parallel_provenance() {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    let mut client = serve_with_registry(service, make_registry()).await;
    let namespace = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "communication-acceptance".into(),
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
    let peer = put(
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
    let specification = put(
        &mut client,
        entity(pb::entity::Kind::Specification(pb::Specification {
            id: Some(id(&namespace, "messages")),
            title: "Notifications".into(),
            messages: ["OrderHint", "OtherHint"]
                .into_iter()
                .map(|name| pb::ContractMessage {
                    name: Some(pb::MemberName { value: name.into() }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })),
        "",
    )
    .await;
    let mut specification_entity = specification.entity.unwrap();
    let Some(pb::entity::Kind::Specification(definition)) = &specification_entity.kind else {
        panic!("expected specification");
    };
    assert!(definition
        .messages
        .iter()
        .all(|message| message.mapping.is_none()));
    let message = pb::ContractMemberRef {
        specification: Some(pb::SpecificationRef {
            id: reference(&specification_entity).id,
        }),
        owner_uid: specification_entity.system.as_ref().unwrap().uid.clone(),
        kind: pb::ContractMemberKind::Message as i32,
        member_id: definition.messages[0].member_id.clone(),
        expected_digest: None,
    };
    let declaration = pb::CommunicationDeclaration {
        direction: pb::CommunicationDirection::Send as i32,
        peer: Some(pb::ArchitecturalEndpoint {
            target: Some(pb::architectural_endpoint::Target::ExternalSystem(
                pb::ExternalSystemRef {
                    id: reference(&peer).id,
                },
            )),
        }),
        message: Some(message.clone()),
        label: Some(pb::CatalogTitle {
            value: "order hint".into(),
        }),
        condition: Some(pb::CatalogText {
            value: "after an order is ready".into(),
        }),
        ..Default::default()
    };
    let created = put(
        &mut client,
        entity(pb::entity::Kind::Component(pb::Component {
            id: Some(id(&namespace, "checkout")),
            title: "Checkout".into(),
            members: vec![reference(&peer)],
            relationships: vec![pb::ArchitecturalRelationship {
                target: declaration.peer.clone(),
                label: Some(pb::CatalogTitle {
                    value: "documents provider".into(),
                }),
                ..Default::default()
            }],
            communications: vec![declaration.clone(), declaration],
            ..Default::default()
        })),
        "",
    )
    .await;
    let mut owner = created.entity.unwrap();
    let ids: Vec<_> = declarations(&owner)
        .iter()
        .map(|declaration| declaration.connection_id.clone().unwrap())
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
    let outgoing = client
        .get_outgoing_references(authed(query(&owner), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(outgoing.references.len(), 6);
    for (index, selector) in selectors.iter().enumerate() {
        let hits: Vec<_> = outgoing
            .references
            .iter()
            .filter(|hit| hit.connection_id == selector.id)
            .collect();
        assert_eq!(hits.len(), 2);
        for hit in &hits {
            assert_eq!(hit.from, selector.owner);
            assert_eq!(hit.from_uid, selector.owner_uid);
        }
        assert!(hits.iter().any(|hit| {
            hit.to == Some(reference(&peer))
                && hit.from_field == format!("communications[{index}].peer.external_system")
        }));
        assert!(hits.iter().any(|hit| {
            hit.to == Some(reference(&specification_entity))
                && hit.from_field == format!("communications[{index}].message.specification")
        }));
    }
    for (target, count) in [(&peer, 4), (&specification_entity, 2)] {
        let incoming = client
            .get_incoming_references(authed(query(target), ROOT_TOKEN))
            .await
            .unwrap()
            .into_inner();
        let expected: Vec<_> = outgoing
            .references
            .iter()
            .filter(|hit| hit.to == Some(reference(target)))
            .collect();
        assert_eq!(expected.len(), count);
        assert_eq!(incoming.references.len(), count);
        assert!(expected.iter().all(|hit| incoming.references.contains(hit)));
    }
    let note = "An authored notification is not evidence of delivery.";
    put(
        &mut client,
        entity(pb::entity::Kind::Document(pb::Document {
            id: Some(id(&namespace, "rationale")),
            title: Some(pb::CatalogTitle {
                value: "Communication rationale".into(),
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
    assert_subject(
        &mut client,
        &selectors[0],
        1,
        pb::ResponsibilityResolutionState::Resolved,
    )
    .await;
    assert_subject(
        &mut client,
        &selectors[1],
        0,
        pb::ResponsibilityResolutionState::Resolved,
    )
    .await;
    let Some(pb::entity::Kind::Component(component)) = &mut owner.kind else {
        panic!("expected component");
    };
    component.communications.reverse();
    component.communications[1].condition = None;
    component.communications[1].label.as_mut().unwrap().value = "documented hint".into();
    let edited = put(&mut client, owner, &created.etag).await;
    let edited_owner = edited.entity.unwrap();
    assert_eq!(
        declarations(&edited_owner)[0].connection_id,
        Some(ids[1].clone())
    );
    assert_eq!(
        declarations(&edited_owner)[1].connection_id,
        Some(ids[0].clone())
    );
    assert_eq!(declarations(&edited_owner)[1].message, Some(message));
    assert_eq!(
        edited_owner.system.as_ref().unwrap().uid,
        selectors[0].owner_uid
    );
    assert_subject(
        &mut client,
        &selectors[0],
        1,
        pb::ResponsibilityResolutionState::Resolved,
    )
    .await;
    let replay = put(&mut client, edited_owner, &edited.etag).await;
    assert!(replay.no_op);
    assert_eq!(replay.etag, edited.etag);
    let Some(pb::entity::Kind::Specification(definition)) = &mut specification_entity.kind else {
        panic!("expected specification");
    };
    definition.messages.remove(0);
    put(&mut client, specification_entity, &specification.etag).await;
    assert_subject(
        &mut client,
        &selectors[0],
        1,
        pb::ResponsibilityResolutionState::Unresolved,
    )
    .await;
}
