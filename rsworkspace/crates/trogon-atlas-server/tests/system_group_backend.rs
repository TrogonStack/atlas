#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use tonic::{Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "groups".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn title(value: &str) -> Option<pb::CatalogTitle> {
    Some(pb::CatalogTitle {
        value: value.into(),
    })
}
fn component(slug: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Component(pb::Component {
            id: Some(id(slug)),
            title: slug.into(),
            ..Default::default()
        })),
        ..Default::default()
    }
}
fn external(slug: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
            id: Some(id(slug)),
            title: slug.into(),
            ..Default::default()
        })),
        ..Default::default()
    }
}
fn persona(slug: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Persona(pb::Persona {
            id: Some(id(slug)),
            title: slug.into(),
            ..Default::default()
        })),
        ..Default::default()
    }
}
fn group_member(target: pb::system_group_member::Target) -> pb::SystemGroupMembership {
    pb::SystemGroupMembership {
        member: Some(pb::SystemGroupMember {
            target: Some(target),
        }),
        doc: Some(pb::CatalogText {
            value: "curated".into(),
        }),
        ..Default::default()
    }
}
fn member_component(slug: &str) -> pb::SystemGroupMembership {
    group_member(pb::system_group_member::Target::Component(
        pb::ComponentRef { id: Some(id(slug)) },
    ))
}
fn member_external(slug: &str) -> pb::SystemGroupMembership {
    group_member(pb::system_group_member::Target::ExternalSystem(
        pb::ExternalSystemRef { id: Some(id(slug)) },
    ))
}
fn member_group(slug: &str) -> pb::SystemGroupMembership {
    group_member(pb::system_group_member::Target::SystemGroup(
        pb::SystemGroupRef { id: Some(id(slug)) },
    ))
}
fn relationship(slug: &str, label: Option<&str>) -> pb::SystemGroupRelationship {
    pb::SystemGroupRelationship {
        target: Some(pb::SystemGroupRef { id: Some(id(slug)) }),
        label: label.and_then(title),
        ..Default::default()
    }
}
fn actor(
    target: pb::system_actor_target::Target,
    direction: pb::SystemActorDirection,
) -> pb::SystemActorLink {
    pb::SystemActorLink {
        target: Some(pb::SystemActorTarget {
            target: Some(target),
        }),
        direction: direction as i32,
        label: title("uses"),
        ..Default::default()
    }
}
fn group(
    slug: &str,
    members: Vec<pb::SystemGroupMembership>,
    relationships: Vec<pb::SystemGroupRelationship>,
    actors: Vec<pb::SystemActorLink>,
) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::SystemGroup(pb::SystemGroup {
            id: Some(id(slug)),
            title: slug.into(),
            scope: pb::SystemGroupScope::Internal as i32,
            members,
            relationships,
            actors,
            ..Default::default()
        })),
        ..Default::default()
    }
}
fn domain_member(target: pb::domain_member::Target) -> pb::DomainMembership {
    pb::DomainMembership {
        member: Some(pb::DomainMember {
            target: Some(target),
        }),
        ..Default::default()
    }
}
fn domain(slug: &str, members: Vec<pb::DomainMembership>) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Domain(pb::Domain {
            id: Some(id(slug)),
            title: slug.into(),
            members,
            ..Default::default()
        })),
        ..Default::default()
    }
}
fn group_mut(entity: &mut pb::Entity) -> &mut pb::SystemGroup {
    match entity.kind.as_mut().unwrap() {
        pb::entity::Kind::SystemGroup(value) => value,
        _ => panic!("system group"),
    }
}
fn domain_mut(entity: &mut pb::Entity) -> &mut pb::Domain {
    match entity.kind.as_mut().unwrap() {
        pb::entity::Kind::Domain(value) => value,
        _ => panic!("domain"),
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
    snapshot_service().await.with_snapshot_cap(0)
}
async fn snapshot_service() -> EventModelServiceImpl {
    EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap()
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
    kind: pb::EntityKind,
    slug: &str,
    branch: Option<&str>,
) -> pb::GetEntityResponse {
    service
        .get_entity(request(
            pb::GetEntityRequest {
                kind: kind as i32,
                id: Some(id(slug)),
            },
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
}
async fn get_group(service: &EventModelServiceImpl, slug: &str) -> pb::GetEntityResponse {
    get(service, pb::EntityKind::SystemGroup, slug, None).await
}
async fn incoming(
    service: &EventModelServiceImpl,
    kind: pb::EntityKind,
    slug: &str,
) -> Vec<pb::Reference> {
    service
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: kind as i32,
            id: Some(id(slug)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references
}
fn member_ids(entity: &pb::Entity) -> Vec<Option<pb::ConnectionId>> {
    match entity.kind.as_ref().unwrap() {
        pb::entity::Kind::SystemGroup(value) => value
            .members
            .iter()
            .map(|m| m.connection_id.clone())
            .collect(),
        pb::entity::Kind::Domain(value) => value
            .members
            .iter()
            .map(|m| m.connection_id.clone())
            .collect(),
        _ => panic!("owner"),
    }
}
async fn fixture() -> EventModelServiceImpl {
    let service = service().await;
    for entity in [
        component("worker"),
        component("scheduler"),
        external("provider"),
        persona("shopper"),
        group("edge", vec![], vec![], vec![]),
    ] {
        save(&service, entity, None).await;
    }
    save(
        &service,
        group(
            "platform",
            vec![
                member_component("worker"),
                member_component("worker"),
                member_external("provider"),
                member_group("edge"),
            ],
            vec![relationship("edge", None)],
            vec![
                actor(
                    pb::system_actor_target::Target::Persona(pb::PersonaRef {
                        id: Some(id("shopper")),
                    }),
                    pb::SystemActorDirection::Inbound,
                ),
                actor(
                    pb::system_actor_target::Target::Component(pb::ComponentRef {
                        id: Some(id("scheduler")),
                    }),
                    pb::SystemActorDirection::Outbound,
                ),
            ],
        ),
        None,
    )
    .await;
    service
}
async fn put_error(
    service: &EventModelServiceImpl,
    entity: pb::Entity,
    etag: &str,
) -> tonic::Status {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity),
            if_match: etag.into(),
            ..Default::default()
        }))
        .await
        .unwrap_err()
}

