#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use tonic::{Code, Request};
use trogon_atlas_core::communication_intent::declarations;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "communication".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn owner(
    slug: &str,
    external: bool,
    communications: Vec<pb::CommunicationDeclaration>,
) -> pb::Entity {
    pb::Entity {
        kind: Some(if external {
            pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
                id: Some(id(slug)),
                title: slug.into(),
                communications,
                ..Default::default()
            })
        } else {
            pb::entity::Kind::Component(pb::Component {
                id: Some(id(slug)),
                title: slug.into(),
                communications,
                ..Default::default()
            })
        }),
        ..Default::default()
    }
}
fn values(entity: &mut pb::Entity) -> &mut Vec<pb::CommunicationDeclaration> {
    match entity.kind.as_mut().unwrap() {
        pb::entity::Kind::Component(v) => &mut v.communications,
        pb::entity::Kind::ExternalSystem(v) => &mut v.communications,
        _ => panic!("owner"),
    }
}
fn declaration(
    slug: &str,
    external: bool,
    member: &pb::ContractMemberRef,
) -> pb::CommunicationDeclaration {
    pb::CommunicationDeclaration {
        direction: pb::CommunicationDirection::Send as i32,
        peer: Some(pb::ArchitecturalEndpoint {
            target: Some(if external {
                pb::architectural_endpoint::Target::ExternalSystem(pb::ExternalSystemRef {
                    id: Some(id(slug)),
                })
            } else {
                pb::architectural_endpoint::Target::Component(pb::ComponentRef {
                    id: Some(id(slug)),
                })
            }),
        }),
        message: Some(member.clone()),
        label: Some(pb::CatalogTitle {
            value: "notifies".into(),
        }),
        condition: Some(pb::CatalogText {
            value: "when a receipt is ready".into(),
        }),
        ..Default::default()
    }
}
fn request<T>(body: T, branch: Option<&str>) -> Request<T> {
    let mut r = Request::new(body);
    if let Some(b) = branch {
        r.metadata_mut()
            .insert("x-trogon-atlas-branch", b.parse().unwrap());
    }
    r
}
async fn save(s: &EventModelServiceImpl, e: pb::Entity, b: Option<&str>) -> pb::PutEntityResponse {
    s.put_entity(request(
        pb::PutEntityRequest {
            entity: Some(e),
            ..Default::default()
        },
        b,
    ))
    .await
    .unwrap()
    .into_inner()
}
async fn get(
    s: &EventModelServiceImpl,
    slug: &str,
    external: bool,
    b: Option<&str>,
) -> pb::GetEntityResponse {
    s.get_entity(request(
        pb::GetEntityRequest {
            kind: if external {
                pb::EntityKind::ExternalSystem
            } else {
                pb::EntityKind::Component
            } as i32,
            id: Some(id(slug)),
        },
        b,
    ))
    .await
    .unwrap()
    .into_inner()
}
async fn fixture() -> (EventModelServiceImpl, pb::ContractMemberRef) {
    let s = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap()
    .with_snapshot_cap(0);
    let spec = save(
        &s,
        pb::Entity {
            kind: Some(pb::entity::Kind::Specification(pb::Specification {
                id: Some(id("notifications")),
                title: "Notifications".into(),
                messages: ["OrderHint", "OtherHint"]
                    .into_iter()
                    .map(|n| pb::ContractMessage {
                        name: Some(pb::MemberName { value: n.into() }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
            ..Default::default()
        },
        None,
    )
    .await
    .entity
    .unwrap();
    let Some(pb::entity::Kind::Specification(v)) = &spec.kind else {
        panic!("spec")
    };
    let member = pb::ContractMemberRef {
        specification: Some(pb::SpecificationRef {
            id: Some(id("notifications")),
        }),
        owner_uid: spec.system.as_ref().unwrap().uid.clone(),
        kind: pb::ContractMemberKind::Message as i32,
        member_id: v.messages[0].member_id.clone(),
        ..Default::default()
    };
    for (slug, external) in [("provider", true), ("auditor", true), ("worker", false)] {
        save(&s, owner(slug, external, vec![]), None).await;
    }
    (s, member)
}
fn selector(e: &pb::Entity, index: usize) -> pb::ConnectionRef {
    pb::ConnectionRef {
        owner: Some(trogon_atlas_server::conv::entity_to_ref(e).unwrap()),
        owner_uid: e.system.as_ref().unwrap().uid.clone(),
        id: declarations(e)[index].connection_id.clone(),
    }
}
fn subject(e: &pb::Entity, index: usize) -> pb::ResponsibilitySubject {
    pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Connection(selector(
            e, index,
        ))),
    }
}

#[tokio::test]
async fn communication_public_native_lifecycle_and_parallel_provenance() {
    let (s, member) = fixture().await;
    for external in [false, true] {
        let slug = if external {
            "external-owner"
        } else {
            "checkout"
        };
        let mut first = declaration("provider", true, &member);
        first.direction = pb::CommunicationDirection::Receive as i32;
        let before = save(
            &s,
            owner(
                slug,
                external,
                vec![first.clone(), first, declaration("worker", false, &member)],
            ),
            None,
        )
        .await;
        let original = before.entity.as_ref().unwrap();
        let selected = selector(original, 0);
        assert_ne!(selected, selector(original, 1));
        let refs = s
            .get_outgoing_references(Request::new(pb::GetReferencesRequest {
                kind: selected.owner.as_ref().unwrap().kind,
                id: Some(id(slug)),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(refs.references.len(), 6);
        for (index, value) in declarations(original).iter().enumerate() {
            let hits: Vec<_> = refs
                .references
                .iter()
                .filter(|r| r.connection_id == value.connection_id)
                .collect();
            assert_eq!(hits.len(), 2);
            for r in &hits {
                assert_eq!(r.from, selected.owner);
                assert_eq!(r.from_uid, selected.owner_uid);
            }
            assert!(hits.iter().any(|r| r.from_field
                == format!("communications[{index}].message.specification")
                && r.to.as_ref().unwrap().id == Some(id("notifications"))));
            assert!(hits.iter().any(|r| r.from_field
                == format!(
                    "communications[{index}].peer.{}",
                    if index == 2 {
                        "component"
                    } else {
                        "external_system"
                    }
                )));
        }
        let mut rejected = Vec::new();
        let mut changed = original.clone();
        values(&mut changed)[0].direction = pb::CommunicationDirection::Send as i32;
        rejected.push(changed);
        let mut stripped = original.clone();
        values(&mut stripped)[0].connection_id = None;
        values(&mut stripped)[0].label.as_mut().unwrap().value = "changed".into();
        rejected.push(stripped);
        for mut invalid in [member.clone(), member.clone(), member.clone()]
            .into_iter()
            .enumerate()
        {
            match invalid.0 {
                0 => invalid.1.owner_uid = "01930000-0000-7000-8000-000000000099".into(),
                1 => {
                    invalid.1.member_id = Some(pb::ContractMemberId {
                        value: "01930000-0000-7000-8000-000000000099".into(),
                    });
                }
                _ => {
                    invalid.1.expected_digest = Some(pb::ContentDigest {
                        sha256: "a".repeat(64),
                    });
                }
            }
            let mut bad = original.clone();
            values(&mut bad).push(declaration("provider", true, &invalid.1));
            rejected.push(bad);
        }
        let mut over = original.clone();
        values(&mut over).resize(1025, declaration("provider", true, &member));
        rejected.push(over);
        for e in rejected {
            assert_eq!(
                s.put_entity(Request::new(pb::PutEntityRequest {
                    entity: Some(e),
                    if_match: before.etag.clone(),
                    ..Default::default()
                }))
                .await
                .unwrap_err()
                .code(),
                Code::InvalidArgument
            );
            let after = get(&s, slug, external, None).await;
            assert_eq!(after.entity, before.entity);
            assert_eq!(after.etag, before.etag);
        }
        let mut edit = original.clone();
        values(&mut edit).swap(0, 1);
        values(&mut edit)[1].condition = None;
        values(&mut edit)[1].label.as_mut().unwrap().value = "intends to notify".into();
        let edited = save(&s, edit, None).await.entity.unwrap();
        assert_eq!(selector(&edited, 1), selected);
        let mut removed = edited.clone();
        values(&mut removed).remove(1);
        let mut removed = save(&s, removed, None).await.entity.unwrap();
        assert!(trogon_atlas_core::connections::resolve_connection(&removed, &selected).is_err());
        values(&mut removed).push(declaration("provider", true, &member));
        let added = save(&s, removed, None).await.entity.unwrap();
        assert_ne!(selector(&added, 2), selected);
        assert_eq!(selector(&added, 0), selector(&edited, 0));
    }
}

#[tokio::test]
async fn communication_final_overlay_refuses_peer_and_member_tombstones_atomically() {
    let (s, member) = fixture().await;
    let before = save(
        &s,
        owner(
            "checkout",
            false,
            vec![declaration("provider", true, &member)],
        ),
        None,
    )
    .await;
    for (kind, target) in [
        (pb::EntityKind::ExternalSystem, "provider"),
        (pb::EntityKind::Specification, "notifications"),
    ] {
        let mut edited = before.entity.clone().unwrap();
        values(&mut edited)[0].condition = None;
        let response = s
            .batch_mutate(Request::new(pb::BatchMutateRequest {
                ops: vec![
                    pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(edited),
                            if_match: before.etag.clone(),
                            ..Default::default()
                        })),
                    },
                    pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                            kind: kind as i32,
                            id: Some(id(target)),
                            mode: pb::delete_entity_request::Mode::Force as i32,
                            ..Default::default()
                        })),
                    },
                ],
                ..Default::default()
            }))
            .await;
        match response {
            Err(e) => assert_eq!(e.code(), Code::InvalidArgument),
            Ok(r) => assert_ne!(
                r.into_inner().status,
                pb::batch_mutate_response::Status::Applied as i32
            ),
        }
        let after = get(&s, "checkout", false, None).await;
        assert_eq!(after.entity, before.entity);
        assert_eq!(after.etag, before.etag);
    }
}

