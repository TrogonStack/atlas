#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use tokio::net::TcpListener;
use tonic::{service::interceptor::InterceptedService, transport::Channel, Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "responsibility-admin";
const READER: &str = "responsibility-reader";
const LIMITED: &str = "responsibility-limited";
type Client = EventModelServiceClient<Channel>;
fn request<T>(body: T, token: &str, branch: Option<&str>) -> Request<T> {
    let mut req = Request::new(body);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    if let Some(branch) = branch {
        req.metadata_mut()
            .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    }
    req
}
async fn client() -> Client {
    client_with_cap(0).await
}
async fn client_with_cap(cap: usize) -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let registry = TokenRegistry::from_file_contents(
        toml::from_str(&format!(
            r#"
[principals.admin]
role = "admin"
tokens = ["{ADMIN}"]
[principals.reader]
role = "reader"
tokens = ["{READER}"]
[principals.limited]
role = "writer"
parent = "visible"
tokens = ["{LIMITED}"]
"#
        ))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap()
    .with_snapshot_cap(cap);
    let grpc = InterceptedService::new(
        tower::ServiceBuilder::new()
            .layer(AuthzLayer)
            .service(EventModelServiceServer::new(service)),
        BearerAuth::new(Arc::new(registry)),
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    Client::connect(format!("http://{addr}")).await.unwrap()
}
fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "directory".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn entity(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}
fn person(slug: &str) -> pb::Entity {
    entity(pb::entity::Kind::Person(pb::Person {
        id: Some(id(slug)),
        title: Some(pb::DirectoryDisplayName { value: slug.into() }),
        ..Default::default()
    }))
}
fn team(slug: &str, people: &[pb::Id], subjects: &[pb::ResponsibilitySubject]) -> pb::Entity {
    entity(pb::entity::Kind::Team(pb::Team {
        id: Some(id(slug)),
        title: Some(pb::DirectoryDisplayName { value: slug.into() }),
        members: people
            .iter()
            .map(|id| pb::TeamMember {
                person: Some(pb::PersonRef {
                    id: Some(id.clone()),
                }),
                ..Default::default()
            })
            .collect(),
        responsibilities: subjects
            .iter()
            .map(|subject| pb::Responsibility {
                kind: pb::ResponsibilityKind::Owner as i32,
                subject: Some(subject.clone()),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }))
}
fn subject(kind: pb::EntityKind, id: pb::Id) -> pb::ResponsibilitySubject {
    pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Entity(
            pb::EntityRef {
                kind: kind as i32,
                id: Some(id),
            },
        )),
    }
}
fn reference(value: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(value).unwrap()
}
async fn put(
    client: &mut Client,
    entity: pb::Entity,
    branch: Option<&str>,
) -> pb::PutEntityResponse {
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(entity),
                ..Default::default()
            },
            ADMIN,
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
}
async fn get(
    client: &mut Client,
    kind: pb::EntityKind,
    id: pb::Id,
    branch: Option<&str>,
) -> pb::GetEntityResponse {
    client
        .get_entity(request(
            pb::GetEntityRequest {
                kind: kind as i32,
                id: Some(id),
            },
            READER,
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
}
async fn delete(
    client: &mut Client,
    reference: pb::EntityRef,
    force: bool,
    branch: Option<&str>,
) -> Result<tonic::Response<pb::DeleteEntityResponse>, tonic::Status> {
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: reference.kind,
                id: reference.id,
                mode: if force {
                    pb::delete_entity_request::Mode::Force as i32
                } else {
                    pb::delete_entity_request::Mode::FailIfReferenced as i32
                },
                ..Default::default()
            },
            ADMIN,
            branch,
        ))
        .await
}
async fn assignments(
    client: &mut Client,
    subject: pb::ResponsibilitySubject,
    branch: Option<&str>,
) -> pb::ListSubjectAssignmentsResponse {
    client
        .list_subject_assignments(request(
            pb::ListSubjectAssignmentsRequest {
                subject: Some(subject),
                kind: None,
            },
            READER,
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
}
fn assigned(entity: &pb::Entity) -> &[pb::Responsibility] {
    match &entity.kind {
        Some(pb::entity::Kind::Team(team)) => &team.responsibilities,
        Some(pb::entity::Kind::Person(person)) => &person.responsibilities,
        _ => panic!("assignee"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn responsibility_public_queries_keep_exact_provenance_and_no_inheritance_at_cap_zero() {
    let mut client = client().await;
    let person = put(&mut client, person("alex"), None).await.entity.unwrap();
    let target = put(
        &mut client,
        entity(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id("result")),
            title: "Result".into(),
            ..Default::default()
        })),
        None,
    )
    .await
    .entity
    .unwrap();
    let subject = subject(pb::EntityKind::ReadModel, id("result"));
    let mut first = team("first", &[id("alex")], std::slice::from_ref(&subject));
    if let Some(pb::entity::Kind::Team(team)) = &mut first.kind {
        team.contacts = vec![pb::ContactPoint {
            channel: Some(pb::contact_point::Channel::Email(pb::EmailAddress {
                value: "team@example.test".into(),
            })),
            label: None,
        }];
    }
    let first = put(&mut client, first, None).await.entity.unwrap();
    let second = put(&mut client, team("second", &[id("alex")], &[]), None)
        .await
        .entity
        .unwrap();
    let memberships = client
        .list_person_teams(request(
            pb::ListPersonTeamsRequest {
                person: Some(pb::PersonRef {
                    id: Some(id("alex")),
                }),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(memberships.memberships.len(), 2);
    for team in [&first, &second] {
        let record = memberships
            .memberships
            .iter()
            .find(|record| record.team.as_ref().unwrap().id == reference(team).id)
            .unwrap();
        assert_eq!(record.team_uid, team.system.as_ref().unwrap().uid);
        assert_eq!(
            record
                .membership
                .as_ref()
                .unwrap()
                .person
                .as_ref()
                .unwrap()
                .id,
            reference(&person).id
        );
        assert!(record.membership.as_ref().unwrap().connection_id.is_some());
    }
    let direct = assignments(&mut client, subject.clone(), None).await;
    assert_eq!(direct.assignments.len(), 1);
    assert_eq!(
        direct.assignments[0].assignment.as_ref().unwrap(),
        &assigned(&first)[0]
    );
    assert_eq!(
        direct.assignments[0].assignee_uid,
        first.system.as_ref().unwrap().uid
    );
    assert_eq!(
        direct.subject_state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );
    assert!(direct
        .scope
        .as_ref()
        .unwrap()
        .visited
        .contains(&reference(&target)));
    assert_eq!(
        assignments(&mut client, subject.clone(), None)
            .await
            .scope
            .unwrap()
            .revision,
        direct.scope.unwrap().revision
    );
    let incoming = client
        .get_incoming_references(request(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::ReadModel as i32,
                id: Some(id("result")),
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(incoming.references.len(), 1);
    assert_eq!(
        incoming.references[0].responsibility.as_ref(),
        Some(&assigned(&first)[0])
    );
    assert_eq!(
        incoming.references[0].from_uid,
        first.system.as_ref().unwrap().uid
    );
    let memberships_incoming = client
        .get_incoming_references(request(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::Person as i32,
                id: Some(id("alex")),
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(memberships_incoming.references.len(), 2);
    assert!(memberships_incoming
        .references
        .iter()
        .all(|reference| reference
            .membership
            .as_ref()
            .unwrap()
            .connection_id
            .is_some()));
    let outgoing = client
        .get_outgoing_references(request(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::Team as i32,
                id: Some(id("first")),
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(outgoing
        .references
        .iter()
        .any(|reference| reference.responsibility.as_ref() == Some(&assigned(&first)[0])));
    let impact = client
        .get_impact(request(
            pb::GetImpactRequest {
                root: Some(reference(&target)),
                max_depth: 1,
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(impact
        .nodes
        .iter()
        .flat_map(|node| &node.path)
        .any(|reference| reference.responsibility.as_ref() == Some(&assigned(&first)[0])));
    let closure = client
        .extract_subgraph(request(
            pb::ExtractSubgraphRequest {
                scope: Some(pb::AnalysisScope {
                    scope: Some(pb::analysis_scope::Scope::Slice(reference(&first))),
                }),
                include_cross_model: true,
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!closure.truncated);
    assert!(closure.entities.values().any(|entity| entity == &first));
    assert!(closure.entities.values().any(|entity| entity == &person));
    assert!(closure.entities.values().any(|entity| entity == &target));
    let personal = client
        .list_assignee_responsibilities(request(
            pb::ListAssigneeResponsibilitiesRequest {
                assignee: Some(pb::ResponsibilityAssignee {
                    selection: Some(pb::responsibility_assignee::Selection::Person(
                        pb::PersonRef {
                            id: Some(id("alex")),
                        },
                    )),
                }),
                kind: None,
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(personal.assignments, []);
    let contacts = client
        .get_team_contacts(request(
            pb::GetTeamContactsRequest {
                team: Some(pb::TeamRef {
                    id: Some(id("first")),
                }),
                include_members: false,
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(contacts.team_contacts.len(), 1);
    assert_eq!(contacts.member_contacts, []);
    let contacts = client
        .get_team_contacts(request(
            pb::GetTeamContactsRequest {
                team: Some(pb::TeamRef {
                    id: Some(id("first")),
                }),
                include_members: true,
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(contacts.member_contacts.len(), 1);
    assert!(contacts.member_contacts[0].contact.is_none());
    assert_eq!(
        contacts.member_contacts[0].state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );
    let denied = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(person.clone()),
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied);
    assert_eq!(
        delete(&mut client, reference(&person), false, None)
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    assert_eq!(
        delete(&mut client, reference(&target), false, None)
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    delete(&mut client, reference(&target), true, None)
        .await
        .unwrap();
    let retained = assignments(&mut client, subject, None).await;
    assert_eq!(retained.assignments.len(), 1);
    assert_eq!(
        retained.subject_state,
        pb::ResponsibilityResolutionState::Unresolved as i32
    );
    let mut edited = first.clone();
    if let Some(pb::entity::Kind::Team(team)) = &mut edited.kind {
        team.purpose = Some("Changed".into());
    }
    let rejected = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(edited),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument);
    assert_eq!(
        get(&mut client, pb::EntityKind::Team, id("first"), None)
            .await
            .entity
            .unwrap(),
        first
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn responsibility_exact_member_port_connection_and_final_overlay() {
    let mut client = client().await;
    let model = put(
        &mut client,
        entity(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id("result")),
            title: "Result".into(),
            ..Default::default()
        })),
        None,
    )
    .await
    .entity
    .unwrap();
    let spec = put(
        &mut client,
        entity(pb::entity::Kind::Specification(pb::Specification {
            id: Some(id("api")),
            title: "API".into(),
            messages: vec![pb::ContractMessage {
                name: Some(pb::MemberName {
                    value: "Payload".into(),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        None,
    )
    .await
    .entity
    .unwrap();
    let product = put(
        &mut client,
        entity(pb::entity::Kind::DataProduct(pb::DataProduct {
            id: Some(id("product")),
            title: "Product".into(),
            outputs: vec![pb::DataOutputPort {
                name: Some(pb::PortName {
                    value: "Result".into(),
                }),
                data: Some(pb::OutputDataSelection {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(id("result")),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        None,
    )
    .await
    .entity
    .unwrap();
    let Some(pb::entity::Kind::Specification(specification)) = &spec.kind else {
        panic!("spec")
    };
    let member = pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Member(
            pb::ContractMemberRef {
                specification: Some(pb::SpecificationRef {
                    id: Some(id("api")),
                }),
                owner_uid: spec.system.as_ref().unwrap().uid.clone(),
                kind: pb::ContractMemberKind::Message as i32,
                member_id: specification.messages[0].member_id.clone(),
                expected_digest: None,
            },
        )),
    };
    let Some(pb::entity::Kind::DataProduct(value)) = &product.kind else {
        panic!("product")
    };
    let port = pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Port(
            pb::DataPortRef {
                product: Some(pb::DataProductRef {
                    id: Some(id("product")),
                }),
                owner_uid: product.system.as_ref().unwrap().uid.clone(),
                direction: pb::DataPortDirection::Output as i32,
                port_id: value.outputs[0].port_id.clone(),
            },
        )),
    };
    let connection = pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Connection(
            pb::ConnectionRef {
                owner: Some(reference(&product)),
                owner_uid: product.system.as_ref().unwrap().uid.clone(),
                id: value.outputs[0]
                    .data
                    .as_ref()
                    .unwrap()
                    .connection_id
                    .clone(),
            },
        )),
    };
    let owner = put(
        &mut client,
        team(
            "owners",
            &[],
            &[member.clone(), port.clone(), connection.clone()],
        ),
        None,
    )
    .await
    .entity
    .unwrap();
    for subject in [&member, &port, &connection] {
        let response = assignments(&mut client, subject.clone(), None).await;
        assert_eq!(response.assignments.len(), 1);
        assert_eq!(
            response.assignments[0]
                .assignment
                .as_ref()
                .unwrap()
                .subject
                .as_ref(),
            Some(subject)
        );
        assert_eq!(
            response.subject_state,
            pb::ResponsibilityResolutionState::Resolved as i32
        );
    }
    assert_eq!(
        assignments(
            &mut client,
            subject(pb::EntityKind::Specification, id("api")),
            None
        )
        .await
        .assignments,
        []
    );
    assert_eq!(
        delete(&mut client, reference(&spec), false, None)
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    let mut wrong = owner.clone();
    if let Some(pb::entity::Kind::Team(value)) = &mut wrong.kind {
        value.responsibilities[0].kind = pb::ResponsibilityKind::Contact as i32;
    }
    assert_eq!(
        client
            .put_entity(request(
                pb::PutEntityRequest {
                    entity: Some(wrong),
                    ..Default::default()
                },
                ADMIN,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    let mut repaired = owner;
    if let Some(pb::entity::Kind::Team(value)) = &mut repaired.kind {
        value.responsibilities.clear();
    }
    let owner_etag = get(&mut client, pb::EntityKind::Team, id("owners"), None)
        .await
        .etag;
    let spec_etag = get(&mut client, pb::EntityKind::Specification, id("api"), None)
        .await
        .etag;
    for preview in [true, false] {
        let response = client
            .batch_mutate(request(
                pb::BatchMutateRequest {
                    validate_only: preview,
                    ops: vec![
                        pb::BatchMutateOp {
                            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                                entity: Some(repaired.clone()),
                                if_match: owner_etag.clone(),
                                ..Default::default()
                            })),
                        },
                        pb::BatchMutateOp {
                            op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                                kind: pb::EntityKind::Specification as i32,
                                id: Some(id("api")),
                                if_match: spec_etag.clone(),
                                ..Default::default()
                            })),
                        },
                    ],
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_ne!(
            response.status,
            pb::batch_mutate_response::Status::Failed as i32,
            "{response:?}"
        );
    }
    assert_eq!(assignments(&mut client, member, None).await.assignments, []);
    assert!(get(
        &mut client,
        pb::EntityKind::ReadModel,
        reference(&model).id.unwrap(),
        None
    )
    .await
    .entity
    .is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn responsibility_hidden_hydration_referrers_and_deletion_do_not_leak() {
    let mut client = client().await;
    let mut namespaces = Vec::new();
    for (name, parent) in [
        ("responsibility-visible", "visible"),
        ("responsibility-private", "hidden"),
    ] {
        namespaces.push(
            client
                .register_namespace(request(
                    pb::RegisterNamespaceRequest {
                        name: name.into(),
                        parent: parent.into(),
                    },
                    ADMIN,
                    None,
                ))
                .await
                .unwrap()
                .into_inner()
                .namespace
                .unwrap()
                .id,
        );
    }
    let mut person = person("hidden-person");
    if let Some(pb::entity::Kind::Person(value)) = &mut person.kind {
        value.id.as_mut().unwrap().namespace = namespaces[1].clone();
        value.title.as_mut().unwrap().value = "Hidden profile title".into();
        value.contacts = vec![pb::ContactPoint {
            channel: Some(pb::contact_point::Channel::Email(pb::EmailAddress {
                value: "hidden@example.test".into(),
            })),
            label: None,
        }];
    }
    let hidden_person = put(&mut client, person, None).await.entity.unwrap();
    let mut target_id = id("visible-result");
    target_id.namespace = namespaces[0].clone();
    let target = put(
        &mut client,
        entity(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(target_id.clone()),
            title: "Result".into(),
            ..Default::default()
        })),
        None,
    )
    .await
    .entity
    .unwrap();
    let target_subject = subject(pb::EntityKind::ReadModel, target_id.clone());
    let mut hidden = team(
        "hidden-referrer",
        &[],
        std::slice::from_ref(&target_subject),
    );
    if let Some(pb::entity::Kind::Team(value)) = &mut hidden.kind {
        value.id.as_mut().unwrap().namespace = namespaces[1].clone();
    }
    put(&mut client, hidden, None).await;
    let mut visible = team(
        "visible-team",
        &[reference(&hidden_person).id.unwrap()],
        &[subject(
            pb::EntityKind::Person,
            reference(&hidden_person).id.unwrap(),
        )],
    );
    if let Some(pb::entity::Kind::Team(value)) = &mut visible.kind {
        value.id.as_mut().unwrap().namespace = namespaces[0].clone();
    }
    let visible = put(&mut client, visible, None).await.entity.unwrap();
    let visible_team = pb::TeamRef {
        id: reference(&visible).id,
    };
    let contacts = client
        .get_team_contacts(request(
            pb::GetTeamContactsRequest {
                team: Some(visible_team.clone()),
                include_members: true,
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        contacts.member_contacts[0].state,
        pb::ResponsibilityResolutionState::Inaccessible as i32
    );
    assert!(contacts.member_contacts[0].contact.is_none());
    for private in [
        "Hidden profile title",
        "hidden@example.test",
        "hidden-referrer",
    ] {
        assert!(!format!("{contacts:?}").contains(private));
    }
    let assignee = client
        .list_assignee_responsibilities(request(
            pb::ListAssigneeResponsibilitiesRequest {
                assignee: Some(pb::ResponsibilityAssignee {
                    selection: Some(pb::responsibility_assignee::Selection::Team(visible_team)),
                }),
                kind: None,
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        assignee.assignments[0].state,
        pb::ResponsibilityResolutionState::Inaccessible as i32
    );
    let query = client
        .list_subject_assignments(request(
            pb::ListSubjectAssignmentsRequest {
                subject: Some(target_subject),
                kind: None,
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(query.assignments, []);
    assert!(!format!("{query:?}").contains("hidden-referrer"));
    let denied = client
        .list_subject_assignments(request(
            pb::ListSubjectAssignmentsRequest {
                subject: Some(subject(
                    pb::EntityKind::Person,
                    reference(&hidden_person).id.unwrap(),
                )),
                kind: None,
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::NotFound);
    let denied = client
        .get_incoming_references(request(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::Person as i32,
                id: reference(&hidden_person).id,
                ..Default::default()
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::NotFound);
    let denied = client
        .get_impact(request(
            pb::GetImpactRequest {
                root: Some(reference(&hidden_person)),
                ..Default::default()
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::NotFound);
    let denied = client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::ReadModel as i32,
                id: Some(target_id),
                ..Default::default()
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::FailedPrecondition);
    assert!(!format!("{denied:?}").contains("hidden-referrer"));
    assert!(get(
        &mut client,
        pb::EntityKind::ReadModel,
        reference(&target).id.unwrap(),
        None
    )
    .await
    .entity
    .is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn responsibility_branch_tombstone_and_unrelated_landing_preserve_dangling_owner() {
    let mut client = client().await;
    let target = put(&mut client, person("target"), None)
        .await
        .entity
        .unwrap();
    let selected = subject(pb::EntityKind::Person, id("target"));
    let owner = put(
        &mut client,
        team("retained", &[id("target")], std::slice::from_ref(&selected)),
        None,
    )
    .await
    .entity
    .unwrap();
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: "responsibility-branch".into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    delete(
        &mut client,
        reference(&target),
        true,
        Some("responsibility-branch"),
    )
    .await
    .unwrap();
    assert_eq!(
        assignments(&mut client, selected.clone(), Some("responsibility-branch"))
            .await
            .subject_state,
        pb::ResponsibilityResolutionState::Unresolved as i32
    );
    assert_eq!(
        assignments(&mut client, selected.clone(), None)
            .await
            .subject_state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );
    put(
        &mut client,
        person("unrelated"),
        Some("responsibility-branch"),
    )
    .await;
    let merged = client
        .merge_branch(request(
            pb::MergeBranchRequest {
                name: "responsibility-branch".into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(merged.conflicts.is_empty(), "{merged:?}");
    assert_eq!(
        get(&mut client, pb::EntityKind::Team, id("retained"), None)
            .await
            .entity
            .unwrap(),
        owner
    );
    assert_eq!(
        assignments(&mut client, selected, None).await.subject_state,
        pb::ResponsibilityResolutionState::Unresolved as i32
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn responsibility_generic_retarget_refuses_to_guess_local_selector_replacements() {
    let mut client = client().await;
    assert_generic_retarget_preserves_local_selectors(&mut client).await;
    let original_person = put(&mut client, person("retarget-person"), None)
        .await
        .entity
        .unwrap();
    let mut next_person = person("retarget-person");
    if let Some(pb::entity::Kind::Person(value)) = &mut next_person.kind {
        value.id.as_mut().unwrap().version = 2;
    }
    let next_person = put(&mut client, next_person, None).await.entity.unwrap();
    let direct_owner = put(
        &mut client,
        team(
            "direct-retarget",
            &[id("retarget-person")],
            &[subject(pb::EntityKind::Person, id("retarget-person"))],
        ),
        None,
    )
    .await
    .entity
    .unwrap();
    for preview in [true, false] {
        let result = client
            .retarget_references(request(
                pb::RetargetReferencesRequest {
                    from: Some(reference(&original_person)),
                    to: Some(reference(&next_person)),
                    dry_run: preview,
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(result.rewritten, 1, "{result:?}");
        if preview {
            assert_eq!(
                get(
                    &mut client,
                    pb::EntityKind::Team,
                    id("direct-retarget"),
                    None
                )
                .await
                .entity
                .as_ref(),
                Some(&direct_owner)
            );
        }
    }
    let updated = get(
        &mut client,
        pb::EntityKind::Team,
        id("direct-retarget"),
        None,
    )
    .await
    .entity
    .unwrap();
    let Some(pb::entity::Kind::Team(before)) = &direct_owner.kind else {
        panic!("team")
    };
    let Some(pb::entity::Kind::Team(after)) = &updated.kind else {
        panic!("team")
    };
    assert_ne!(
        before.members[0].connection_id,
        after.members[0].connection_id
    );
    assert_ne!(
        before.responsibilities[0].connection_id,
        after.responsibilities[0].connection_id
    );
    assert_eq!(
        after.members[0].person.as_ref().unwrap().id,
        reference(&next_person).id
    );
    assert_eq!(
        after.responsibilities[0].subject.as_ref(),
        Some(&subject(
            pb::EntityKind::Person,
            reference(&next_person).id.unwrap()
        ))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn responsibility_query_occurrence_and_contact_output_budgets_are_shared_per_request() {
    let mut client = client_with_cap(1000).await;
    let mut people = Vec::new();
    for index in 0..256 {
        let mut value = person(&format!("member-{index}"));
        if let Some(pb::entity::Kind::Person(person)) = &mut value.kind {
            person.contacts = (0..16)
                .map(|contact| pb::ContactPoint {
                    channel: Some(pb::contact_point::Channel::Email(pb::EmailAddress {
                        value: format!("member-{index}-{contact}@example.test"),
                    })),
                    label: None,
                })
                .collect();
        }
        people.push(value);
    }
    let response = client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops: people
                    .iter()
                    .cloned()
                    .map(|value| pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(value),
                            ..Default::default()
                        })),
                    })
                    .collect(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        response.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "{response:?}"
    );
    let mut contact_team = team(
        "contacts",
        &(0..256)
            .map(|index| id(&format!("member-{index}")))
            .collect::<Vec<_>>(),
        &[],
    );
    if let Some(pb::entity::Kind::Team(value)) = &mut contact_team.kind {
        value.contacts.push(pb::ContactPoint {
            channel: Some(pb::contact_point::Channel::Email(pb::EmailAddress {
                value: "team@example.test".into(),
            })),
            label: None,
        });
    }
    put(&mut client, contact_team, None).await;
    let error = client
        .get_team_contacts(request(
            pb::GetTeamContactsRequest {
                team: Some(pb::TeamRef {
                    id: Some(id("contacts")),
                }),
                include_members: true,
            },
            READER,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::ResourceExhausted);
    assert_eq!(
        client
            .get_team_contacts(request(
                pb::GetTeamContactsRequest {
                    team: Some(pb::TeamRef {
                        id: Some(id("contacts"))
                    }),
                    include_members: false
                },
                READER,
                None
            ))
            .await
            .unwrap()
            .into_inner()
            .team_contacts
            .len(),
        1
    );
    seed_responsibility_budget_owners(&mut client).await;
    let error = client
        .list_subject_assignments(request(
            pb::ListSubjectAssignmentsRequest {
                subject: Some(subject(pb::EntityKind::Person, id("member-0"))),
                kind: None,
            },
            READER,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::ResourceExhausted);
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: "@limited:budget-branch".into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Person as i32,
                id: Some(id("member-0")),
                mode: pb::delete_entity_request::Mode::DryRun as i32,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let namespace = client
        .register_namespace(request(
            pb::RegisterNamespaceRequest {
                name: "budget-visible".into(),
                parent: "visible".into(),
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let mut visible_person = person("visible-budget-person");
    if let Some(pb::entity::Kind::Person(value)) = &mut visible_person.kind {
        value.id.as_mut().unwrap().namespace = namespace.clone();
    }
    let visible_person = put(&mut client, visible_person, None).await.entity.unwrap();
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Person as i32,
                id: Some(id("member-0")),
                mode: pb::delete_entity_request::Mode::DryRun as i32,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Person as i32,
                id: reference(&visible_person).id,
                mode: pb::delete_entity_request::Mode::DryRun as i32,
                ..Default::default()
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap();
    let mut statuses = Vec::new();
    for branch in [None, Some("@limited:budget-branch")] {
        let incoming = client
            .get_incoming_references(request(
                pb::GetReferencesRequest {
                    kind: pb::EntityKind::Person as i32,
                    id: Some(id("member-0")),
                    page_size: 1,
                    ..Default::default()
                },
                READER,
                branch,
            ))
            .await;
        statuses.push(incoming.map_or_else(|error| error.code(), |_| Code::Ok));
        let impact = client
            .get_impact(request(
                pb::GetImpactRequest {
                    root: Some(pb::EntityRef {
                        kind: pb::EntityKind::Person as i32,
                        id: Some(id("member-0")),
                    }),
                    page_size: 1,
                    ..Default::default()
                },
                READER,
                branch,
            ))
            .await;
        statuses.push(impact.map_or_else(|error| error.code(), |_| Code::Ok));
    }
    assert_eq!(statuses, vec![Code::ResourceExhausted; 4]);
    for branch in [None, Some("@limited:budget-branch")] {
        let incoming = client
            .get_incoming_references(request(
                pb::GetReferencesRequest {
                    kind: pb::EntityKind::Person as i32,
                    id: reference(&visible_person).id,
                    page_size: 1,
                    ..Default::default()
                },
                LIMITED,
                branch,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(incoming.references, []);
        let impact = client
            .get_impact(request(
                pb::GetImpactRequest {
                    root: Some(reference(&visible_person)),
                    page_size: 1,
                    ..Default::default()
                },
                LIMITED,
                branch,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(impact.nodes.len(), 1);
    }
    let outgoing = client
        .get_outgoing_references(request(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::Team as i32,
                id: Some(id("budget-owner-0")),
                page_size: 1,
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(outgoing.references.len(), 1);
    let closure = client
        .extract_subgraph(request(
            pb::ExtractSubgraphRequest {
                scope: Some(pb::AnalysisScope {
                    scope: Some(pb::analysis_scope::Scope::Slice(pb::EntityRef {
                        kind: pb::EntityKind::Team as i32,
                        id: Some(id("budget-owner-0")),
                    })),
                }),
                include_cross_model: true,
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!closure.truncated);
    let model = put(
        &mut client,
        entity(pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id("budget-model")),
            title: "Budget model".into(),
            members: (0..10)
                .map(|index| pb::EntityRef {
                    kind: pb::EntityKind::Team as i32,
                    id: Some(id(&format!("budget-owner-{index}"))),
                })
                .collect(),
            ..Default::default()
        })),
        None,
    )
    .await
    .entity
    .unwrap();
    let closure = client
        .extract_subgraph(request(
            pb::ExtractSubgraphRequest {
                scope: Some(pb::AnalysisScope {
                    scope: Some(pb::analysis_scope::Scope::EventModel(reference(&model))),
                }),
                include_cross_model: true,
                ..Default::default()
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(closure.truncated);
    let original = get(&mut client, pb::EntityKind::Person, id("member-0"), None)
        .await
        .entity
        .unwrap();
    let mut replacement = person("member-0");
    if let Some(pb::entity::Kind::Person(value)) = &mut replacement.kind {
        value.id.as_mut().unwrap().version = 2;
    }
    let replacement = put(&mut client, replacement, None).await.entity.unwrap();
    let mut migrations = Vec::new();
    for branch in [Some("@limited:budget-branch"), None] {
        let before = get(
            &mut client,
            pb::EntityKind::Team,
            id("budget-owner-0"),
            branch,
        )
        .await
        .entity
        .unwrap();
        for preview in [true, false] {
            let result = client
                .retarget_references(request(
                    pb::RetargetReferencesRequest {
                        from: Some(reference(&original)),
                        to: Some(reference(&replacement)),
                        dry_run: preview,
                        ..Default::default()
                    },
                    ADMIN,
                    branch,
                ))
                .await
                .map(tonic::Response::into_inner);
            if preview {
                assert_eq!(
                    get(
                        &mut client,
                        pb::EntityKind::Team,
                        id("budget-owner-0"),
                        branch
                    )
                    .await
                    .entity
                    .as_ref(),
                    Some(&before)
                );
            }
            migrations.push(result);
        }
    }
    let statuses: Vec<_> = migrations
        .iter()
        .map(|result| {
            result
                .as_ref()
                .map(|value| value.rewritten)
                .map_err(tonic::Status::code)
        })
        .collect();
    assert_eq!(
        statuses,
        vec![Ok(11); 4],
        "branch/baseline preview/apply: {statuses:?}"
    );
    for migration in migrations {
        let migration = migration.unwrap();
        assert_eq!(migration.failed, 0);
        assert_eq!(migration.results.len(), 11);
        assert_eq!(
            migration
                .results
                .iter()
                .map(|result| result.fields.len())
                .sum::<usize>(),
            41
        );
        assert!(migration
            .results
            .iter()
            .all(|result| result.error.is_empty()));
    }
    for branch in [None, Some("@limited:budget-branch")] {
        for index in 0..10 {
            let owner = get(
                &mut client,
                pb::EntityKind::Team,
                id(&format!("budget-owner-{index}")),
                branch,
            )
            .await
            .entity
            .unwrap();
            let Some(pb::entity::Kind::Team(owner)) = owner.kind else {
                panic!("team")
            };
            assert_eq!(owner.responsibilities.len(), 1024);
            assert_eq!(
                owner
                    .responsibilities
                    .iter()
                    .filter(|assignment| assignment.subject.as_ref()
                        == Some(&subject(
                            pb::EntityKind::Person,
                            reference(&replacement).id.unwrap()
                        )))
                    .count(),
                4
            );
            assert!(owner
                .responsibilities
                .iter()
                .all(|assignment| assignment.subject.as_ref()
                    != Some(&subject(pb::EntityKind::Person, id("member-0")))));
        }
        let contacts = get(&mut client, pb::EntityKind::Team, id("contacts"), branch)
            .await
            .entity
            .unwrap();
        let Some(pb::entity::Kind::Team(contacts)) = contacts.kind else {
            panic!("team")
        };
        assert_eq!(contacts.members.len(), 256);
        assert_eq!(
            contacts.members[0].person.as_ref().unwrap().id,
            reference(&replacement).id
        );
        let error = client
            .get_incoming_references(request(
                pb::GetReferencesRequest {
                    kind: pb::EntityKind::Person as i32,
                    id: reference(&replacement).id,
                    ..Default::default()
                },
                READER,
                branch,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::ResourceExhausted);
    }
    assert_generic_retarget_preserves_local_selectors(&mut client).await;
    let probe = put(&mut client, person("safe-delete-probe"), None)
        .await
        .entity
        .unwrap();
    delete(&mut client, reference(&probe), false, None)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn responsibility_validation_retains_structured_dangling_selection_diagnostics() {
    let mut client = client_with_cap(1000).await;
    let target = put(&mut client, person("validation-target"), None)
        .await
        .entity
        .unwrap();
    let owner = put(
        &mut client,
        team(
            "validation-owner",
            &[id("validation-target")],
            &[subject(pb::EntityKind::Person, id("validation-target"))],
        ),
        None,
    )
    .await
    .entity
    .unwrap();
    delete(&mut client, reference(&target), true, None)
        .await
        .unwrap();
    let report = client
        .validate_project(request(
            pb::ValidateProjectRequest {
                scope: Some(pb::validate_project_request::Scope::Namespace(
                    "directory".into(),
                )),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let issues: Vec<_> = report
        .reports
        .iter()
        .flat_map(|report| &report.issues)
        .filter(|issue| issue.code == "RESPONSIBILITY_REFERENCE_UNRESOLVED")
        .collect();
    assert!(
        issues
            .iter()
            .any(|issue| issue.subject.as_ref() == Some(&reference(&owner))
                && issue.subject_field == "responsibilities[0].subject"),
        "{report:?}"
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.subject_field == "members[0].person"),
        "{report:?}"
    );
    assert_eq!(
        get(
            &mut client,
            pb::EntityKind::Team,
            id("validation-owner"),
            None
        )
        .await
        .entity
        .as_ref(),
        Some(&owner)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn responsibility_connection_admission_reads_selected_context_without_unrelated_siblings() {
    let mut client = client().await;
    let mut namespaces = Vec::new();
    for (name, parent) in [("edge-visible", "visible"), ("edge-hidden", "hidden")] {
        namespaces.push(
            client
                .register_namespace(request(
                    pb::RegisterNamespaceRequest {
                        name: name.into(),
                        parent: parent.into(),
                    },
                    ADMIN,
                    None,
                ))
                .await
                .unwrap()
                .into_inner()
                .namespace
                .unwrap()
                .id,
        );
    }
    let local = |slug: &str| pb::Id {
        namespace: namespaces[0].clone(),
        ..id(slug)
    };
    let hidden_ui = pb::Id {
        namespace: namespaces[1].clone(),
        ..id("private-screen")
    };
    put(
        &mut client,
        entity(pb::entity::Kind::Ui(pb::Ui {
            id: Some(hidden_ui.clone()),
            title: "Private screen".into(),
            ..Default::default()
        })),
        None,
    )
    .await;
    put(
        &mut client,
        entity(pb::entity::Kind::Event(pb::Event {
            id: Some(local("fact")),
            title: "Fact".into(),
            ..Default::default()
        })),
        None,
    )
    .await;
    put(
        &mut client,
        entity(pb::entity::Kind::Command(pb::Command {
            id: Some(local("command")),
            title: "Command".into(),
            ..Default::default()
        })),
        None,
    )
    .await;
    let slice = put(
        &mut client,
        entity(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(local("emit")),
            title: "Emit".into(),
            ui: Some(pb::UiEdge {
                ui: Some(pb::UiRef {
                    id: Some(hidden_ui.clone()),
                }),
                ..Default::default()
            }),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(local("command")),
                }),
                ..Default::default()
            }),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(local("fact")),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        None,
    )
    .await
    .entity
    .unwrap();
    let Some(pb::entity::Kind::CommandSlice(value)) = &slice.kind else {
        panic!("slice")
    };
    let selected = pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Connection(
            pb::ConnectionRef {
                owner: Some(reference(&slice)),
                owner_uid: slice.system.as_ref().unwrap().uid.clone(),
                id: value.emitted_events[0].connection_id.clone(),
            },
        )),
    };
    let mut owner = team("edge-owner", &[], std::slice::from_ref(&selected));
    if let Some(pb::entity::Kind::Team(value)) = &mut owner.kind {
        value.id = Some(local("edge-owner"));
    }
    let response = client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(owner),
                ..Default::default()
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(response.entity.is_some());
    let found = client
        .list_subject_assignments(request(
            pb::ListSubjectAssignmentsRequest {
                subject: Some(selected),
                kind: None,
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(found.assignments.len(), 1);
    assert_eq!(
        found.subject_state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );
    assert!(!format!("{found:?}").contains("Private screen"));
    put(
        &mut client,
        entity(pb::entity::Kind::Persona(pb::Persona {
            id: Some(local("viewer")),
            title: "Viewer".into(),
            ..Default::default()
        })),
        None,
    )
    .await;
    put(
        &mut client,
        entity(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(local("entry-model")),
            title: "Entry".into(),
            ..Default::default()
        })),
        None,
    )
    .await;
    let board = put(
        &mut client,
        entity(pb::entity::Kind::Storyboard(pb::Storyboard {
            id: Some(local("story")),
            title: "Story".into(),
            entry: Some(pb::StoryboardEntry {
                read_model: Some(pb::ReadModelEdge {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(local("entry-model")),
                    }),
                    ..Default::default()
                }),
                observer: Some(pb::storyboard_entry::Observer::Human(
                    pb::storyboard_entry::HumanObserver {
                        persona: Some(pb::PersonaEdge {
                            persona: Some(pb::PersonaRef {
                                id: Some(local("viewer")),
                            }),
                            ..Default::default()
                        }),
                        ui: Some(pb::UiEdge {
                            ui: Some(pb::UiRef {
                                id: Some(hidden_ui),
                            }),
                            ..Default::default()
                        }),
                    },
                )),
            }),
            ..Default::default()
        })),
        None,
    )
    .await
    .entity
    .unwrap();
    let Some(pb::entity::Kind::Storyboard(value)) = &board.kind else {
        panic!("storyboard")
    };
    let entry = value.entry.as_ref().unwrap();
    let Some(pb::storyboard_entry::Observer::Human(human)) = &entry.observer else {
        panic!("human")
    };
    for (slug, connection_id, permitted) in [
        (
            "entry-owner",
            entry.read_model.as_ref().unwrap().connection_id.clone(),
            true,
        ),
        (
            "observer-owner",
            human.persona.as_ref().unwrap().connection_id.clone(),
            false,
        ),
    ] {
        let selected = pb::ResponsibilitySubject {
            selection: Some(pb::responsibility_subject::Selection::Connection(
                pb::ConnectionRef {
                    owner: Some(reference(&board)),
                    owner_uid: board.system.as_ref().unwrap().uid.clone(),
                    id: connection_id,
                },
            )),
        };
        let mut owner = team(slug, &[], &[selected]);
        if let Some(pb::entity::Kind::Team(value)) = &mut owner.kind {
            value.id = Some(local(slug));
        }
        let response = client
            .put_entity(request(
                pb::PutEntityRequest {
                    entity: Some(owner),
                    ..Default::default()
                },
                LIMITED,
                None,
            ))
            .await;
        if permitted {
            assert!(response.is_ok(), "{response:?}");
        } else {
            assert_eq!(response.unwrap_err().code(), Code::NotFound);
        }
    }
}

async fn seed_responsibility_budget_owners(client: &mut Client) {
    let mut owners = Vec::new();
    for index in 0..10 {
        let mut value = team(&format!("budget-owner-{index}"), &[], &[]);
        if let Some(pb::entity::Kind::Team(value)) = &mut value.kind {
            for person in 0..256 {
                for kind in [
                    pb::ResponsibilityKind::Owner,
                    pb::ResponsibilityKind::Maintainer,
                    pb::ResponsibilityKind::Steward,
                    pb::ResponsibilityKind::Contact,
                ] {
                    value.responsibilities.push(pb::Responsibility {
                        kind: kind as i32,
                        subject: Some(subject(
                            pb::EntityKind::Person,
                            id(&format!("member-{person}")),
                        )),
                        ..Default::default()
                    });
                }
            }
        }
        owners.push(value);
    }
    let response = client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops: owners
                    .into_iter()
                    .map(|value| pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(value),
                            ..Default::default()
                        })),
                    })
                    .collect(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        response.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "{response:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn responsibility_root_closure_truncates_reachable_occurrences_at_zero_snapshot_cap() {
    let mut client = client().await;
    let response = client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops: (0..256)
                    .map(|index| pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(person(&format!("member-{index}"))),
                            ..Default::default()
                        })),
                    })
                    .collect(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        response.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "{response:?}"
    );
    seed_responsibility_budget_owners(&mut client).await;
    let root = put(
        &mut client,
        team(
            "closure-root",
            &[],
            &(0..10)
                .map(|index| subject(pb::EntityKind::Team, id(&format!("budget-owner-{index}"))))
                .collect::<Vec<_>>(),
        ),
        None,
    )
    .await
    .entity
    .unwrap();
    let mut person_root = person("closure-person");
    if let Some(pb::entity::Kind::Person(value)) = &mut person_root.kind {
        value.responsibilities.push(pb::Responsibility {
            kind: pb::ResponsibilityKind::Owner as i32,
            subject: Some(subject(pb::EntityKind::Team, id("closure-root"))),
            ..Default::default()
        });
    }
    let person_root = put(&mut client, person_root, None).await.entity.unwrap();
    let mut root = root;
    if let Some(pb::entity::Kind::Team(value)) = &mut root.kind {
        value.members.push(pb::TeamMember {
            person: Some(pb::PersonRef {
                id: Some(id("closure-person")),
            }),
            ..Default::default()
        });
    }
    let root = put(&mut client, root, None).await.entity.unwrap();
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: "closure-budget-branch".into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    for (token, expected) in [
        ("invalid-token", Code::Unauthenticated),
        (LIMITED, Code::NotFound),
    ] {
        let error = client
            .extract_subgraph(request(
                pb::ExtractSubgraphRequest {
                    scope: Some(pb::AnalysisScope {
                        scope: Some(pb::analysis_scope::Scope::Slice(reference(&root))),
                    }),
                    include_cross_model: true,
                    ..Default::default()
                },
                token,
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), expected);
    }
    let mut results = Vec::new();
    for branch in [None, Some("closure-budget-branch")] {
        for root in [&root, &person_root] {
            results.push(
                client
                    .extract_subgraph(request(
                        pb::ExtractSubgraphRequest {
                            scope: Some(pb::AnalysisScope {
                                scope: Some(pb::analysis_scope::Scope::Slice(reference(root))),
                            }),
                            include_cross_model: true,
                            ..Default::default()
                        },
                        READER,
                        branch,
                    ))
                    .await
                    .map(tonic::Response::into_inner),
            );
        }
    }
    let statuses: Vec<_> = results
        .iter()
        .map(|result| {
            result
                .as_ref()
                .map(|value| value.truncated)
                .map_err(tonic::Status::code)
        })
        .collect();
    assert_eq!(
        statuses,
        vec![Ok(true); 4],
        "baseline/branch Team/Person closures: {statuses:?}"
    );
    for result in results {
        let closure = result.unwrap();
        assert!(closure.entities.values().any(|value| value == &root));
        assert!(closure.entities.values().any(|value| value == &person_root));
        let owners = closure.entities.values().filter(|value| matches!(&value.kind,
            Some(pb::entity::Kind::Team(team)) if team.id.as_ref().is_some_and(|id| id.slug.starts_with("budget-owner-")))).count();
        assert_eq!(
            owners, 9,
            "the owner that exceeds the budget must remain outside the partial closure"
        );
    }
    let mut last_owner = get(
        &mut client,
        pb::EntityKind::Team,
        id("budget-owner-9"),
        Some("closure-budget-branch"),
    )
    .await
    .entity
    .unwrap();
    if let Some(pb::entity::Kind::Team(value)) = &mut last_owner.kind {
        value.responsibilities.truncate(772);
    }
    put(&mut client, last_owner, Some("closure-budget-branch")).await;
    for root in [&root, &person_root] {
        let closure = client
            .extract_subgraph(request(
                pb::ExtractSubgraphRequest {
                    scope: Some(pb::AnalysisScope {
                        scope: Some(pb::analysis_scope::Scope::Slice(reference(root))),
                    }),
                    include_cross_model: true,
                    ..Default::default()
                },
                READER,
                Some("closure-budget-branch"),
            ))
            .await
            .unwrap()
            .into_inner();
        assert!(
            !closure.truncated,
            "exactly 10,000 occurrences with cycles must fit"
        );
        assert_eq!(closure.entities.len(), 268);
    }
}

async fn assert_generic_retarget_preserves_local_selectors(client: &mut Client) {
    let spec = |version| {
        entity(pb::entity::Kind::Specification(pb::Specification {
            id: Some(pb::Id {
                version,
                ..id("retarget-api")
            }),
            title: "API".into(),
            messages: vec![pb::ContractMessage {
                name: Some(pb::MemberName {
                    value: "Message".into(),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }))
    };
    let original = put(client, spec(1), None).await.entity.unwrap();
    let replacement = put(client, spec(2), None).await.entity.unwrap();
    let Some(pb::entity::Kind::Specification(value)) = &original.kind else {
        panic!("spec")
    };
    let pinned_subject = pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Member(
            pb::ContractMemberRef {
                specification: Some(pb::SpecificationRef {
                    id: reference(&original).id,
                }),
                owner_uid: original.system.as_ref().unwrap().uid.clone(),
                kind: pb::ContractMemberKind::Message as i32,
                member_id: value.messages[0].member_id.clone(),
                expected_digest: None,
            },
        )),
    };
    let owner = put(client, team("pinned", &[], &[pinned_subject]), None)
        .await
        .entity
        .unwrap();
    for preview in [true, false] {
        let error = client
            .retarget_references(request(
                pb::RetargetReferencesRequest {
                    from: Some(reference(&original)),
                    to: Some(reference(&replacement)),
                    dry_run: preview,
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::FailedPrecondition);
    }
    assert_eq!(
        get(client, pb::EntityKind::Team, id("pinned"), None)
            .await
            .entity
            .unwrap(),
        owner
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn architectural_relationship_hidden_and_missing_endpoints_share_unavailable_state() {
    let mut client = client().await;
    let mut namespaces = Vec::new();
    for (name, parent) in [
        ("architectural-visible", "visible"),
        ("architectural-private", "hidden"),
    ] {
        namespaces.push(
            client
                .register_namespace(request(
                    pb::RegisterNamespaceRequest {
                        name: name.into(),
                        parent: parent.into(),
                    },
                    ADMIN,
                    None,
                ))
                .await
                .unwrap()
                .into_inner()
                .namespace
                .unwrap()
                .id,
        );
    }
    let target_id = pb::Id {
        namespace: namespaces[1].clone(),
        slug: "private-provider".into(),
        version: 1,
    };
    let target = entity(pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
        id: Some(target_id.clone()),
        title: "Private provider title".into(),
        ..Default::default()
    }));
    put(&mut client, target.clone(), None).await;
    let source_id = pb::Id {
        namespace: namespaces[0].clone(),
        slug: "public-owner".into(),
        version: 1,
    };
    let source = entity(pb::entity::Kind::Component(pb::Component {
        id: Some(source_id.clone()),
        title: "Readable owner".into(),
        relationships: vec![pb::ArchitecturalRelationship {
            target: Some(pb::ArchitecturalEndpoint {
                target: Some(pb::architectural_endpoint::Target::ExternalSystem(
                    pb::ExternalSystemRef {
                        id: Some(target_id.clone()),
                    },
                )),
            }),
            label: Some(pb::CatalogTitle {
                value: "calls".into(),
            }),
            ..Default::default()
        }],
        ..Default::default()
    }));
    assert_eq!(
        client
            .put_entity(request(
                pb::PutEntityRequest {
                    entity: Some(source.clone()),
                    ..Default::default()
                },
                READER,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    assert_eq!(
        client
            .put_entity(request(
                pb::PutEntityRequest {
                    entity: Some(source.clone()),
                    ..Default::default()
                },
                LIMITED,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let source = put(&mut client, source, None).await.entity.unwrap();
    let occurrence = trogon_atlas_core::architectural_relationships::relationships(&source)[0]
        .connection_id
        .clone();
    let selection = pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Connection(
            pb::ConnectionRef {
                owner: Some(reference(&source)),
                owner_uid: source.system.as_ref().unwrap().uid.clone(),
                id: occurrence,
            },
        )),
    };
    let person_id = pb::Id {
        namespace: namespaces[0].clone(),
        slug: "contact".into(),
        version: 1,
    };
    let contact = entity(pb::entity::Kind::Person(pb::Person {
        id: Some(person_id.clone()),
        title: Some(pb::DirectoryDisplayName {
            value: "Contact".into(),
        }),
        responsibilities: vec![pb::Responsibility {
            kind: pb::ResponsibilityKind::Contact as i32,
            subject: Some(selection.clone()),
            ..Default::default()
        }],
        ..Default::default()
    }));
    put(&mut client, contact, None).await;
    let query = pb::ListAssigneeResponsibilitiesRequest {
        assignee: Some(pb::ResponsibilityAssignee {
            selection: Some(pb::responsibility_assignee::Selection::Person(
                pb::PersonRef {
                    id: Some(person_id),
                },
            )),
        }),
        ..Default::default()
    };
    let hidden = client
        .list_assignee_responsibilities(request(query.clone(), LIMITED, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(hidden.assignments.len(), 1);
    assert_eq!(
        hidden.assignments[0].assignment.as_ref().unwrap().subject,
        Some(selection)
    );
    assert_eq!(
        hidden.assignments[0].state,
        pb::ResponsibilityResolutionState::Unresolved as i32
    );
    assert!(!format!("{hidden:?}").contains("Private provider title"));
    assert_eq!(
        client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::ExternalSystem as i32,
                    id: Some(target_id.clone())
                },
                LIMITED,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    delete(&mut client, reference(&target), true, None)
        .await
        .unwrap();
    let missing = client
        .list_assignee_responsibilities(request(query, LIMITED, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(missing.assignments, hidden.assignments);
    assert_eq!(
        client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Component as i32,
                    id: Some(source_id)
                },
                LIMITED,
                None
            ))
            .await
            .unwrap()
            .into_inner()
            .entity,
        Some(source)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn communication_hidden_and_missing_message_share_unavailable_state_with_live_peer() {
    let mut client = client().await;
    let mut namespaces = Vec::new();
    for (name, parent) in [
        ("communication-visible", "visible"),
        ("communication-private", "hidden"),
    ] {
        namespaces.push(
            client
                .register_namespace(request(
                    pb::RegisterNamespaceRequest {
                        name: name.into(),
                        parent: parent.into(),
                    },
                    ADMIN,
                    None,
                ))
                .await
                .unwrap()
                .into_inner()
                .namespace
                .unwrap()
                .id,
        );
    }
    let target_id = pb::Id {
        namespace: namespaces[0].clone(),
        slug: "public-provider".into(),
        version: 1,
    };
    let target = entity(pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
        id: Some(target_id.clone()),
        title: "Public provider".into(),
        ..Default::default()
    }));
    put(&mut client, target.clone(), None).await;
    let specification = entity(pb::entity::Kind::Specification(pb::Specification {
        id: Some(pb::Id {
            namespace: namespaces[1].clone(),
            slug: "private-contract".into(),
            version: 1,
        }),
        title: "Private contract title".into(),
        messages: vec![pb::ContractMessage {
            name: Some(pb::MemberName {
                value: "Hint".into(),
            }),
            ..Default::default()
        }],
        ..Default::default()
    }));
    let specification = put(&mut client, specification, None).await.entity.unwrap();
    let Some(pb::entity::Kind::Specification(spec)) = &specification.kind else {
        panic!("specification")
    };
    let message = pb::ContractMemberRef {
        specification: Some(pb::SpecificationRef {
            id: spec.id.clone(),
        }),
        owner_uid: specification.system.as_ref().unwrap().uid.clone(),
        kind: pb::ContractMemberKind::Message as i32,
        member_id: spec.messages[0].member_id.clone(),
        ..Default::default()
    };

    let source_id = pb::Id {
        namespace: namespaces[0].clone(),
        slug: "public-owner".into(),
        version: 1,
    };
    let source = entity(pb::entity::Kind::Component(pb::Component {
        id: Some(source_id.clone()),
        title: "Readable owner".into(),
        communications: vec![pb::CommunicationDeclaration {
            direction: pb::CommunicationDirection::Receive as i32,
            message: Some(message),
            peer: Some(pb::ArchitecturalEndpoint {
                target: Some(pb::architectural_endpoint::Target::ExternalSystem(
                    pb::ExternalSystemRef {
                        id: Some(target_id.clone()),
                    },
                )),
            }),
            label: Some(pb::CatalogTitle {
                value: "calls".into(),
            }),
            ..Default::default()
        }],
        ..Default::default()
    }));
    assert_eq!(
        client
            .put_entity(request(
                pb::PutEntityRequest {
                    entity: Some(source.clone()),
                    ..Default::default()
                },
                READER,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    assert_eq!(
        client
            .put_entity(request(
                pb::PutEntityRequest {
                    entity: Some(source.clone()),
                    ..Default::default()
                },
                LIMITED,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let source = put(&mut client, source, None).await.entity.unwrap();
    let occurrence = trogon_atlas_core::communication_intent::declarations(&source)[0]
        .connection_id
        .clone();
    let selection = pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Connection(
            pb::ConnectionRef {
                owner: Some(reference(&source)),
                owner_uid: source.system.as_ref().unwrap().uid.clone(),
                id: occurrence,
            },
        )),
    };
    let person_id = pb::Id {
        namespace: namespaces[0].clone(),
        slug: "contact".into(),
        version: 1,
    };
    let contact = entity(pb::entity::Kind::Person(pb::Person {
        id: Some(person_id.clone()),
        title: Some(pb::DirectoryDisplayName {
            value: "Contact".into(),
        }),
        responsibilities: vec![pb::Responsibility {
            kind: pb::ResponsibilityKind::Contact as i32,
            subject: Some(selection.clone()),
            ..Default::default()
        }],
        ..Default::default()
    }));
    put(&mut client, contact, None).await;
    let query = pb::ListAssigneeResponsibilitiesRequest {
        assignee: Some(pb::ResponsibilityAssignee {
            selection: Some(pb::responsibility_assignee::Selection::Person(
                pb::PersonRef {
                    id: Some(person_id),
                },
            )),
        }),
        ..Default::default()
    };
    let hidden = client
        .list_assignee_responsibilities(request(query.clone(), LIMITED, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(hidden.assignments.len(), 1);
    assert_eq!(
        hidden.assignments[0].assignment.as_ref().unwrap().subject,
        Some(selection)
    );
    assert_eq!(
        hidden.assignments[0].state,
        pb::ResponsibilityResolutionState::Unresolved as i32
    );
    assert!(!format!("{hidden:?}").contains("Private contract title"));
    assert_eq!(
        client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Specification as i32,
                    id: spec.id.clone()
                },
                LIMITED,
                None
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    delete(&mut client, reference(&specification), true, None)
        .await
        .unwrap();
    let missing = client
        .list_assignee_responsibilities(request(query, LIMITED, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(missing.assignments, hidden.assignments);
    assert_eq!(
        client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Component as i32,
                    id: Some(source_id)
                },
                LIMITED,
                None
            ))
            .await
            .unwrap()
            .into_inner()
            .entity,
        Some(source)
    );
}
