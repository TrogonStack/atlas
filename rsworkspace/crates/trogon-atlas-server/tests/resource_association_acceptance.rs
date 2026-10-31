#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use prost::Message;
use tokio::net::TcpListener;
use tonic::{service::interceptor::InterceptedService, transport::Channel, Code, Request};
use trogon_atlas_client::compat::{self, CompatibilityError, MutationIntent};
use trogon_atlas_core::specification::content_digest;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "assoc-accept-admin";
const ARTIFACT: &str = "44444444-4444-4444-8444-444444444444";
const SCHEMA: &str = "33333333-3333-4333-8333-333333333333";
const MESSAGE_A: &str = "11111111-1111-4111-8111-111111111111";
const MESSAGE_B: &str = "22222222-2222-4222-8222-222222222222";
const INPUT_PORT: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const OUTPUT_PORT: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

type Client = EventModelServiceClient<Channel>;

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::now_v7())
}

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn member(value: &str) -> pb::ContractMemberId {
    pb::ContractMemberId {
        value: value.into(),
    }
}

fn member_name(value: &str) -> pb::MemberName {
    pb::MemberName {
        value: value.into(),
    }
}

fn env(value: &str) -> pb::EnvironmentName {
    pb::EnvironmentName {
        value: value.into(),
    }
}

fn entity(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}

fn request<T>(body: T) -> Request<T> {
    let mut req = Request::new(body);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {ADMIN}").parse().unwrap());
    req
}

