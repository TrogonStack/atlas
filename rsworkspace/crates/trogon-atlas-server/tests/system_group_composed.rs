#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{collections::HashSet, sync::Arc};

use support::two_tenants::{authed, make_registry, serve_with_registry, ACME_TOKEN, BETA_TOKEN};
use tonic::{transport::Channel, Code};
use trogon_atlas_proto::{self as pb, event_model_service_client::EventModelServiceClient};
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};
use trogon_atlas_store::Store;

type Client = EventModelServiceClient<Channel>;
fn entity(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}
fn id(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}
fn reference(value: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(value).unwrap()
}
async fn put(client: &mut Client, value: pb::Entity) -> pb::Entity {
    client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(value),
                ..Default::default()
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}
async fn register(client: &mut Client, token: &str, name: &str) -> String {
    client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: name.into(),
                ..Default::default()
            },
            token,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id
}
async fn scoped(client: &mut Client, domain: &pb::Id) -> HashSet<pb::EntityKey> {
    client
        .list_entities_by_domain(authed(
            pb::ListEntitiesByDomainRequest {
                domain_id: Some(domain.clone()),
                ..Default::default()
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .entities
        .iter()
        .map(|v| {
            let r = reference(v);
            pb::EntityKey::new(
                pb::EntityKind::try_from(r.kind).unwrap(),
                r.id.as_ref().unwrap(),
            )
        })
        .collect()
}
async fn incoming(client: &mut Client, target: &pb::EntityRef) -> Vec<pb::Reference> {
    client
        .get_incoming_references(authed(
            pb::GetReferencesRequest {
                kind: target.kind,
                id: target.id.clone(),
                ..Default::default()
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .references
}

#[tokio::test(flavor = "multi_thread")]
async fn authenticated_group_domain_and_topology_keep_exact_repeated_members_and_scope() {
    Box::pin(composed()).await;
}
async fn composed() {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
    let mut client = serve_with_registry(service, make_registry()).await;
    let ns = register(&mut client, ACME_TOKEN, "integrated-groups").await;
    let hidden_ns = register(&mut client, BETA_TOKEN, "hidden-groups").await;
    let schema_id = id(&ns, "model", u64::MAX);
    let graph_id = id(&ns, "topology", 9_007_199_254_740_993);
    let group_id = id(&ns, "system", 2);
    let domain_id = id(&ns, "domain", 10);
    let schema = put(
        &mut client,
        entity(pb::entity::Kind::Schema(pb::Schema {
            id: Some(schema_id.clone()),
            title: "Model".into(),
            ..Default::default()
        })),
    )
    .await;
    let mut group = put(
        &mut client,
        entity(pb::entity::Kind::SystemGroup(pb::SystemGroup {
            id: Some(group_id.clone()),
            title: "System".into(),
            scope: pb::SystemGroupScope::External as i32,
            ..Default::default()
        })),
    )
    .await;
    let graph = put(
        &mut client,
        entity(pb::entity::Kind::DesignGraph(pb::DesignGraph {
            id: Some(graph_id.clone()),
            title: Some(pb::CatalogTitle {
                value: "Topology".into(),
            }),
            profile: pb::DesignGraphProfile::Diagram as i32,
            steps: vec![pb::TopologyStep {
                title: Some(pb::CatalogTitle {
                    value: "Group step".into(),
                }),
                target: Some(pb::TopologyStepTarget {
                    selection: Some(pb::topology_step_target::Selection::SystemGroup(
                        pb::SystemGroupRef {
                            id: Some(group_id.clone()),
                        },
                    )),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
    )
    .await;
    let mut domain = put(
        &mut client,
        entity(pb::entity::Kind::Domain(pb::Domain {
            id: Some(domain_id.clone()),
            title: "Domain".into(),
            ..Default::default()
        })),
    )
    .await;
    let scope_before = scoped(&mut client, &domain_id).await;
    let pb::entity::Kind::SystemGroup(value) = group.kind.as_mut().unwrap() else {
        panic!("group")
    };
    for _ in 0..2 {
        value.members.push(pb::SystemGroupMembership {
            member: Some(pb::SystemGroupMember {
                target: Some(pb::system_group_member::Target::Schema(pb::SchemaRef {
                    id: Some(schema_id.clone()),
                })),
            }),
            ..Default::default()
        });
        value.members.push(pb::SystemGroupMembership {
            member: Some(pb::SystemGroupMember {
                target: Some(pb::system_group_member::Target::DesignGraph(
                    pb::DesignGraphRef {
                        id: Some(graph_id.clone()),
                    },
                )),
            }),
            ..Default::default()
        });
    }
    group = put(&mut client, group).await;
    let pb::entity::Kind::Domain(value) = domain.kind.as_mut().unwrap() else {
        panic!("domain")
    };
    for _ in 0..2 {
        value.members.push(pb::DomainMembership {
            member: Some(pb::DomainMember {
                target: Some(pb::domain_member::Target::Schema(pb::SchemaRef {
                    id: Some(schema_id.clone()),
                })),
            }),
            ..Default::default()
        });
        value.members.push(pb::DomainMembership {
            member: Some(pb::DomainMember {
                target: Some(pb::domain_member::Target::DesignGraph(pb::DesignGraphRef {
                    id: Some(graph_id.clone()),
                })),
            }),
            ..Default::default()
        });
    }
    domain = put(&mut client, domain).await;
    assert_eq!(
        scoped(&mut client, &domain_id).await,
        scope_before,
        "curation never expands namespace-derived domain scope"
    );
    let detail = client
        .get_design_graph_detail(authed(
            pb::GetDesignGraphDetailRequest {
                graph: Some(pb::DesignGraphRef {
                    id: Some(graph_id.clone()),
                }),
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!detail.incomplete);
    assert_eq!(detail.targets.len(), 1);
    assert_eq!(
        detail.targets[0].state,
        pb::CatalogResolutionState::Resolved as i32
    );
    assert_eq!(
        detail.targets[0].owner.as_ref().unwrap().owner,
        Some(reference(&group))
    );
    let pb::entity::Kind::DesignGraph(native_graph) = graph.kind.as_ref().unwrap() else {
        panic!("graph")
    };
    let step = native_graph.steps[0].step_id.clone();
    let graph_refs = incoming(&mut client, &reference(&group)).await;
    let hit = graph_refs
        .iter()
        .find(|v| v.from == Some(reference(&graph)))
        .unwrap();
    assert_eq!(hit.step_id, step);
    assert_eq!(hit.from_field, "steps[0].target.system_group");
    assert_eq!(hit.from_uid, graph.system.as_ref().unwrap().uid);
    let mut selectors = Vec::new();
    for owner in [&group, &domain] {
        let occurrences = trogon_atlas_core::connections::connection_occurrences(owner).unwrap();
        assert_eq!(occurrences.len(), 4);
        assert_eq!(
            occurrences
                .iter()
                .map(|v| v.id)
                .collect::<HashSet<_>>()
                .len(),
            4
        );
        for occurrence in occurrences {
            let selected = pb::ConnectionRef {
                owner: Some(reference(owner)),
                owner_uid: owner.system.as_ref().unwrap().uid.clone(),
                id: Some(occurrence.id.unwrap().into()),
            };
            let target = occurrence.target.unwrap();
            assert!(target == reference(&schema) || target == reference(&graph));
            let refs = incoming(&mut client, &target).await;
            let hit = refs
                .iter()
                .find(|v| v.from == Some(reference(owner)) && v.connection_id == selected.id)
                .unwrap();
            assert_eq!(hit.from_uid, selected.owner_uid);
            assert_eq!(hit.to, Some(target));
            selectors.push(selected);
        }
    }
    assert_eq!(
        selectors
            .iter()
            .map(|v| v.id.clone())
            .collect::<HashSet<_>>()
            .len(),
        8
    );
    let selected = selectors[1].clone();
    let document = put(
        &mut client,
        entity(pb::entity::Kind::Document(pb::Document {
            id: Some(id(&ns, "rationale", 1)),
            title: Some(pb::CatalogTitle {
                value: "Rationale".into(),
            }),
            subjects: vec![pb::SubjectAssociation {
                subject: Some(pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Connection(selected.clone())),
                }),
                ..Default::default()
            }],
            body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                value: "https://example.com/rationale".into(),
            })),
            ..Default::default()
        })),
    )
    .await;
    for (index, selector) in selectors.iter().enumerate() {
        let found = client
            .list_subject_documents(authed(
                pb::ListSubjectDocumentsRequest {
                    subject: Some(pb::CatalogSubject {
                        selection: Some(pb::catalog_subject::Selection::Connection(
                            selector.clone(),
                        )),
                    }),
                },
                ACME_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        assert!(!found.incomplete);
        assert_eq!(found.declarations.len(), usize::from(index == 1));
        if index == 1 {
            assert_eq!(
                found.declarations[0].owner.as_ref().unwrap().owner,
                Some(reference(&document))
            );
        }
    }
    for target in [reference(&group), reference(&graph), reference(&schema)] {
        let refused = client
            .delete_entity(authed(
                pb::DeleteEntityRequest {
                    kind: target.kind,
                    id: target.id,
                    mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                    ..Default::default()
                },
                ACME_TOKEN,
            ))
            .await
            .unwrap_err();
        assert_eq!(refused.code(), Code::FailedPrecondition);
    }
    let mut hidden_target = graph.clone();
    let pb::entity::Kind::DesignGraph(v) = hidden_target.kind.as_mut().unwrap() else {
        panic!("graph")
    };
    v.steps[0].target = Some(pb::TopologyStepTarget {
        selection: Some(pb::topology_step_target::Selection::SystemGroup(
            pb::SystemGroupRef {
                id: Some(id(&hidden_ns, "secret-group", 1)),
            },
        )),
    });
    let before_hidden = store
        .get(pb::EntityKind::DesignGraph, &graph_id, None)
        .await
        .unwrap();
    let refused = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(hidden_target),
                ..Default::default()
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        refused.code(),
        Code::InvalidArgument | Code::NotFound
    ));
    let after_hidden = store
        .get(pb::EntityKind::DesignGraph, &graph_id, None)
        .await
        .unwrap();
    assert_eq!(before_hidden.entity, after_hidden.entity);
    assert_eq!(before_hidden.etag, after_hidden.etag);
    let owners = [schema, graph, group, domain, document];
    let pool = trogon_atlas_core::transcode::TranscodePool::builtin().unwrap();
    let exported = trogon_atlas_client::export::render_board(&owners, &[], &pool).unwrap();
    assert_eq!(
        exported.skipped,
        [] as [trogon_atlas_proto::SkippedEntity; 0]
    );
    let parsed =
        trogon_atlas_client::manifest::parse_str(&exported.manifests, "composed.yaml").unwrap();
    for owner in &owners {
        let r = reference(owner);
        let before = store
            .get(
                pb::EntityKind::try_from(r.kind).unwrap(),
                r.id.as_ref().unwrap(),
                None,
            )
            .await
            .unwrap();
        let manifest = parsed.iter().find(|v| reference(&v.entity) == r).unwrap();
        assert_eq!(
            trogon_atlas_client::apply::classify(&manifest.entity, Some(owner)),
            trogon_atlas_client::apply::Action::Unchanged
        );
        let reapplied = put(&mut client, manifest.entity.clone()).await;
        assert_eq!(reapplied, *owner);
        let after = store
            .get(
                pb::EntityKind::try_from(r.kind).unwrap(),
                r.id.as_ref().unwrap(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(before.etag, after.etag);
    }
    let mut forbidden = owners[2].clone();
    let pb::entity::Kind::SystemGroup(v) = forbidden.kind.as_mut().unwrap() else {
        panic!("group")
    };
    v.members.push(pb::SystemGroupMembership {
        member: Some(pb::SystemGroupMember {
            target: Some(pb::system_group_member::Target::DesignGraph(
                pb::DesignGraphRef {
                    id: Some(id(&hidden_ns, "secret", 1)),
                },
            )),
        }),
        ..Default::default()
    });
    let before = store
        .get(pb::EntityKind::SystemGroup, &group_id, None)
        .await
        .unwrap();
    let refused = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(forbidden),
                ..Default::default()
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        refused.code(),
        Code::InvalidArgument | Code::NotFound
    ));
    let after = store
        .get(pb::EntityKind::SystemGroup, &group_id, None)
        .await
        .unwrap();
    assert_eq!(after.entity, before.entity);
    assert_eq!(after.etag, before.etag);
}
