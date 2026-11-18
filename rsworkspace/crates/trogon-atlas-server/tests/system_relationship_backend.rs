#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use tonic::{Code, Request};
use trogon_atlas_core::architectural_relationships::relationships;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "architecture".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn owner(
    slug: &str,
    external: bool,
    relationships: Vec<pb::ArchitecturalRelationship>,
) -> pb::Entity {
    pb::Entity {
        kind: Some(if external {
            pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
                id: Some(id(slug)),
                title: slug.into(),
                relationships,
                ..Default::default()
            })
        } else {
            pb::entity::Kind::Component(pb::Component {
                id: Some(id(slug)),
                title: slug.into(),
                relationships,
                ..Default::default()
            })
        }),
        ..Default::default()
    }
}
fn relationship(slug: &str, external: bool) -> pb::ArchitecturalRelationship {
    pb::ArchitecturalRelationship {
        target: Some(pb::ArchitecturalEndpoint {
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
        label: Some(pb::CatalogTitle {
            value: "calls".into(),
        }),
        doc: Some(pb::CatalogText {
            value: "authored architectural intent".into(),
        }),
        ..Default::default()
    }
}
fn values(entity: &mut pb::Entity) -> &mut Vec<pb::ArchitecturalRelationship> {
    match entity.kind.as_mut().unwrap() {
        pb::entity::Kind::Component(value) => &mut value.relationships,
        pb::entity::Kind::ExternalSystem(value) => &mut value.relationships,
        _ => panic!("architectural owner"),
    }
}
fn request<T>(body: T, branch: Option<&str>) -> Request<T> {
    let mut request = Request::new(body);
    if let Some(branch) = branch {
        request
            .metadata_mut()
            .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    }
    request
}
async fn service() -> EventModelServiceImpl {
    EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap()
    .with_snapshot_cap(0)
}
async fn save(
    service: &EventModelServiceImpl,
    entity: pb::Entity,
    branch: Option<&str>,
) -> pb::PutEntityResponse {
    service
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(entity),
                ..Default::default()
            },
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
}
async fn get(
    service: &EventModelServiceImpl,
    slug: &str,
    external: bool,
    branch: Option<&str>,
) -> pb::GetEntityResponse {
    service
        .get_entity(request(
            pb::GetEntityRequest {
                kind: if external {
                    pb::EntityKind::ExternalSystem
                } else {
                    pb::EntityKind::Component
                } as i32,
                id: Some(id(slug)),
            },
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
}
fn selector(entity: &pb::Entity, index: usize) -> pb::ConnectionRef {
    pb::ConnectionRef {
        owner: Some(trogon_atlas_server::conv::entity_to_ref(entity).unwrap()),
        owner_uid: entity.system.as_ref().unwrap().uid.clone(),
        id: relationships(entity)[index].connection_id.clone(),
    }
}
async fn fixture() -> EventModelServiceImpl {
    let service = service().await;
    for (slug, external) in [("provider", true), ("auditor", true), ("worker", false)] {
        save(&service, owner(slug, external, vec![]), None).await;
    }
    save(
        &service,
        owner(
            "checkout",
            false,
            vec![
                relationship("provider", true),
                relationship("provider", true),
                relationship("worker", false),
            ],
        ),
        None,
    )
    .await;
    service
}

#[tokio::test]
async fn relationship_public_lifecycle_preserves_parallel_provenance_and_rejects_atomic_edits() {
    let service = fixture().await;
    let before = get(&service, "checkout", false, None).await;
    let original = before.entity.as_ref().unwrap();
    let selected = selector(original, 0);
    assert_ne!(
        relationships(original)[0].connection_id,
        relationships(original)[1].connection_id
    );
    let incoming = service
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::ExternalSystem as i32,
            id: Some(id("provider")),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(incoming.references.len(), 2);
    for (index, reference) in incoming.references.iter().enumerate() {
        assert_eq!(reference.from, selected.owner);
        assert_eq!(reference.from_uid, selected.owner_uid);
        assert_eq!(
            reference.connection_id,
            relationships(original)[index].connection_id
        );
        assert_eq!(
            reference.from_field,
            format!("relationships[{index}].target.external_system")
        );
        assert_eq!(reference.to.as_ref().unwrap().id, Some(id("provider")));
    }
    let mut rejected = Vec::new();
    let mut stripped = original.clone();
    values(&mut stripped)[0].connection_id = None;
    values(&mut stripped)[0].label.as_mut().unwrap().value = "changed".into();
    rejected.push(stripped);
    let mut retarget = original.clone();
    values(&mut retarget)[0].target = relationship("auditor", true).target;
    rejected.push(retarget);
    let mut dangling = original.clone();
    values(&mut dangling).push(relationship("missing", true));
    rejected.push(dangling);
    let mut self_target = original.clone();
    values(&mut self_target).push(relationship("checkout", false));
    rejected.push(self_target);
    let mut oversized = original.clone();
    values(&mut oversized).resize(1025, relationship("provider", true));
    rejected.push(oversized);
    let mut blank = original.clone();
    values(&mut blank)[0].label.as_mut().unwrap().value = " ".into();
    rejected.push(blank);
    for entity in rejected {
        let error = service
            .put_entity(Request::new(pb::PutEntityRequest {
                entity: Some(entity),
                if_match: before.etag.clone(),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{error}");
        assert_eq!(get(&service, "checkout", false, None).await, before);
    }
    let mut edit = original.clone();
    values(&mut edit).swap(0, 1);
    values(&mut edit)[1].label.as_mut().unwrap().value = "submits".into();
    let edited = save(&service, edit, None).await.entity.unwrap();
    assert_eq!(selector(&edited, 1), selected);
    let mut removed = edited.clone();
    values(&mut removed).remove(1);
    let removed = save(&service, removed, None).await.entity.unwrap();
    assert!(trogon_atlas_core::connections::resolve_connection(&removed, &selected).is_err());
    let mut readded = removed.clone();
    values(&mut readded).push(relationship("provider", true));
    let readded = save(&service, readded, None).await.entity.unwrap();
    assert_ne!(selector(&readded, 2), selected);
    assert_eq!(selector(&readded, 0), selector(&edited, 0));
}

#[tokio::test]
async fn relationship_public_final_batch_overlay_rejects_tombstones_atomically() {
    let service = service().await;
    let batch = |entities: Vec<pb::Entity>| pb::BatchMutateRequest {
        ops: entities
            .into_iter()
            .map(|entity| pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                    entity: Some(entity),
                    create_only: true,
                    ..Default::default()
                })),
            })
            .collect(),
        ..Default::default()
    };
    let response = service
        .batch_mutate(Request::new(batch(vec![
            owner("checkout", false, vec![relationship("provider", true)]),
            owner("provider", true, vec![]),
        ])))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        response.status,
        pb::batch_mutate_response::Status::Applied as i32
    );
    let before = get(&service, "checkout", false, None).await;
    let mut edited = before.entity.clone().unwrap();
    values(&mut edited)[0].label.as_mut().unwrap().value = "new wording".into();
    let response = service
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
                        kind: pb::EntityKind::ExternalSystem as i32,
                        id: Some(id("provider")),
                        mode: pb::delete_entity_request::Mode::Force as i32,
                        ..Default::default()
                    })),
                },
            ],
            ..Default::default()
        }))
        .await;
    match response {
        Err(error) => assert_eq!(error.code(), Code::InvalidArgument),
        Ok(response) => assert_ne!(
            response.into_inner().status,
            pb::batch_mutate_response::Status::Applied as i32
        ),
    }
    assert_eq!(get(&service, "checkout", false, None).await, before);
    get(&service, "provider", true, None).await;
}

