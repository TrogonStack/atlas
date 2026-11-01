#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use async_trait::async_trait;
use support::two_tenants::{
    authed, make_registry, serve_with_registry, ACME_TOKEN, BETA_TOKEN, ROOT_TOKEN,
};
use tonic::{transport::Channel, Code, Status};
use trogon_atlas_core::{NamespaceId, OwnerId};
use trogon_atlas_proto::{self as pb, event_model_service_client::EventModelServiceClient};
use trogon_atlas_server::{
    metrics::ServerMetrics,
    ownership::{Lens, NamespaceAuthorizer, RegistryAuthorizer, Visibility, WriteVerdict},
    service::EventModelServiceImpl,
};
use trogon_atlas_store::{NamespaceRecord, Store};

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

#[derive(Debug)]
struct FlakyAuthorizer {
    inner: RegistryAuthorizer,
    failing: std::sync::Mutex<Option<(NamespaceId, Code)>>,
}

impl FlakyAuthorizer {
    fn fail(&self, namespace: &str, code: Option<Code>) {
        *self.failing.lock().unwrap() =
            code.map(|code| (NamespaceId::parse(namespace).unwrap(), code));
    }
}

#[async_trait]
impl NamespaceAuthorizer for FlakyAuthorizer {
    async fn admits(&self, vis: &Visibility, namespace: &NamespaceId) -> Result<bool, Status> {
        if let Some((failing, code)) = self.failing.lock().unwrap().as_ref() {
            if failing == namespace {
                return Err(Status::new(*code, "injected authorization outage"));
            }
        }
        self.inner.admits(vis, namespace).await
    }

    async fn lens(&self, vis: &Visibility) -> Result<Lens, Status> {
        self.inner.lens(vis).await
    }

    async fn may_write(
        &self,
        vis: &Visibility,
        namespace: &NamespaceId,
    ) -> Result<WriteVerdict, Status> {
        self.inner.may_write(vis, namespace).await
    }

    async fn on_registered(&self, record: &NamespaceRecord) -> Result<(), Status> {
        self.inner.on_registered(record).await
    }

    async fn on_moved(&self, record: &NamespaceRecord, from: &OwnerId) -> Result<(), Status> {
        self.inner.on_moved(record, from).await
    }

    async fn on_released(&self, record: &NamespaceRecord) -> Result<(), Status> {
        self.inner.on_released(record).await
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn authorization_outages_propagate_instead_of_reading_as_unresolved_targets() {
    let store: Arc<dyn Store> = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
    let authorizer = Arc::new(FlakyAuthorizer {
        inner: RegistryAuthorizer::new(service.directory(), store),
        failing: std::sync::Mutex::new(None),
    });
    let mut client =
        serve_with_registry(service.with_authorizer(authorizer.clone()), make_registry()).await;
    let home = register(&mut client, ACME_TOKEN, "group-home").await;
    let remote = register(&mut client, ACME_TOKEN, "group-remote").await;
    let beta = register(&mut client, BETA_TOKEN, "group-beta-owned").await;
    let target = put(&mut client, ACME_TOKEN, component(&remote, "worker"), "")
        .await
        .unwrap()
        .entity
        .unwrap();
    let hidden = put(&mut client, BETA_TOKEN, component(&beta, "private"), "")
        .await
        .unwrap()
        .entity
        .unwrap();

    for code in [Code::Unavailable, Code::DeadlineExceeded] {
        authorizer.fail(&remote, Some(code));
        let error = put(
            &mut client,
            ACME_TOKEN,
            group(&home, "view", vec![component_member(&target)]),
            "",
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), code, "{error}");
        assert!(!error
            .message()
            .contains("ARCHITECTURE_REFERENCE_UNRESOLVED"));
        let absent = client
            .get_entity(authed(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::SystemGroup as i32,
                    id: Some(id(&home, "view")),
                },
                ACME_TOKEN,
            ))
            .await
            .unwrap_err();
        assert_eq!(absent.code(), Code::NotFound);
    }

