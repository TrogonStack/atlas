//! Native resource-group metadata through the public stack: a real tonic
//! server behind `BearerAuth` and `AuthzLayer` over a real JetStream store.
//!
//! Covers authorization, hidden-target and hidden-referrer privacy, stale
//! guards, owner incarnation and the branch lifecycle of group and item IDs.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use prost::Message;
use tokio::net::TcpListener;
use tonic::{service::interceptor::InterceptedService, transport::Channel, Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "native-authz-admin";
const READER: &str = "native-authz-reader";
const WRITER: &str = "native-authz-writer";
const GROUPS: &str = "type.googleapis.com/trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation";
const BRANCH: &str = "@visible:native-identity";

type Client = EventModelServiceClient<Channel>;

struct World {
    client: Client,
    visible: String,
    hidden: String,
}

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

async fn world() -> World {
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
kind = "agent"
parent = "visible"
tokens = ["{READER}"]
[principals.writer]
role = "writer"
kind = "agent"
parent = "visible"
tokens = ["{WRITER}"]
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
    .with_snapshot_cap(0);
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
    let mut client = Client::connect(format!("http://{addr}")).await.unwrap();
    let mut ids = Vec::new();
    for (name, parent) in [("offering", "visible"), ("private-area", "hidden")] {
        ids.push(
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
    World {
        client,
        hidden: ids.pop().unwrap(),
        visible: ids.pop().unwrap(),
    }
}

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn target(namespace: &str, slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::Component as i32,
        id: Some(id(namespace, slug)),
    }
}

fn item(target: pb::EntityRef) -> pb::ResourceGroupItem {
    pb::ResourceGroupItem {
        connection_id: None,
        target: Some(target),
    }
}

fn group(title: &str, items: Vec<pb::ResourceGroupItem>) -> pb::ResourceGroup {
    pb::ResourceGroup {
        title: Some(pb::MetadataTitle {
            value: title.into(),
        }),
        items,
        ..Default::default()
    }
}

fn component(namespace: &str, slug: &str, groups: Vec<pb::ResourceGroup>) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Component(pb::Component {
            id: Some(id(namespace, slug)),
            title: slug.into(),
            metadata: if groups.is_empty() {
                Vec::new()
            } else {
                vec![prost_types::Any {
                    type_url: GROUPS.into(),
                    value: pb::ResourceGroupsAnnotation { groups }.encode_to_vec(),
                }]
            },
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn component_mut(entity: &mut pb::Entity) -> &mut pb::Component {
    let Some(pb::entity::Kind::Component(value)) = entity.kind.as_mut() else {
        panic!("expected a component")
    };
    value
}

fn groups_of(entity: &pb::Entity) -> Vec<pb::ResourceGroup> {
    let Some(pb::entity::Kind::Component(value)) = entity.kind.as_ref() else {
        panic!("expected a component")
    };
    value
        .metadata
        .iter()
        .find(|any| any.type_url == GROUPS)
        .map(|any| {
            pb::ResourceGroupsAnnotation::decode(any.value.as_slice())
                .unwrap()
                .groups
        })
        .unwrap_or_default()
}

fn edit_groups(entity: &mut pb::Entity, edit: impl FnOnce(&mut Vec<pb::ResourceGroup>)) {
    let mut groups = groups_of(entity);
    edit(&mut groups);
    let metadata = &mut component_mut(entity).metadata;
    metadata.retain(|any| any.type_url != GROUPS);
    metadata.push(prost_types::Any {
        type_url: GROUPS.into(),
        value: pb::ResourceGroupsAnnotation { groups }.encode_to_vec(),
    });
}

fn item_ids(entity: &pb::Entity) -> Vec<String> {
    groups_of(entity)
        .iter()
        .flat_map(|group| group.items.iter())
        .map(|item| item.connection_id.clone().unwrap().value)
        .collect()
}

fn group_ids(entity: &pb::Entity) -> Vec<String> {
    groups_of(entity)
        .iter()
        .map(|group| group.group_id.clone().unwrap().value)
        .collect()
}

fn uid(entity: &pb::Entity) -> String {
    entity.system.as_ref().unwrap().uid.clone()
}

fn put_request(entity: pb::Entity, if_match: &str) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        entity: Some(entity),
        if_match: if_match.into(),
        ..Default::default()
    }
}

impl World {
    async fn put_as(
        &mut self,
        token: &str,
        branch: Option<&str>,
        entity: pb::Entity,
        if_match: &str,
    ) -> Result<pb::PutEntityResponse, tonic::Status> {
        self.client
            .put_entity(request(put_request(entity, if_match), token, branch))
            .await
            .map(tonic::Response::into_inner)
    }

    async fn put(&mut self, entity: pb::Entity) -> pb::PutEntityResponse {
        self.put_as(ADMIN, None, entity, "").await.unwrap()
    }

    async fn put_on_branch(&mut self, entity: pb::Entity) -> pb::Entity {
        self.put_as(ADMIN, Some(BRANCH), entity, "")
            .await
            .unwrap()
            .entity
            .unwrap()
    }

    async fn get_as(
        &mut self,
        token: &str,
        branch: Option<&str>,
        namespace: &str,
        slug: &str,
    ) -> Result<pb::GetEntityResponse, tonic::Status> {
        self.client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Component as i32,
                    id: Some(id(namespace, slug)),
                },
                token,
                branch,
            ))
            .await
            .map(tonic::Response::into_inner)
    }

    async fn stored(&mut self, slug: &str) -> pb::GetEntityResponse {
        let namespace = self.visible.clone();
        self.get_as(ADMIN, None, &namespace, slug).await.unwrap()
    }

    async fn on_branch(&mut self, slug: &str) -> pb::Entity {
        let namespace = self.visible.clone();
        self.get_as(ADMIN, Some(BRANCH), &namespace, slug)
            .await
            .unwrap()
            .entity
            .unwrap()
    }

    async fn absent(&mut self, namespace: &str, slug: &str) -> bool {
        self.get_as(ADMIN, None, namespace, slug)
            .await
            .is_err_and(|status| status.code() == Code::NotFound)
    }

    async fn batch_put_as(
        &mut self,
        token: &str,
        branch: Option<&str>,
        entities: Vec<pb::Entity>,
    ) -> Result<pb::BatchMutateResponse, tonic::Status> {
        self.client
            .batch_mutate(request(
                pb::BatchMutateRequest {
                    ops: entities
                        .into_iter()
                        .map(|entity| pb::BatchMutateOp {
                            op: Some(pb::batch_mutate_op::Op::Put(put_request(entity, ""))),
                        })
                        .collect(),
                    ..Default::default()
                },
                token,
                branch,
            ))
            .await
            .map(tonic::Response::into_inner)
    }

    async fn delete_as(
        &mut self,
        token: &str,
        branch: Option<&str>,
        namespace: &str,
        slug: &str,
        mode: pb::delete_entity_request::Mode,
    ) -> Result<pb::DeleteEntityResponse, tonic::Status> {
        self.client
            .delete_entity(request(
                pb::DeleteEntityRequest {
                    kind: pb::EntityKind::Component as i32,
                    id: Some(id(namespace, slug)),
                    mode: mode as i32,
                    ..Default::default()
                },
                token,
                branch,
            ))
            .await
            .map(tonic::Response::into_inner)
    }

    async fn create_branch(&mut self) {
        self.client
            .create_branch(request(
                pb::CreateBranchRequest {
                    name: BRANCH.into(),
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap();
    }

    async fn merge(&mut self, dry_run: bool) -> pb::MergeBranchResponse {
        self.client
            .merge_branch(request(
                pb::MergeBranchRequest {
                    name: BRANCH.into(),
                    dry_run,
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap()
            .into_inner()
    }

    async fn resolve(
        &mut self,
        slug: &str,
        resolution: pb::resolve_branch_entry_request::Resolution,
    ) -> Result<(), tonic::Status> {
        let namespace = self.visible.clone();
        self.client
            .resolve_branch_entry(request(
                pb::ResolveBranchEntryRequest {
                    name: BRANCH.into(),
                    r#ref: Some(target(&namespace, slug)),
                    resolution: resolution as i32,
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .map(|_| ())
    }

    async fn bound(&mut self, slug: &str, owner: &pb::Entity, occurrence: &str) -> bool {
        let namespace = self.visible.clone();
        self.incoming_as(ADMIN, &namespace, slug)
            .await
            .unwrap()
            .iter()
            .any(|reference| {
                reference.from_uid == uid(owner)
                    && reference
                        .connection_id
                        .as_ref()
                        .is_some_and(|id| id.value == occurrence)
            })
    }

    async fn incoming_as(
        &mut self,
        token: &str,
        namespace: &str,
        slug: &str,
    ) -> Result<Vec<pb::Reference>, tonic::Status> {
        self.client
            .get_incoming_references(request(
                pb::GetReferencesRequest {
                    kind: pb::EntityKind::Component as i32,
                    id: Some(id(namespace, slug)),
                    ..Default::default()
                },
                token,
                None,
            ))
            .await
            .map(|reply| reply.into_inner().references)
    }
}

fn applied(response: &pb::BatchMutateResponse) -> bool {
    response.status == pb::batch_mutate_response::Status::Applied as i32
}

fn merged(response: &pb::MergeBranchResponse, dry_run: bool) -> bool {
    response.status
        == if dry_run {
            pb::merge_branch_response::Status::Validated
        } else {
            pb::merge_branch_response::Status::Applied
        } as i32
}

#[tokio::test(flavor = "multi_thread")]
async fn reader_is_denied_native_group_writes_on_every_route() {
    let mut world = world().await;
    let visible = world.visible.clone();
    world.put(component(&visible, "target", vec![])).await;
    let owner = world
        .put(component(
            &visible,
            "owner",
            vec![group("related", vec![item(target(&visible, "target"))])],
        ))
        .await;
    let before = world.stored("owner").await;

    let created = component(
        &visible,
        "reader-created",
        vec![group("related", vec![item(target(&visible, "target"))])],
    );
    let denied = world
        .put_as(READER, None, created.clone(), "")
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied, "{denied}");

    let mut edited = owner.entity.clone().unwrap();
    edit_groups(&mut edited, |groups| groups[0].items.clear());
    let denied = world
        .put_as(READER, None, edited.clone(), &owner.etag)
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied, "{denied}");

    let denied = world
        .batch_put_as(READER, None, vec![created, edited])
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied, "{denied}");

    let denied = world
        .delete_as(
            READER,
            None,
            &visible,
            "owner",
            pb::delete_entity_request::Mode::Force,
        )
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied, "{denied}");

    assert!(world.absent(&visible, "reader-created").await);
    assert_eq!(world.stored("owner").await, before);
    assert_eq!(
        world
            .incoming_as(ADMIN, &visible, "target")
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        world.get_as(READER, None, &visible, "owner").await.is_ok(),
        "a Reader keeps read access"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_group_targets_are_rejected_exactly_like_missing_ones() {
    let mut world = world().await;
    let (visible, hidden) = (world.visible.clone(), world.hidden.clone());
    world.create_branch().await;
    let secret = component(&hidden, "salary-forecast", vec![]);
    world.put(secret).await;
    world.put(component(&visible, "target", vec![])).await;

    let proposal = |slug: &str, namespace: &str, referenced: &str| {
        component(
            &visible,
            slug,
            vec![group("related", vec![item(target(namespace, referenced))])],
        )
    };
    for branch in [None, Some(BRANCH)] {
        let hidden_error = world
            .put_as(
                WRITER,
                branch,
                proposal("via-hidden", &hidden, "salary-forecast"),
                "",
            )
            .await
            .unwrap_err();
        let never_written = world
            .put_as(
                WRITER,
                branch,
                proposal("via-never-written", &hidden, "never-written"),
                "",
            )
            .await
            .unwrap_err();
        let missing_error = world
            .put_as(
                WRITER,
                branch,
                proposal("via-missing", &visible, "salary-forecast"),
                "",
            )
            .await
            .unwrap_err();
        assert_eq!(
            hidden_error.code(),
            Code::NotFound,
            "branch {branch:?}: {hidden_error:?}"
        );
        assert_eq!(hidden_error.code(), never_written.code());
        assert_eq!(
            hidden_error.message(),
            never_written.message(),
            "a hidden target that exists must not differ from one that never did"
        );
        assert_eq!(
            missing_error.code(),
            Code::InvalidArgument,
            "{missing_error:?}"
        );
        let rendered = format!("{hidden_error:?}");
        for private in [hidden.as_str(), "private-area", "salary-forecast", "hidden"] {
            assert!(!rendered.contains(private), "{rendered}");
        }

        let batch = world
            .batch_put_as(
                WRITER,
                branch,
                vec![
                    proposal("batch-fine", &visible, "target"),
                    proposal("batch-hidden", &hidden, "salary-forecast"),
                ],
            )
            .await;
        let rendered = format!("{batch:?}");
        assert!(
            batch.is_err() || !applied(batch.as_ref().unwrap()),
            "{rendered}"
        );
        for private in [hidden.as_str(), "private-area", "salary-forecast"] {
            assert!(!rendered.contains(private), "{rendered}");
        }
        for slug in [
            "via-hidden",
            "via-never-written",
            "via-missing",
            "batch-fine",
            "batch-hidden",
        ] {
            assert!(world
                .get_as(ADMIN, branch, &visible, slug)
                .await
                .is_err_and(|status| status.code() == Code::NotFound));
        }
    }

    let read_hidden = world
        .get_as(WRITER, None, &hidden, "salary-forecast")
        .await
        .unwrap_err();
    let read_missing = world
        .get_as(WRITER, None, &hidden, "never-written")
        .await
        .unwrap_err();
    assert_eq!(read_hidden.code(), Code::NotFound);
    assert_eq!(read_hidden.code(), read_missing.code());

    let mut retained = world
        .put(proposal("retained", &hidden, "salary-forecast"))
        .await;
    component_mut(retained.entity.as_mut().unwrap()).title = "retitled".into();
    let rejected = world
        .put_as(WRITER, None, retained.entity.unwrap(), &retained.etag)
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), Code::NotFound, "{rejected}");
    assert!(!format!("{rejected:?}").contains("salary-forecast"));
    assert_eq!(
        component_title(&world.stored("retained").await.entity.unwrap()),
        "retained"
    );
}

fn component_title(entity: &pb::Entity) -> String {
    let Some(pb::entity::Kind::Component(value)) = entity.kind.as_ref() else {
        panic!("expected a component")
    };
    value.title.clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_referrer_blocks_safe_delete_without_revealing_itself() {
    let mut world = world().await;
    let (visible, hidden) = (world.visible.clone(), world.hidden.clone());
    world.put(component(&visible, "shared", vec![])).await;
    world
        .put(component(
            &hidden,
            "secret-referrer",
            vec![group("private", vec![item(target(&visible, "shared"))])],
        ))
        .await;

    let leaks = |rendered: &str| {
        for private in [hidden.as_str(), "private-area", "secret-referrer", "hidden"] {
            assert!(!rendered.contains(private), "{rendered}");
        }
    };
    let blocked = world
        .delete_as(
            WRITER,
            None,
            &visible,
            "shared",
            pb::delete_entity_request::Mode::FailIfReferenced,
        )
        .await
        .unwrap_err();
    assert_eq!(blocked.code(), Code::FailedPrecondition, "{blocked}");
    leaks(&format!("{blocked:?}"));

    let batch = world
        .client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops: vec![pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                        kind: pb::EntityKind::Component as i32,
                        id: Some(id(&visible, "shared")),
                        mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                        ..Default::default()
                    })),
                }],
                ..Default::default()
            },
            WRITER,
            None,
        ))
        .await;
    leaks(&format!("{batch:?}"));
    assert!(
        batch.is_err() || !applied(batch.as_ref().unwrap().get_ref()),
        "{batch:?}"
    );

    let visible_view = world.incoming_as(WRITER, &visible, "shared").await;
    leaks(&format!("{visible_view:?}"));
    assert!(visible_view.is_ok_and(|references| references.is_empty()));
    assert!(world.get_as(WRITER, None, &visible, "shared").await.is_ok());
    assert_eq!(
        world
            .incoming_as(ADMIN, &visible, "shared")
            .await
            .unwrap()
            .len(),
        1,
        "the index still holds the hidden occurrence"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_guards_publish_nothing_for_group_changes() {
    let mut world = world().await;
    let visible = world.visible.clone();
    world.put(component(&visible, "first", vec![])).await;
    world.put(component(&visible, "second", vec![])).await;
    let created = world
        .put_as(
            WRITER,
            None,
            component(
                &visible,
                "owner",
                vec![group("related", vec![item(target(&visible, "first"))])],
            ),
            "",
        )
        .await
        .unwrap();
    let mut retitled = created.entity.clone().unwrap();
    component_mut(&mut retitled).title = "current".into();
    let current = world
        .put_as(WRITER, None, retitled, &created.etag)
        .await
        .unwrap();
    let before = world.stored("owner").await;

    let mut stale = created.entity.clone().unwrap();
    edit_groups(&mut stale, |groups| {
        groups[0].items[0].connection_id = None;
        groups[0].items[0].target = Some(target(&visible, "second"));
    });
    let rejected = world
        .put_as(WRITER, None, stale.clone(), &created.etag)
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), Code::Aborted, "{rejected}");

    let batch = world
        .client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops: vec![pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(put_request(
                        stale,
                        &created.etag,
                    ))),
                }],
                ..Default::default()
            },
            WRITER,
            None,
        ))
        .await;
    assert!(
        batch.is_err() || !applied(batch.as_ref().unwrap().get_ref()),
        "{batch:?}"
    );

    let after = world.stored("owner").await;
    assert_eq!(after, before);
    assert_eq!(after.etag, current.etag);
    assert_eq!(
        world
            .incoming_as(ADMIN, &visible, "first")
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        world.incoming_as(ADMIN, &visible, "second").await.unwrap(),
        []
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn clones_and_recreated_owners_never_inherit_group_selectors() {
    let mut world = world().await;
    let visible = world.visible.clone();
    world.put(component(&visible, "target", vec![])).await;
    let original = world
        .put(component(
            &visible,
            "owner",
            vec![group(
                "related",
                vec![
                    item(target(&visible, "target")),
                    item(target(&visible, "target")),
                ],
            )],
        ))
        .await
        .entity
        .unwrap();
    let ids = item_ids(&original);
    assert!(world.bound("target", &original, &ids[0]).await);

    let mut copy = original.clone();
    component_mut(&mut copy).id = Some(id(&visible, "copy"));
    copy.system = None;
    let copy = world.put(copy).await.entity.unwrap();
    assert_ne!(uid(&copy), uid(&original));
    assert!(world.bound("target", &original, &ids[0]).await);

    let mut next_version = original.clone();
    component_mut(&mut next_version)
        .id
        .as_mut()
        .unwrap()
        .version = 2;
    next_version.system = None;
    let next_version = world.put(next_version).await.entity.unwrap();
    assert_ne!(uid(&next_version), uid(&original));
    assert!(world.bound("target", &original, &ids[0]).await);

    world
        .delete_as(
            ADMIN,
            None,
            &visible,
            "owner",
            pb::delete_entity_request::Mode::Force,
        )
        .await
        .unwrap();
    let mut recreated = original.clone();
    recreated.system = None;
    let recreated = world.put(recreated).await.entity.unwrap();
    assert_ne!(uid(&recreated), uid(&original));
    assert!(
        !world.bound("target", &original, &ids[0]).await,
        "the old incarnation never binds to the recreated owner"
    );
    assert_eq!(item_ids(&original), ids);
}

#[tokio::test(flavor = "multi_thread")]
async fn group_and_item_ids_follow_the_occurrence_lifecycle_on_public_writes() {
    let mut world = world().await;
    let visible = world.visible.clone();
    for slug in ["first", "second"] {
        world.put(component(&visible, slug, vec![])).await;
    }
    let created = world
        .put(component(
            &visible,
            "owner",
            vec![
                group(
                    "alpha",
                    vec![
                        item(target(&visible, "first")),
                        item(target(&visible, "first")),
                    ],
                ),
                group("beta", vec![item(target(&visible, "second"))]),
            ],
        ))
        .await;
    let entity = created.entity.clone().unwrap();
    let (items, groups) = (item_ids(&entity), group_ids(&entity));
    assert_eq!(items.len(), 3);
    assert_ne!(
        items[0], items[1],
        "equal siblings are separate occurrences"
    );

    let mut reordered = entity.clone();
    edit_groups(&mut reordered, |groups| {
        groups.reverse();
        groups[1].items.reverse();
        groups[1].title = Some(pb::MetadataTitle {
            value: "renamed".into(),
        });
    });
    let reordered = world
        .put_as(ADMIN, None, reordered, &created.etag)
        .await
        .unwrap();
    let after = reordered.entity.clone().unwrap();
    assert_eq!(group_ids(&after), [groups[1].clone(), groups[0].clone()]);
    assert_eq!(
        item_ids(&after),
        [items[2].clone(), items[1].clone(), items[0].clone()]
    );

    let mut retargeted = after.clone();
    edit_groups(&mut retargeted, |groups| {
        groups[1].items[0].target = Some(target(&visible, "second"));
    });
    let rejected = world
        .put_as(ADMIN, None, retargeted, &reordered.etag)
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument, "{rejected}");

    let mut moved = after.clone();
    edit_groups(&mut moved, |groups| {
        let travelling = groups[1].items.remove(0);
        groups[0].items.push(travelling);
    });
    let rejected = world
        .put_as(ADMIN, None, moved, &reordered.etag)
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument, "{rejected}");

    let mut fresh = after.clone();
    edit_groups(&mut fresh, |groups| {
        groups[0].items[0].connection_id = None;
        groups[0].items[0].target = Some(target(&visible, "first"));
    });
    let renewed = world
        .put_as(ADMIN, None, fresh, &reordered.etag)
        .await
        .unwrap();
    let renewed_entity = renewed.entity.clone().unwrap();
    assert_ne!(item_ids(&renewed_entity)[0], items[2]);
    assert_eq!(item_ids(&renewed_entity)[1..], item_ids(&after)[1..]);

    let mut removed = renewed_entity.clone();
    edit_groups(&mut removed, |groups| {
        groups[1].items.remove(0);
    });
    let removed = world
        .put_as(ADMIN, None, removed, &renewed.etag)
        .await
        .unwrap();
    let mut reinserted = removed.entity.clone().unwrap();
    edit_groups(&mut reinserted, |groups| {
        groups[1].items.insert(
            0,
            pb::ResourceGroupItem {
                connection_id: Some(pb::ConnectionId {
                    value: item_ids(&renewed_entity)[1].clone(),
                }),
                target: Some(target(&visible, "first")),
            },
        );
    });
    let rejected = world
        .put_as(ADMIN, None, reinserted, &removed.etag)
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument, "{rejected}");

    let mut duplicated = removed.entity.clone().unwrap();
    edit_groups(&mut duplicated, |groups| {
        let clone = groups[1].items[0].clone();
        groups[1].items.push(clone);
    });
    let rejected = world
        .put_as(ADMIN, None, duplicated, &removed.etag)
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument, "{rejected}");
    assert_eq!(world.stored("owner").await.entity, removed.entity);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_equal_sibling_is_an_independent_occurrence_selector() {
    let mut world = world().await;
    let visible = world.visible.clone();
    world.put(component(&visible, "target", vec![])).await;
    let created = world
        .put(component(
            &visible,
            "owner",
            vec![group(
                "related",
                vec![
                    item(target(&visible, "target")),
                    item(target(&visible, "target")),
                ],
            )],
        ))
        .await;
    let entity = created.entity.clone().unwrap();
    let ids = item_ids(&entity);
    assert_ne!(ids[0], ids[1]);
    assert!(world.bound("target", &entity, &ids[0]).await);
    assert!(world.bound("target", &entity, &ids[1]).await);

    let mut shrunk = entity.clone();
    edit_groups(&mut shrunk, |groups| {
        groups[0].items.remove(0);
    });
    let shrunk = world
        .put_as(ADMIN, None, shrunk, &created.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    assert_eq!(item_ids(&shrunk), [ids[1].clone()]);
    assert!(!world.bound("target", &shrunk, &ids[0]).await);
    assert!(world.bound("target", &shrunk, &ids[1]).await);

    let references = world.incoming_as(ADMIN, &visible, "target").await.unwrap();
    assert_eq!(references.len(), 1, "{references:?}");
    assert_eq!(
        references[0]
            .connection_id
            .as_ref()
            .map(|id| id.value.clone()),
        Some(ids[1].clone())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_edits_keep_group_identity_and_land_with_dry_run_parity() {
    let mut world = world().await;
    let visible = world.visible.clone();
    for slug in ["first", "second"] {
        world.put(component(&visible, slug, vec![])).await;
    }
    let baseline = world
        .put(component(
            &visible,
            "owner",
            vec![group(
                "related",
                vec![
                    item(target(&visible, "first")),
                    item(target(&visible, "first")),
                ],
            )],
        ))
        .await
        .entity
        .unwrap();
    world.create_branch().await;

    let mut proposal = baseline.clone();
    edit_groups(&mut proposal, |groups| {
        groups[0].items.push(item(target(&visible, "second")));
        groups[0].items.remove(0);
    });
    let on_branch = world.put_on_branch(proposal).await;
    assert_eq!(uid(&on_branch), uid(&baseline));
    let (baseline_ids, branch_ids) = (item_ids(&baseline), item_ids(&on_branch));
    assert_eq!(branch_ids[0], baseline_ids[1]);
    assert_eq!(world.stored("owner").await.entity.unwrap(), baseline);

    let validated = world.merge(true).await;
    assert!(merged(&validated, true), "{validated:?}");
    assert_eq!(world.stored("owner").await.entity.unwrap(), baseline);
    let landed = world.merge(false).await;
    assert!(merged(&landed, false), "{landed:?}");
    let stored = world.stored("owner").await.entity.unwrap();
    assert_eq!(uid(&stored), uid(&baseline));
    assert_eq!(item_ids(&stored), branch_ids);
    assert!(world.bound("first", &stored, &branch_ids[0]).await);
    assert!(world.bound("second", &stored, &branch_ids[1]).await);
    assert!(
        !world.bound("first", &stored, &baseline_ids[0]).await,
        "an item removed on the branch stays removed once landed"
    );

    let mut reused = stored.clone();
    edit_groups(&mut reused, |groups| {
        groups[0].items.push(pb::ResourceGroupItem {
            connection_id: Some(pb::ConnectionId {
                value: baseline_ids[0].clone(),
            }),
            target: Some(target(&visible, "first")),
        });
    });
    let rejected = world.put_as(ADMIN, None, reused, "").await.unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument, "{rejected}");
    assert_eq!(
        world
            .incoming_as(ADMIN, &visible, "second")
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_removal_of_a_group_item_survives_rebase_and_conflict_resolution() {
    let mut world = world().await;
    let visible = world.visible.clone();
    world.put(component(&visible, "target", vec![])).await;
    let baseline = world
        .put(component(
            &visible,
            "owner",
            vec![group(
                "related",
                vec![
                    item(target(&visible, "target")),
                    item(target(&visible, "target")),
                ],
            )],
        ))
        .await
        .entity
        .unwrap();
    let ids = item_ids(&baseline);
    world.create_branch().await;

    let mut pruned = baseline.clone();
    edit_groups(&mut pruned, |groups| {
        groups[0].items.remove(0);
    });
    world.put_on_branch(pruned).await;

    let mut retitled = baseline.clone();
    edit_groups(&mut retitled, |groups| {
        groups[0].title = Some(pb::MetadataTitle {
            value: "baseline title".into(),
        });
    });
    let current = world.put(retitled).await.entity.unwrap();
    assert_eq!(item_ids(&current), ids);

    world
        .client
        .update_branch(request(
            pb::UpdateBranchRequest {
                name: BRANCH.into(),
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let diff = world
        .client
        .diff_branch(request(
            pb::DiffBranchRequest {
                name: BRANCH.into(),
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(diff.entries.len(), 1, "{diff:?}");

    world
        .resolve(
            "owner",
            pb::resolve_branch_entry_request::Resolution::KeepOurs,
        )
        .await
        .unwrap();
    let kept = world.on_branch("owner").await;
    assert_eq!(item_ids(&kept), [ids[1].clone()]);
    assert_eq!(uid(&kept), uid(&baseline));
    let landed = world.merge(false).await;
    assert!(merged(&landed, false), "{landed:?}");
    let stored = world.stored("owner").await.entity.unwrap();
    assert_eq!(item_ids(&stored), [ids[1].clone()]);
    assert!(!world.bound("target", &stored, &ids[0]).await);
    assert!(world.bound("target", &stored, &ids[1]).await);

    let mut reused = stored.clone();
    edit_groups(&mut reused, |groups| {
        groups[0].items.push(pb::ResourceGroupItem {
            connection_id: Some(pb::ConnectionId {
                value: ids[0].clone(),
            }),
            target: Some(target(&visible, "target")),
        });
    });
    assert_eq!(
        world
            .put_as(ADMIN, None, reused, "")
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn taking_baseline_content_discards_branch_group_identity() {
    let mut world = world().await;
    let visible = world.visible.clone();
    for slug in ["first", "second"] {
        world.put(component(&visible, slug, vec![])).await;
    }
    let baseline = world
        .put(component(
            &visible,
            "owner",
            vec![group("related", vec![item(target(&visible, "first"))])],
        ))
        .await
        .entity
        .unwrap();
    world.create_branch().await;
    let mut proposal = baseline.clone();
    edit_groups(&mut proposal, |groups| {
        groups[0].items.push(item(target(&visible, "second")));
    });
    let branched = world.put_on_branch(proposal).await;
    let mut current = baseline.clone();
    edit_groups(&mut current, |groups| {
        groups[0].title = Some(pb::MetadataTitle {
            value: "newer".into(),
        });
    });
    let current = world.put(current).await.entity.unwrap();

    world
        .resolve(
            "owner",
            pb::resolve_branch_entry_request::Resolution::TakeTheirs,
        )
        .await
        .unwrap();
    assert!(world
        .get_as(ADMIN, Some(BRANCH), &visible, "owner")
        .await
        .is_ok_and(|reply| reply.entity == Some(current.clone())));
    let landed = world.merge(false).await;
    assert!(merged(&landed, false), "{landed:?}");
    assert_eq!(world.stored("owner").await.entity.unwrap(), current);
    let discarded = item_ids(&branched)[1].clone();
    assert!(!world.bound("second", &current, &discarded).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_landing_and_resolution_revalidate_group_targets() {
    let mut world = world().await;
    let visible = world.visible.clone();
    for slug in ["first", "doomed"] {
        world.put(component(&visible, slug, vec![])).await;
    }
    let baseline = world
        .put(component(
            &visible,
            "owner",
            vec![group("related", vec![item(target(&visible, "first"))])],
        ))
        .await
        .entity
        .unwrap();
    world.create_branch().await;
    let mut proposal = baseline.clone();
    edit_groups(&mut proposal, |groups| {
        groups[0].items.push(item(target(&visible, "doomed")));
    });
    world.put_on_branch(proposal).await;
    world
        .delete_as(
            ADMIN,
            None,
            &visible,
            "doomed",
            pb::delete_entity_request::Mode::FailIfReferenced,
        )
        .await
        .unwrap();

    let before = world.stored("owner").await;
    let preview = world
        .client
        .merge_branch(request(
            pb::MergeBranchRequest {
                name: BRANCH.into(),
                dry_run: true,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await;
    assert!(
        preview.is_err() || !merged(preview.as_ref().unwrap().get_ref(), true),
        "{preview:?}"
    );
    let landing = world
        .client
        .merge_branch(request(
            pb::MergeBranchRequest {
                name: BRANCH.into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await;
    assert!(
        landing.is_err() || !merged(landing.as_ref().unwrap().get_ref(), false),
        "{landing:?}"
    );
    assert_eq!(world.stored("owner").await, before);
    assert_eq!(
        world.incoming_as(ADMIN, &visible, "doomed").await.unwrap(),
        []
    );

    let mut edited = baseline.clone();
    component_mut(&mut edited).title = "baseline edit".into();
    world.put(edited).await;
    let resolution = world
        .resolve(
            "owner",
            pb::resolve_branch_entry_request::Resolution::KeepOurs,
        )
        .await;
    let landing = world
        .client
        .merge_branch(request(
            pb::MergeBranchRequest {
                name: BRANCH.into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await;
    assert!(
        resolution.is_err()
            || landing.is_err()
            || !merged(landing.as_ref().unwrap().get_ref(), false),
        "keeping a branch value that targets a deleted entity must not land"
    );
    assert_eq!(
        world.incoming_as(ADMIN, &visible, "doomed").await.unwrap(),
        []
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_owner_deletion_and_restore_renew_group_identity() {
    let mut world = world().await;
    let visible = world.visible.clone();
    world.put(component(&visible, "target", vec![])).await;
    let baseline = world
        .put(component(
            &visible,
            "owner",
            vec![group("related", vec![item(target(&visible, "target"))])],
        ))
        .await
        .entity
        .unwrap();
    let ids = item_ids(&baseline);
    world.create_branch().await;
    world
        .delete_as(
            ADMIN,
            Some(BRANCH),
            &visible,
            "owner",
            pb::delete_entity_request::Mode::Force,
        )
        .await
        .unwrap();
    assert!(world
        .get_as(ADMIN, Some(BRANCH), &visible, "owner")
        .await
        .is_err_and(|status| status.code() == Code::NotFound));
    assert_eq!(world.stored("owner").await.entity.unwrap(), baseline);

    let mut restored = baseline.clone();
    restored.system = None;
    let restored = world.put_on_branch(restored).await;
    assert_ne!(uid(&restored), uid(&baseline));

    let landed = world.merge(false).await;
    assert!(merged(&landed, false), "{landed:?}");
    let stored = world.stored("owner").await.entity.unwrap();
    assert_eq!(uid(&stored), uid(&restored));
    assert!(
        !world.bound("target", &baseline, &ids[0]).await,
        "the deleted incarnation's selector never binds to the restored owner"
    );
    assert_eq!(
        world
            .incoming_as(ADMIN, &visible, "target")
            .await
            .unwrap()
            .len(),
        1
    );
}