#[tokio::test]
async fn relationship_public_retarget_renews_only_changed_occurrences_and_safe_delete_protects() {
    let service = fixture().await;
    let before = get(&service, "checkout", false, None).await.entity.unwrap();
    let selected = selector(&before, 0);
    let delete = |force| pb::DeleteEntityRequest {
        kind: pb::EntityKind::ExternalSystem as i32,
        id: Some(id("provider")),
        mode: if force {
            pb::delete_entity_request::Mode::Force
        } else {
            pb::delete_entity_request::Mode::FailIfReferenced
        } as i32,
        ..Default::default()
    };
    let protected = service
        .delete_entity(Request::new(delete(false)))
        .await
        .unwrap_err();
    assert_eq!(protected.code(), Code::FailedPrecondition);
    get(&service, "provider", true, None).await;
    for dry_run in [true, false] {
        let response = service
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
                dry_run,
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(response.rewritten, 1);
        assert_eq!(response.failed, 0);
        if dry_run {
            assert_eq!(
                get(&service, "checkout", false, None).await.entity,
                Some(before.clone())
            );
        }
    }
    let after = get(&service, "checkout", false, None).await.entity.unwrap();
    assert_ne!(selector(&after, 0), selected);
    assert_ne!(selector(&after, 1), selector(&before, 1));
    assert_eq!(selector(&after, 2), selector(&before, 2));
    assert!(trogon_atlas_core::connections::resolve_connection(&after, &selected).is_err());
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            id: Some(id("auditor")),
            ..delete(true)
        }))
        .await
        .unwrap();
    assert_eq!(
        get(&service, "checkout", false, None).await.entity,
        Some(after)
    );
    save(&service, owner("unrelated", false, vec![]), None).await;
}

