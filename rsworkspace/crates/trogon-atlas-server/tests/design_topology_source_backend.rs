#![allow(clippy::unwrap_used, clippy::expect_used, clippy::large_futures)]

use std::{future::Future, sync::Arc};

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use tonic::{transport::Channel, Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{BearerAuth, RegistryFile, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "source-admin-token-aaaaaaaa";
struct Server {
    client: EventModelServiceClient<Channel>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn server(store: impl trogon_atlas_store::Store + 'static) -> impl Future<Output = Server> {
    Box::pin(async move {
        let registry: RegistryFile = toml::from_str(&format!(
            r#"
[principals.admin]
role="admin"
tokens=["{ADMIN}"]
"#
        ))
        .unwrap();
        let implementation =
            EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default())
                .unwrap()
                .with_snapshot_cap(0);
        let implementation = Arc::new(implementation);
        let grpc = EventModelServiceServer::from_arc(implementation.clone())
            .max_decoding_message_size(36 * 1024 * 1024)
            .max_encoding_message_size(36 * 1024 * 1024);
        let service = trogon_atlas_server::mcp::data_plane(
            grpc,
            BearerAuth::new(Arc::new(
                TokenRegistry::from_file_contents(registry).unwrap(),
            )),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(service)
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        let client = EventModelServiceClient::connect(format!("http://{addr}"))
            .await
            .unwrap()
            .max_decoding_message_size(36 * 1024 * 1024)
            .max_encoding_message_size(36 * 1024 * 1024);
        Server { client, task }
    })
}

fn req<T>(token: &str, value: T, branch: Option<&str>) -> Request<T> {
    let mut request = Request::new(value);
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    if let Some(branch) = branch {
        request
            .metadata_mut()
            .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    }
    request
}
async fn namespace(
    client: &mut EventModelServiceClient<Channel>,
    name: &str,
    parent: &str,
) -> String {
    client
        .register_namespace(req(
            ADMIN,
            pb::RegisterNamespaceRequest {
                name: name.into(),
                parent: parent.into(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id
}
async fn registration(
    client: &mut EventModelServiceClient<Channel>,
    id: &str,
    namespace: &str,
    budgets: Option<pb::SourceRunBudgets>,
    retention: Option<pb::SourceRetentionPolicy>,
) -> pb::SourceRegistration {
    client
        .create_source_registration(req(
            ADMIN,
            pb::CreateSourceRegistrationRequest {
                registration: Some(pb::SourceRegistration {
                    id: id.into(),
                    administration_namespace: namespace.into(),
                    destination_namespaces: vec![namespace.into()],
                    profile_revision: 1,
                    ownership_policy_revision: "policy-v1".into(),
                    deletion_policy: pb::SourceDeletionPolicy::RetainNative as i32,
                    budgets,
                    retention,
                    ..Default::default()
                }),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap()
}
fn inventory(registration: &str, version: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"profile":"supplied-native-v1","profile_revision":1,"registration":registration,"source_version":version,"partition":{"categories":["model"],"logical_roots":["native"],"selected_keys":[]},"coverage":"PARTIAL","resources":[{"key":{"category":"model","key":"read-model"},"file":"native/model","pointer":{"kind":"native","raw":"opaque"}}],"relations":[]})).unwrap()
}
fn source(registration: &str) -> pb::SourceResourceKey {
    pb::SourceResourceKey {
        registration_id: registration.into(),
        category: "model".into(),
        key: "read-model".into(),
    }
}

type Client = EventModelServiceClient<Channel>;

struct Fixture {
    namespace: String,
    registration: pb::SourceRegistration,
    snapshot: pb::SourceSnapshot,
}

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}
fn title(value: &str) -> pb::CatalogTitle {
    pb::CatalogTitle {
        value: value.into(),
    }
}
fn owner(namespace: &str, slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::DesignGraph as i32,
        id: Some(id(namespace, slug)),
    }
}
fn design(namespace: &str, slug: &str, steps: Vec<pb::TopologyStep>) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::DesignGraph(pb::DesignGraph {
            id: Some(id(namespace, slug)),
            title: Some(title("Native title")),
            profile: pb::DesignGraphProfile::Flow as i32,
            steps,
            ..Default::default()
        })),
        ..Default::default()
    }
}
fn step(name: &str) -> pb::TopologyStep {
    pb::TopologyStep {
        title: Some(title(name)),
        ..Default::default()
    }
}
fn graph(value: &pb::Entity) -> &pb::DesignGraph {
    match value.kind.as_ref() {
        Some(pb::entity::Kind::DesignGraph(graph)) => graph,
        _ => panic!("design graph"),
    }
}
fn graph_mut(value: &mut pb::Entity) -> &mut pb::DesignGraph {
    match value.kind.as_mut() {
        Some(pb::entity::Kind::DesignGraph(graph)) => graph,
        _ => panic!("design graph"),
    }
}
fn fields(values: &[pb::SourceOwnedField]) -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::Fields(
            pb::SourceOwnedFields {
                fields: values.iter().map(|value| *value as i32).collect(),
            },
        )),
    }
}
fn whole_owner() -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::WholeOwner(
            pb::SourceWholeOwnerOwnership {},
        )),
    }
}
fn no_ownership() -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::None(
            pb::SourceNoOwnership {},
        )),
    }
}

