#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "responsibility-fixture".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn display(value: &str) -> pb::DirectoryDisplayName {
    pb::DirectoryDisplayName {
        value: value.into(),
    }
}

fn model_ref() -> pb::ReadModelRef {
    pb::ReadModelRef {
        id: Some(id("receipt")),
    }
}

fn entity_ref(entity: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(entity).unwrap()
}

fn uid(entity: &pb::Entity) -> String {
    entity.system.as_ref().unwrap().uid.clone()
}

fn selected(selection: pb::responsibility_subject::Selection) -> pb::ResponsibilitySubject {
    pb::ResponsibilitySubject {
        selection: Some(selection),
    }
}

fn assignment(subject: pb::ResponsibilitySubject) -> pb::Responsibility {
    pb::Responsibility {
        kind: pb::ResponsibilityKind::Owner as i32,
        subject: Some(subject),
        ..Default::default()
    }
}

fn contact(value: &str) -> pb::ContactPoint {
    pb::ContactPoint {
        channel: Some(pb::contact_point::Channel::Email(pb::EmailAddress {
            value: value.into(),
        })),
        ..Default::default()
    }
}

async fn put(service: &EventModelServiceImpl, kind: pb::entity::Kind) -> pb::Entity {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(kind),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

async fn lookup(
    service: &EventModelServiceImpl,
    subject: pb::ResponsibilitySubject,
) -> pb::ListSubjectAssignmentsResponse {
    service
        .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
            subject: Some(subject),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
}

#[tokio::test]
async fn responsibility_layer_preserves_connected_native_subjects_and_membership_provenance() {
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap()
    .with_snapshot_cap(0);
    put(
        &service,
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id("receipt")),
            title: "Receipt".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &service,
        pb::entity::Kind::Processor(pb::Processor {
            id: Some(id("notifier")),
            title: "Notifier".into(),
            ..Default::default()
        }),
    )
    .await;
    let observed = put(
        &service,
        pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id("observe")),
            title: "Observe receipt".into(),
            source_read_models: (0..2)
                .map(|_| pb::ReadModelEdge {
                    read_model: Some(model_ref()),
                    ..Default::default()
                })
                .collect(),
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(id("notifier")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await;
    let specification = put(
        &service,
        pb::entity::Kind::Specification(pb::Specification {
            id: Some(id("receipt-contract")),
            title: "Receipt contract".into(),
            messages: vec![pb::ContractMessage {
                name: Some(pb::MemberName {
                    value: "Receipt".into(),
                }),
                mapping: Some(pb::MessageReadModelMapping {
                    read_model: Some(model_ref()),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let product = put(
        &service,
        pb::entity::Kind::DataProduct(pb::DataProduct {
            id: Some(id("receipts")),
            title: "Receipts".into(),
            outputs: vec![pb::DataOutputPort {
                name: Some(pb::PortName {
                    value: "Receipts".into(),
                }),
                data: Some(pb::OutputDataSelection {
                    read_model: Some(model_ref()),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let pb::entity::Kind::Specification(spec) = specification.kind.as_ref().unwrap() else {
        panic!("Specification required")
    };
    let pb::entity::Kind::DataProduct(data) = product.kind.as_ref().unwrap() else {
        panic!("DataProduct required")
    };
    let pb::entity::Kind::AutomationSlice(slice) = observed.kind.as_ref().unwrap() else {
        panic!("AutomationSlice required")
    };
    let member = selected(pb::responsibility_subject::Selection::Member(
        pb::ContractMemberRef {
            specification: Some(pb::SpecificationRef {
                id: Some(id("receipt-contract")),
            }),
            owner_uid: uid(&specification),
            kind: pb::ContractMemberKind::Message as i32,
            member_id: spec.messages[0].member_id.clone(),
            expected_digest: None,
        },
    ));
    let port = selected(pb::responsibility_subject::Selection::Port(
        pb::DataPortRef {
            product: Some(pb::DataProductRef {
                id: Some(id("receipts")),
            }),
            owner_uid: uid(&product),
            direction: pb::DataPortDirection::Output as i32,
            port_id: data.outputs[0].port_id.clone(),
        },
    ));
    let occurrence = |index: usize| {
        selected(pb::responsibility_subject::Selection::Connection(
            pb::ConnectionRef {
                owner: Some(entity_ref(&observed)),
                owner_uid: uid(&observed),
                id: slice.source_read_models[index].connection_id.clone(),
            },
        ))
    };
    let chosen = occurrence(0);
    let sibling = occurrence(1);
    assert_ne!(chosen, sibling);
    let person = put(
        &service,
        pb::entity::Kind::Person(pb::Person {
            id: Some(id("maintainer")),
            title: Some(display("Receipt maintainer")),
            contacts: vec![contact("test@example.com")],
            responsibilities: vec![assignment(member.clone()), assignment(port.clone())],
            ..Default::default()
        }),
    )
    .await;
    let mut teams = Vec::new();
    for slug in ["receipts-team", "support-team"] {
        teams.push(
            put(
                &service,
                pb::entity::Kind::Team(pb::Team {
                    id: Some(id(slug)),
                    title: Some(display(slug)),
                    members: vec![pb::TeamMember {
                        person: Some(pb::PersonRef {
                            id: Some(id("maintainer")),
                        }),
                        ..Default::default()
                    }],
                    contacts: vec![contact("test@example.com")],
                    responsibilities: if slug == "receipts-team" {
                        vec![
                            assignment(selected(pb::responsibility_subject::Selection::Entity(
                                entity_ref(&product),
                            ))),
                            assignment(chosen.clone()),
                        ]
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                }),
            )
            .await,
        );
    }
    for subject in [&member, &port, &chosen] {
        let result = lookup(&service, subject.clone()).await;
        assert!(!result.incomplete);
        assert_eq!(
            result.subject_state,
            pb::ResponsibilityResolutionState::Resolved as i32
        );
        assert_eq!(result.assignments.len(), 1);
        assert_eq!(
            result.assignments[0]
                .assignment
                .as_ref()
                .unwrap()
                .subject
                .as_ref(),
            Some(subject)
        );
        let scope = result.scope.unwrap();
        assert_ne!(scope.revision, "");
        assert!(scope.visited_subjects.contains(subject));
    }
    assert_eq!(lookup(&service, sibling).await.assignments, []);
    assert_eq!(
        lookup(
            &service,
            selected(pb::responsibility_subject::Selection::Entity(entity_ref(
                &specification
            )))
        )
        .await
        .assignments,
        []
    );
    let membership = service
        .list_person_teams(Request::new(pb::ListPersonTeamsRequest {
            person: Some(pb::PersonRef {
                id: Some(id("maintainer")),
            }),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(membership.memberships.len(), 2);
    for team in &teams {
        let pb::entity::Kind::Team(value) = team.kind.as_ref().unwrap() else {
            panic!("Team required")
        };
        assert!(membership.memberships.iter().any(|record| {
            record.team.as_ref().unwrap().id == value.id
                && record.team_uid == uid(team)
                && record.membership.as_ref() == Some(&value.members[0])
        }));
    }
    let contacts_request = pb::GetTeamContactsRequest {
        team: Some(pb::TeamRef {
            id: Some(id("receipts-team")),
        }),
        include_members: false,
    };
    let contacts = service
        .get_team_contacts(Request::new(contacts_request.clone()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(contacts.team_contacts.len(), 1);
    assert_eq!(contacts.member_contacts, []);
    let expanded = service
        .get_team_contacts(Request::new(pb::GetTeamContactsRequest {
            include_members: true,
            ..contacts_request
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(expanded.member_contacts.len(), 1);
    assert_eq!(
        expanded.member_contacts[0].person.as_ref().unwrap().id,
        Some(id("maintainer"))
    );
    let direct = service
        .list_assignee_responsibilities(Request::new(pb::ListAssigneeResponsibilitiesRequest {
            assignee: Some(pb::ResponsibilityAssignee {
                selection: Some(pb::responsibility_assignee::Selection::Person(
                    pb::PersonRef {
                        id: Some(id("maintainer")),
                    },
                )),
            }),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(direct.assignments.len(), 2);
    assert!(direct
        .assignments
        .iter()
        .all(|record| record.assignee_uid == uid(&person)));
    let outgoing = service
        .get_outgoing_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Person as i32,
            id: Some(id("maintainer")),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(outgoing.references.iter().any(|record| {
        record.from_uid == uid(&person)
            && record
                .responsibility
                .as_ref()
                .and_then(|value| value.subject.as_ref())
                == Some(&port)
    }));
    let incoming = service
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Person as i32,
            id: Some(id("maintainer")),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    for team in &teams {
        assert!(incoming.references.iter().any(|record| {
            record.from.as_ref() == Some(&entity_ref(team))
                && record.from_uid == uid(team)
                && record.membership.is_some()
        }));
    }
    let impact = service
        .get_impact(Request::new(pb::GetImpactRequest {
            root: Some(entity_ref(&product)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(impact.nodes.iter().any(|node| {
        node.entity.as_ref() == Some(&entity_ref(&person))
            && node.path.iter().any(|record| {
                record.from_uid == uid(&person)
                    && record
                        .responsibility
                        .as_ref()
                        .and_then(|value| value.subject.as_ref())
                        == Some(&port)
            })
    }));
    let current = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::AutomationSlice as i32,
            id: Some(id("observe")),
        }))
        .await
        .unwrap()
        .into_inner();
    let mut reordered = current.entity.unwrap();
    let Some(pb::entity::Kind::AutomationSlice(value)) = &mut reordered.kind else {
        panic!("AutomationSlice required")
    };
    value.source_read_models.reverse();
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(reordered),
            if_match: current.etag,
            ..Default::default()
        }))
        .await
        .unwrap();
    assert_eq!(lookup(&service, chosen).await.assignments.len(), 1);
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Specification as i32,
            id: Some(id("receipt-contract")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let retained = lookup(&service, member.clone()).await;
    assert_eq!(
        retained.subject_state,
        pb::ResponsibilityResolutionState::Unresolved as i32
    );
    assert_eq!(retained.assignments.len(), 1);
    assert_eq!(
        retained.assignments[0]
            .assignment
            .as_ref()
            .unwrap()
            .subject
            .as_ref(),
        Some(&member)
    );
    assert_eq!(
        lookup(&service, port).await.subject_state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );
}