#[tokio::test]
async fn communication_subject_hydrates_message_separately_and_never_substitutes_sibling() {
    let (s, member) = fixture().await;
    let owner = save(
        &s,
        owner(
            "checkout",
            false,
            vec![declaration("provider", true, &member); 2],
        ),
        None,
    )
    .await
    .entity
    .unwrap();
    let selected = subject(&owner, 0);
    let sibling = subject(&owner, 1);
    save(
        &s,
        pb::Entity {
            kind: Some(pb::entity::Kind::Person(pb::Person {
                id: Some(id("contact")),
                title: Some(pb::DirectoryDisplayName {
                    value: "Contact".into(),
                }),
                responsibilities: vec![pb::Responsibility {
                    kind: pb::ResponsibilityKind::Contact as i32,
                    subject: Some(selected.clone()),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        },
        None,
    )
    .await;
    let query = |subject| {
        Request::new(pb::ListSubjectAssignmentsRequest {
            subject: Some(subject),
            ..Default::default()
        })
    };
    let response = s
        .list_subject_assignments(query(selected.clone()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        response.subject_state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );
    assert_eq!(response.assignments.len(), 1);
    assert_eq!(
        s.list_subject_assignments(query(sibling))
            .await
            .unwrap()
            .into_inner()
            .assignments,
        []
    );
    let spec = s
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Specification as i32,
            id: Some(id("notifications")),
        }))
        .await
        .unwrap()
        .into_inner();
    let mut edited = spec.entity.unwrap();
    let Some(pb::entity::Kind::Specification(v)) = &mut edited.kind else {
        panic!("spec")
    };
    v.messages.remove(0);
    save(&s, edited, None).await;
    let response = s
        .list_subject_assignments(query(selected.clone()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        response.subject_state,
        pb::ResponsibilityResolutionState::Unresolved as i32
    );
    assert_eq!(response.assignments.len(), 1);
    assert_eq!(
        get(&s, "checkout", false, None).await.entity,
        Some(owner.clone())
    );
    let rejected = save_unavailable_subject(&s, selected).await;
    assert_eq!(rejected.code(), Code::InvalidArgument);
}
async fn save_unavailable_subject(
    s: &EventModelServiceImpl,
    subject: pb::ResponsibilitySubject,
) -> tonic::Status {
    s.put_entity(Request::new(pb::PutEntityRequest {
        entity: Some(pb::Entity {
            kind: Some(pb::entity::Kind::Person(pb::Person {
                id: Some(id("invalid-contact")),
                title: Some(pb::DirectoryDisplayName {
                    value: "Invalid".into(),
                }),
                responsibilities: vec![pb::Responsibility {
                    kind: pb::ResponsibilityKind::Contact as i32,
                    subject: Some(subject),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        }),
        ..Default::default()
    }))
    .await
    .unwrap_err()
}

#[tokio::test]
async fn communication_branch_landing_and_peer_retarget_preserve_independent_message() {
    let (s, member) = fixture().await;
    let original = save(
        &s,
        owner(
            "checkout",
            false,
            vec![declaration("provider", true, &member); 2],
        ),
        None,
    )
    .await
    .entity
    .unwrap();
    s.create_branch(Request::new(pb::CreateBranchRequest {
        name: "communication-work".into(),
        ..Default::default()
    }))
    .await
    .unwrap();
    let mut edit = original.clone();
    values(&mut edit).swap(0, 1);
    values(&mut edit)[0].doc = Some(pb::CatalogText {
        value: "branch rationale".into(),
    });
    let staged = save(&s, edit, Some("communication-work"))
        .await
        .entity
        .unwrap();
    assert_eq!(
        get(&s, "checkout", false, None).await.entity,
        Some(original.clone())
    );
    assert_eq!(
        s.merge_branch(Request::new(pb::MergeBranchRequest {
            name: "communication-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .status,
        pb::merge_branch_response::Status::Applied as i32
    );
    assert_eq!(
        get(&s, "checkout", false, None).await.entity,
        Some(staged.clone())
    );
    for (kind, slug) in [
        (pb::EntityKind::ExternalSystem, "provider"),
        (pb::EntityKind::Specification, "notifications"),
    ] {
        assert_eq!(
            s.delete_entity(Request::new(pb::DeleteEntityRequest {
                kind: kind as i32,
                id: Some(id(slug)),
                mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                ..Default::default()
            }))
            .await
            .unwrap_err()
            .code(),
            Code::FailedPrecondition
        );
    }
    let response = s
        .retarget_references(Request::new(pb::RetargetReferencesRequest {
            from: Some(pb::EntityRef {
                kind: pb::EntityKind::ExternalSystem as i32,
                id: Some(id("provider")),
            }),
            to: Some(pb::EntityRef {
                kind: pb::EntityKind::ExternalSystem as i32,
                id: Some(id("auditor")),
            }),
            filter_kinds: vec![pb::EntityKind::Component as i32],
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(response.rewritten, 1);
    let after = get(&s, "checkout", false, None).await.entity.unwrap();
    for i in 0..2 {
        assert_ne!(selector(&after, i), selector(&staged, i));
        assert_eq!(declarations(&after)[i].message, Some(member.clone()));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mixed_owner_inventories_share_nested_subject_budget_without_charging_directory_queries() {
    let (service, member) = fixture().await;
    let mut assignments = Vec::new();
    for index in 0..10 {
        let mut entity = owner(
            &format!("bounded-{index}"),
            index % 2 == 0,
            vec![declaration("provider", true, &member); 512],
        );
        let relationships = vec![
            pb::ArchitecturalRelationship {
                target: Some(pb::ArchitecturalEndpoint {
                    target: Some(pb::architectural_endpoint::Target::ExternalSystem(
                        pb::ExternalSystemRef {
                            id: Some(id("provider"))
                        }
                    ))
                }),
                label: Some(pb::CatalogTitle {
                    value: "uses".into()
                }),
                ..Default::default()
            };
            512
        ];
        match entity.kind.as_mut().unwrap() {
            pb::entity::Kind::Component(value) => value.relationships = relationships,
            pb::entity::Kind::ExternalSystem(value) => value.relationships = relationships,
            _ => panic!("owner"),
        }
        let entity = save(&service, entity, None).await.entity.unwrap();
        assignments.push(pb::Responsibility {
            kind: pb::ResponsibilityKind::Owner as i32,
            subject: Some(subject(&entity, 0)),
            ..Default::default()
        });
    }
    save(
        &service,
        pb::Entity {
            kind: Some(pb::entity::Kind::Person(pb::Person {
                id: Some(id("maintainer")),
                title: Some(pb::DirectoryDisplayName {
                    value: "Maintainer".into(),
                }),
                responsibilities: assignments,
                ..Default::default()
            })),
            ..Default::default()
        },
        None,
    )
    .await;
    let error = service
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
        .unwrap_err();
    assert_eq!(error.code(), Code::ResourceExhausted);
    let teams = service
        .list_person_teams(Request::new(pb::ListPersonTeamsRequest {
            person: Some(pb::PersonRef {
                id: Some(id("maintainer")),
            }),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!teams.incomplete);
    assert_eq!(teams.memberships, []);
    let assignments = service
        .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
            subject: Some(pb::ResponsibilitySubject {
                selection: Some(pb::responsibility_subject::Selection::Entity(
                    pb::EntityRef {
                        kind: pb::EntityKind::ExternalSystem as i32,
                        id: Some(id("provider")),
                    },
                )),
            }),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!assignments.incomplete);
    assert_eq!(assignments.assignments, []);
    assert_eq!(
        assignments.subject_state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );
    assert_eq!(
        service
            .get_incoming_references(Request::new(pb::GetReferencesRequest {
                kind: pb::EntityKind::ExternalSystem as i32,
                id: Some(id("provider")),
                ..Default::default()
            }))
            .await
            .unwrap_err()
            .code(),
        Code::ResourceExhausted
    );
}

#[tokio::test]
async fn nested_message_catalog_inventory_uses_shared_member_budget_and_same_owner_deduplication() {
    let (service, _) = fixture().await;
    let mut members = Vec::new();
    for index in 0..10 {
        let spec_id = id(&format!("large-contract-{index}"));
        let specification = save(
            &service,
            pb::Entity {
                kind: Some(pb::entity::Kind::Specification(pb::Specification {
                    id: Some(spec_id.clone()),
                    title: "Large contract".into(),
                    messages: (0..1024)
                        .map(|index| pb::ContractMessage {
                            name: Some(pb::MemberName {
                                value: format!("Message{index}"),
                            }),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                })),
                ..Default::default()
            },
            None,
        )
        .await
        .entity
        .unwrap();
        let Some(pb::entity::Kind::Specification(value)) = &specification.kind else {
            panic!("Specification")
        };
        members.push(pb::ContractMemberRef {
            specification: Some(pb::SpecificationRef { id: Some(spec_id) }),
            owner_uid: specification.system.as_ref().unwrap().uid.clone(),
            kind: pb::ContractMemberKind::Message as i32,
            member_id: value.messages[0].member_id.clone(),
            ..Default::default()
        });
    }
    let distinct = save(
        &service,
        owner(
            "distinct-contract-owner",
            false,
            members
                .iter()
                .map(|member| declaration("provider", true, member))
                .collect(),
        ),
        None,
    )
    .await
    .entity
    .unwrap();
    let repeated = save(
        &service,
        owner(
            "repeated-contract-owner",
            false,
            vec![declaration("provider", true, &members[0]); 10],
        ),
        None,
    )
    .await
    .entity
    .unwrap();
    for (slug, selected_owner) in [
        ("same-contract-document", &repeated),
        ("distinct-contract-document", &distinct),
    ] {
        save(
            &service,
            pb::Entity {
                kind: Some(pb::entity::Kind::Document(pb::Document {
                    id: Some(id(slug)),
                    title: Some(pb::CatalogTitle {
                        value: "Communication rationale".into(),
                    }),
                    body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
                        media_type: Some(pb::MediaType {
                            value: "text/plain".into(),
                        }),
                        content: "Rationale".into(),
                        digest: Some(trogon_atlas_core::specification::content_digest(
                            "Rationale",
                        )),
                    })),
                    subjects: (0..10)
                        .map(|index| pb::SubjectAssociation {
                            subject: Some(pb::CatalogSubject {
                                selection: Some(pb::catalog_subject::Selection::Connection(
                                    selector(selected_owner, index),
                                )),
                            }),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                })),
                ..Default::default()
            },
            None,
        )
        .await;
    }
    let query = |slug| {
        Request::new(pb::GetDocumentDetailRequest {
            document: Some(pb::DocumentSelection {
                selection: Some(pb::document_selection::Selection::Document(
                    pb::DocumentRef { id: Some(id(slug)) },
                )),
            }),
        })
    };
    let repeated = service
        .get_document_detail(query("same-contract-document"))
        .await
        .unwrap()
        .into_inner();
    assert!(!repeated.incomplete);
    assert_eq!(repeated.declarations.len(), 10);
    let overflow = service
        .get_document_detail(query("distinct-contract-document"))
        .await
        .unwrap_err();
    assert_eq!(overflow.code(), Code::ResourceExhausted);
}

#[tokio::test]
async fn communication_prepared_owner_limit_includes_server_assigned_identities() {
    use prost::Message as _;
    let (service, member) = fixture().await;
    let mut entity = owner(
        "size-boundary",
        false,
        vec![declaration("provider", true, &member); 64],
    );
    let requested_limit = trogon_atlas_core::communication_intent::MAX_ENCODED_BYTES - 256;
    for index in 0..64 {
        values(&mut entity)[index].doc = Some(pb::CatalogText {
            value: "x".repeat(16_384),
        });
        let size = entity.encoded_len();
        if size > requested_limit {
            let excess = size - requested_limit;
            let doc = values(&mut entity)[index].doc.as_mut().unwrap();
            doc.value.truncate(doc.value.len() - excess);
            break;
        }
    }
    assert!(entity.encoded_len() <= requested_limit);
    trogon_atlas_core::communication_intent::validate_values(&entity).unwrap();
    let error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity.clone()),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Component as i32,
                id: Some(id("size-boundary"))
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let doc = values(&mut entity)
        .iter_mut()
        .find_map(|value| value.doc.as_mut())
        .unwrap();
    doc.value.truncate(doc.value.len() - 8_192);
    let accepted = save(&service, entity, None).await.entity.unwrap();
    assert!(accepted.encoded_len() <= trogon_atlas_core::communication_intent::MAX_ENCODED_BYTES);
    assert_eq!(declarations(&accepted).len(), 64);
    assert!(declarations(&accepted)
        .iter()
        .all(|value| value.connection_id.is_some()));
}