#[tokio::test]
async fn relationship_public_branch_edit_and_landing_keep_incarnation_and_ids() {
    let service = fixture().await;
    let before = get(&service, "checkout", false, None).await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "relationship-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut edit = before.entity.clone().unwrap();
    values(&mut edit).swap(0, 1);
    values(&mut edit)[0].doc = None;
    let branch = save(&service, edit, Some("relationship-work"))
        .await
        .entity
        .unwrap();
    assert_eq!(get(&service, "checkout", false, None).await, before);
    for dry_run in [true, false] {
        let response = service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: "relationship-work".into(),
                dry_run,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            response.status,
            if dry_run {
                pb::merge_branch_response::Status::Validated
            } else {
                pb::merge_branch_response::Status::Applied
            } as i32,
            "{response:?}"
        );
    }
    assert_eq!(
        get(&service, "checkout", false, None).await.entity,
        Some(branch.clone())
    );
    assert_eq!(
        selector(&branch, 1),
        selector(before.entity.as_ref().unwrap(), 0)
    );
}

#[tokio::test]
async fn nested_exact_relationship_owners_share_responsibility_occurrence_budget() {
    let service = service().await;
    save(&service, owner("target", true, vec![]), None).await;
    let mut assignments = Vec::new();
    for index in 0..10 {
        let entity = save(
            &service,
            owner(
                &format!("owner-{index}"),
                false,
                vec![relationship("target", true); 1024],
            ),
            None,
        )
        .await
        .entity
        .unwrap();
        assignments.push(pb::Responsibility {
            kind: pb::ResponsibilityKind::Owner as i32,
            subject: Some(pb::ResponsibilitySubject {
                selection: Some(pb::responsibility_subject::Selection::Connection(selector(
                    &entity, 0,
                ))),
            }),
            ..Default::default()
        });
    }
    let person = pb::Entity {
        kind: Some(pb::entity::Kind::Person(pb::Person {
            id: Some(id("maintainer")),
            title: Some(pb::DirectoryDisplayName {
                value: "Maintainer".into(),
            }),
            responsibilities: assignments,
            ..Default::default()
        })),
        ..Default::default()
    };
    save(&service, person, None).await;
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
    assert_eq!(error.code(), Code::ResourceExhausted, "{error}");
}