    authorizer.fail(&remote, None);
    let created = put(
        &mut client,
        ACME_TOKEN,
        group(&home, "view", vec![component_member(&target)]),
        "",
    )
    .await
    .unwrap();

    authorizer.fail(&remote, Some(Code::Unavailable));
    let mut edited = created.entity.clone().unwrap();
    let Some(pb::entity::Kind::SystemGroup(authored)) = &mut edited.kind else {
        panic!("expected system group");
    };
    authored.title = "renamed".into();
    let error = put(&mut client, ACME_TOKEN, edited, &created.etag)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Unavailable, "{error}");
    authorizer.fail(&remote, None);
    let stored = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::SystemGroup as i32,
                id: Some(id(&home, "view")),
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(stored.entity, created.entity);
    assert_eq!(stored.etag, created.etag);

    let hidden_error = put(
        &mut client,
        ACME_TOKEN,
        group(&home, "peek", vec![component_member(&hidden)]),
        "",
    )
    .await
    .unwrap_err();
    assert_eq!(hidden_error.code(), Code::InvalidArgument);
    assert!(hidden_error
        .message()
        .contains("ARCHITECTURE_REFERENCE_UNRESOLVED"));
}

fn group_ref_member(target: &pb::Entity) -> pb::SystemGroupMembership {
    pb::SystemGroupMembership {
        member: Some(pb::SystemGroupMember {
            target: Some(pb::system_group_member::Target::SystemGroup(
                pb::SystemGroupRef {
                    id: reference(target).id,
                },
            )),
        }),
        ..Default::default()
    }
}

async fn read_group(client: &mut Client, token: &str, namespace: &str, slug: &str) -> Status {
    client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::SystemGroup as i32,
                id: Some(id(namespace, slug)),
            },
            token,
        ))
        .await
        .unwrap_err()
}

async fn namespace_findings(
    client: &mut Client,
    token: &str,
    namespace: &str,
) -> Vec<pb::ValidationIssue> {
    client
        .validate_project(authed(
            pb::ValidateProjectRequest {
                scope: Some(pb::validate_project_request::Scope::Namespace(
                    namespace.into(),
                )),
            },
            token,
        ))
        .await
        .unwrap()
        .into_inner()
        .reports
        .into_iter()
        .flat_map(|report| report.issues)
        .collect()
}

async fn impact_of(client: &mut Client, token: &str, root: &pb::Entity) -> Vec<pb::ImpactNode> {
    client
        .get_impact(authed(
            pb::GetImpactRequest {
                root: Some(reference(root)),
                ..Default::default()
            },
            token,
        ))
        .await
        .unwrap()
        .into_inner()
        .nodes
}