#[tokio::test]
async fn group_lifecycle_keeps_parallel_members_distinct_and_rejects_invalid_curation() {
    let service = fixture().await;
    let before = get_group(&service, "platform").await;
    let original = before.entity.as_ref().unwrap();
    let ids = member_ids(original);
    assert!(ids.iter().all(Option::is_some));
    assert_ne!(ids[0], ids[1]);

    let worker = incoming(&service, pb::EntityKind::Component, "worker").await;
    let from_platform: Vec<_> = worker
        .iter()
        .filter(|reference| reference.from_field.starts_with("members["))
        .collect();
    assert_eq!(from_platform.len(), 2);
    for (index, reference) in from_platform.iter().enumerate() {
        assert_eq!(reference.from_field, format!("members[{index}].component"));
        assert_eq!(reference.connection_id, ids[index]);
        assert_eq!(
            reference.from.as_ref().unwrap().kind,
            pb::EntityKind::SystemGroup as i32
        );
    }
    let actors = incoming(&service, pb::EntityKind::Persona, "shopper").await;
    assert_eq!(actors.len(), 1);
    assert_eq!(actors[0].from_field, "actors[0].target.persona");
    assert!(actors[0].connection_id.is_some());
    let relationships = incoming(&service, pb::EntityKind::SystemGroup, "edge").await;
    let fields: Vec<_> = relationships
        .iter()
        .map(|r| r.from_field.as_str())
        .collect();
    assert!(fields.contains(&"relationships[0].target"));
    assert!(fields.contains(&"members[3].system_group"));

    let mut rejected = Vec::new();
    let mut self_member = original.clone();
    group_mut(&mut self_member)
        .members
        .push(member_group("platform"));
    rejected.push(self_member);
    let mut self_relationship = original.clone();
    group_mut(&mut self_relationship)
        .relationships
        .push(relationship("platform", None));
    rejected.push(self_relationship);
    let mut dangling = original.clone();
    group_mut(&mut dangling)
        .members
        .push(member_component("missing"));
    rejected.push(dangling);
    let mut dangling_actor = original.clone();
    group_mut(&mut dangling_actor).actors.push(actor(
        pb::system_actor_target::Target::Persona(pb::PersonaRef {
            id: Some(id("missing")),
        }),
        pb::SystemActorDirection::Inbound,
    ));
    rejected.push(dangling_actor);
    let mut graph = original.clone();
    group_mut(&mut graph)
        .members
        .push(group_member(pb::system_group_member::Target::DesignGraph(
            pb::DesignGraphRef {
                id: Some(id("graph")),
            },
        )));
    rejected.push(graph);
    let mut unspecified_scope = original.clone();
    group_mut(&mut unspecified_scope).scope = pb::SystemGroupScope::Unspecified as i32;
    rejected.push(unspecified_scope);
    let mut direction = original.clone();
    group_mut(&mut direction).actors[0].direction = pb::SystemActorDirection::Unspecified as i32;
    rejected.push(direction);
    let mut retargeted = original.clone();
    group_mut(&mut retargeted).members[2] = member_component("scheduler");
    group_mut(&mut retargeted).members[2].connection_id = ids[2].clone();
    rejected.push(retargeted);
    let mut oversized = original.clone();
    group_mut(&mut oversized)
        .members
        .resize(1025, member_component("worker"));
    rejected.push(oversized);
    for entity in rejected {
        let error = put_error(&service, entity, &before.etag).await;
        assert_eq!(error.code(), Code::InvalidArgument, "{error}");
        assert_eq!(get_group(&service, "platform").await, before);
    }

    let mut edit = original.clone();
    group_mut(&mut edit).members.swap(0, 3);
    group_mut(&mut edit).members[0].doc = None;
    let edited = save(&service, edit, None).await.entity.unwrap();
    let edited_ids = member_ids(&edited);
    assert_eq!(edited_ids[0], ids[3]);
    assert_eq!(edited_ids[3], ids[0]);
    let mut removed = edited.clone();
    group_mut(&mut removed).members.remove(3);
    let removed = save(&service, removed, None).await.entity.unwrap();
    assert!(!member_ids(&removed).contains(&ids[0]));
    let mut readded = removed.clone();
    group_mut(&mut readded)
        .members
        .push(member_component("worker"));
    let readded = save(&service, readded, None).await.entity.unwrap();
    assert!(!member_ids(&readded).contains(&ids[0]));
}