async fn unrelated_relationship_scan_fixture() -> EventModelServiceImpl {
    let service = service().await;
    save(&service, owner("target", true, vec![]), None).await;
    for index in 0..10 {
        save(
            &service,
            owner(
                &format!("unrelated-{index}"),
                index % 2 == 0,
                vec![relationship("target", true); 1024],
            ),
            None,
        )
        .await;
    }
    save(
        &service,
        pb::Entity {
            kind: Some(pb::entity::Kind::Person(pb::Person {
                id: Some(id("maintainer")),
                title: Some(pb::DirectoryDisplayName {
                    value: "Maintainer".into(),
                }),
                ..Default::default()
            })),
            ..Default::default()
        },
        None,
    )
    .await;
    save(
        &service,
        pb::Entity {
            kind: Some(pb::entity::Kind::Team(pb::Team {
                id: Some(id("team")),
                title: Some(pb::DirectoryDisplayName {
                    value: "Team".into(),
                }),
                members: vec![pb::TeamMember {
                    person: Some(pb::PersonRef {
                        id: Some(id("maintainer")),
                    }),
                    ..Default::default()
                }],
                responsibilities: vec![pb::Responsibility {
                    kind: pb::ResponsibilityKind::Owner as i32,
                    subject: Some(target_subject()),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        },
        None,
    )
    .await;
    service
}

fn target_subject() -> pb::ResponsibilitySubject {
    pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Entity(
            pb::EntityRef {
                kind: pb::EntityKind::ExternalSystem as i32,
                id: Some(id("target")),
            },
        )),
    }
}

#[tokio::test]
async fn person_teams_does_not_examine_unrelated_relationship_occurrences() {
    let service = unrelated_relationship_scan_fixture().await;
    let response = service
        .list_person_teams(Request::new(pb::ListPersonTeamsRequest {
            person: Some(pb::PersonRef {
                id: Some(id("maintainer")),
            }),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!response.incomplete);
    assert_eq!(response.memberships.len(), 1);
    assert_eq!(
        response.memberships[0].team.as_ref().unwrap().id,
        Some(id("team"))
    );
}

#[tokio::test]
async fn subject_assignments_does_not_examine_unrelated_relationship_occurrences() {
    let service = unrelated_relationship_scan_fixture().await;
    let response = service
        .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
            subject: Some(target_subject()),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!response.incomplete);
    assert_eq!(
        response.subject_state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );
    assert_eq!(response.assignments.len(), 1);
    assert_eq!(
        response.assignments[0].assignment.as_ref().unwrap().subject,
        Some(target_subject())
    );
}

#[tokio::test]
async fn relationship_public_conflict_resolution_renews_only_restored_ids() {
    for take_theirs in [false, true] {
        let service = fixture().await;
        let original = get(&service, "checkout", false, None).await.entity.unwrap();
        let selected = selector(&original, 0);
        let unaffected = selector(&original, 1);
        service
            .create_branch(Request::new(pb::CreateBranchRequest {
                name: "relationship-conflict".into(),
                ..Default::default()
            }))
            .await
            .unwrap();
        let mut branch = original.clone();
        values(&mut branch)[1].label.as_mut().unwrap().value = "branch wording".into();
        if take_theirs {
            values(&mut branch).remove(0);
        }
        save(&service, branch, Some("relationship-conflict")).await;
        let mut baseline = original.clone();
        values(&mut baseline)[1].label.as_mut().unwrap().value = "baseline wording".into();
        if !take_theirs {
            values(&mut baseline).remove(0);
        }
        save(&service, baseline, None).await;
        let before = get(&service, "checkout", false, None).await;
        let blocked = service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: "relationship-conflict".into(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            blocked.status,
            pb::merge_branch_response::Status::Conflicts as i32
        );
        assert_eq!(get(&service, "checkout", false, None).await, before);
        service
            .resolve_branch_entry(Request::new(pb::ResolveBranchEntryRequest {
                name: "relationship-conflict".into(),
                r#ref: selected.owner.clone(),
                resolution: if take_theirs {
                    pb::resolve_branch_entry_request::Resolution::TakeTheirs
                } else {
                    pb::resolve_branch_entry_request::Resolution::KeepOurs
                } as i32,
                ..Default::default()
            }))
            .await
            .unwrap();
        let resolved = get(&service, "checkout", false, Some("relationship-conflict"))
            .await
            .entity
            .unwrap();
        assert_eq!(relationships(&resolved).len(), 3);
        assert_eq!(resolved.system.as_ref().unwrap().uid, selected.owner_uid);
        assert!(trogon_atlas_core::connections::resolve_connection(&resolved, &selected).is_err());
        assert!(trogon_atlas_core::connections::resolve_connection(&resolved, &unaffected).is_ok());
        let merged = service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: "relationship-conflict".into(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            merged.status,
            pb::merge_branch_response::Status::Applied as i32,
            "{merged:?}"
        );
        assert_eq!(
            get(&service, "checkout", false, None).await.entity,
            Some(resolved)
        );
    }
}