fn rendered(nodes: &[pb::ImpactNode]) -> String {
    format!("{nodes:?}")
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_group_in_a_cycle_is_invisible_and_indistinguishable_from_absence() {
    let mut client = serve().await;
    let acme = register(&mut client, ACME_TOKEN, "cycle-acme").await;
    let beta = register(&mut client, BETA_TOKEN, "cycle-beta").await;
    let leaf = put(&mut client, ROOT_TOKEN, component(&acme, "leaf"), "")
        .await
        .unwrap()
        .entity
        .unwrap();
    let front = put(
        &mut client,
        ROOT_TOKEN,
        group(&acme, "front", vec![component_member(&leaf)]),
        "",
    )
    .await
    .unwrap();
    let deep = put(&mut client, ROOT_TOKEN, group(&beta, "deep", vec![]), "")
        .await
        .unwrap()
        .entity
        .unwrap();
    let back = put(
        &mut client,
        ROOT_TOKEN,
        group(
            &beta,
            "back",
            vec![
                group_ref_member(front.entity.as_ref().unwrap()),
                group_ref_member(&deep),
            ],
        ),
        "",
    )
    .await
    .unwrap()
    .entity
    .unwrap();
    let mut closing = front.entity.clone().unwrap();
    let Some(pb::entity::Kind::SystemGroup(authored)) = &mut closing.kind else {
        panic!("expected system group");
    };
    authored.members.push(group_ref_member(&back));
    let closed = put(&mut client, ROOT_TOKEN, closing, &front.etag)
        .await
        .unwrap();
    let front_entity = closed.entity.clone().unwrap();

    let cycle = |issues: &[pb::ValidationIssue]| {
        issues
            .iter()
            .filter(|issue| issue.code == "SYSTEM_GROUP_MEMBERSHIP_CYCLE")
            .count()
    };
    let root_view = namespace_findings(&mut client, ROOT_TOKEN, &acme).await;
    assert_eq!(cycle(&root_view), 1, "{root_view:?}");
    let root_impact = impact_of(&mut client, ROOT_TOKEN, &leaf).await;
    let root_nodes: Vec<_> = root_impact
        .iter()
        .map(|node| (reference_slug(node), node.depth))
        .collect();
    assert_eq!(
        root_nodes,
        [
            ("leaf".into(), 0),
            ("front".into(), 1),
            ("back".into(), 2),
            ("deep".into(), 3)
        ]
    );
    for node in root_impact.iter().filter(|node| node.depth > 0) {
        assert_eq!(node.path.len(), node.depth as usize);
        for hop in &node.path {
            assert!(hop.connection_id.is_some(), "{hop:?}");
            assert!(!hop.from_uid.is_empty(), "{hop:?}");
            assert!(hop.from_field.starts_with("members["), "{hop:?}");
        }
    }

    let hidden_view = namespace_findings(&mut client, ACME_TOKEN, &acme).await;
    assert_eq!(cycle(&hidden_view), 0, "{hidden_view:?}");
    let hidden_impact = impact_of(&mut client, ACME_TOKEN, &leaf).await;
    assert_eq!(
        hidden_impact
            .iter()
            .map(|node| (reference_slug(node), node.depth))
            .collect::<Vec<_>>(),
        [("leaf".into(), 0), ("front".into(), 1), ("back".into(), 2)]
    );
    assert!(hidden_impact
        .iter()
        .flat_map(|node| &node.path)
        .all(|hop| hop.from.as_ref().unwrap().id.as_ref().unwrap().namespace == acme));
    let hidden_incoming = incoming_of(&mut client, ACME_TOKEN, &front_entity).await;
    assert_eq!(hidden_incoming, [] as [pb::Reference; 0]);
    let hidden_read = read_group(&mut client, ACME_TOKEN, &beta, "back").await;
    let hidden_batch = batch_of(&mut client, ACME_TOKEN, &[&front_entity, &back]).await;
    let hidden_listing = list_groups(&mut client, ACME_TOKEN).await;
    assert_eq!(hidden_batch.found, [true, false]);
    assert_eq!(hidden_batch.etags[1], "");
    assert_eq!(hidden_batch.entities[1], pb::Entity::default());
    assert_eq!(hidden_listing.len(), 1);
    assert!(!format!("{hidden_view:?}{hidden_incoming:?}").contains(&beta));
    let hidden_front = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::SystemGroup as i32,
                id: Some(id(&acme, "front")),
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(members(hidden_front.entity.as_ref().unwrap()).len(), 2);
    let hidden_outgoing = outgoing_of(&mut client, ACME_TOKEN, &front_entity).await;

    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::SystemGroup as i32,
                id: Some(id(&beta, "back")),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();

    let absent_read = read_group(&mut client, ACME_TOKEN, &beta, "back").await;
    assert_eq!(hidden_read.code(), absent_read.code());
    assert_eq!(hidden_read.message(), absent_read.message());
    assert_eq!(
        namespace_findings(&mut client, ACME_TOKEN, &acme).await,
        hidden_view
    );
    assert_eq!(
        rendered(&impact_of(&mut client, ACME_TOKEN, &leaf).await),
        rendered(&hidden_impact)
    );
    assert_eq!(
        batch_of(&mut client, ACME_TOKEN, &[&front_entity, &back]).await,
        hidden_batch
    );
    assert_eq!(list_groups(&mut client, ACME_TOKEN).await, hidden_listing);
    assert_eq!(
        outgoing_of(&mut client, ACME_TOKEN, &front_entity).await,
        hidden_outgoing
    );
    assert_eq!(
        incoming_of(&mut client, ACME_TOKEN, &front_entity).await,
        hidden_incoming
    );
}

fn reference_slug(node: &pb::ImpactNode) -> String {
    node.entity
        .as_ref()
        .unwrap()
        .id
        .as_ref()
        .unwrap()
        .slug
        .clone()
}

async fn incoming_of(client: &mut Client, token: &str, target: &pb::Entity) -> Vec<pb::Reference> {
    let target = reference(target);
    client
        .get_incoming_references(authed(
            pb::GetReferencesRequest {
                kind: target.kind,
                id: target.id,
                ..Default::default()
            },
            token,
        ))
        .await
        .unwrap()
        .into_inner()
        .references
}

async fn outgoing_of(client: &mut Client, token: &str, owner: &pb::Entity) -> Vec<pb::Reference> {
    let owner = reference(owner);
    client
        .get_outgoing_references(authed(
            pb::GetReferencesRequest {
                kind: owner.kind,
                id: owner.id,
                ..Default::default()
            },
            token,
        ))
        .await
        .unwrap()
        .into_inner()
        .references
}

async fn batch_of(
    client: &mut Client,
    token: &str,
    keys: &[&pb::Entity],
) -> pb::BatchGetEntitiesResponse {
    client
        .batch_get_entities(authed(
            pb::BatchGetEntitiesRequest {
                keys: keys.iter().map(|key| reference(key)).collect(),
                dense: true,
            },
            token,
        ))
        .await
        .unwrap()
        .into_inner()
}

async fn list_groups(client: &mut Client, token: &str) -> Vec<pb::Entity> {
    client
        .list_entities(authed(
            pb::ListEntitiesRequest {
                kinds: vec![pb::EntityKind::SystemGroup as i32],
                ..Default::default()
            },
            token,
        ))
        .await
        .unwrap()
        .into_inner()
        .entities
}

#[tokio::test(flavor = "multi_thread")]
async fn restricted_caller_is_blocked_by_a_hidden_member_edge_without_learning_about_it() {
    use pb::delete_entity_request::Mode;
    let mut client = serve().await;
    let acme = register(&mut client, ACME_TOKEN, "guard-acme").await;
    let beta = register(&mut client, BETA_TOKEN, "guard-beta").await;
    let shared = put(&mut client, ACME_TOKEN, component(&acme, "shared"), "")
        .await
        .unwrap()
        .entity
        .unwrap();
    put(&mut client, ACME_TOKEN, component(&acme, "alone"), "")
        .await
        .unwrap();
    put(
        &mut client,
        ROOT_TOKEN,
        group(&beta, "hidden-owner", vec![component_member(&shared)]),
        "",
    )
    .await
    .unwrap();
    let delete = |slug: &str, mode: Mode| pb::DeleteEntityRequest {
        kind: pb::EntityKind::Component as i32,
        id: Some(id(&acme, slug)),
        mode: mode as i32,
        ..Default::default()
    };

    let preview = client
        .delete_entity(authed(delete("shared", Mode::DryRun), ACME_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(preview.referrers, [] as [pb::Reference; 0]);
    let refused = client
        .delete_entity(authed(delete("shared", Mode::FailIfReferenced), ACME_TOKEN))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused}");
    assert!(
        !refused.message().contains("hidden-owner") && !refused.message().contains(&beta),
        "{refused}"
    );
    let survivor = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Component as i32,
                id: Some(id(&acme, "shared")),
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(survivor.entity.is_some());

    let owner_view = client
        .delete_entity(authed(delete("shared", Mode::DryRun), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(owner_view.referrers.len(), 1, "{owner_view:?}");

    client
        .delete_entity(authed(delete("alone", Mode::FailIfReferenced), ACME_TOKEN))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn fixture_client_expands_exact_members_once_and_marks_back_edges() {
    let mut client = serve().await;
    let ns = register(&mut client, ROOT_TOKEN, "member-expansion").await;
    let checkout = put(&mut client, ROOT_TOKEN, component(&ns, "checkout"), "")
        .await
        .unwrap()
        .entity
        .unwrap();
    let mut next = component(&ns, "checkout");
    if let Some(pb::entity::Kind::Component(body)) = next.kind.as_mut() {
        body.id.as_mut().unwrap().version = 2;
    }
    let checkout_v2 = put(&mut client, ROOT_TOKEN, next, "")
        .await
        .unwrap()
        .entity
        .unwrap();
    let platform = put(&mut client, ROOT_TOKEN, group(&ns, "platform", vec![]), "")
        .await
        .unwrap()
        .entity
        .unwrap();
    let group_member = |target: &pb::Entity| pb::SystemGroupMembership {
        member: Some(pb::SystemGroupMember {
            target: Some(pb::system_group_member::Target::SystemGroup(
                pb::SystemGroupRef {
                    id: reference(target).id,
                },
            )),
        }),
        ..Default::default()
    };
    let storefront = put(
        &mut client,
        ROOT_TOKEN,
        group(
            &ns,
            "storefront",
            vec![group_member(&platform), component_member(&checkout)],
        ),
        "",
    )
    .await
    .unwrap()
    .entity
    .unwrap();
    let store_plat = members(&storefront)[0].connection_id.clone().unwrap();
    let platform = put(
        &mut client,
        ROOT_TOKEN,
        group(
            &ns,
            "platform",
            vec![
                group_member(&storefront),
                component_member(&checkout),
                component_member(&checkout_v2),
            ],
        ),
        "",
    )
    .await
    .unwrap()
    .entity
    .unwrap();
    let mut pending = vec![(reference(&platform), Vec::<pb::EntityRef>::new())];
    let mut expanded = Vec::new();
    let mut back_edges = Vec::new();
    let mut repeated = Vec::new();
    while let Some((selected, mut ancestors)) = pending.pop() {
        if expanded.contains(&selected) {
            repeated.push(selected);
            continue;
        }
        let read = client
            .get_entity(authed(
                pb::GetEntityRequest {
                    kind: selected.kind,
                    id: selected.id.clone(),
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner()
            .entity
            .unwrap();
        assert_eq!(reference(&read), selected);
        expanded.push(selected.clone());
        ancestors.push(selected.clone());
        let outgoing = client
            .get_outgoing_references(authed(
                pb::GetReferencesRequest {
                    kind: selected.kind,
                    id: selected.id.clone(),
                    page_size: 100,
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(outgoing.next_page_token, "");
        for edge in outgoing
            .references
            .into_iter()
            .rev()
            .filter(|edge| edge.from_field.starts_with("members["))
        {
            let target = edge.to.unwrap();
            if ancestors.contains(&target) {
                back_edges.push(edge.connection_id.unwrap());
            } else {
                pending.push((target, ancestors.clone()));
            }
        }
    }
    assert_eq!(expanded.len(), 4);
    assert_eq!(
        expanded
            .iter()
            .filter(|selected| **selected == reference(&checkout))
            .count(),
        1
    );
    assert!(expanded.contains(&reference(&checkout_v2)));
    assert_eq!(repeated, vec![reference(&checkout)]);
    assert_eq!(back_edges, vec![store_plat]);
}