#[tokio::test]
async fn membership_is_shared_across_groups_and_external_scope_is_descriptive() {
    let service = fixture().await;
    let mut external_group = group(
        "partners",
        vec![member_component("worker"), member_external("provider")],
        vec![],
        vec![],
    );
    group_mut(&mut external_group).scope = pb::SystemGroupScope::External as i32;
    save(&service, external_group, None).await;
    let worker = incoming(&service, pb::EntityKind::Component, "worker").await;
    let owners: std::collections::BTreeSet<_> = worker
        .iter()
        .filter_map(|reference| {
            reference
                .from
                .as_ref()?
                .id
                .as_ref()
                .map(|id| id.slug.clone())
        })
        .collect();
    assert!(owners.contains("platform"));
    assert!(owners.contains("partners"));
    let platform = get_group(&service, "platform").await.entity.unwrap();
    assert_eq!(member_ids(&platform).len(), 4);
}

#[tokio::test]
async fn domain_members_are_exact_and_authoring_without_members_is_unchanged() {
    let service = fixture().await;
    save(&service, domain("fulfilment", vec![]), None).await;
    let retail = domain(
        "retail",
        vec![
            domain_member(pb::domain_member::Target::SystemGroup(pb::SystemGroupRef {
                id: Some(id("platform")),
            })),
            domain_member(pb::domain_member::Target::Component(pb::ComponentRef {
                id: Some(id("worker")),
            })),
            domain_member(pb::domain_member::Target::Domain(pb::DomainRef {
                id: Some(id("fulfilment")),
            })),
            domain_member(pb::domain_member::Target::ExternalSystem(
                pb::ExternalSystemRef {
                    id: Some(id("provider")),
                },
            )),
        ],
    );
    let saved = save(&service, retail, None).await.entity.unwrap();
    let ids = member_ids(&saved);
    assert_eq!(ids.len(), 4);
    assert!(ids.iter().all(Option::is_some));
    let refs = incoming(&service, pb::EntityKind::Domain, "fulfilment").await;
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].from_field, "members[2].domain");
    assert_eq!(refs[0].connection_id, ids[2]);

    let before = get(&service, pb::EntityKind::Domain, "retail", None).await;
    let mut rejected = Vec::new();
    let mut self_domain = saved.clone();
    domain_mut(&mut self_domain)
        .members
        .push(domain_member(pb::domain_member::Target::Domain(
            pb::DomainRef {
                id: Some(id("retail")),
            },
        )));
    rejected.push(self_domain);
    let mut dangling = saved.clone();
    domain_mut(&mut dangling)
        .members
        .push(domain_member(pb::domain_member::Target::Component(
            pb::ComponentRef {
                id: Some(id("missing")),
            },
        )));
    rejected.push(dangling);
    let mut graph = saved.clone();
    domain_mut(&mut graph)
        .members
        .push(domain_member(pb::domain_member::Target::DesignGraph(
            pb::DesignGraphRef {
                id: Some(id("graph")),
            },
        )));
    rejected.push(graph);
    for entity in rejected {
        let error = put_error(&service, entity, &before.etag).await;
        assert_eq!(error.code(), Code::InvalidArgument, "{error}");
        assert_eq!(
            get(&service, pb::EntityKind::Domain, "retail", None).await,
            before
        );
    }
    let mut cleared = saved.clone();
    domain_mut(&mut cleared).members.clear();
    let cleared = save(&service, cleared, None).await.entity.unwrap();
    assert_eq!(
        member_ids(&cleared),
        [] as [std::option::Option<trogon_atlas_proto::ConnectionId>; 0]
    );
}

