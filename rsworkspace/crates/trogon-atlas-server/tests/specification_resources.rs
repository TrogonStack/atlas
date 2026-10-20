#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use tonic::{Code, Request};
use trogon_atlas_core::specification::{content_digest, is_api};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "contracts".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn name(value: &str) -> pb::MemberName {
    pb::MemberName {
        value: value.into(),
    }
}
fn entity(spec: pb::Specification) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Specification(spec)),
        ..Default::default()
    }
}
fn spec(entity: &pb::Entity) -> pb::Specification {
    match &entity.kind {
        Some(pb::entity::Kind::Specification(value)) => value.clone(),
        _ => panic!("Specification"),
    }
}
fn artifact(content: &str) -> pb::ContractArtifact {
    pb::ContractArtifact {
        name: Some(name("Source")),
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
fn declaration(slug: &str) -> pb::Specification {
    pb::Specification {
        id: Some(id(slug)),
        title: slug.into(),
        artifacts: vec![artifact("schema\r\n")],
        messages: vec![pb::ContractMessage {
            name: Some(name("Message")),
            ..Default::default()
        }],
        ..Default::default()
    }
}
async fn service() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default()).unwrap()
}
async fn save(service: &EventModelServiceImpl, spec: pb::Specification) -> pb::Entity {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity(spec)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}
async fn get(service: &EventModelServiceImpl, slug: &str) -> pb::Entity {
    service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Specification as i32,
            id: Some(id(slug)),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}
