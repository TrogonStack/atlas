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
fn title(value: &str) -> pb::CatalogTitle {
    pb::CatalogTitle {
        value: value.into(),
    }
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
        label: label.map(title),
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
        label: Some(title("uses")),
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
    group_mut(&mut edited)
        .members
        .push(member_component("worker"));
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

    let mut renamed = before.entity.clone().unwrap();
    group_mut(&mut renamed).title = "renamed".into();
    let response = service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: vec![
                pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        entity: Some(renamed),
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
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        response.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "{response:?}"
    );
    let stored = get_group(&service, "platform").await.entity.unwrap();
    assert_eq!(
        member_ids(&stored),
        member_ids(before.entity.as_ref().unwrap())
    );
    assert!(service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Component as i32,
            id: Some(id("worker")),
        }))
        .await
        .is_err());
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

async fn force_delete(service: &EventModelServiceImpl, kind: pb::EntityKind, slug: &str) {
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: kind as i32,
            id: Some(id(slug)),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
}

async fn put_with(
    service: &EventModelServiceImpl,
    entity: pb::Entity,
    etag: &str,
) -> Result<pb::PutEntityResponse, tonic::Status> {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity),
            if_match: etag.into(),
            ..Default::default()
        }))
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test]
async fn retained_dangling_curation_never_blocks_unrelated_writes_but_new_or_edited_links_stay_strict(
) {
    let service = fixture().await;
    save(
        &service,
        domain(
            "retail",
            vec![
                domain_member(pb::domain_member::Target::Component(pb::ComponentRef {
                    id: Some(id("worker")),
                })),
                domain_member(pb::domain_member::Target::SystemGroup(pb::SystemGroupRef {
                    id: Some(id("edge")),
                })),
            ],
        ),
        None,
    )
    .await;
    let group_before = get_group(&service, "platform").await;
    let domain_before = get(&service, pb::EntityKind::Domain, "retail", None).await;
    for (kind, slug) in [
        (pb::EntityKind::Component, "worker"),
        (pb::EntityKind::Component, "scheduler"),
        (pb::EntityKind::ExternalSystem, "provider"),
        (pb::EntityKind::Persona, "shopper"),
        (pb::EntityKind::SystemGroup, "edge"),
    ] {
        force_delete(&service, kind, slug).await;
    }
    assert_eq!(get_group(&service, "platform").await, group_before);
    assert_eq!(
        get(&service, pb::EntityKind::Domain, "retail", None).await,
        domain_before
    );

    let mut renamed = group_before.entity.clone().unwrap();
    group_mut(&mut renamed).title = "Platform".into();
    let renamed = put_with(&service, renamed, &group_before.etag)
        .await
        .expect("an unrelated edit must not require repairing retained dangling links");
    assert_eq!(
        member_ids(renamed.entity.as_ref().unwrap()),
        member_ids(group_before.entity.as_ref().unwrap())
    );
    assert_eq!(
        renamed
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid,
        group_before
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid
    );
    let mut annotated = renamed.entity.clone().unwrap();
    group_mut(&mut annotated).members.swap(0, 2);
    group_mut(&mut annotated).members[0].doc = None;
    let annotated = put_with(&service, annotated, &renamed.etag)
        .await
        .expect("reordering or annotating retained links keeps their endpoints");

    let mut retitled = domain_before.entity.clone().unwrap();
    domain_mut(&mut retitled).title = "Retail".into();
    let retitled = put_with(&service, retitled, &domain_before.etag)
        .await
        .expect("an unrelated Domain edit must not require repairing retained members");
    assert_eq!(
        member_ids(retitled.entity.as_ref().unwrap()),
        member_ids(domain_before.entity.as_ref().unwrap())
    );

    let group_state = get_group(&service, "platform").await;
    assert_eq!(group_state.entity, annotated.entity);
    let domain_state = get(&service, pb::EntityKind::Domain, "retail", None).await;
    assert_eq!(domain_state.entity, retitled.entity);

    let rejected = |mutate: &dyn Fn(&mut pb::Entity)| {
        let mut entity = group_state.entity.clone().unwrap();
        mutate(&mut entity);
        entity
    };
    let attempts = [
        rejected(&|entity| group_mut(entity).members.push(member_component("ghost"))),
        rejected(&|entity| {
            let link = &mut group_mut(entity).members[0];
            link.connection_id = None;
            link.member = member_component("other-ghost").member;
        }),
        rejected(&|entity| {
            group_mut(entity)
                .relationships
                .push(relationship("ghost-group", None));
        }),
        rejected(&|entity| {
            let link = &mut group_mut(entity).relationships[0];
            link.connection_id = None;
            link.target = Some(pb::SystemGroupRef {
                id: Some(id("ghost-group")),
            });
        }),
        rejected(&|entity| {
            let link = &mut group_mut(entity).actors[0];
            link.connection_id = None;
            link.target = Some(pb::SystemActorTarget {
                target: Some(pb::system_actor_target::Target::Persona(pb::PersonaRef {
                    id: Some(id("ghost-persona")),
                })),
            });
        }),
        rejected(&|entity| {
            group_mut(entity).actors.push(actor(
                pb::system_actor_target::Target::Persona(pb::PersonaRef {
                    id: Some(id("ghost-persona")),
                }),
                pb::SystemActorDirection::Inbound,
            ));
        }),
    ];
    for attempt in attempts {
        let error = put_error(&service, attempt, &group_state.etag).await;
        assert_eq!(error.code(), Code::InvalidArgument, "{error}");
        assert!(
            error
                .message()
                .contains("ARCHITECTURE_REFERENCE_UNRESOLVED"),
            "{error}"
        );
        assert_eq!(get_group(&service, "platform").await, group_state);
    }
    let domain_attempts = [
        {
            let mut entity = domain_state.entity.clone().unwrap();
            domain_mut(&mut entity).members.push(domain_member(
                pb::domain_member::Target::Component(pb::ComponentRef {
                    id: Some(id("ghost")),
                }),
            ));
            entity
        },
        {
            let mut entity = domain_state.entity.clone().unwrap();
            let link = &mut domain_mut(&mut entity).members[0];
            link.connection_id = None;
            link.member = domain_member(pb::domain_member::Target::Component(pb::ComponentRef {
                id: Some(id("other-ghost")),
            }))
            .member;
            entity
        },
    ];
    for attempt in domain_attempts {
        let error = put_error(&service, attempt, &domain_state.etag).await;
        assert_eq!(error.code(), Code::InvalidArgument, "{error}");
        assert!(
            error
                .message()
                .contains("ARCHITECTURE_REFERENCE_UNRESOLVED"),
            "{error}"
        );
        assert_eq!(
            get(&service, pb::EntityKind::Domain, "retail", None).await,
            domain_state
        );
    }

    let mut in_place = group_state.entity.clone().unwrap();
    group_mut(&mut in_place).members[1].member = member_component("other-ghost").member;
    let error = put_error(&service, in_place, &group_state.etag).await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    assert!(
        error.message().contains("changed role or endpoint"),
        "{error}"
    );
    assert_eq!(get_group(&service, "platform").await, group_state);

    save(&service, component("worker-next"), None).await;
    let mut repaired = group_state.entity.clone().unwrap();
    let link = &mut group_mut(&mut repaired).members[1];
    link.connection_id = None;
    link.member = member_component("worker-next").member;
    let repaired = put_with(&service, repaired, &group_state.etag)
        .await
        .expect("retargeting a dangling link to an existing entity is the repair");
    assert_ne!(
        member_ids(repaired.entity.as_ref().unwrap())[1],
        member_ids(group_state.entity.as_ref().unwrap())[1]
    );
}

