#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use support::two_tenants::{
    authed, make_registry, serve_with_registry, ACME_TOKEN, BETA_TOKEN, ROOT_TOKEN,
};
use tonic::{transport::Channel, Code};
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

async fn register(client: &mut Client, token: &str, name: &str) -> String {
    client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: name.into(),
                parent: String::new(),
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

async fn put(
    client: &mut Client,
    token: &str,
    entity: pb::Entity,
    etag: &str,
) -> Result<pb::PutEntityResponse, tonic::Status> {
    client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(entity),
                if_match: etag.into(),
                ..Default::default()
            },
            token,
        ))
        .await
        .map(tonic::Response::into_inner)
}

fn component(namespace: &str, slug: &str) -> pb::Entity {
    entity(pb::entity::Kind::Component(pb::Component {
        id: Some(id(namespace, slug)),
        title: slug.into(),
        ..Default::default()
    }))
}

fn component_member(target: &pb::Entity) -> pb::SystemGroupMembership {
    pb::SystemGroupMembership {
        member: Some(pb::SystemGroupMember {
            target: Some(pb::system_group_member::Target::Component(
                pb::ComponentRef {
                    id: reference(target).id,
                },
            )),
        }),
        ..Default::default()
    }
}

fn group(namespace: &str, slug: &str, members: Vec<pb::SystemGroupMembership>) -> pb::Entity {
    entity(pb::entity::Kind::SystemGroup(pb::SystemGroup {
        id: Some(id(namespace, slug)),
        title: slug.into(),
        scope: pb::SystemGroupScope::Internal as i32,
        members,
        ..Default::default()
    }))
}

fn members(value: &pb::Entity) -> &[pb::SystemGroupMembership] {
    let Some(pb::entity::Kind::SystemGroup(group)) = &value.kind else {
        panic!("expected system group");
    };
    &group.members
}

async fn serve() -> Client {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    serve_with_registry(service, make_registry()).await
}

async fn subject_counts(client: &mut Client, selected: &pb::ConnectionRef) -> (usize, usize) {
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
    (documents.declarations.len(), assignments.assignments.len())
}