async fn fixture(client: &mut Client, name: &str) -> Fixture {
    let namespace = namespace(client, name, "visible-owner").await;
    let registration = registration(client, "design-source", &namespace, None, None).await;
    let snapshot = client
        .accept_source_snapshot(req(
            ADMIN,
            pb::AcceptSourceSnapshotRequest {
                registration_id: registration.id.clone(),
                registration_revision: registration.revision.clone(),
                inventory_bytes: inventory(&registration.id, "v1"),
                files: vec![pb::SourceFile {
                    logical_name: "native/model".into(),
                    media_type: "application/octet-stream".into(),
                    content: b"native supplied content".to_vec(),
                    ..Default::default()
                }],
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap();
    Fixture {
        namespace,
        registration,
        snapshot,
    }
}

async fn put(
    client: &mut Client,
    value: pb::Entity,
    if_match: String,
) -> Result<pb::PutEntityResponse, tonic::Status> {
    client
        .put_entity(req(
            ADMIN,
            pb::PutEntityRequest {
                entity: Some(value),
                if_match,
                ..Default::default()
            },
            None,
        ))
        .await
        .map(tonic::Response::into_inner)
}

async fn authored(client: &mut Client, namespace: &str, slug: &str) -> pb::PutEntityResponse {
    let first = put(
        client,
        design(namespace, slug, vec![step("A"), step("B"), step("C")]),
        String::new(),
    )
    .await
    .unwrap();
    let mut entity = first.entity.unwrap();
    let ids: Vec<_> = graph(&entity)
        .steps
        .iter()
        .map(|value| value.step_id.clone().unwrap())
        .collect();
    let value = graph_mut(&mut entity);
    value.entry_steps = vec![ids[0].clone()];
    value.steps[0].transitions = vec![
        pb::TopologyTransition {
            label: Some(title("next")),
            target_step_id: Some(ids[1].clone()),
            ..Default::default()
        },
        pb::TopologyTransition {
            label: Some(title("skip")),
            target_step_id: Some(ids[2].clone()),
            ..Default::default()
        },
    ];
    put(client, entity, first.etag).await.unwrap()
}

async fn create_mapping(
    client: &mut Client,
    fixture: &Fixture,
    selection: pb::SourceSelection,
    ownership: pb::SourceFieldOwnership,
    adoption: Option<pb::SourceAdoption>,
) -> Result<pb::SourceMapping, tonic::Status> {
    client
        .create_source_mapping(req(
            ADMIN,
            pb::CreateSourceMappingRequest {
                registration_revision: fixture.registration.revision.clone(),
                mapping: Some(pb::SourceMapping {
                    id: "design-mapping".into(),
                    source: Some(source(&fixture.registration.id)),
                    selection: Some(selection),
                    ownership: Some(ownership),
                    adoption,
                    ..Default::default()
                }),
            },
            None,
        ))
        .await
        .map(|response| response.into_inner().mapping.unwrap())
}
fn entity_selection(reference: pb::EntityRef) -> pb::SourceSelection {
    pb::SourceSelection {
        selection: Some(pb::source_selection::Selection::Entity(reference)),
    }
}
fn adoption(reference: pb::EntityRef, native: &pb::PutEntityResponse) -> pb::SourceAdoption {
    pb::SourceAdoption {
        authorized_adoption: true,
        observed: Some(pb::SourceObservedOwner {
            owner: Some(reference),
            uid: native
                .entity
                .as_ref()
                .unwrap()
                .system
                .as_ref()
                .unwrap()
                .uid
                .clone(),
            etag: native.etag.clone(),
        }),
    }
}
async fn try_preview(
    client: &mut Client,
    fixture: &Fixture,
    run: &str,
    value: pb::Entity,
) -> Result<pb::ReconciliationRun, tonic::Status> {
    client
        .preview_source_run(req(
            ADMIN,
            pb::PreviewSourceRunRequest {
                registration_id: fixture.registration.id.clone(),
                registration_revision: fixture.registration.revision.clone(),
                snapshot_id: fixture.snapshot.id.clone(),
                run_id: run.into(),
                native_payloads: vec![pb::SourceNativePayload {
                    source: Some(source(&fixture.registration.id)),
                    entity: Some(value),
                }],
                ..Default::default()
            },
            None,
        ))
        .await
        .map(|response| response.into_inner().run.unwrap())
}
async fn read(client: &mut Client, namespace: &str, slug: &str) -> Result<pb::Entity, Code> {
    client
        .get_entity(req(
            ADMIN,
            pb::GetEntityRequest {
                kind: pb::EntityKind::DesignGraph as i32,
                id: Some(id(namespace, slug)),
            },
            None,
        ))
        .await
        .map(|response| response.into_inner().entity.unwrap())
        .map_err(|status| status.code())
}
async fn latest(
    client: &mut Client,
    mapping: &pb::SourceMapping,
    registration: &pb::SourceRegistration,
) -> (pb::SourceMapping, pb::SourceRegistration) {
    let mapping = client
        .get_source_mapping(req(
            ADMIN,
            pb::GetSourceMappingRequest {
                registration_id: registration.id.clone(),
                mapping_id: mapping.id.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    let registration = client
        .get_source_registration(req(
            ADMIN,
            pb::GetSourceRegistrationRequest {
                registration_id: registration.id.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap();
    (mapping, registration)
}
async fn reapply(
    client: &mut Client,
    mapping: &pb::SourceMapping,
    registration: &pb::SourceRegistration,
    ownership: pb::SourceFieldOwnership,
) -> Result<pb::SourceMapping, tonic::Status> {
    let (mapping, registration) = latest(client, mapping, registration).await;
    let mut next = mapping.clone();
    next.ownership = Some(ownership);
    next.adoption = Some(pb::SourceAdoption {
        authorized_adoption: true,
        observed: mapping.observed.clone(),
    });
    client
        .update_source_mapping(req(
            ADMIN,
            pb::UpdateSourceMappingRequest {
                registration_revision: registration.revision.clone(),
                if_match: mapping.revision.clone(),
                mapping: Some(next),
            },
            None,
        ))
        .await
        .map(|response| response.into_inner().mapping.unwrap())
}

const NATIVE_ONLY: &str = "design topology support native authoring only";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn source_create_of_design_topology_rejects_under_every_ownership_policy() {
    let policies = [
        ("title-only", fields(&[pb::SourceOwnedField::Title])),
        (
            "unrelated-fields",
            fields(&[
                pb::SourceOwnedField::Title,
                pb::SourceOwnedField::Doc,
                pb::SourceOwnedField::Profile,
                pb::SourceOwnedField::LayoutDocument,
            ]),
        ),
        ("whole-owner", whole_owner()),
        ("none", no_ownership()),
    ];
    for (name, ownership) in policies {
        let nats = trogon_atlas_testsupport::shared().await;
        let mut server = server(nats.store().await).await;
        let fixture = fixture(&mut server.client, "source-design-create").await;
        let selection = entity_selection(owner(&fixture.namespace, "source-owned"));
        let mapping = create_mapping(
            &mut server.client,
            &fixture,
            selection.clone(),
            ownership,
            None,
        )
        .await
        .unwrap();
        assert!(mapping.base.is_none(), "{name}");

        let mut carrying = design(
            &fixture.namespace,
            "source-owned",
            vec![step("Imported A"), step("Imported B")],
        );
        graph_mut(&mut carrying).entry_steps = Vec::new();
        let error = try_preview(&mut server.client, &fixture, "carrying", carrying)
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{name}: {error}");
        assert!(error.message().contains(NATIVE_ONLY), "{name}: {error}");

        let empty = design(&fixture.namespace, "source-owned", Vec::new());
        match try_preview(&mut server.client, &fixture, "empty", empty).await {
            Err(error) => assert_eq!(error.code(), Code::InvalidArgument, "{name}: {error}"),
            Ok(run) => assert!(
                run.plan.is_none() && run.operation_count == 0,
                "{name}: {run:?}"
            ),
        }

        assert_eq!(
            read(&mut server.client, &fixture.namespace, "source-owned")
                .await
                .unwrap_err(),
            Code::NotFound,
            "{name}"
        );
        let after = server
            .client
            .get_source_mapping(req(
                ADMIN,
                pb::GetSourceMappingRequest {
                    registration_id: fixture.registration.id.clone(),
                    mapping_id: mapping.id.clone(),
                },
                None,
            ))
            .await
            .unwrap()
            .into_inner()
            .mapping
            .unwrap();
        assert_eq!(after, mapping, "{name}");
    }
}

async fn apply(client: &mut Client, run: &pb::ReconciliationRun) -> pb::ReconciliationRun {
    client
        .apply_source_run(req(
            ADMIN,
            pb::ApplySourceRunRequest {
                registration_id: run.registration_id.clone(),
                run_id: run.id.clone(),
                if_match: run.revision.clone(),
                preview_digest: run.preview_digest.clone(),
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap()
}
async fn adopt(
    client: &mut Client,
    mapping: &pb::SourceMapping,
    registration: &pb::SourceRegistration,
    adoption: pb::SourceAdoption,
) -> Result<pb::SourceMapping, tonic::Status> {
    let (mapping, registration) = latest(client, mapping, registration).await;
    let mut next = mapping.clone();
    next.adoption = Some(adoption);
    client
        .update_source_mapping(req(
            ADMIN,
            pb::UpdateSourceMappingRequest {
                registration_revision: registration.revision.clone(),
                if_match: mapping.revision.clone(),
                mapping: Some(next),
            },
            None,
        ))
        .await
        .map(|response| response.into_inner().mapping.unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn source_title_and_disposed_unrelated_fields_preserve_local_topology_exactly() {
    let nats = trogon_atlas_testsupport::shared().await;
    let mut server = server(nats.store().await).await;
    let fixture = fixture(&mut server.client, "source-design-update").await;
    let native = authored(&mut server.client, &fixture.namespace, "adopted").await;
    let native_entity = native.entity.clone().unwrap();
    let layout = server
        .client
        .put_entity(req(
            ADMIN,
            pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Document(pb::Document {
                        id: Some(id(&fixture.namespace, "layout")),
                        title: Some(title("Layout")),
                        body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
                            media_type: Some(pb::MediaType {
                                value: "text/markdown".into(),
                            }),
                            content: "layout".into(),
                            digest: Some(trogon_atlas_core::specification::content_digest(
                                "layout",
                            )),
                        })),
                        subjects: vec![pb::SubjectAssociation {
                            subject: Some(pb::CatalogSubject {
                                selection: Some(pb::catalog_subject::Selection::Entity(owner(
                                    &fixture.namespace,
                                    "adopted",
                                ))),
                            }),
                            ..Default::default()
                        }],
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    let layout_ref = pb::DocumentRef {
        id: trogon_atlas_core::refs::entity_id(layout.entity.as_ref().unwrap()).cloned(),
    };
    let reference = owner(&fixture.namespace, "adopted");
    let mapping = create_mapping(
        &mut server.client,
        &fixture,
        entity_selection(reference.clone()),
        fields(&[pb::SourceOwnedField::Title]),
        Some(adoption(reference.clone(), &native)),
    )
    .await
    .unwrap();

    let mut incoming = native_entity.clone();
    graph_mut(&mut incoming).title = Some(title("Source title"));
    graph_mut(&mut incoming).steps = vec![step("Source only")];
    graph_mut(&mut incoming).entry_steps.clear();
    graph_mut(&mut incoming).profile = pb::DesignGraphProfile::Diagram as i32;
    graph_mut(&mut incoming).layout_document = Some(layout_ref.clone());
    let run = try_preview(&mut server.client, &fixture, "title-update", incoming)
        .await
        .unwrap();
    let applied = apply(&mut server.client, &run).await;
    assert_eq!(applied.status, pb::SourceRunStatus::Applied as i32);
    let persisted = read(&mut server.client, &fixture.namespace, "adopted")
        .await
        .unwrap();
    assert_eq!(graph(&persisted).title, Some(title("Source title")));
    assert_eq!(graph(&persisted).steps, graph(&native_entity).steps);
    assert_eq!(
        graph(&persisted).entry_steps,
        graph(&native_entity).entry_steps
    );
    assert_eq!(
        graph(&persisted).profile,
        pb::DesignGraphProfile::Flow as i32
    );
    assert!(graph(&persisted).layout_document.is_none());

    let widened = reapply(
        &mut server.client,
        &mapping,
        &fixture.registration,
        fields(&[
            pb::SourceOwnedField::Title,
            pb::SourceOwnedField::Profile,
            pb::SourceOwnedField::LayoutDocument,
        ]),
    )
    .await
    .unwrap();
    let mut incoming = persisted.clone();
    graph_mut(&mut incoming).profile = pb::DesignGraphProfile::Diagram as i32;
    graph_mut(&mut incoming).layout_document = Some(layout_ref.clone());
    graph_mut(&mut incoming).steps.clear();
    graph_mut(&mut incoming).entry_steps.clear();
    let run = try_preview(&mut server.client, &fixture, "profile-layout", incoming)
        .await
        .unwrap();
    let applied = apply(&mut server.client, &run).await;
    assert_eq!(applied.status, pb::SourceRunStatus::Applied as i32);
    let persisted = read(&mut server.client, &fixture.namespace, "adopted")
        .await
        .unwrap();
    assert_eq!(
        graph(&persisted).profile,
        pb::DesignGraphProfile::Diagram as i32
    );
    assert_eq!(graph(&persisted).layout_document, Some(layout_ref));
    assert_eq!(graph(&persisted).steps, graph(&native_entity).steps);
    assert_eq!(
        graph(&persisted).entry_steps,
        graph(&native_entity).entry_steps
    );

    for closed in [
        vec![pb::SourceOwnedField::Steps],
        vec![pb::SourceOwnedField::EntrySteps],
        vec![pb::SourceOwnedField::Transitions],
        vec![pb::SourceOwnedField::Title, pb::SourceOwnedField::Steps],
    ] {
        let error = reapply(
            &mut server.client,
            &widened,
            &fixture.registration,
            fields(&closed),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{closed:?}: {error}");
    }
    let error = reapply(
        &mut server.client,
        &widened,
        &fixture.registration,
        whole_owner(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(error.message().contains(NATIVE_ONLY), "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn source_selections_cannot_address_steps_or_transitions_under_any_policy() {
    let nats = trogon_atlas_testsupport::shared().await;
    let mut server = server(nats.store().await).await;
    let fixture = fixture(&mut server.client, "source-design-selectors").await;
    let native = authored(&mut server.client, &fixture.namespace, "addressed").await;
    let entity = native.entity.unwrap();
    let transition = graph(&entity).steps[0].transitions[0]
        .connection_id
        .clone()
        .unwrap();
    for ownership in [
        fields(&[pb::SourceOwnedField::Title]),
        whole_owner(),
        no_ownership(),
    ] {
        let selection = pb::SourceSelection {
            selection: Some(pb::source_selection::Selection::Connection(
                pb::ConnectionRef {
                    owner: Some(owner(&fixture.namespace, "addressed")),
                    id: Some(transition.clone()),
                    owner_uid: entity.system.as_ref().unwrap().uid.clone(),
                },
            )),
        };
        let error = create_mapping(&mut server.client, &fixture, selection, ownership, None)
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    }
    let step = graph(&entity).steps[0].step_id.clone().unwrap();
    let error = create_mapping(
        &mut server.client,
        &fixture,
        pb::SourceSelection {
            selection: Some(pb::source_selection::Selection::Connection(
                pb::ConnectionRef {
                    owner: Some(owner(&fixture.namespace, "addressed")),
                    id: Some(pb::ConnectionId { value: step.value }),
                    ..Default::default()
                },
            )),
        },
        no_ownership(),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whole_owner_source_preview_rejects_current_and_cleared_incoming_topology() {
    let nats = trogon_atlas_testsupport::shared().await;
    let mut server = server(nats.store().await).await;
    let fixture = fixture(&mut server.client, "source-design-whole").await;
    let reference = owner(&fixture.namespace, "whole-owner");
    let mapping = create_mapping(
        &mut server.client,
        &fixture,
        entity_selection(reference.clone()),
        whole_owner(),
        None,
    )
    .await
    .unwrap();
    let native = authored(&mut server.client, &fixture.namespace, "whole-owner").await;
    let native_entity = native.entity.clone().unwrap();
    for (name, mutate) in [("identical", 0), ("cleared", 1), ("replaced", 2)] {
        let mut incoming = native_entity.clone();
        match mutate {
            1 => {
                graph_mut(&mut incoming).steps.clear();
                graph_mut(&mut incoming).entry_steps.clear();
            }
            2 => graph_mut(&mut incoming).steps = vec![step("Replacement")],
            _ => {}
        }
        let error = try_preview(&mut server.client, &fixture, name, incoming)
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{name}: {error}");
        assert!(error.message().contains(NATIVE_ONLY), "{name}: {error}");
    }
    let adopted = adopt(
        &mut server.client,
        &mapping,
        &fixture.registration,
        adoption(reference, &native),
    )
    .await
    .unwrap_err();
    assert_eq!(adopted.code(), Code::InvalidArgument);
    assert!(adopted.message().contains(NATIVE_ONLY), "{adopted}");
    let persisted = read(&mut server.client, &fixture.namespace, "whole-owner")
        .await
        .unwrap();
    assert_eq!(persisted, native_entity);
}