const RESTORE_BRANCH: &str = "restore-work";

async fn open_branch(service: &EventModelServiceImpl) {
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: RESTORE_BRANCH.into(),
            ..Default::default()
        }))
        .await
        .unwrap();
}

fn member_selector(owner: &pb::Entity, index: usize) -> pb::ConnectionRef {
    let pb::entity::Kind::SystemGroup(value) = owner.kind.as_ref().unwrap() else {
        panic!("system group");
    };
    pb::ConnectionRef {
        owner: Some(trogon_atlas_server::conv::entity_to_ref(owner).unwrap()),
        owner_uid: owner.system.as_ref().unwrap().uid.clone(),
        id: value.members[index].connection_id.clone(),
    }
}

fn resolves(owner: &pb::Entity, selector: &pb::ConnectionRef) -> bool {
    trogon_atlas_core::connections::resolve_connection(owner, selector).is_ok()
}

fn uid(owner: &pb::Entity) -> &str {
    &owner.system.as_ref().unwrap().uid
}

async fn merge(
    service: &EventModelServiceImpl,
    dry_run: bool,
    auto_merge: bool,
) -> pb::MergeBranchResponse {
    service
        .merge_branch(Request::new(pb::MergeBranchRequest {
            name: RESTORE_BRANCH.into(),
            dry_run,
            auto_merge,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
}

async fn diff(service: &EventModelServiceImpl) -> Vec<pb::BranchDiffEntry> {
    service
        .diff_branch(Request::new(pb::DiffBranchRequest {
            name: RESTORE_BRANCH.into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .entries
}

async fn resolve_entry(
    service: &EventModelServiceImpl,
    owner: &pb::ConnectionRef,
    resolution: pb::resolve_branch_entry_request::Resolution,
) {
    service
        .resolve_branch_entry(Request::new(pb::ResolveBranchEntryRequest {
            name: RESTORE_BRANCH.into(),
            r#ref: owner.owner.clone(),
            resolution: resolution as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
}

async fn group_on_branch(service: &EventModelServiceImpl, slug: &str) -> pb::GetEntityResponse {
    get(
        service,
        pb::EntityKind::SystemGroup,
        slug,
        Some(RESTORE_BRANCH),
    )
    .await
}

fn relationship_ids(owner: &pb::Entity) -> Vec<Option<pb::ConnectionId>> {
    let pb::entity::Kind::SystemGroup(value) = owner.kind.as_ref().unwrap() else {
        panic!("system group");
    };
    value
        .relationships
        .iter()
        .map(|link| link.connection_id.clone())
        .chain(value.actors.iter().map(|link| link.connection_id.clone()))
        .collect()
}

#[tokio::test]
async fn branch_baseline_removal_and_branch_addition_land_without_touching_unrelated_identity() {
    let service = fixture().await;
    let platform = get_group(&service, "platform").await.entity.unwrap();
    let edge = get_group(&service, "edge").await.entity.unwrap();
    open_branch(&service).await;

    let mut added = edge.clone();
    group_mut(&mut added)
        .members
        .push(member_component("scheduler"));
    let branch_edge = save(&service, added, Some(RESTORE_BRANCH))
        .await
        .entity
        .unwrap();
    let assigned = member_selector(&branch_edge, 0);
    assert!(assigned.id.is_some());

    let mut removed = platform.clone();
    group_mut(&mut removed).members.remove(0);
    let baseline_platform = save(&service, removed, None).await.entity.unwrap();
    let removed_selector = member_selector(&platform, 0);
    assert!(!resolves(&baseline_platform, &removed_selector));
    assert!(resolves(&baseline_platform, &member_selector(&platform, 1)));

    for dry_run in [true, false] {
        let response = merge(&service, dry_run, false).await;
        assert_eq!(
            response.status,
            if dry_run {
                pb::merge_branch_response::Status::Validated
            } else {
                pb::merge_branch_response::Status::Applied
            } as i32,
            "{response:?}"
        );
        let stored = get_group(&service, "edge").await.entity.unwrap();
        if dry_run {
            assert_eq!(stored, edge);
        } else {
            assert_eq!(stored, branch_edge);
            assert!(resolves(&stored, &assigned));
            assert_eq!(uid(&stored), uid(&edge));
        }
    }
    let landed = get_group(&service, "platform").await.entity.unwrap();
    assert_eq!(landed, baseline_platform);
    assert_eq!(uid(&landed), uid(&platform));
    assert!(!resolves(&landed, &removed_selector));
}

#[tokio::test]
async fn ordinary_landing_refuses_to_revive_removed_ids() {
    let service = fixture().await;
    let edge = get_group(&service, "edge").await.entity.unwrap();
    open_branch(&service).await;
    let mut proposed = edge.clone();
    group_mut(&mut proposed)
        .members
        .push(member_component("scheduler"));
    let mut added = save(&service, proposed, Some(RESTORE_BRANCH))
        .await
        .entity
        .unwrap();
    let revived = group_mut(&mut added).members.pop().unwrap();
    assert!(revived.connection_id.is_some());
    save(&service, added, Some(RESTORE_BRANCH)).await;
    assert_eq!(get_group(&service, "edge").await.entity, Some(edge));

    let response = merge(&service, false, false).await;
    assert_eq!(
        response.status,
        pb::merge_branch_response::Status::Applied as i32,
        "{response:?}"
    );
    let landed = get_group(&service, "edge").await;
    let mut attempt = landed.entity.clone().unwrap();
    group_mut(&mut attempt).members.push(revived);
    let error = put_error(&service, attempt, &landed.etag).await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    assert_eq!(get_group(&service, "edge").await, landed);
}

#[tokio::test]
async fn explicit_keep_or_take_renews_only_the_restored_member_id() {
    for take_theirs in [false, true] {
        let service = fixture().await;
        let original = get_group(&service, "platform").await.entity.unwrap();
        let selected = member_selector(&original, 0);
        let unaffected = member_selector(&original, 2);
        open_branch(&service).await;

        let mut branch = original.clone();
        group_mut(&mut branch).members[1].doc = Some(pb::CatalogText {
            value: "branch wording".into(),
        });
        if take_theirs {
            group_mut(&mut branch).members.remove(0);
        }
        save(&service, branch, Some(RESTORE_BRANCH)).await;
        let mut baseline = original.clone();
        group_mut(&mut baseline).members[1].doc = Some(pb::CatalogText {
            value: "baseline wording".into(),
        });
        if !take_theirs {
            group_mut(&mut baseline).members.remove(0);
        }
        save(&service, baseline, None).await;
        let before = get_group(&service, "platform").await;

        let blocked = merge(&service, false, false).await;
        assert_eq!(
            blocked.status,
            pb::merge_branch_response::Status::Conflicts as i32
        );
        assert_eq!(get_group(&service, "platform").await, before);

        resolve_entry(
            &service,
            &selected,
            if take_theirs {
                pb::resolve_branch_entry_request::Resolution::TakeTheirs
            } else {
                pb::resolve_branch_entry_request::Resolution::KeepOurs
            },
        )
        .await;
        let resolved = group_on_branch(&service, "platform").await.entity.unwrap();
        assert_eq!(uid(&resolved), uid(&original));
        let ids = member_ids(&resolved);
        let original_ids = member_ids(&original);
        assert_eq!(ids.len(), original_ids.len());
        assert_ne!(ids[0], original_ids[0]);
        assert_eq!(ids[1..], original_ids[1..]);
        assert_eq!(relationship_ids(&resolved), relationship_ids(&original));
        assert!(!resolves(&resolved, &selected));
        assert!(resolves(&resolved, &unaffected));

        for dry_run in [true, false] {
            let landed = merge(&service, dry_run, false).await;
            assert_eq!(
                landed.status,
                if dry_run {
                    pb::merge_branch_response::Status::Validated
                } else {
                    pb::merge_branch_response::Status::Applied
                } as i32,
                "{landed:?}"
            );
        }
        let stored = get_group(&service, "platform").await.entity.unwrap();
        assert_eq!(stored, resolved);
        assert!(!resolves(&stored, &selected));
        assert!(resolves(&stored, &unaffected));
    }
}

#[tokio::test]
async fn keeping_the_branch_owner_after_baseline_deletion_restores_its_member_identity() {
    let service = fixture().await;
    let original = get_group(&service, "platform").await.entity.unwrap();
    let selectors: Vec<_> = (0..4)
        .map(|index| member_selector(&original, index))
        .collect();
    open_branch(&service).await;
    let mut edit = original.clone();
    group_mut(&mut edit).title = "Platform on branch".into();
    let branch = save(&service, edit, Some(RESTORE_BRANCH))
        .await
        .entity
        .unwrap();
    force_delete(&service, pb::EntityKind::SystemGroup, "platform").await;

    let entries = diff(&service).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].status,
        pb::branch_diff_entry::Status::ConflictDeleteEdit as i32
    );
    let blocked = merge(&service, false, false).await;
    assert_eq!(
        blocked.status,
        pb::merge_branch_response::Status::Conflicts as i32
    );
    assert!(service
        .get_entity(request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::SystemGroup as i32,
                id: Some(id("platform")),
            },
            None,
        ))
        .await
        .is_err());

    resolve_entry(
        &service,
        &selectors[0],
        pb::resolve_branch_entry_request::Resolution::KeepOurs,
    )
    .await;
    let resolved = group_on_branch(&service, "platform").await.entity.unwrap();
    assert_eq!(uid(&resolved), uid(&branch));
    assert_eq!(member_ids(&resolved), member_ids(&branch));
    for selector in &selectors {
        assert!(resolves(&resolved, selector));
    }
    for dry_run in [true, false] {
        let landed = merge(&service, dry_run, false).await;
        assert_eq!(
            landed.status,
            if dry_run {
                pb::merge_branch_response::Status::Validated
            } else {
                pb::merge_branch_response::Status::Applied
            } as i32,
            "{landed:?}"
        );
    }
    let stored = get_group(&service, "platform").await.entity.unwrap();
    assert_eq!(uid(&stored), uid(&branch));
    assert_eq!(member_ids(&stored), member_ids(&branch));
    for selector in &selectors {
        assert!(resolves(&stored, selector));
    }
}

#[tokio::test]
async fn independent_same_key_creation_keeps_distinct_uids_until_an_explicit_choice() {
    let service = service().await;
    save(&service, component("worker"), None).await;
    open_branch(&service).await;
    let twin = group("twin", vec![member_component("worker")], vec![], vec![]);
    let branch = save(&service, twin.clone(), Some(RESTORE_BRANCH))
        .await
        .entity
        .unwrap();
    let baseline = save(&service, twin, None).await.entity.unwrap();
    assert_ne!(uid(&branch), uid(&baseline));
    assert_ne!(member_ids(&branch), member_ids(&baseline));
    let branch_selector = member_selector(&branch, 0);
    let baseline_selector = member_selector(&baseline, 0);
    assert!(!resolves(&baseline, &branch_selector));
    assert!(!resolves(&branch, &baseline_selector));

    let entries = diff(&service).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].status,
        pb::branch_diff_entry::Status::ConflictEditEdit as i32
    );
    assert!(entries[0].auto_merged.is_none());
    assert!(entries[0]
        .conflict_field_paths
        .contains(&"system.uid".into()));
    let blocked = merge(&service, false, true).await;
    assert_eq!(
        blocked.status,
        pb::merge_branch_response::Status::Conflicts as i32
    );
    assert_eq!(
        get_group(&service, "twin").await.entity.as_ref(),
        Some(&baseline)
    );

    resolve_entry(
        &service,
        &branch_selector,
        pb::resolve_branch_entry_request::Resolution::KeepOurs,
    )
    .await;
    let landed = merge(&service, false, false).await;
    assert_eq!(
        landed.status,
        pb::merge_branch_response::Status::Applied as i32,
        "{landed:?}"
    );
    let stored = get_group(&service, "twin").await.entity.unwrap();
    assert_eq!(uid(&stored), uid(&branch));
    assert!(resolves(&stored, &branch_selector));
    assert!(!resolves(&stored, &baseline_selector));
}

fn person_with(slug: &str, responsibilities: Vec<pb::Responsibility>) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Person(pb::Person {
            id: Some(id(slug)),
            title: Some(pb::DirectoryDisplayName { value: slug.into() }),
            responsibilities,
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn responsibility_for(
    kind: pb::ResponsibilityKind,
    subject: &pb::ResponsibilitySubject,
) -> pb::Responsibility {
    pb::Responsibility {
        kind: kind as i32,
        subject: Some(subject.clone()),
        ..Default::default()
    }
}

fn connection_subject(selector: pb::ConnectionRef) -> pb::ResponsibilitySubject {
    pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Connection(selector)),
    }
}

async fn assignee_responsibilities(
    service: &EventModelServiceImpl,
    slug: &str,
) -> Result<pb::ListAssigneeResponsibilitiesResponse, tonic::Status> {
    service
        .list_assignee_responsibilities(Request::new(pb::ListAssigneeResponsibilitiesRequest {
            assignee: Some(pb::ResponsibilityAssignee {
                selection: Some(pb::responsibility_assignee::Selection::Person(
                    pb::PersonRef { id: Some(id(slug)) },
                )),
            }),
            ..Default::default()
        }))
        .await
        .map(tonic::Response::into_inner)
}

async fn subject_assignments(
    service: &EventModelServiceImpl,
    subject: &pb::ResponsibilitySubject,
    kind: Option<pb::ResponsibilityKind>,
) -> Result<pb::ListSubjectAssignmentsResponse, tonic::Status> {
    service
        .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
            subject: Some(subject.clone()),
            kind: kind.map(|kind| kind as i32),
        }))
        .await
        .map(tonic::Response::into_inner)
}