async fn client() -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let registry = TokenRegistry::from_file_contents(
        toml::from_str(&format!(
            r#"
[principals.admin]
role = "admin"
tokens = ["{ADMIN}"]
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
    Client::connect(format!("http://{addr}")).await.unwrap()
}

async fn put(client: &mut Client, kind: pb::entity::Kind) -> pb::Entity {
    client
        .put_entity(request(pb::PutEntityRequest {
            entity: Some(entity(kind)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

async fn fetch(client: &mut Client, kind: i32, id: &pb::Id) -> (pb::Entity, String) {
    let response = client
        .get_entity(request(pb::GetEntityRequest {
            kind,
            id: Some(id.clone()),
        }))
        .await
        .unwrap()
        .into_inner();
    (response.entity.unwrap(), response.etag)
}

async fn reject(client: &mut Client, kind: pb::entity::Kind) -> tonic::Status {
    client
        .put_entity(request(pb::PutEntityRequest {
            entity: Some(entity(kind)),
            ..Default::default()
        }))
        .await
        .unwrap_err()
}

async fn assignments(
    client: &mut Client,
    selected: pb::ResponsibilitySubject,
) -> pb::ListSubjectAssignmentsResponse {
    client
        .list_subject_assignments(request(pb::ListSubjectAssignmentsRequest {
            subject: Some(selected),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
}

async fn inspect(client: &mut Client, product_id: pb::Id) -> pb::InspectDataProductResponse {
    client
        .inspect_data_product(request(pb::InspectDataProductRequest {
            product: Some(pb::DataProductRef {
                id: Some(product_id),
            }),
        }))
        .await
        .unwrap()
        .into_inner()
}

fn spec_of(entity: &pb::Entity) -> &pb::Specification {
    let pb::entity::Kind::Specification(value) = entity.kind.as_ref().unwrap() else {
        panic!("specification");
    };
    value
}

fn spec_mut(entity: &mut pb::Entity) -> &mut pb::Specification {
    let pb::entity::Kind::Specification(value) = entity.kind.as_mut().unwrap() else {
        panic!("specification");
    };
    value
}

fn product_of(entity: &pb::Entity) -> &pb::DataProduct {
    let pb::entity::Kind::DataProduct(value) = entity.kind.as_ref().unwrap() else {
        panic!("product");
    };
    value
}

fn product_mut(entity: &mut pb::Entity) -> &mut pb::DataProduct {
    let pb::entity::Kind::DataProduct(value) = entity.kind.as_mut().unwrap() else {
        panic!("product");
    };
    value
}

fn binding(
    role: pb::schema_binding::Role,
    kind: pb::schema_binding::Type,
    environments: &[&str],
    is_default: Option<bool>,
) -> pb::SchemaBinding {
    pb::SchemaBinding {
        role: role as i32,
        r#type: Some(kind),
        environments: environments.iter().copied().map(env).collect(),
        is_default,
        ..Default::default()
    }
}

fn artifact(content: &str) -> pb::ContractArtifact {
    pb::ContractArtifact {
        member_id: Some(member(ARTIFACT)),
        name: Some(member_name("Source")),
        path: Some(pb::ArtifactPath {
            value: "contract.txt".into(),
        }),
        format: Some(pb::ContractFormat {
            kind: Some(pb::contract_format::Kind::Other("opaque".into())),
        }),
        media_type: Some(pb::MediaType {
            value: "text/plain".into(),
        }),
        content: content.into(),
        digest: Some(content_digest(content)),
        ..Default::default()
    }
}

fn local_schema() -> pb::ContractSchema {
    pb::ContractSchema {
        member_id: Some(member(SCHEMA)),
        name: Some(member_name("Shape")),
        definition: Some(pb::SchemaDefinition {
            target: Some(pb::schema_definition::Target::Artifact(
                pb::LocalArtifactSelection {
                    artifact_id: Some(member(ARTIFACT)),
                    expected_digest: Some(content_digest("schema\n")),
                },
            )),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn connection(owner: &pb::Entity, id: Option<pb::ConnectionId>) -> pb::ConnectionRef {
    pb::ConnectionRef {
        owner: Some(trogon_atlas_server::conv::entity_to_ref(owner).unwrap()),
        owner_uid: owner.system.as_ref().unwrap().uid.clone(),
        id,
    }
}

fn association_key(value: &pb::ProductResourceAssociation) -> Vec<u8> {
    let mut copy = value.clone();
    copy.connection_id = None;
    copy.doc = None;
    copy.encode_to_vec()
}

#[tokio::test(flavor = "multi_thread")]
async fn schema_variants_keep_scope_case_defaults_and_mixed_targets() {
    let mut client = client().await;
    let namespace = unique("variants");
    let shape = id(&namespace, "shape");
    let library_id = id(&namespace, "orders");
    let pinned_id = id(&namespace, "pinned");
    let spec_id = id(&namespace, "contract");
    put(
        &mut client,
        pb::entity::Kind::Schema(pb::Schema {
            id: Some(shape.clone()),
            title: "Shape".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::TypeLibrary(pb::TypeLibrary {
            id: Some(library_id.clone()),
            title: "Types".into(),
            files: vec![pb::ProtoSourceFile {
                path: "orders.proto".into(),
                content: "syntax = \"proto3\"; package orders.v1; message Order { string id = 1; }"
                    .into(),
            }],
            ..Default::default()
        }),
    )
    .await;
    let pinned = put(
        &mut client,
        pb::entity::Kind::Specification(pb::Specification {
            id: Some(pinned_id.clone()),
            title: "Pinned".into(),
            artifacts: vec![artifact("schema\n")],
            schemas: vec![local_schema()],
            ..Default::default()
        }),
    )
    .await;
    let mut names = Vec::new();
    for index in 0..32 {
        names.push(format!("e{index:02}"));
    }
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let native = pb::schema_binding::Type::Schema(pb::SchemaRef {
        id: Some(shape.clone()),
    });
    let library = pb::schema_binding::Type::LibraryType(pb::TypeLibraryTypeRef {
        library: Some(pb::TypeLibraryRef {
            id: Some(library_id),
        }),
        message: Some(pb::QualifiedProtoName {
            value: "orders.v1.Order".into(),
        }),
    });
    let contract = pb::schema_binding::Type::ContractType(pb::ContractTypeRef {
        schema: Some(pb::ContractMemberRef {
            specification: Some(pb::SpecificationRef {
                id: Some(pinned_id),
            }),
            owner_uid: pinned.system.as_ref().unwrap().uid.clone(),
            kind: pb::ContractMemberKind::Schema as i32,
            member_id: Some(member(SCHEMA)),
            expected_digest: None,
        }),
        expected_artifact_id: Some(member(ARTIFACT)),
        expected_digest: Some(content_digest("schema\n")),
        expected_selector: None,
    });
    let local = pb::schema_binding::Type::LocalSchema(member(SCHEMA));
    let mut payload_default = binding(pb::schema_binding::Role::Payload, local, &[], Some(true));
    payload_default.doc = Some("default payload".into());
    let mut wide = binding(
        pb::schema_binding::Role::Headers,
        pb::schema_binding::Type::Schema(pb::SchemaRef {
            id: Some(shape.clone()),
        }),
        &[],
        None,
    );
    wide.environments = vec![env(&"a".repeat(64))];
    let spec = put(
        &mut client,
        pb::entity::Kind::Specification(pb::Specification {
            id: Some(spec_id.clone()),
            title: "Contract".into(),
            artifacts: vec![artifact("schema\n")],
            schemas: vec![local_schema()],
            messages: vec![
                pb::ContractMessage {
                    member_id: Some(member(MESSAGE_A)),
                    name: Some(member_name("Order")),
                    schemas: vec![
                        binding(
                            pb::schema_binding::Role::Payload,
                            native.clone(),
                            &["Blue"],
                            None,
                        ),
                        binding(
                            pb::schema_binding::Role::Payload,
                            library,
                            &["blue"],
                            Some(false),
                        ),
                        binding(
                            pb::schema_binding::Role::Payload,
                            contract,
                            &name_refs,
                            None,
                        ),
                        payload_default,
                        binding(
                            pb::schema_binding::Role::Headers,
                            native.clone(),
                            &["Blue"],
                            None,
                        ),
                        binding(
                            pb::schema_binding::Role::Headers,
                            native.clone(),
                            &[],
                            Some(true),
                        ),
                        wide,
                    ],
                    ..Default::default()
                },
                pb::ContractMessage {
                    member_id: Some(member(MESSAGE_B)),
                    name: Some(member_name("Query")),
                    schemas: vec![
                        binding(
                            pb::schema_binding::Role::Payload,
                            native.clone(),
                            &["Blue"],
                            None,
                        ),
                        binding(pb::schema_binding::Role::Payload, native, &[], Some(true)),
                    ],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
    )
    .await;
    let stored = spec_of(&spec);
    let first = &stored.messages[0].schemas;
    assert_eq!(first[0].environments, vec![env("Blue")]);
    assert!(first[0].is_default.is_none());
    assert_eq!(first[1].environments, vec![env("blue")]);
    assert_eq!(first[1].is_default, Some(false));
    assert_eq!(first[2].environments.len(), 32);
    assert!(first[2].is_default.is_none());
    assert!(matches!(
        first[2].r#type,
        Some(pb::schema_binding::Type::ContractType(_))
    ));
    assert!(matches!(
        first[1].r#type,
        Some(pb::schema_binding::Type::LibraryType(_))
    ));
    assert!(matches!(
        first[0].r#type,
        Some(pb::schema_binding::Type::Schema(_))
    ));
    assert_eq!(first[3].is_default, Some(true));
    assert_eq!(first[3].environments, []);
    assert_eq!(first[4].role, pb::schema_binding::Role::Headers as i32);
    assert!(first[4].is_default.is_none());
    assert_eq!(first[4].environments, vec![env("Blue")]);
    assert_eq!(first[5].role, pb::schema_binding::Role::Headers as i32);
    assert_eq!(first[5].is_default, Some(true));
    assert_eq!(first[5].environments, []);
    assert_eq!(first[6].role, pb::schema_binding::Role::Headers as i32);
    assert!(first[6].is_default.is_none());
    assert_eq!(first[6].environments.len(), 1);
    assert_eq!(first[6].environments[0].value.len(), 64);
    assert_eq!(first[6].environments[0].value, "a".repeat(64));
    assert!(stored.messages[1].schemas[0].is_default.is_none());
    assert_eq!(
        stored.messages[1].schemas[0].environments,
        vec![env("Blue")]
    );
    assert_eq!(stored.messages[1].schemas[1].is_default, Some(true));
    assert_eq!(stored.messages[1].schemas[1].environments, []);
    let ids: std::collections::HashSet<_> = first
        .iter()
        .map(|value| value.connection_id.clone().unwrap().value)
        .collect();
    assert_eq!(ids.len(), 7);
    assert_ne!(first[0].is_default, Some(true));
    assert_ne!(
        stored.messages[0].schemas[0].connection_id,
        first[3].connection_id
    );

    let (before, etag) = fetch(&mut client, pb::EntityKind::Specification as i32, &spec_id).await;
    let propose = |index: usize, edit: &mut dyn FnMut(&mut pb::SchemaBinding)| {
        let mut rejected = before.clone();
        let binding = &mut spec_mut(&mut rejected).messages[0].schemas[index];
        edit(binding);
        binding.connection_id = None;
        spec_of(&rejected).clone()
    };
    let long = "a".repeat(65);
    let cases = [
        ("", "environment name must be 1..=64"),
        (" ", "environment name must be 1..=64"),
        ("\u{0001}", "environment name must be 1..=64"),
        (long.as_str(), "environment name must be 1..=64"),
    ];
    for (value, phrase) in cases {
        let error = reject(
            &mut client,
            pb::entity::Kind::Specification(propose(0, &mut |binding| {
                binding.environments = vec![env(value)];
            })),
        )
        .await;
        assert_eq!(error.code(), Code::InvalidArgument, "{value:?}: {error}");
        assert!(error.message().contains(phrase), "{error}");
    }
    let error = reject(
        &mut client,
        pb::entity::Kind::Specification(propose(0, &mut |binding| {
            binding.environments = vec![env("Blue"), env("Blue")];
        })),
    )
    .await;
    assert!(
        error
            .message()
            .contains("environment names must be unique within a schema binding"),
        "{error}"
    );
    let mut overlap = before.clone();
    spec_mut(&mut overlap).messages[0].schemas[1].environments = vec![env("Blue")];
    spec_mut(&mut overlap).messages[0].schemas[1].connection_id = None;
    let error = reject(
        &mut client,
        pb::entity::Kind::Specification(spec_of(&overlap).clone()),
    )
    .await;
    assert!(
        error
            .message()
            .contains("environment names must be unique per message and role"),
        "{error}"
    );
    let mut too_many = names.clone();
    too_many.push("e32".into());
    let too_many_refs: Vec<&str> = too_many.iter().map(String::as_str).collect();
    let error = reject(
        &mut client,
        pb::entity::Kind::Specification(propose(2, &mut |binding| {
            binding.environments = too_many_refs.iter().copied().map(env).collect();
        })),
    )
    .await;
    assert!(
        error
            .message()
            .contains("schema binding environment limit exceeded"),
        "{error}"
    );
    let error = reject(
        &mut client,
        pb::entity::Kind::Specification(propose(0, &mut |binding| {
            binding.environments.clear();
            binding.is_default = Some(true);
        })),
    )
    .await;
    assert!(
        error
            .message()
            .contains("at most one default schema binding per message and role"),
        "{error}"
    );
    let error = reject(
        &mut client,
        pb::entity::Kind::Specification(propose(3, &mut |binding| {
            binding.environments = vec![env("extra")];
        })),
    )
    .await;
    assert!(
        error
            .message()
            .contains("default schema binding cannot name environments"),
        "{error}"
    );
    let (after, now) = fetch(&mut client, pb::EntityKind::Specification as i32, &spec_id).await;
    assert_eq!(now, etag);
    assert_eq!(after, before);
}

#[tokio::test(flavor = "multi_thread")]
async fn default_and_environment_edits_renew_ids_and_retire_old_selectors() {
    let mut client = client().await;
    let namespace = unique("variant-ids");
    let shape = id(&namespace, "shape");
    let receipt = id(&namespace, "receipt");
    let spec_id = id(&namespace, "contract");
    put(
        &mut client,
        pb::entity::Kind::Schema(pb::Schema {
            id: Some(shape.clone()),
            title: "Shape".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(receipt.clone()),
            title: "Receipt".into(),
            ..Default::default()
        }),
    )
    .await;
    let native = pb::schema_binding::Type::Schema(pb::SchemaRef {
        id: Some(shape.clone()),
    });
    let spec = put(
        &mut client,
        pb::entity::Kind::Specification(pb::Specification {
            id: Some(spec_id.clone()),
            title: "Contract".into(),
            messages: vec![pb::ContractMessage {
                member_id: Some(member(MESSAGE_A)),
                name: Some(member_name("Order")),
                schemas: vec![
                    binding(
                        pb::schema_binding::Role::Payload,
                        native.clone(),
                        &[],
                        Some(true),
                    ),
                    binding(
                        pb::schema_binding::Role::Headers,
                        native,
                        &["staging"],
                        None,
                    ),
                ],
                mapping: Some(pb::MessageReadModelMapping {
                    read_model: Some(pb::ReadModelRef { id: Some(receipt) }),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            operations: vec![pb::ContractOperation {
                member_id: Some(member("55555555-5555-4555-8555-555555555555")),
                name: Some(member_name("Send")),
                messages: vec![pb::OperationMessage {
                    message_id: Some(member(MESSAGE_A)),
                    role: pb::operation_message::Role::Request as i32,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let payload_id = spec_of(&spec).messages[0].schemas[0]
        .connection_id
        .clone()
        .unwrap();
    let header_id = spec_of(&spec).messages[0].schemas[1]
        .connection_id
        .clone()
        .unwrap();
    let subject = |binding_id: pb::ConnectionId| pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Connection(
            connection(&spec, Some(binding_id)),
        )),
    };
    let team_id = id(&namespace, "owners");
    put(
        &mut client,
        pb::entity::Kind::Team(pb::Team {
            id: Some(team_id.clone()),
            title: Some(pb::DirectoryDisplayName {
                value: "Owners".into(),
            }),
            responsibilities: vec![
                pb::Responsibility {
                    kind: pb::ResponsibilityKind::Owner as i32,
                    subject: Some(subject(payload_id.clone())),
                    ..Default::default()
                },
                pb::Responsibility {
                    kind: pb::ResponsibilityKind::Owner as i32,
                    subject: Some(subject(header_id.clone())),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
    )
    .await;
    let resolved = assignments(&mut client, subject(payload_id.clone())).await;
    assert_eq!(
        resolved.subject_state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );
    assert_eq!(resolved.assignments.len(), 1);

    let mut edited = spec.clone();
    spec_mut(&mut edited).messages[0].schemas[0].doc = Some("kept".into());
    spec_mut(&mut edited).messages[0].schemas.swap(0, 1);
    let saved = put(
        &mut client,
        pb::entity::Kind::Specification(spec_of(&edited).clone()),
    )
    .await;
    let schemas = &spec_of(&saved).messages[0].schemas;
    assert_eq!(schemas[0].connection_id.as_ref(), Some(&header_id));
    assert_eq!(schemas[1].connection_id.as_ref(), Some(&payload_id));
    assert_eq!(schemas[1].doc.as_deref(), Some("kept"));
    assert_eq!(schemas[1].is_default, Some(true));
    assert_eq!(
        spec_of(&saved).messages[0]
            .mapping
            .as_ref()
            .unwrap()
            .read_model,
        spec_of(&spec).messages[0]
            .mapping
            .as_ref()
            .unwrap()
            .read_model
    );
    assert_eq!(
        spec_of(&saved).operations[0].messages[0].role,
        pb::operation_message::Role::Request as i32
    );
    assert_eq!(
        assignments(&mut client, subject(payload_id.clone()))
            .await
            .subject_state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );

    let mut retained = saved.clone();
    spec_mut(&mut retained).messages[0].schemas[1].is_default = Some(false);
    let error = reject(
        &mut client,
        pb::entity::Kind::Specification(spec_of(&retained).clone()),
    )
    .await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    assert!(
        error
            .message()
            .contains("changed role or endpoint; use a new ID"),
        "{error}"
    );
    let (unchanged, _) = fetch(&mut client, pb::EntityKind::Specification as i32, &spec_id).await;
    assert_eq!(unchanged, saved);

    let mut renewed = saved.clone();
    let binding = &mut spec_mut(&mut renewed).messages[0].schemas[1];
    binding.is_default = Some(false);
    binding.connection_id = None;
    let renewed = put(
        &mut client,
        pb::entity::Kind::Specification(spec_of(&renewed).clone()),
    )
    .await;
    let new_payload = spec_of(&renewed).messages[0].schemas[1]
        .connection_id
        .clone()
        .unwrap();
    assert_ne!(new_payload, payload_id);
    assert_eq!(
        spec_of(&renewed).messages[0].schemas[0]
            .connection_id
            .as_ref(),
        Some(&header_id)
    );
    let stale = assignments(&mut client, subject(payload_id.clone())).await;
    assert_eq!(
        stale.subject_state,
        pb::ResponsibilityResolutionState::Unresolved as i32
    );
    assert_eq!(
        stale.assignments[0]
            .assignment
            .as_ref()
            .unwrap()
            .subject
            .as_ref()
            .unwrap()
            .selection,
        subject(payload_id).selection
    );
    let rebound = assignments(&mut client, subject(new_payload)).await;
    assert_eq!(
        rebound.subject_state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );
    assert_eq!(rebound.assignments, []);

    let mut header = renewed.clone();
    spec_mut(&mut header).messages[0].schemas[0].environments = vec![env("other")];
    let error = reject(
        &mut client,
        pb::entity::Kind::Specification(spec_of(&header).clone()),
    )
    .await;
    assert!(
        error
            .message()
            .contains("changed role or endpoint; use a new ID"),
        "{error}"
    );
    let mut header = renewed.clone();
    let binding = &mut spec_mut(&mut header).messages[0].schemas[0];
    binding.environments = vec![env("other")];
    binding.connection_id = None;
    let header = put(
        &mut client,
        pb::entity::Kind::Specification(spec_of(&header).clone()),
    )
    .await;
    let new_header = spec_of(&header).messages[0].schemas[0]
        .connection_id
        .clone()
        .unwrap();
    assert_ne!(new_header, header_id);
    let stale_header = assignments(&mut client, subject(header_id)).await;
    assert_eq!(
        stale_header.subject_state,
        pb::ResponsibilityResolutionState::Unresolved as i32
    );
    assert_eq!(
        assignments(&mut client, subject(new_header))
            .await
            .assignments,
        []
    );
    assert_eq!(
        spec_of(&header).operations[0].messages[0].role,
        pb::operation_message::Role::Request as i32
    );
    assert_eq!(
        spec_of(&header).messages[0].mapping,
        spec_of(&spec).messages[0].mapping
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn product_associations_cover_directions_targets_and_native_controls() {
    let mut client = client().await;
    let namespace = unique("products");
    let event = id(&namespace, "placed");
    let command = id(&namespace, "place");
    let receipt = id(&namespace, "receipt");
    let processor = id(&namespace, "worker");
    let component = id(&namespace, "checkout");
    let external = id(&namespace, "partner");
    let shape = id(&namespace, "shape");
    let store_id = id(&namespace, "cache");
    let channel_id = id(&namespace, "orders");
    let spec_id = id(&namespace, "contract");
    let product_id = id(&namespace, "offering");
    for kind in [
        pb::entity::Kind::Event(pb::Event {
            id: Some(event.clone()),
            title: "Placed".into(),
            ..Default::default()
        }),
        pb::entity::Kind::Command(pb::Command {
            id: Some(command.clone()),
            title: "Place".into(),
            ..Default::default()
        }),
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(receipt.clone()),
            title: "Receipt".into(),
            ..Default::default()
        }),
        pb::entity::Kind::Processor(pb::Processor {
            id: Some(processor.clone()),
            title: "Worker".into(),
            ..Default::default()
        }),
        pb::entity::Kind::Component(pb::Component {
            id: Some(component.clone()),
            title: "Checkout".into(),
            ..Default::default()
        }),
        pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
            id: Some(external.clone()),
            title: "Partner".into(),
            ..Default::default()
        }),
        pb::entity::Kind::Schema(pb::Schema {
            id: Some(shape.clone()),
            title: "Shape".into(),
            ..Default::default()
        }),
    ] {
        put(&mut client, kind).await;
    }
    let store = put(
        &mut client,
        pb::entity::Kind::DataStore(pb::DataStore {
            id: Some(store_id.clone()),
            title: "Cache".into(),
            store_class: Some(pb::StoreClass {
                kind: Some(pb::store_class::Kind::Known(
                    pb::store_class::Known::Cache as i32,
                )),
            }),
            authoritative: Some(false),
            attachments: vec![pb::ReadModelAttachment {
                read_model: Some(pb::ReadModelRef {
                    id: Some(receipt.clone()),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let pb::entity::Kind::DataStore(store_value) = store.kind.as_ref().unwrap() else {
        panic!("store");
    };
    let automation = put(
        &mut client,
        pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id(&namespace, "observe")),
            title: "Observe".into(),
            source_read_models: vec![pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: Some(receipt.clone()),
                }),
                ..Default::default()
            }],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(processor),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await;
    let pb::entity::Kind::AutomationSlice(slice) = automation.kind.as_ref().unwrap() else {
        panic!("automation");
    };
    put(
        &mut client,
        pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
            id: Some(id(&namespace, "project")),
            title: "Project".into(),
            source_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(event.clone()),
                }),
                ..Default::default()
            }],
            read_model: Some(pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: Some(receipt.clone()),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(&namespace, "handle")),
            title: "Handle".into(),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(command.clone()),
                }),
                ..Default::default()
            }),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(event.clone()),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::Channel(pb::Channel {
            id: Some(channel_id.clone()),
            title: "Orders".into(),
            bindings: vec![pb::ChannelBinding {
                observation: Some(connection(
                    &automation,
                    slice.source_read_models[0].connection_id.clone(),
                )),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let spec = put(
        &mut client,
        pb::entity::Kind::Specification(pb::Specification {
            id: Some(spec_id.clone()),
            title: "Contract".into(),
            messages: vec![pb::ContractMessage {
                member_id: Some(member(MESSAGE_A)),
                name: Some(member_name("Order")),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let message = pb::product_resource_association::Target::Message(pb::ContractMemberRef {
        specification: Some(pb::SpecificationRef { id: Some(spec_id) }),
        owner_uid: spec.system.as_ref().unwrap().uid.clone(),
        kind: pb::ContractMemberKind::Message as i32,
        member_id: Some(member(MESSAGE_A)),
        expected_digest: None,
    });
    let targets = [
        pb::product_resource_association::Target::Event(pb::EventRef {
            id: Some(event.clone()),
        }),
        pb::product_resource_association::Target::Command(pb::CommandRef { id: Some(command) }),
        message,
        pb::product_resource_association::Target::Component(pb::ComponentRef {
            id: Some(component),
        }),
        pb::product_resource_association::Target::ExternalSystem(pb::ExternalSystemRef {
            id: Some(external),
        }),
        pb::product_resource_association::Target::Channel(pb::ChannelRef {
            id: Some(channel_id),
        }),
        pb::product_resource_association::Target::DataStore(pb::DataStoreRef {
            id: Some(store_id.clone()),
        }),
    ];
    let contract = Some(pb::ProductResourceContract {
        r#type: Some(pb::product_resource_contract::Type::Schema(pb::SchemaRef {
            id: Some(shape.clone()),
        })),
    });
    let mut associations = Vec::new();
    for target in targets {
        associations.push(pb::ProductResourceAssociation {
            direction: pb::DataPortDirection::Input as i32,
            target: Some(target.clone()),
            doc: Some("descriptive".into()),
            ..Default::default()
        });
        associations.push(pb::ProductResourceAssociation {
            direction: pb::DataPortDirection::Output as i32,
            target: Some(target),
            contract: if associations.len() == 1 {
                contract.clone()
            } else {
                None
            },
            doc: Some("descriptive".into()),
            ..Default::default()
        });
    }
    let ports = pb::DataProduct {
        id: Some(product_id.clone()),
        title: "Offering".into(),
        inputs: vec![pb::DataInputPort {
            port_id: Some(pb::DataPortId {
                value: INPUT_PORT.into(),
            }),
            name: Some(pb::PortName {
                value: "Placed".into(),
            }),
            data: Some(pb::InputDataSelection {
                target: Some(pb::input_data_selection::Target::Event(pb::EventRef {
                    id: Some(event),
                })),
                ..Default::default()
            }),
            ..Default::default()
        }],
        outputs: vec![pb::DataOutputPort {
            port_id: Some(pb::DataPortId {
                value: OUTPUT_PORT.into(),
            }),
            name: Some(pb::PortName {
                value: "Receipt".into(),
            }),
            data: Some(pb::OutputDataSelection {
                read_model: Some(pb::ReadModelRef { id: Some(receipt) }),
                ..Default::default()
            }),
            contract: Some(pb::OutputContract {
                r#type: Some(pb::output_contract::Type::Schema(pb::SchemaRef {
                    id: Some(shape),
                })),
                ..Default::default()
            }),
            access: vec![pb::OutputAccess {
                selection: Some(pb::output_access::Selection::Store(pb::StoreAccess {
                    store: Some(pb::DataStoreRef { id: Some(store_id) }),
                    attachment: Some(connection(
                        &store,
                        store_value.attachments[0].connection_id.clone(),
                    )),
                })),
                ..Default::default()
            }],
            derived_from: Some(pb::DeclaredLineage {
                inputs: vec![pb::LineageInput {
                    input_port_id: Some(pb::DataPortId {
                        value: INPUT_PORT.into(),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    };
    let bare = put(&mut client, pb::entity::Kind::DataProduct(ports.clone())).await;
    let before = inspect(&mut client, product_id.clone()).await;
    assert!(!before.producing.is_empty(), "{before:?}");
    assert_eq!(
        before.ports[1].access[0].state,
        pb::DataProductSelectionState::Resolved as i32
    );
    assert_eq!(
        before.ports[1].lineage,
        pb::DataProductSelectionState::Declared as i32
    );
    assert_eq!(
        before.ports[1].contract,
        pb::DataProductSelectionState::Resolved as i32
    );
    let native_ports = (
        product_of(&bare).inputs.clone(),
        product_of(&bare).outputs.clone(),
    );
    let mut declared = product_of(&bare).clone();
    declared.resource_associations = associations;
    let declared = put(&mut client, pb::entity::Kind::DataProduct(declared)).await;
    let product = product_of(&declared);
    assert_eq!(product.resource_associations.len(), 14);
    assert_eq!(
        product
            .resource_associations
            .iter()
            .filter(|value| value.direction == pb::DataPortDirection::Input as i32)
            .count(),
        7
    );
    assert_eq!(
        product
            .resource_associations
            .iter()
            .filter(|value| value.direction == pb::DataPortDirection::Output as i32)
            .count(),
        7
    );
    assert!(product.resource_associations[1].contract.is_some());
    assert!(product.resource_associations[0].contract.is_none());
    let ids: std::collections::HashSet<_> = product
        .resource_associations
        .iter()
        .map(|value| value.connection_id.clone().unwrap().value)
        .collect();
    assert_eq!(ids.len(), 14);
    assert_eq!(
        (product.inputs.clone(), product.outputs.clone()),
        native_ports
    );
    let after = inspect(&mut client, product_id.clone()).await;
    assert_eq!(after.producing, before.producing);
    assert_eq!(
        after.ports[1].access[0].state,
        before.ports[1].access[0].state
    );
    assert_eq!(after.ports[1].lineage, before.ports[1].lineage);
    assert_eq!(after.ports[1].contract, before.ports[1].contract);
    assert_eq!(after.ports[1].data, before.ports[1].data);

    let mut reordered = declared.clone();
    let value = product_mut(&mut reordered);
    value.resource_associations.reverse();
    for association in &mut value.resource_associations {
        association.doc = Some("reworded".into());
    }
    let reordered = put(
        &mut client,
        pb::entity::Kind::DataProduct(product_of(&reordered).clone()),
    )
    .await;
    let mut before_ids = std::collections::HashMap::new();
    for association in &product_of(&declared).resource_associations {
        before_ids.insert(
            association_key(association),
            association.connection_id.clone(),
        );
    }
    for association in &product_of(&reordered).resource_associations {
        assert_eq!(association.doc.as_deref(), Some("reworded"));
        assert_eq!(
            before_ids.get(&association_key(association)),
            Some(&association.connection_id)
        );
    }
    assert_eq!(
        (
            product_of(&reordered).inputs.clone(),
            product_of(&reordered).outputs.clone()
        ),
        native_ports
    );

    let renew = |source: &pb::Entity,
                 change: &mut dyn FnMut(&mut pb::ProductResourceAssociation)| {
        let mut next = source.clone();
        change(&mut product_mut(&mut next).resource_associations[0]);
        next
    };
    let mut kept = reordered.clone();
    product_mut(&mut kept).resource_associations[0].direction = pb::DataPortDirection::Input as i32;
    product_mut(&mut kept).resource_associations[0].contract = None;
    let error = reject(
        &mut client,
        pb::entity::Kind::DataProduct(product_of(&kept).clone()),
    )
    .await;
    assert!(
        error
            .message()
            .contains("changed role or endpoint; use a new ID"),
        "{error}"
    );
    let fresh = renew(&reordered, &mut |association| {
        association.direction = pb::DataPortDirection::Input as i32;
        association.contract = None;
        association.connection_id = None;
    });
    let fresh = put(
        &mut client,
        pb::entity::Kind::DataProduct(product_of(&fresh).clone()),
    )
    .await;
    assert_ne!(
        product_of(&fresh).resource_associations[0].connection_id,
        product_of(&reordered).resource_associations[0].connection_id
    );
    assert_eq!(
        product_of(&fresh).resource_associations[1].connection_id,
        product_of(&reordered).resource_associations[1].connection_id
    );

    let old_target = product_of(&fresh).resource_associations[1]
        .target
        .clone()
        .unwrap();
    let mut retarget = fresh.clone();
    product_mut(&mut retarget).resource_associations[1].target = Some(
        pb::product_resource_association::Target::Event(pb::EventRef {
            id: Some(id(&namespace, "placed")),
        }),
    );
    let error = reject(
        &mut client,
        pb::entity::Kind::DataProduct(product_of(&retarget).clone()),
    )
    .await;
    assert!(
        error
            .message()
            .contains("changed role or endpoint; use a new ID"),
        "{error}"
    );
    product_mut(&mut retarget).resource_associations[1].connection_id = None;
    let retarget = put(
        &mut client,
        pb::entity::Kind::DataProduct(product_of(&retarget).clone()),
    )
    .await;
    assert_ne!(
        product_of(&retarget).resource_associations[1].connection_id,
        product_of(&fresh).resource_associations[1].connection_id
    );
    assert_ne!(
        product_of(&retarget).resource_associations[1].target,
        Some(old_target)
    );

    let (stored, stored_etag) =
        fetch(&mut client, pb::EntityKind::DataProduct as i32, &product_id).await;
    assert_eq!(stored, retarget);
    let mut contract_edit = stored.clone();
    let index = product_of(&contract_edit)
        .resource_associations
        .iter()
        .position(|association| {
            association.direction == pb::DataPortDirection::Output as i32
                && association.contract.is_none()
        })
        .expect("output association without a contract");
    let kept_target = product_of(&contract_edit).resource_associations[index]
        .target
        .clone();
    let kept_id = product_of(&contract_edit).resource_associations[index]
        .connection_id
        .clone();
    let edited = &mut product_mut(&mut contract_edit).resource_associations[index];
    edited.contract = contract.clone();
    assert_eq!(edited.direction, pb::DataPortDirection::Output as i32);
    assert_eq!(edited.target, kept_target);
    let error = reject(
        &mut client,
        pb::entity::Kind::DataProduct(product_of(&contract_edit).clone()),
    )
    .await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    assert!(
        error
            .message()
            .contains("changed role or endpoint; use a new ID"),
        "{error}"
    );
    let (unchanged, unchanged_etag) =
        fetch(&mut client, pb::EntityKind::DataProduct as i32, &product_id).await;
    assert_eq!(unchanged_etag, stored_etag);
    assert_eq!(unchanged, stored);
    let inventory: Vec<_> = product_of(&unchanged)
        .resource_associations
        .iter()
        .map(|association| association.connection_id.clone())
        .collect();
    let stored_inventory: Vec<_> = product_of(&stored)
        .resource_associations
        .iter()
        .map(|association| association.connection_id.clone())
        .collect();
    assert_eq!(inventory, stored_inventory);
    product_mut(&mut contract_edit).resource_associations[index].connection_id = None;
    let contract_edit = put(
        &mut client,
        pb::entity::Kind::DataProduct(product_of(&contract_edit).clone()),
    )
    .await;
    let mut renewed = 0;
    for (position, association) in product_of(&contract_edit)
        .resource_associations
        .iter()
        .enumerate()
    {
        if position == index {
            assert_ne!(association.connection_id, kept_id);
            assert_eq!(association.direction, pb::DataPortDirection::Output as i32);
            assert_eq!(association.target, kept_target);
            assert_eq!(association.contract, contract);
            renewed += 1;
        } else {
            assert_eq!(
                association.connection_id,
                product_of(&stored).resource_associations[position].connection_id
            );
        }
    }
    assert_eq!(renewed, 1);
    assert_eq!(
        (
            product_of(&contract_edit).inputs.clone(),
            product_of(&contract_edit).outputs.clone()
        ),
        native_ports
    );
    let finished = inspect(&mut client, product_id.clone()).await;
    assert_eq!(finished.producing, before.producing);
    assert_eq!(
        finished.ports[1].access[0].state,
        pb::DataProductSelectionState::Resolved as i32
    );
    assert_eq!(
        finished.ports[1].lineage,
        pb::DataProductSelectionState::Declared as i32
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn removed_variant_target_stays_readable_and_a_new_unresolved_binding_rejects() {
    let mut client = client().await;
    let namespace = unique("variant-dangling");
    let shape = id(&namespace, "shape");
    let spec_id = id(&namespace, "contract");
    put(
        &mut client,
        pb::entity::Kind::Schema(pb::Schema {
            id: Some(shape.clone()),
            title: "Shape".into(),
            ..Default::default()
        }),
    )
    .await;
    let saved = put(
        &mut client,
        pb::entity::Kind::Specification(pb::Specification {
            id: Some(spec_id.clone()),
            title: "Contract".into(),
            messages: vec![pb::ContractMessage {
                member_id: Some(member(MESSAGE_A)),
                name: Some(member_name("Order")),
                schemas: vec![binding(
                    pb::schema_binding::Role::Payload,
                    pb::schema_binding::Type::Schema(pb::SchemaRef {
                        id: Some(shape.clone()),
                    }),
                    &["Blue"],
                    None,
                )],
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let binding_id = spec_of(&saved).messages[0].schemas[0].connection_id.clone();
    client
        .delete_entity(request(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(shape.clone()),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    assert_eq!(
        client
            .get_entity(request(pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(shape),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let (readable, _) = fetch(&mut client, pb::EntityKind::Specification as i32, &spec_id).await;
    assert_eq!(
        spec_of(&readable).messages[0].schemas,
        spec_of(&saved).messages[0].schemas
    );
    assert_eq!(
        spec_of(&readable).messages[0].schemas[0].connection_id,
        binding_id
    );
    let mut renamed = readable.clone();
    spec_mut(&mut renamed).title = "Renamed contract".into();
    let renamed = put(
        &mut client,
        pb::entity::Kind::Specification(spec_of(&renamed).clone()),
    )
    .await;
    assert_eq!(spec_of(&renamed).title, "Renamed contract");
    assert_eq!(
        spec_of(&renamed).messages[0].schemas,
        spec_of(&readable).messages[0].schemas
    );
    let (kept, kept_etag) =
        fetch(&mut client, pb::EntityKind::Specification as i32, &spec_id).await;
    assert_eq!(kept, renamed);
    let mut unresolved = kept.clone();
    let edited = &mut spec_mut(&mut unresolved).messages[0].schemas[0];
    edited.connection_id = None;
    edited.r#type = Some(pb::schema_binding::Type::Schema(pb::SchemaRef {
        id: Some(id(&namespace, "missing")),
    }));
    let error = reject(
        &mut client,
        pb::entity::Kind::Specification(spec_of(&unresolved).clone()),
    )
    .await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    assert!(
        error
            .message()
            .contains("SPECIFICATION_REFERENCE_UNRESOLVED"),
        "{error}"
    );
    let (after, after_etag) =
        fetch(&mut client, pb::EntityKind::Specification as i32, &spec_id).await;
    assert_eq!(after_etag, kept_etag);
    assert_eq!(after, kept);
    assert_eq!(
        spec_of(&after).messages[0].schemas[0].connection_id,
        binding_id
    );
    assert_eq!(
        spec_of(&after).messages[0].schemas[0].environments,
        vec![env("Blue")]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn advertised_writer_floor_is_revision_19_and_get_sends_no_mutation() {
    let mut client = client().await;
    let namespace = unique("writer-floor");
    let spec_id = id(&namespace, "contract");
    let shape = id(&namespace, "shape");
    put(
        &mut client,
        pb::entity::Kind::Schema(pb::Schema {
            id: Some(shape.clone()),
            title: "Shape".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::Specification(pb::Specification {
            id: Some(spec_id.clone()),
            title: "Contract".into(),
            messages: vec![pb::ContractMessage {
                member_id: Some(member(MESSAGE_A)),
                name: Some(member_name("Order")),
                schemas: vec![binding(
                    pb::schema_binding::Role::Payload,
                    pb::schema_binding::Type::Schema(pb::SchemaRef { id: Some(shape) }),
                    &["Blue"],
                    Some(false),
                )],
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let (before, etag) = fetch(&mut client, pb::EntityKind::Specification as i32, &spec_id).await;
    let info = client
        .get_server_info(request(pb::GetServerInfoRequest {}))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(info.contract_revision, 19);
    assert_eq!(info.min_client_contract_revision, 19);
    compat::check(&info, MutationIntent::default()).unwrap();
    let refused =
        compat::writer_revision_decision(18, info.min_client_contract_revision).unwrap_err();
    assert!(matches!(
        refused,
        CompatibilityError::ClientTooOld {
            client: 18,
            minimum: 19
        }
    ));
    assert!(
        refused
            .to_string()
            .contains("upgrade this client before mutating"),
        "{refused}"
    );
    let (after, now) = fetch(&mut client, pb::EntityKind::Specification as i32, &spec_id).await;
    assert_eq!(now, etag);
    assert_eq!(after, before);
    assert_eq!(
        spec_of(&after).messages[0].schemas[0].environments,
        vec![env("Blue")]
    );
    assert_eq!(
        spec_of(&after).messages[0].schemas[0].is_default,
        Some(false)
    );
}