#[tokio::test(flavor = "multi_thread")]
async fn parallel_members_keep_provenance_and_exact_subjects_after_reorder() {
    let mut client = serve().await;
    let namespace = register(&mut client, ROOT_TOKEN, "group-acceptance").await;
    let worker = put(&mut client, ROOT_TOKEN, component(&namespace, "worker"), "")
        .await
        .unwrap()
        .entity
        .unwrap();
    let created = put(
        &mut client,
        ROOT_TOKEN,
        group(
            &namespace,
            "platform",
            vec![component_member(&worker), component_member(&worker)],
        ),
        "",
    )
    .await
    .unwrap();
    let mut owner = created.entity.clone().unwrap();
    let ids: Vec<_> = members(&owner)
        .iter()
        .map(|value| value.connection_id.clone().unwrap())
        .collect();
    assert_ne!(ids[0], ids[1]);
    let uid = owner.system.as_ref().unwrap().uid.clone();
    let selectors: Vec<_> = ids
        .iter()
        .map(|id| pb::ConnectionRef {
            owner: Some(reference(&owner)),
            owner_uid: uid.clone(),
            id: Some(id.clone()),
        })
        .collect();

    let incoming = client
        .get_incoming_references(authed(
            pb::GetReferencesRequest {
                kind: reference(&worker).kind,
                id: reference(&worker).id,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    for (index, connection_id) in ids.iter().enumerate() {
        let hit = incoming
            .references
            .iter()
            .find(|hit| hit.connection_id.as_ref() == Some(connection_id))
            .unwrap();
        assert_eq!(hit.from, Some(reference(&owner)));
        assert_eq!(hit.from_uid, uid);
        assert_eq!(hit.from_field, format!("members[{index}].component"));
    }

    let note = "Curated for the platform view.";
    put(
        &mut client,
        ROOT_TOKEN,
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
    .await
    .unwrap();
    put(
        &mut client,
        ROOT_TOKEN,
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
    .await
    .unwrap();
    assert_eq!(subject_counts(&mut client, &selectors[0]).await, (1, 1));
    assert_eq!(subject_counts(&mut client, &selectors[1]).await, (0, 0));

    let Some(pb::entity::Kind::SystemGroup(authored)) = &mut owner.kind else {
        panic!("expected system group");
    };
    authored.members.reverse();
    authored.members[1].doc = Some(pb::CatalogText {
        value: "annotated".into(),
    });
    let edited = put(&mut client, ROOT_TOKEN, owner, &created.etag)
        .await
        .unwrap();
    let edited_owner = edited.entity.clone().unwrap();
    assert_eq!(
        members(&edited_owner)[0].connection_id.as_ref(),
        Some(&ids[1])
    );
    assert_eq!(
        members(&edited_owner)[1].connection_id.as_ref(),
        Some(&ids[0])
    );
    assert_eq!(edited_owner.system.as_ref().unwrap().uid, uid);
    assert_eq!(subject_counts(&mut client, &selectors[0]).await, (1, 1));
    assert_eq!(subject_counts(&mut client, &selectors[1]).await, (0, 0));
    let replay = put(&mut client, ROOT_TOKEN, edited_owner, &edited.etag)
        .await
        .unwrap();
    assert!(replay.no_op);
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_and_missing_targets_are_indistinguishable_and_leave_nothing_behind() {
    let mut client = serve().await;
    let acme = register(&mut client, ACME_TOKEN, "group-acme").await;
    let beta = register(&mut client, BETA_TOKEN, "group-beta").await;
    let hidden = put(&mut client, BETA_TOKEN, component(&beta, "private"), "")
        .await
        .unwrap()
        .entity
        .unwrap();
    let missing = component(&acme, "ghost");

    let hidden_error = put(
        &mut client,
        ACME_TOKEN,
        group(&acme, "view", vec![component_member(&hidden)]),
        "",
    )
    .await
    .unwrap_err();
    let missing_error = put(
        &mut client,
        ACME_TOKEN,
        group(&acme, "view", vec![component_member(&missing)]),
        "",
    )
    .await
    .unwrap_err();
    assert_eq!(hidden_error.code(), missing_error.code());
    for status in [&hidden_error, &missing_error] {
        assert!(!status.message().contains("private"));
        assert!(!status.message().contains(&beta));
    }
    let absent = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::SystemGroup as i32,
                id: Some(id(&acme, "view")),
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(absent.code(), Code::NotFound);

    let design_graph = pb::SystemGroupMembership {
        member: Some(pb::SystemGroupMember {
            target: Some(pb::system_group_member::Target::DesignGraph(
                pb::DesignGraphRef {
                    id: Some(id(&acme, "graph")),
                },
            )),
        }),
        ..Default::default()
    };
    let pending = put(
        &mut client,
        ACME_TOKEN,
        group(&acme, "view", vec![design_graph]),
        "",
    )
    .await
    .unwrap_err();
    assert_eq!(pending.code(), missing_error.code());
    assert!(pending
        .message()
        .contains("ARCHITECTURE_REFERENCE_UNRESOLVED"));
}

#[tokio::test(flavor = "multi_thread")]
async fn writers_without_access_cannot_author_or_see_groups_across_tenants() {
    let mut client = serve().await;
    let acme = register(&mut client, ACME_TOKEN, "group-owned").await;
    let _beta = register(&mut client, BETA_TOKEN, "group-other").await;
    let created = put(&mut client, ACME_TOKEN, group(&acme, "own", vec![]), "")
        .await
        .unwrap();
    assert!(created.entity.is_some());

    let foreign_write = put(
        &mut client,
        BETA_TOKEN,
        group(&acme, "intruder", vec![]),
        "",
    )
    .await
    .unwrap_err();
    assert!(matches!(
        foreign_write.code(),
        Code::PermissionDenied | Code::NotFound
    ));
    let foreign_read = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::SystemGroup as i32,
                id: Some(id(&acme, "own")),
            },
            BETA_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(foreign_read.code(), Code::NotFound);
    let listed = client
        .list_entities(authed(
            pb::ListEntitiesRequest {
                kinds: vec![pb::EntityKind::SystemGroup as i32],
                ..Default::default()
            },
            BETA_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.entities, [] as [trogon_atlas_proto::Entity; 0]);
}