async fn seeded_service() -> (EventModelServiceImpl, Arc<trogon_atlas_store::NatsStore>) {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default())
        .unwrap()
        .with_snapshot_cap(0);
    (service, store)
}

async fn seed_people(
    store: &trogon_atlas_store::NatsStore,
    people: impl Iterator<Item = pb::Entity>,
) {
    use trogon_atlas_store::{store::MutationOp, Store as _, WriteContext};
    let people: Vec<_> = people.collect();
    for chunk in people.chunks(128) {
        let ops: Vec<_> = chunk
            .iter()
            .cloned()
            .map(|entity| MutationOp::Create {
                kind: pb::EntityKind::Person,
                entity,
            })
            .collect();
        store
            .batch_apply(&ops, WriteContext::baseline())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn shared_occurrence_budget_counts_each_group_once_across_repeats_and_cycles() {
    let service = service().await;
    save(&service, component("worker"), None).await;
    for index in 0..10 {
        save(
            &service,
            group(&format!("ring-{index}"), vec![], vec![], vec![]),
            None,
        )
        .await;
    }
    let fill = |index: usize, count: usize| {
        let mut members = vec![member_group(&format!("ring-{}", (index + 1) % 10))];
        members.extend(std::iter::repeat_n(member_component("worker"), count - 1));
        members
    };
    let mut selectors = Vec::new();
    for index in 0..10 {
        let slug = format!("ring-{index}");
        let mut entity = get_group(&service, &slug).await.entity.unwrap();
        group_mut(&mut entity).members = fill(index, if index == 9 { 774 } else { 1_024 });
        save(&service, entity, None).await;
        selectors.push(member_selector(
            &get_group(&service, &slug).await.entity.unwrap(),
            0,
        ));
    }
    save(
        &service,
        person_with(
            "maintainer",
            selectors
                .iter()
                .map(|selector| {
                    responsibility_for(
                        pb::ResponsibilityKind::Owner,
                        &connection_subject(selector.clone()),
                    )
                })
                .collect(),
        ),
        None,
    )
    .await;

    let boundary = assignee_responsibilities(&service, "maintainer")
        .await
        .unwrap();
    assert_eq!(boundary.assignments.len(), 10);
    assert!(!boundary.incomplete);
    assert!(boundary
        .assignments
        .iter()
        .all(|row| row.state == pb::ResponsibilityResolutionState::Resolved as i32));

    let mut last = get_group(&service, "ring-9").await.entity.unwrap();
    let stored_etag = get_group(&service, "ring-9").await.etag;
    group_mut(&mut last)
        .members
        .push(member_component("worker"));
    save(&service, last, None).await;
    let over = assignee_responsibilities(&service, "maintainer")
        .await
        .unwrap_err();
    assert_eq!(over.code(), Code::ResourceExhausted, "{over}");
    assert!(over.message().contains("occurrence"), "{over}");
    assert_ne!(get_group(&service, "ring-9").await.etag, stored_etag);

    let mut trimmed = get_group(&service, "ring-9").await.entity.unwrap();
    group_mut(&mut trimmed).members.pop();
    save(&service, trimmed, None).await;
    assert_eq!(
        assignee_responsibilities(&service, "maintainer")
            .await
            .unwrap()
            .assignments
            .len(),
        10
    );
}

#[tokio::test]
async fn shared_output_budget_is_exact_and_filtered_rows_do_not_spend_it() {
    let (service, store) = seeded_service().await;
    save(&service, component("worker"), None).await;
    save(
        &service,
        group("edge", vec![member_component("worker")], vec![], vec![]),
        None,
    )
    .await;
    let subject = connection_subject(member_selector(
        &get_group(&service, "edge").await.entity.unwrap(),
        0,
    ));
    let every_kind = [
        pb::ResponsibilityKind::Owner,
        pb::ResponsibilityKind::Maintainer,
        pb::ResponsibilityKind::Steward,
        pb::ResponsibilityKind::Contact,
    ];
    seed_people(
        &store,
        (0..1_024).map(|index| {
            person_with(
                &format!("holder-{index}"),
                every_kind
                    .iter()
                    .map(|kind| responsibility_for(*kind, &subject))
                    .collect(),
            )
        }),
    )
    .await;

    let boundary = subject_assignments(&service, &subject, None).await.unwrap();
    assert_eq!(boundary.assignments.len(), 4_096);
    assert!(!boundary.incomplete);

    seed_people(
        &store,
        std::iter::once(person_with(
            "holder-extra",
            vec![responsibility_for(pb::ResponsibilityKind::Owner, &subject)],
        )),
    )
    .await;
    let over = subject_assignments(&service, &subject, None)
        .await
        .unwrap_err();
    assert_eq!(over.code(), Code::ResourceExhausted, "{over}");
    assert!(over.message().contains("output"), "{over}");

    let filtered = subject_assignments(&service, &subject, Some(pb::ResponsibilityKind::Owner))
        .await
        .unwrap();
    assert_eq!(filtered.assignments.len(), 1_025);
}

#[tokio::test]
async fn shared_row_budget_is_exact_and_non_resource_entities_do_not_spend_it() {
    let (service, store) = seeded_service().await;
    save(&service, component("worker"), None).await;
    save(
        &service,
        group("edge", vec![member_component("worker")], vec![], vec![]),
        None,
    )
    .await;
    let subject = connection_subject(member_selector(
        &get_group(&service, "edge").await.entity.unwrap(),
        0,
    ));
    seed_people(
        &store,
        (0..4_998).map(|index| {
            person_with(
                &format!("holder-{index}"),
                if index == 0 {
                    vec![responsibility_for(pb::ResponsibilityKind::Owner, &subject)]
                } else {
                    vec![]
                },
            )
        }),
    )
    .await;

    let boundary = subject_assignments(&service, &subject, None).await.unwrap();
    assert_eq!(boundary.assignments.len(), 1);
    assert!(!boundary.incomplete);

    save(&service, persona("shopper"), None).await;
    assert_eq!(
        subject_assignments(&service, &subject, None)
            .await
            .unwrap()
            .assignments
            .len(),
        1
    );

    seed_people(&store, std::iter::once(person_with("holder-extra", vec![]))).await;
    let over = subject_assignments(&service, &subject, None)
        .await
        .unwrap_err();
    assert_eq!(over.code(), Code::ResourceExhausted, "{over}");
}