#[tokio::test]
async fn retarget_renews_only_changed_members_and_safe_delete_protects_hidden_edges() {
    let service = fixture().await;
    let before = get_group(&service, "platform").await.entity.unwrap();
    let ids = member_ids(&before);
    let delete = |force| pb::DeleteEntityRequest {
        kind: pb::EntityKind::Component as i32,
        id: Some(id("worker")),
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
    get(&service, pb::EntityKind::Component, "worker", None).await;
    for dry_run in [true, false] {
        let response = service
            .retarget_references(Request::new(pb::RetargetReferencesRequest {
                from: Some(pb::EntityRef {
                    kind: pb::EntityKind::Component as i32,
                    id: Some(id("worker")),
                }),
                to: Some(pb::EntityRef {
                    kind: pb::EntityKind::Component as i32,
                    id: Some(id("scheduler")),
                }),
                filter_kinds: vec![pb::EntityKind::SystemGroup as i32],
                dry_run,
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(response.rewritten, 1, "{response:?}");
        assert_eq!(response.failed, 0);
        if dry_run {
            assert_eq!(
                get_group(&service, "platform").await.entity,
                Some(before.clone())
            );
        }
    }
    let after = get_group(&service, "platform").await.entity.unwrap();
    let after_ids = member_ids(&after);
    assert_ne!(after_ids[0], ids[0]);
    assert_ne!(after_ids[1], ids[1]);
    assert_eq!(after_ids[2], ids[2]);
    assert_eq!(after_ids[3], ids[3]);
    service
        .delete_entity(Request::new(delete(true)))
        .await
        .unwrap();
    assert_eq!(get_group(&service, "platform").await.entity, Some(after));
}

#[tokio::test]
async fn final_batch_overlay_rejects_tombstoned_members_atomically() {
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
            group("platform", vec![member_component("worker")], vec![], vec![]),
            component("worker"),
        ])))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        response.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "{response:?}"
    );
    let before = get_group(&service, "platform").await;
    let mut edited = before.entity.clone().unwrap();
    group_mut(&mut edited).title = "renamed".into();
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
                        kind: pb::EntityKind::Component as i32,
                        id: Some(id("worker")),
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
    assert_eq!(get_group(&service, "platform").await, before);
    get(&service, pb::EntityKind::Component, "worker", None).await;
}

#[tokio::test]
async fn branch_edit_and_landing_keep_incarnation_and_member_identity() {
    let service = fixture().await;
    let before = get_group(&service, "platform").await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "group-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut edit = before.entity.clone().unwrap();
    group_mut(&mut edit).members.swap(0, 2);
    group_mut(&mut edit).members[0].doc = None;
    let branch = save(&service, edit, Some("group-work"))
        .await
        .entity
        .unwrap();
    assert_eq!(get_group(&service, "platform").await, before);
    for dry_run in [true, false] {
        let response = service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: "group-work".into(),
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
        get_group(&service, "platform").await.entity,
        Some(branch.clone())
    );
    assert_eq!(
        member_ids(&branch)[2],
        member_ids(before.entity.as_ref().unwrap())[0]
    );
}

#[tokio::test]
async fn cycles_are_allowed_and_reported_as_bounded_advisories() {
    let service = snapshot_service().await;
    save(&service, group("alpha", vec![], vec![], vec![]), None).await;
    save(
        &service,
        group("beta", vec![member_group("alpha")], vec![], vec![]),
        None,
    )
    .await;
    let alpha = get_group(&service, "alpha").await.entity.unwrap();
    let mut closing = alpha.clone();
    group_mut(&mut closing).members.push(member_group("beta"));
    save(&service, closing, None).await;
    let response = service
        .validate_project(Request::new(pb::ValidateProjectRequest {
            scope: Some(pb::validate_project_request::Scope::Namespace(
                "groups".into(),
            )),
        }))
        .await
        .unwrap()
        .into_inner();
    let findings: Vec<_> = response
        .reports
        .into_iter()
        .flat_map(|report| report.issues)
        .filter(|issue| issue.code == "SYSTEM_GROUP_MEMBERSHIP_CYCLE")
        .collect();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(
        findings[0].severity,
        pb::validation_issue::Severity::Info as i32
    );
    assert!(findings[0].message.contains("groups/alpha@1"));
    assert!(findings[0].message.contains("groups/beta@1"));
}