async fn invalid_every_surface(service: &EventModelServiceImpl, value: pb::Specification) {
    for preview in [true, false] {
        let error = service
            .put_entity(Request::new(pb::PutEntityRequest {
                entity: Some(entity(value.clone())),
                validate_only: preview,
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{error:?}");
        let error = service
            .batch_mutate(Request::new(pb::BatchMutateRequest {
                ops: vec![pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        entity: Some(entity(value.clone())),
                        ..Default::default()
                    })),
                }],
                validate_only: preview,
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{error:?}");
    }
}
async fn contract(service: &EventModelServiceImpl, slug: &str) -> pb::Entity {
    let first = save(service, declaration(slug)).await;
    let mut value = spec(&first);
    value.schemas.push(pb::ContractSchema {
        name: Some(name("Shape")),
        definition: Some(pb::SchemaDefinition {
            target: Some(pb::schema_definition::Target::Artifact(
                pb::LocalArtifactSelection {
                    artifact_id: value.artifacts[0].member_id.clone(),
                    expected_digest: value.artifacts[0].digest.clone(),
                },
            )),
            selector: Some(pb::OpaqueTypeSelector {
                syntax: "unrecognized".into(),
                value: "arbitrary opaque text".into(),
            }),
            ..Default::default()
        }),
        ..Default::default()
    });
    save(service, value).await
}
fn pin(owner: &pb::Entity) -> pb::ContractTypeRef {
    let value = spec(owner);
    pb::ContractTypeRef {
        schema: Some(pb::ContractMemberRef {
            specification: Some(pb::SpecificationRef {
                id: value.id.clone(),
            }),
            owner_uid: owner.system.as_ref().unwrap().uid.clone(),
            kind: pb::ContractMemberKind::Schema as i32,
            member_id: value.schemas[0].member_id.clone(),
            expected_digest: None,
        }),
        expected_artifact_id: value.artifacts[0].member_id.clone(),
        expected_digest: value.artifacts[0].digest.clone(),
        expected_selector: value.schemas[0]
            .definition
            .as_ref()
            .unwrap()
            .selector
            .clone(),
    }
}
fn consumer(reference: pb::ContractTypeRef) -> pb::Specification {
    let mut value = declaration("consumer");
    value.messages[0].schemas.push(pb::SchemaBinding {
        role: pb::schema_binding::Role::Payload as i32,
        r#type: Some(pb::schema_binding::Type::ContractType(reference)),
        ..Default::default()
    });
    value
}
fn branch<T>(value: T) -> Request<T> {
    let mut request = Request::new(value);
    request
        .metadata_mut()
        .insert("x-trogon-atlas-branch", "contract-work".parse().unwrap());
    request
}

#[tokio::test]
async fn exact_type_pin_checks_owner_kind_artifact_digest_and_selector_presence() {
    let service = service().await.with_snapshot_cap(0);
    let owner = contract(&service, "shared").await;
    let good = pin(&owner);
    save(&service, consumer(good.clone())).await;
    let mut cases = Vec::new();
    let mut value = good.clone();
    value.schema.as_mut().unwrap().owner_uid = "stale".into();
    cases.push(value);
    let mut value = good.clone();
    value.schema.as_mut().unwrap().kind = pb::ContractMemberKind::Artifact as i32;
    cases.push(value);
    let mut value = good.clone();
    value.expected_artifact_id = Some(trogon_atlas_core::specification::MemberId::new().into());
    cases.push(value);
    let mut value = good.clone();
    value.expected_digest.as_mut().unwrap().sha256 = "0".repeat(64);
    cases.push(value);
    let mut value = good.clone();
    value.expected_selector = None;
    cases.push(value);
    let mut value = good.clone();
    value.expected_selector.as_mut().unwrap().value = "changed".into();
    cases.push(value);
    let mut value = good;
    value.schema.as_mut().unwrap().expected_digest = value.expected_digest.clone();
    cases.push(value);
    let before = get(&service, "consumer").await;
    for reference in cases {
        let mut bad = spec(&before);
        bad.messages[0].schemas[0].r#type = Some(pb::schema_binding::Type::ContractType(reference));
        bad.messages[0].schemas[0].connection_id = None;
        invalid_every_surface(&service, bad).await;
        assert_eq!(get(&service, "consumer").await, before);
    }
}

#[tokio::test]
async fn malformed_values_limits_and_roles_are_atomic_on_public_mutations() {
    let service = service().await;
    let mut cases = Vec::new();
    let mut value = declaration("bad");
    value.artifacts[0].digest.as_mut().unwrap().sha256 = "0".repeat(64);
    cases.push(value);
    let mut value = declaration("bad");
    value.artifacts[0].path.as_mut().unwrap().value = "../secret".into();
    cases.push(value);
    let mut value = declaration("bad");
    value.artifacts[0].format.as_mut().unwrap().kind = Some(pb::contract_format::Kind::Known(999));
    cases.push(value);
    let mut value = declaration("bad");
    value.artifacts[0] = artifact(&"a".repeat(256 * 1024 + 1));
    cases.push(value);
    let mut value = declaration("bad");
    value.messages = (0..1025)
        .map(|i| pb::ContractMessage {
            name: Some(name(&format!("message-{i}"))),
            ..Default::default()
        })
        .collect();
    cases.push(value);
    let mut value = declaration("bad");
    value.messages[0].schemas.push(pb::SchemaBinding {
        role: 999,
        ..Default::default()
    });
    cases.push(value);
    let mut value = declaration("bad");
    value.operations.push(pb::ContractOperation {
        name: Some(name("Operation")),
        messages: vec![pb::OperationMessage {
            role: 999,
            ..Default::default()
        }],
        ..Default::default()
    });
    cases.push(value);
    for value in cases {
        invalid_every_surface(&service, value).await;
    }
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Specification as i32,
                id: Some(id("bad"))
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}

#[tokio::test]
async fn artifact_bytes_are_immutable_but_labels_and_member_order_preserve_identity() {
    let service = service().await;
    let original = contract(&service, "immutable").await;
    let mut changed = spec(&original);
    changed.artifacts[0].content = "new bytes\n".into();
    changed.artifacts[0].digest = Some(content_digest("new bytes\n"));
    invalid_every_surface(&service, changed).await;
    let mut renamed = spec(&original);
    renamed.doc = Some("Revised documentation".into());
    renamed.artifacts[0].name = Some(name("Renamed"));
    renamed.artifacts[0].path.as_mut().unwrap().value = "renamed.txt".into();
    let persisted = save(&service, renamed).await;
    assert_eq!(
        spec(&persisted).artifacts[0].member_id,
        spec(&original).artifacts[0].member_id
    );
    assert_eq!(
        spec(&persisted).artifacts[0].content.as_bytes(),
        b"schema\r\n"
    );
    assert_eq!(
        spec(&persisted).schemas[0]
            .definition
            .as_ref()
            .unwrap()
            .connection_id,
        spec(&original).schemas[0]
            .definition
            .as_ref()
            .unwrap()
            .connection_id
    );
}

#[tokio::test]
async fn permitted_schema_target_edit_invalidates_retained_foreign_pin_without_blocking_landing() {
    let service = service().await;
    let owner = contract(&service, "shared").await;
    let stored_consumer = save(&service, consumer(pin(&owner))).await;
    let mut updated = spec(&owner);
    updated.artifacts.push(pb::ContractArtifact {
        name: Some(name("Replacement")),
        path: Some(pb::ArtifactPath {
            value: "replacement.txt".into(),
        }),
        ..artifact("replacement")
    });
    let added = save(&service, updated).await;
    let mut retargeted = spec(&added);
    let new = retargeted.artifacts[1].clone();
    let definition = retargeted.schemas[0].definition.as_mut().unwrap();
    definition.connection_id = None;
    definition.target = Some(pb::schema_definition::Target::Artifact(
        pb::LocalArtifactSelection {
            artifact_id: new.member_id,
            expected_digest: new.digest,
        },
    ));
    save(&service, retargeted).await;
    assert_eq!(get(&service, "consumer").await, stored_consumer);
    let mut edited = spec(&stored_consumer);
    edited.doc = Some("New edit must validate stale selection".into());
    invalid_every_surface(&service, edited).await;
    let report = service
        .validate_project(Request::new(pb::ValidateProjectRequest {
            scope: Some(pb::validate_project_request::Scope::Namespace(
                "contracts".into(),
            )),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(report
        .reports
        .iter()
        .flat_map(|v| &v.issues)
        .any(|v| v.code == "SPECIFICATION_REFERENCE_UNRESOLVED"));
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "contract-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    service
        .put_entity(branch(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::Event(pb::Event {
                    id: Some(id("unrelated")),
                    title: "Unrelated".into(),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap();
    for dry_run in [true, false] {
        service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: "contract-work".into(),
                dry_run,
                ..Default::default()
            }))
            .await
            .unwrap();
    }
    assert_eq!(get(&service, "consumer").await, stored_consumer);
}

#[tokio::test]
async fn branch_tombstone_cannot_resolve_from_baseline_and_forced_delete_keeps_declaration() {
    let service = service().await.with_snapshot_cap(0);
    let owner = contract(&service, "shared").await;
    let reference = pin(&owner);
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "contract-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    service
        .delete_entity(branch(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Specification as i32,
            id: Some(id("shared")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    for validate_only in [true, false] {
        let error = service
            .put_entity(branch(pb::PutEntityRequest {
                entity: Some(entity(consumer(reference.clone()))),
                validate_only,
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument);
    }
    let stored = save(&service, consumer(reference)).await;
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Specification as i32,
            id: Some(id("shared")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    assert_eq!(get(&service, "consumer").await, stored);
}

#[tokio::test]
async fn operation_bearing_standalone_contract_requires_no_invented_provider() {
    let service = service().await;
    let original = save(&service, declaration("standalone")).await;
    let mut value = spec(&original);
    value.operations.push(pb::ContractOperation {
        name: Some(name("Notify")),
        messages: vec![pb::OperationMessage {
            message_id: value.messages[0].member_id.clone(),
            role: pb::operation_message::Role::Notification as i32,
            ..Default::default()
        }],
        ..Default::default()
    });
    let persisted = save(&service, value).await;
    assert!(!is_api(&spec(&persisted)));
    assert_eq!(
        spec(&persisted).operations[0].messages[0].role,
        pb::operation_message::Role::Notification as i32
    );
    assert_eq!(spec(&persisted).exposures, []);
}

fn library(slug: &str, message: &str) -> pb::TypeLibrary {
    pb::TypeLibrary {
        id: Some(id(slug)),
        title: slug.into(),
        files: vec![pb::ProtoSourceFile {
            path: format!("{slug}.proto"),
            content: format!(
                "syntax = \"proto3\"; package {slug}.v1; message {message} {{ string id = 1; }}"
            ),
        }],
        ..Default::default()
    }
}
async fn save_library(service: &EventModelServiceImpl, value: pb::TypeLibrary) {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::TypeLibrary(value)),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap();
}

#[tokio::test]
async fn exact_selected_library_message_compiles_with_own_namespace_imports_beyond_projection() {
    let service = service().await.with_snapshot_cap(0);
    let mut dependency = library("shared", "Dependency");
    dependency.id.as_mut().unwrap().namespace = "types".into();
    save_library(&service, dependency.clone()).await;
    let mut selected = library("orders", "Order");
    selected.id.as_mut().unwrap().namespace = "types".into();
    selected.dependencies.push(pb::TypeLibraryRef {
        id: dependency.id.clone(),
    });
    selected.files[0].content = "syntax = \"proto3\"; package orders.v1; import \"shared.proto\"; message Order { shared.v1.Dependency value = 1; }".into();
    save_library(&service, selected.clone()).await;
    let mut value = declaration("typed");
    value.messages[0].schemas.push(pb::SchemaBinding {
        role: pb::schema_binding::Role::Payload as i32,
        r#type: Some(pb::schema_binding::Type::LibraryType(
            pb::TypeLibraryTypeRef {
                library: Some(pb::TypeLibraryRef {
                    id: selected.id.clone(),
                }),
                message: Some(pb::QualifiedProtoName {
                    value: "orders.v1.Order".into(),
                }),
            },
        )),
        ..Default::default()
    });
    let persisted = save(&service, value).await;
    for name in [
        "shared.v1.Dependency",
        "orders.v1.Missing",
        ".orders.v1.Order",
    ] {
        let mut invalid = spec(&persisted);
        if let Some(pb::schema_binding::Type::LibraryType(reference)) =
            invalid.messages[0].schemas[0].r#type.as_mut()
        {
            reference.message.as_mut().unwrap().value = name.into();
        }
        invalid.messages[0].schemas[0].connection_id = None;
        invalid_every_surface(&service, invalid).await;
    }
}

#[tokio::test]
async fn breaking_library_edit_is_guarded_and_forced_removal_is_diagnosed_without_blocking_landing()
{
    let service = service().await;
    save_library(&service, library("orders", "Order")).await;
    let mut value = declaration("typed");
    value.messages[0].schemas.push(pb::SchemaBinding {
        role: pb::schema_binding::Role::Payload as i32,
        r#type: Some(pb::schema_binding::Type::LibraryType(
            pb::TypeLibraryTypeRef {
                library: Some(pb::TypeLibraryRef {
                    id: Some(id("orders")),
                }),
                message: Some(pb::QualifiedProtoName {
                    value: "orders.v1.Order".into(),
                }),
            },
        )),
        ..Default::default()
    });
    let persisted = save(&service, value).await;
    let error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::TypeLibrary(library(
                    "orders",
                    "Replacement",
                ))),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
    assert!(error
        .message()
        .contains("message orders.v1.Order was removed"));
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::TypeLibrary as i32,
            id: Some(id("orders")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let report = service
        .validate_project(Request::new(pb::ValidateProjectRequest {
            scope: Some(pb::validate_project_request::Scope::Namespace(
                "contracts".into(),
            )),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(
        report
            .reports
            .iter()
            .flat_map(|v| &v.issues)
            .any(|v| v.code == "SPECIFICATION_REFERENCE_UNRESOLVED"
                && v.subject_field == "messages.schemas.library_type"),
        "{report:?}"
    );
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "contract-work".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    service
        .put_entity(branch(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::Event(pb::Event {
                    id: Some(id("unrelated")),
                    title: "Unrelated".into(),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap();
    for dry_run in [true, false] {
        service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: "contract-work".into(),
                dry_run,
                ..Default::default()
            }))
            .await
            .unwrap();
    }
    assert_eq!(get(&service, "typed").await, persisted);
    let mut edited = spec(&persisted);
    edited.doc = Some("Must validate selected type".into());
    invalid_every_surface(&service, edited).await;
}

async fn authenticated_client(
    service: EventModelServiceImpl,
) -> pb::event_model_service_client::EventModelServiceClient<tonic::transport::Channel> {
    use trogon_atlas_server::auth::{AuthzLayer, BearerAuth, TokenRegistry};
    let registry = toml::from_str::<trogon_atlas_server::auth::RegistryFile>(
        r#"
[principals.contract-writer]
role = "writer"
tokens = ["contract-writer-token"]
parent = "owned"
[principals.contract-reader]
role = "reader"
tokens = ["contract-reader-token"]
parent = "owned"
"#,
    )
    .unwrap();
    let data_plane = tonic::service::interceptor::InterceptedService::new(
        tower::ServiceBuilder::new()
            .layer(AuthzLayer)
            .service(pb::event_model_service_server::EventModelServiceServer::new(service)),
        BearerAuth::new(Arc::new(
            TokenRegistry::from_file_contents(registry).unwrap(),
        )),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(data_plane)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    pb::event_model_service_client::EventModelServiceClient::connect(format!("http://{address}"))
        .await
        .unwrap()
}
fn authed<T>(body: T, reader: bool) -> Request<T> {
    let mut request = Request::new(body);
    request.metadata_mut().insert(
        "authorization",
        if reader {
            "Bearer contract-reader-token"
        } else {
            "Bearer contract-writer-token"
        }
        .parse()
        .unwrap(),
    );
    request
}

#[tokio::test]
async fn hidden_nested_referrer_protects_delete_without_exposing_its_details() {
    let service = service().await.with_snapshot_cap(0);
    let own = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "owned-contracts".into(),
            parent: "owned".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let hidden = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "private-contracts".into(),
            parent: "other".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let target = pb::Id {
        namespace: own.clone(),
        slug: "feed".into(),
        version: 1,
    };
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                    id: Some(target.clone()),
                    title: "Feed".into(),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut value = declaration("hidden-contract");
    value.id.as_mut().unwrap().namespace = hidden.clone();
    value.messages[0].name = Some(name("Hidden message details"));
    value.messages[0].mapping = Some(pb::MessageReadModelMapping {
        read_model: Some(pb::ReadModelRef {
            id: Some(target.clone()),
        }),
        ..Default::default()
    });
    save(&service, value).await;
    let mut client = authenticated_client(service).await;
    let error = client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::ReadModel as i32,
                id: Some(target.clone()),
                mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                ..Default::default()
            },
            false,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition, "{error:?}");
    for secret in [&hidden, "hidden-contract", "Hidden message details"] {
        assert!(!error.message().contains(secret));
    }
    for dry_run in [true, false] {
        let result = client
            .batch_mutate(authed(
                pb::BatchMutateRequest {
                    ops: vec![pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                            kind: pb::EntityKind::ReadModel as i32,
                            id: Some(target.clone()),
                            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                            ..Default::default()
                        })),
                    }],
                    validate_only: dry_run,
                    ..Default::default()
                },
                false,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            result.status,
            pb::batch_mutate_response::Status::Failed as i32,
            "{result:?}"
        );
        assert!(result.failure.iter().any(|v| v.code == "ENTITY_REFERENCED"));
        let details = format!("{result:?}");
        assert!(!details.contains(&hidden));
        assert!(!details.contains("hidden-contract"));
    }
    let references = client
        .get_incoming_references(authed(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::ReadModel as i32,
                id: Some(target),
                ..Default::default()
            },
            true,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(references.references.is_empty(), "{references:?}");
    let mut forbidden = declaration("owned-contract");
    forbidden.id.as_mut().unwrap().namespace = own;
    forbidden.dependencies.push(pb::ContractDependency {
        target: Some(pb::contract_dependency::Target::Specification(
            pb::SpecificationRef {
                id: Some(pb::Id {
                    namespace: hidden.clone(),
                    slug: "hidden-contract".into(),
                    version: 1,
                }),
            },
        )),
        ..Default::default()
    });
    let error = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(entity(forbidden)),
                ..Default::default()
            },
            false,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::NotFound, "{error:?}");
    assert!(!error.message().contains("hidden-contract"));
}

#[tokio::test]
async fn authenticated_reader_sees_unresolved_provider_without_hidden_target_details_or_reclassification(
) {
    let service = service().await;
    let own = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "owned-contracts".into(),
            parent: "owned".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let hidden = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "hidden-provider".into(),
            parent: "other".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let provider_id = pb::Id {
        namespace: hidden.clone(),
        slug: "secret-provider-key".into(),
        version: 1,
    };
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::Component(pb::Component {
                    id: Some(provider_id.clone()),
                    title: "Secret provider display".into(),
                    doc: "Secret provider documentation".into(),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut value = declaration("visible-api");
    value.id.as_mut().unwrap().namespace = own.clone();
    let first = save(&service, value).await;
    let mut value = spec(&first);
    value.operations.push(pb::ContractOperation {
        name: Some(name("Notify")),
        messages: vec![pb::OperationMessage {
            message_id: value.messages[0].member_id.clone(),
            role: pb::operation_message::Role::Notification as i32,
            ..Default::default()
        }],
        ..Default::default()
    });
    let next = save(&service, value).await;
    let mut value = spec(&next);
    value.exposures.push(pb::ProviderExposure {
        provider: Some(pb::ProviderTarget {
            target: Some(pb::provider_target::Target::Component(pb::ComponentRef {
                id: Some(provider_id.clone()),
            })),
        }),
        operation_id: value.operations[0].member_id.clone(),
        ..Default::default()
    });
    let api = save(&service, value).await;
    assert!(is_api(&spec(&api)));
    let mut client = authenticated_client(service).await;
    let error = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Component as i32,
                id: Some(provider_id),
            },
            true,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::NotFound);
    let visible = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Specification as i32,
                id: spec(&api).id.clone(),
            },
            true,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert!(is_api(&spec(&visible)));
    let report = client
        .validate_project(authed(
            pb::ValidateProjectRequest {
                scope: Some(pb::validate_project_request::Scope::Namespace(own)),
            },
            true,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(report
        .reports
        .iter()
        .flat_map(|v| &v.issues)
        .any(|v| v.code == "SPECIFICATION_REFERENCE_UNRESOLVED"));
    let rendered = format!("{report:?}");
    for secret in [
        &hidden,
        "secret-provider-key",
        "Secret provider display",
        "Secret provider documentation",
    ] {
        assert!(!rendered.contains(secret), "{rendered}");
    }
    let write = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(visible),
                ..Default::default()
            },
            true,
        ))
        .await
        .unwrap_err();
    assert_eq!(write.code(), Code::PermissionDenied);
}
