#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use tonic::{Code, Request};
use trogon_atlas_core::specification::content_digest;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};
use trogon_atlas_store::Store;

const MESSAGE: &str = "11111111-1111-4111-8111-111111111111";
const QUERY: &str = "22222222-2222-4222-8222-222222222222";
const SHAPE: &str = "33333333-3333-4333-8333-333333333333";

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::now_v7())
}

fn eid(namespace: &str, slug: &str) -> pb::Id {
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

fn name(value: &str) -> pb::MemberName {
    pb::MemberName {
        value: value.into(),
    }
}

fn entity(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}

async fn service() -> EventModelServiceImpl {
    EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap()
    .with_snapshot_cap(0)
}

async fn put(
    service: &EventModelServiceImpl,
    kind: pb::entity::Kind,
    branch: Option<&str>,
) -> pb::PutEntityResponse {
    let label = trogon_atlas_core::refs::entity_id(&entity(kind.clone()))
        .map(|id| format!("{}/{}@{}", id.namespace, id.slug, id.version))
        .unwrap_or_else(|| "entity".into());
    let mut request = Request::new(pb::PutEntityRequest {
        entity: Some(entity(kind)),
        ..Default::default()
    });
    if let Some(branch) = branch {
        request
            .metadata_mut()
            .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    }
    service
        .put_entity(request)
        .await
        .unwrap_or_else(|status| panic!("put {label} failed: {status}"))
        .into_inner()
}

async fn put_expect(service: &EventModelServiceImpl, kind: pb::entity::Kind) -> tonic::Status {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity(kind)),
            ..Default::default()
        }))
        .await
        .unwrap_err()
}

fn kind<'a>(entity: &'a pb::Entity) -> &'a pb::entity::Kind {
    entity.kind.as_ref().unwrap()
}

struct World {
    namespace: String,
    channel: pb::Id,
    other_channel: pb::Id,
    event: pb::Id,
    specification: pb::Entity,
    product: pb::Entity,
}

async fn world(service: &EventModelServiceImpl) -> World {
    world_in(service, &unique("assoc")).await
}

async fn world_in(service: &EventModelServiceImpl, namespace: &str) -> World {
    let namespace = namespace.to_string();
    let event = eid(&namespace, "placed");
    let command = eid(&namespace, "place");
    let read_model = eid(&namespace, "receipt");
    let processor = eid(&namespace, "worker");
    let component = eid(&namespace, "checkout");
    let external = eid(&namespace, "partner");
    let shape = eid(&namespace, "shape");
    let store = eid(&namespace, "cache");
    let slice = eid(&namespace, "observe");
    let channel = eid(&namespace, "orders");
    let other_channel = eid(&namespace, "backup");
    for (slug, kind) in [
        (
            "placed",
            pb::entity::Kind::Event(pb::Event {
                id: Some(event.clone()),
                title: "Placed".into(),
                ..Default::default()
            }),
        ),
        (
            "place",
            pb::entity::Kind::Command(pb::Command {
                id: Some(command.clone()),
                title: "Place".into(),
                ..Default::default()
            }),
        ),
        (
            "receipt",
            pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(read_model.clone()),
                title: "Receipt".into(),
                ..Default::default()
            }),
        ),
        (
            "worker",
            pb::entity::Kind::Processor(pb::Processor {
                id: Some(processor.clone()),
                title: "Worker".into(),
                ..Default::default()
            }),
        ),
        (
            "checkout",
            pb::entity::Kind::Component(pb::Component {
                id: Some(component.clone()),
                title: "Checkout".into(),
                ..Default::default()
            }),
        ),
        (
            "partner",
            pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
                id: Some(external.clone()),
                title: "Partner".into(),
                ..Default::default()
            }),
        ),
        (
            "shape",
            pb::entity::Kind::Schema(pb::Schema {
                id: Some(shape.clone()),
                title: "Shape".into(),
                ..Default::default()
            }),
        ),
        (
            "cache",
            pb::entity::Kind::DataStore(pb::DataStore {
                id: Some(store.clone()),
                title: "Cache".into(),
                store_class: Some(pb::StoreClass {
                    kind: Some(pb::store_class::Kind::Known(
                        pb::store_class::Known::Cache as i32,
                    )),
                }),
                authoritative: Some(false),
                attachments: vec![pb::ReadModelAttachment {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(read_model.clone()),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
        ),
    ] {
        let _ = slug;
        put(service, kind, None).await;
    }
    let automation = put(
        service,
        pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(slice.clone()),
            title: "Observe".into(),
            source_read_models: vec![pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: Some(read_model.clone()),
                }),
                ..Default::default()
            }],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(processor.clone()),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
        None,
    )
    .await
    .entity
    .unwrap();
    let pb::entity::Kind::AutomationSlice(slice_value) = kind(&automation) else {
        panic!("automation");
    };
    let observation = pb::ConnectionRef {
        owner: Some(pb::EntityRef {
            kind: pb::EntityKind::AutomationSlice as i32,
            id: Some(slice.clone()),
        }),
        owner_uid: automation.system.as_ref().unwrap().uid.clone(),
        id: slice_value.source_read_models[0].connection_id.clone(),
    };
    let channel_entity = |id: pb::Id, title: &str| {
        pb::entity::Kind::Channel(pb::Channel {
            id: Some(id),
            title: title.into(),
            address: Some(pb::ChannelAddress {
                template: "orders.{topic}".into(),
                parameters: vec![pb::AddressParameter {
                    name: Some(pb::ParameterName {
                        value: "topic".into(),
                    }),
                    ..Default::default()
                }],
            }),
            bindings: vec![pb::ChannelBinding {
                observation: Some(observation.clone()),
                parameter_values: [("topic".into(), "orders".into())].into(),
                ..Default::default()
            }],
            ..Default::default()
        })
    };
    put(service, channel_entity(channel.clone(), "Orders"), None).await;
    put(
        service,
        channel_entity(other_channel.clone(), "Backup"),
        None,
    )
    .await;
    let content = "schema\r\n";
    let specification = put(
        service,
        pb::entity::Kind::Specification(pb::Specification {
            id: Some(eid(&namespace, "contract")),
            title: "Contract".into(),
            artifacts: vec![pb::ContractArtifact {
                member_id: Some(member("44444444-4444-4444-8444-444444444444")),
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
            }],
            schemas: vec![pb::ContractSchema {
                member_id: Some(member(SHAPE)),
                name: Some(name("Shape")),
                definition: Some(pb::SchemaDefinition {
                    target: Some(pb::schema_definition::Target::Artifact(
                        pb::LocalArtifactSelection {
                            artifact_id: Some(member("44444444-4444-4444-8444-444444444444")),
                            expected_digest: Some(content_digest(content)),
                        },
                    )),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            messages: vec![
                pb::ContractMessage {
                    member_id: Some(member(MESSAGE)),
                    name: Some(name("Order")),
                    schemas: vec![
                        pb::SchemaBinding {
                            role: pb::schema_binding::Role::Payload as i32,
                            r#type: Some(pb::schema_binding::Type::LocalSchema(member(SHAPE))),
                            environments: vec![pb::EnvironmentName {
                                value: "a".repeat(64),
                            }],
                            is_default: Some(false),
                            ..Default::default()
                        },
                        pb::SchemaBinding {
                            role: pb::schema_binding::Role::Headers as i32,
                            r#type: Some(pb::schema_binding::Type::LocalSchema(member(SHAPE))),
                            environments: vec![pb::EnvironmentName {
                                value: "a".repeat(64),
                            }],
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                pb::ContractMessage {
                    member_id: Some(member(QUERY)),
                    name: Some(name("Query intent")),
                    ..Default::default()
                },
            ],
            message_channels: vec![
                pb::MessageChannelDeclaration {
                    member_id: Some(member(MESSAGE)),
                    channel: Some(pb::ChannelRef {
                        id: Some(channel.clone()),
                    }),
                    parameter_values: [("topic".into(), "orders".into())].into(),
                    doc: Some("carried".into()),
                    ..Default::default()
                },
                pb::MessageChannelDeclaration {
                    member_id: Some(member(MESSAGE)),
                    channel: Some(pb::ChannelRef {
                        id: Some(channel.clone()),
                    }),
                    parameter_values: [("topic".into(), "orders".into())].into(),
                    delivery_mode: Some(pb::ClaimDeliveryMode::Push as i32),
                    ..Default::default()
                },
            ],
            operations: vec![pb::ContractOperation {
                member_id: Some(member("55555555-5555-4555-8555-555555555555")),
                name: Some(name("Send")),
                messages: vec![pb::OperationMessage {
                    message_id: Some(member(MESSAGE)),
                    role: pb::operation_message::Role::Request as i32,
                    ..Default::default()
                }],
                binding: Some(pb::OperationBinding {
                    kind: Some(pb::operation_binding::Kind::Channel(
                        pb::ChannelOperationBinding {
                            channel: Some(pb::ChannelRef {
                                id: Some(channel.clone()),
                            }),
                            ..Default::default()
                        },
                    )),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
        None,
    )
    .await
    .entity
    .unwrap();
    let uid = specification.system.as_ref().unwrap().uid.clone();
    let spec_id = eid(&namespace, "contract");
    let product = put(
        service,
        pb::entity::Kind::DataProduct(pb::DataProduct {
            id: Some(eid(&namespace, "offering")),
            title: "Offering".into(),
            outputs: vec![pb::DataOutputPort {
                name: Some(pb::PortName {
                    value: "Receipt".into(),
                }),
                data: Some(pb::OutputDataSelection {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(read_model.clone()),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            resource_associations: vec![
                association(
                    pb::DataPortDirection::Output,
                    pb::product_resource_association::Target::Event(pb::EventRef {
                        id: Some(event.clone()),
                    }),
                    Some(pb::ProductResourceContract {
                        r#type: Some(pb::product_resource_contract::Type::Schema(pb::SchemaRef {
                            id: Some(shape.clone()),
                        })),
                    }),
                ),
                association(
                    pb::DataPortDirection::Output,
                    pb::product_resource_association::Target::Event(pb::EventRef {
                        id: Some(event.clone()),
                    }),
                    None,
                ),
                association(
                    pb::DataPortDirection::Input,
                    pb::product_resource_association::Target::Command(pb::CommandRef {
                        id: Some(command),
                    }),
                    None,
                ),
                association(
                    pb::DataPortDirection::Output,
                    pb::product_resource_association::Target::Message(pb::ContractMemberRef {
                        specification: Some(pb::SpecificationRef { id: Some(spec_id) }),
                        owner_uid: uid,
                        kind: pb::ContractMemberKind::Message as i32,
                        member_id: Some(member(QUERY)),
                        expected_digest: None,
                    }),
                    None,
                ),
                association(
                    pb::DataPortDirection::Output,
                    pb::product_resource_association::Target::Component(pb::ComponentRef {
                        id: Some(component),
                    }),
                    None,
                ),
                association(
                    pb::DataPortDirection::Output,
                    pb::product_resource_association::Target::ExternalSystem(
                        pb::ExternalSystemRef { id: Some(external) },
                    ),
                    None,
                ),
                association(
                    pb::DataPortDirection::Output,
                    pb::product_resource_association::Target::Channel(pb::ChannelRef {
                        id: Some(channel.clone()),
                    }),
                    None,
                ),
                association(
                    pb::DataPortDirection::Output,
                    pb::product_resource_association::Target::DataStore(pb::DataStoreRef {
                        id: Some(store),
                    }),
                    None,
                ),
            ],
            ..Default::default()
        }),
        None,
    )
    .await
    .entity
    .unwrap();
    World {
        namespace,
        channel,
        other_channel,
        event,
        specification,
        product,
    }
}

fn association(
    direction: pb::DataPortDirection,
    target: pb::product_resource_association::Target,
    contract: Option<pb::ProductResourceContract>,
) -> pb::ProductResourceAssociation {
    pb::ProductResourceAssociation {
        direction: direction as i32,
        target: Some(target),
        contract,
        doc: Some("descriptive".into()),
        ..Default::default()
    }
}

fn spec_of(entity: &pb::Entity) -> &pb::Specification {
    let pb::entity::Kind::Specification(value) = kind(entity) else {
        panic!("specification");
    };
    value
}

fn product_of(entity: &pb::Entity) -> &pb::DataProduct {
    let pb::entity::Kind::DataProduct(value) = kind(entity) else {
        panic!("product");
    };
    value
}

#[tokio::test]
async fn native_declarations_keep_exact_targets_presence_and_distinct_occurrences() {
    let service = service().await;
    let world = world(&service).await;
    let specification = spec_of(&world.specification);
    assert_eq!(specification.message_channels.len(), 2);
    assert!(specification.message_channels[0].delivery_mode.is_none());
    assert_eq!(
        specification.message_channels[1].delivery_mode,
        Some(pb::ClaimDeliveryMode::Push as i32)
    );
    assert_ne!(
        specification.message_channels[0].connection_id,
        specification.message_channels[1].connection_id
    );
    let payload = &specification.messages[0].schemas[0];
    let headers = &specification.messages[0].schemas[1];
    assert_eq!(payload.is_default, Some(false));
    assert!(headers.is_default.is_none());
    assert_eq!(payload.environments[0].value.len(), 64);
    assert_ne!(payload.connection_id, headers.connection_id);
    let product = product_of(&world.product);
    assert_eq!(product.resource_associations.len(), 8);
    assert_eq!(product.outputs.len(), 1);
    let ids: std::collections::HashSet<_> = product
        .resource_associations
        .iter()
        .map(|association| {
            association
                .connection_id
                .as_ref()
                .map(|id| id.value.clone())
        })
        .collect();
    assert_eq!(ids.len(), 8);
    assert!(product.resource_associations[0].contract.is_some());
    assert!(product.resource_associations[1].contract.is_none());
    let message = product.resource_associations[3].target.as_ref().unwrap();
    let pb::product_resource_association::Target::Message(message) = message else {
        panic!("query intent target");
    };
    assert_eq!(message.kind, pb::ContractMemberKind::Message as i32);
    let subject = pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Connection(
            pb::ConnectionRef {
                owner: Some(pb::EntityRef {
                    kind: pb::EntityKind::DataProduct as i32,
                    id: Some(eid(&world.namespace, "offering")),
                }),
                owner_uid: world.product.system.as_ref().unwrap().uid.clone(),
                id: product.resource_associations[0].connection_id.clone(),
            },
        )),
    };
    put(
        &service,
        pb::entity::Kind::Team(pb::Team {
            id: Some(eid(&world.namespace, "owners")),
            title: Some(pb::DirectoryDisplayName {
                value: "Owners".into(),
            }),
            responsibilities: vec![pb::Responsibility {
                kind: pb::ResponsibilityKind::Owner as i32,
                subject: Some(subject.clone()),
                ..Default::default()
            }],
            ..Default::default()
        }),
        None,
    )
    .await;
    let assignments = service
        .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
            subject: Some(subject),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(assignments.assignments.len(), 1);
    let references = service
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(world.event.clone()),
            filter_kinds: vec![pb::EntityKind::DataProduct as i32],
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references;
    assert!(references.iter().any(|reference| {
        reference.from_field.starts_with("resource_associations[")
            && reference.connection_id.is_some()
    }));
    let channel_refs = service
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Channel as i32,
            id: Some(world.channel.clone()),
            filter_kinds: vec![pb::EntityKind::Specification as i32],
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references;
    assert!(channel_refs.iter().any(|reference| {
        reference.from_field.starts_with("message_channels[") && reference.connection_id.is_some()
    }));
}

#[tokio::test]
async fn invalid_variants_direction_and_delivery_mode_are_rejected() {
    let service = service().await;
    let world = world(&service).await;
    let mut product = product_of(&world.product).clone();
    product.resource_associations[2].contract = Some(pb::ProductResourceContract {
        r#type: Some(pb::product_resource_contract::Type::Schema(pb::SchemaRef {
            id: Some(eid(&world.namespace, "shape")),
        })),
    });
    let error = put_expect(&service, pb::entity::Kind::DataProduct(product.clone())).await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    product.resource_associations[2].contract = None;
    product.resource_associations[0].direction = 0;
    let error = put_expect(&service, pb::entity::Kind::DataProduct(product)).await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    let mut specification = spec_of(&world.specification).clone();
    specification.message_channels[0].delivery_mode =
        Some(pb::ClaimDeliveryMode::Unspecified as i32);
    let error = put_expect(
        &service,
        pb::entity::Kind::Specification(specification.clone()),
    )
    .await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    specification.message_channels[0].delivery_mode = None;
    specification.message_channels[0]
        .parameter_values
        .insert("unknown".into(), "x".into());
    let error = put_expect(
        &service,
        pb::entity::Kind::Specification(specification.clone()),
    )
    .await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    specification.message_channels[0]
        .parameter_values
        .remove("unknown");
    specification.messages[0].schemas[1].environments = vec![pb::EnvironmentName {
        value: "a".repeat(64),
    }];
    specification.messages[0].schemas[1].role = pb::schema_binding::Role::Payload as i32;
    let error = put_expect(
        &service,
        pb::entity::Kind::Specification(specification.clone()),
    )
    .await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    specification.messages[0].schemas[1].role = pb::schema_binding::Role::Headers as i32;
    specification.messages[0].schemas[0].environments.clear();
    specification.messages[0].schemas[0].is_default = Some(true);
    specification.messages[0].schemas[1].is_default = Some(true);
    specification.messages[0].schemas[1].environments.clear();
    specification.messages[0].schemas[1].role = pb::schema_binding::Role::Payload as i32;
    let error = put_expect(&service, pb::entity::Kind::Specification(specification)).await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    let mut too_long = spec_of(&world.specification).clone();
    too_long.messages[0].schemas[0].environments[0].value = "a".repeat(65);
    let error = put_expect(&service, pb::entity::Kind::Specification(too_long)).await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
}

#[tokio::test]
async fn channel_message_pages_are_independent_of_operation_bindings() {
    let service = service().await;
    let world = world(&service).await;
    let first = service
        .list_channel_messages(Request::new(pb::ListChannelMessagesRequest {
            channel: Some(pb::ChannelRef {
                id: Some(world.channel.clone()),
            }),
            page_size: 1,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.records.len(), 1);
    assert!(!first.next_page_token.is_empty());
    assert!(!first.complete);
    assert!(!first.incomplete);
    assert_eq!(
        first.records[0].authority,
        pb::ChannelMessageAuthority::SpecificationDeclaration as i32
    );
    assert_eq!(
        first.records[0].message.as_ref().unwrap().kind,
        pb::ContractMemberKind::Message as i32
    );
    let second = service
        .list_channel_messages(Request::new(pb::ListChannelMessagesRequest {
            channel: Some(pb::ChannelRef {
                id: Some(world.channel.clone()),
            }),
            page_size: 1,
            page_token: first.next_page_token.clone(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(second.records.len(), 1);
    assert!(second.next_page_token.is_empty());
    assert!(second.complete);
    assert_ne!(
        first.records[0].connection_id,
        second.records[0].connection_id
    );
    let modes = [
        first.records[0].delivery_mode,
        second.records[0].delivery_mode,
    ];
    assert!(modes.contains(&None));
    assert!(modes.contains(&Some(pb::ClaimDeliveryMode::Push as i32)));
    let mismatched = service
        .list_channel_messages(Request::new(pb::ListChannelMessagesRequest {
            channel: Some(pb::ChannelRef {
                id: Some(world.other_channel.clone()),
            }),
            page_token: first.next_page_token,
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(mismatched.code(), Code::InvalidArgument, "{mismatched}");
    assert_eq!(mismatched.message(), "page token does not match this query");
    let missing = service
        .list_channel_messages(Request::new(pb::ListChannelMessagesRequest {
            channel: Some(pb::ChannelRef {
                id: Some(eid(&world.namespace, "missing")),
            }),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(missing.code(), Code::NotFound);
    assert_eq!(missing.message(), "resource unavailable");
}

#[tokio::test]
async fn retarget_renews_the_declaration_and_delete_stays_generic() {
    let service = service().await;
    let world = world(&service).await;
    let before = spec_of(&world.specification).message_channels[0]
        .connection_id
        .clone();
    let applied = service
        .retarget_references(Request::new(pb::RetargetReferencesRequest {
            from: Some(pb::EntityRef {
                kind: pb::EntityKind::Channel as i32,
                id: Some(world.channel.clone()),
            }),
            to: Some(pb::EntityRef {
                kind: pb::EntityKind::Channel as i32,
                id: Some(world.other_channel.clone()),
            }),
            filter_kinds: vec![pb::EntityKind::Specification as i32],
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(applied.rewritten >= 1, "{applied:?}");
    let stored = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Specification as i32,
            id: Some(eid(&world.namespace, "contract")),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let declaration = &spec_of(&stored).message_channels[0];
    assert_eq!(
        declaration.channel.as_ref().unwrap().id.as_ref(),
        Some(&world.other_channel)
    );
    assert_ne!(declaration.connection_id, before);
    assert!(trogon_atlas_core::connections::resolve_connection(
        &stored,
        &pb::ConnectionRef {
            owner: Some(pb::EntityRef {
                kind: pb::EntityKind::Specification as i32,
                id: Some(eid(&world.namespace, "contract")),
            }),
            owner_uid: stored.system.as_ref().unwrap().uid.clone(),
            id: before,
        },
    )
    .is_err());
    let blocked = service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Channel as i32,
            id: Some(world.other_channel.clone()),
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(blocked.code(), Code::FailedPrecondition);
    assert!(!blocked.message().contains("Contract"));
    assert_eq!(
        blocked.message(),
        "entity is referenced; pass mode=FORCE or repair referrers first"
    );
}

#[tokio::test]
async fn branch_tombstone_hides_declarations_and_landing_renews_changed_bindings() {
    let service = service().await;
    let world = world(&service).await;
    let hidden = unique("assoc-hide");
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: hidden.clone(),
            ..Default::default()
        }))
        .await
        .unwrap();
    service
        .delete_entity({
            let mut request = Request::new(pb::DeleteEntityRequest {
                kind: pb::EntityKind::Specification as i32,
                id: Some(eid(&world.namespace, "contract")),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            });
            request
                .metadata_mut()
                .insert("x-trogon-atlas-branch", hidden.parse().unwrap());
            request
        })
        .await
        .unwrap();
    let on_branch = service
        .list_channel_messages({
            let mut request = Request::new(pb::ListChannelMessagesRequest {
                channel: Some(pb::ChannelRef {
                    id: Some(world.channel.clone()),
                }),
                ..Default::default()
            });
            request
                .metadata_mut()
                .insert("x-trogon-atlas-branch", hidden.parse().unwrap());
            request
        })
        .await
        .unwrap()
        .into_inner();
    assert!(on_branch.records.is_empty(), "{on_branch:?}");
    assert!(on_branch.complete);
    let baseline = service
        .list_channel_messages(Request::new(pb::ListChannelMessagesRequest {
            channel: Some(pb::ChannelRef {
                id: Some(world.channel.clone()),
            }),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(baseline.records.len(), 2);
    let work = unique("assoc-land");
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: work.clone(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut edited = world.specification.clone();
    let pb::entity::Kind::Specification(specification) = edited.kind.as_mut().unwrap() else {
        panic!("specification");
    };
    let old_binding = specification.messages[0].schemas[0].connection_id.clone();
    specification.messages[0].schemas[0].environments = vec![pb::EnvironmentName {
        value: "stage".into(),
    }];
    specification.messages[0].schemas[0].connection_id = None;
    edited.system = None;
    let staged = put(
        &service,
        pb::entity::Kind::Specification(specification.clone()),
        Some(&work),
    )
    .await
    .entity
    .unwrap();
    assert_ne!(
        spec_of(&staged).messages[0].schemas[0].connection_id,
        old_binding
    );
    assert_eq!(
        service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: work,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner()
            .status,
        pb::merge_branch_response::Status::Applied as i32
    );
    let landed = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Specification as i32,
            id: Some(eid(&world.namespace, "contract")),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(
        spec_of(&landed).messages[0].schemas[0].environments[0].value,
        "stage"
    );
    assert_eq!(
        spec_of(&landed).messages[0].schemas[0].connection_id,
        spec_of(&staged).messages[0].schemas[0].connection_id
    );
}

fn owner(entity: &pb::Entity) -> pb::EntityRef {
    pb::EntityRef {
        kind: trogon_atlas_core::refs::entity_kind(entity).unwrap() as i32,
        id: trogon_atlas_core::refs::entity_id(entity).cloned(),
    }
}

fn ownership(policy: pb::source_field_ownership::Policy) -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(policy),
    }
}

fn title_policy() -> pb::SourceFieldOwnership {
    ownership(pb::source_field_ownership::Policy::Fields(
        pb::SourceOwnedFields {
            fields: vec![pb::SourceOwnedField::Title as i32],
        },
    ))
}

fn whole_policy() -> pb::SourceFieldOwnership {
    ownership(pb::source_field_ownership::Policy::WholeOwner(
        pb::SourceWholeOwnerOwnership {},
    ))
}

fn none_policy() -> pb::SourceFieldOwnership {
    ownership(pb::source_field_ownership::Policy::None(
        pb::SourceNoOwnership {},
    ))
}

async fn source_snapshot(
    service: &EventModelServiceImpl,
    registration: &pb::SourceRegistration,
    key: &str,
) -> pb::SourceSnapshot {
    let inventory_bytes = serde_json::to_vec(&serde_json::json!({
        "profile": "supplied-native-v1", "profile_revision": 1,
        "registration": registration.id, "source_version": key,
        "partition": {"categories": ["model"], "logical_roots": ["native"], "selected_keys": []},
        "coverage": "PARTIAL",
        "resources": [{"key": {"category": "model", "key": key}, "file": "native/owners.json", "pointer": {"kind": "native", "raw": format!("#/{key}")}}],
        "relations": []
    }))
    .unwrap();
    service
        .accept_source_snapshot(Request::new(pb::AcceptSourceSnapshotRequest {
            registration_id: registration.id.clone(),
            registration_revision: registration.revision.clone(),
            inventory_bytes,
            files: vec![pb::SourceFile {
                logical_name: "native/owners.json".into(),
                media_type: "application/json".into(),
                content: b"{}\n".to_vec(),
                ..Default::default()
            }],
        }))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap()
}

fn source_key(registration: &str, key: &str) -> pb::SourceResourceKey {
    pb::SourceResourceKey {
        registration_id: registration.into(),
        category: "model".into(),
        key: key.into(),
    }
}

async fn source_map(
    service: &EventModelServiceImpl,
    registration: &pb::SourceRegistration,
    key: &str,
    entity: &pb::Entity,
    etag: &str,
    policy: pb::SourceFieldOwnership,
    selection: pb::source_selection::Selection,
) -> Result<pb::SourceMapping, tonic::Status> {
    service
        .create_source_mapping(Request::new(pb::CreateSourceMappingRequest {
            registration_revision: registration.revision.clone(),
            mapping: Some(pb::SourceMapping {
                id: unique("map"),
                source: Some(source_key(&registration.id, key)),
                selection: Some(pb::SourceSelection {
                    selection: Some(selection),
                }),
                ownership: Some(policy),
                adoption: Some(pb::SourceAdoption {
                    authorized_adoption: true,
                    observed: Some(pb::SourceObservedOwner {
                        owner: Some(owner(entity)),
                        uid: entity.system.as_ref().unwrap().uid.clone(),
                        etag: etag.into(),
                    }),
                }),
                ..Default::default()
            }),
        }))
        .await
        .map(|response| response.into_inner().mapping.unwrap())
}

async fn source_preview(
    service: &EventModelServiceImpl,
    registration: &pb::SourceRegistration,
    snapshot: &pb::SourceSnapshot,
    key: &str,
    incoming: pb::Entity,
) -> Result<pb::ReconciliationRun, tonic::Status> {
    service
        .preview_source_run(Request::new(pb::PreviewSourceRunRequest {
            registration_id: registration.id.clone(),
            registration_revision: registration.revision.clone(),
            snapshot_id: snapshot.id.clone(),
            run_id: unique("run"),
            native_payloads: vec![pb::SourceNativePayload {
                source: Some(source_key(&registration.id, key)),
                entity: Some(incoming),
            }],
            ..Default::default()
        }))
        .await
        .map(|response| response.into_inner().run.unwrap())
}

#[tokio::test]
async fn source_paths_reject_native_content_without_blanket_title_rejection() {
    let service = service().await;
    let minted = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: unique("assoc-src"),
            parent: "assoc-owner".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let world = world_in(&service, &minted).await;
    let registration = service
        .create_source_registration(Request::new(pb::CreateSourceRegistrationRequest {
            registration: Some(pb::SourceRegistration {
                id: unique("assoc-source"),
                administration_namespace: minted.clone(),
                destination_namespaces: vec![minted],
                profile_revision: 1,
                ownership_policy_revision: "policy-v1".into(),
                deletion_policy: pb::SourceDeletionPolicy::RetainNative as i32,
                ..Default::default()
            }),
        }))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap();
    let product = &world.product;
    let etag = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::DataProduct as i32,
            id: Some(eid(&world.namespace, "offering")),
        }))
        .await
        .unwrap()
        .into_inner()
        .etag;
    let whole = source_map(
        &service,
        &registration,
        "offering",
        product,
        &etag,
        whole_policy(),
        pb::source_selection::Selection::Entity(owner(product)),
    )
    .await
    .unwrap_err();
    assert_eq!(whole.code(), Code::InvalidArgument, "{whole}");
    assert!(whole.message().contains("native authoring only"), "{whole}");
    let association = product_of(product).resource_associations[0]
        .connection_id
        .clone()
        .unwrap();
    let selected = source_map(
        &service,
        &registration,
        "association",
        product,
        &etag,
        none_policy(),
        pb::source_selection::Selection::Connection(pb::ConnectionRef {
            owner: Some(owner(product)),
            owner_uid: product.system.as_ref().unwrap().uid.clone(),
            id: Some(association),
        }),
    )
    .await
    .unwrap_err();
    assert!(
        selected.message().contains("native authoring only"),
        "{selected}"
    );
    let specification = &world.specification;
    let spec_etag = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Specification as i32,
            id: Some(eid(&world.namespace, "contract")),
        }))
        .await
        .unwrap()
        .into_inner()
        .etag;
    let spec_whole = source_map(
        &service,
        &registration,
        "contract",
        specification,
        &spec_etag,
        whole_policy(),
        pb::source_selection::Selection::Entity(owner(specification)),
    )
    .await
    .unwrap_err();
    assert!(
        spec_whole.message().contains("native authoring only"),
        "{spec_whole}"
    );
    let declared = spec_of(specification).message_channels[0]
        .connection_id
        .clone()
        .unwrap();
    let channel_selected = source_map(
        &service,
        &registration,
        "channel",
        specification,
        &spec_etag,
        none_policy(),
        pb::source_selection::Selection::Connection(pb::ConnectionRef {
            owner: Some(owner(specification)),
            owner_uid: specification.system.as_ref().unwrap().uid.clone(),
            id: Some(declared),
        }),
    )
    .await
    .unwrap_err();
    assert!(
        channel_selected.message().contains("native authoring only"),
        "{channel_selected}"
    );
    let binding = spec_of(specification).messages[0].schemas[0]
        .connection_id
        .clone()
        .unwrap();
    let binding_selected = source_map(
        &service,
        &registration,
        "binding",
        specification,
        &spec_etag,
        none_policy(),
        pb::source_selection::Selection::Connection(pb::ConnectionRef {
            owner: Some(owner(specification)),
            owner_uid: specification.system.as_ref().unwrap().uid.clone(),
            id: Some(binding),
        }),
    )
    .await
    .unwrap_err();
    assert!(
        binding_selected.message().contains("native authoring only"),
        "{binding_selected}"
    );
    source_map(
        &service,
        &registration,
        "offering",
        product,
        &etag,
        title_policy(),
        pb::source_selection::Selection::Entity(owner(product)),
    )
    .await
    .unwrap();
    let snapshot = source_snapshot(&service, &registration, "offering").await;
    let mut incoming = product.clone();
    incoming.system = None;
    let pb::entity::Kind::DataProduct(value) = incoming.kind.as_mut().unwrap() else {
        panic!("product");
    };
    let preserved = value.resource_associations.clone();
    value.title = "Renamed".into();
    value.resource_associations.clear();
    let run = source_preview(&service, &registration, &snapshot, "offering", incoming)
        .await
        .unwrap();
    let applied = service
        .apply_source_run(Request::new(pb::ApplySourceRunRequest {
            registration_id: registration.id.clone(),
            run_id: run.id,
            if_match: run.revision,
            preview_digest: run.preview_digest,
        }))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(applied.status, pb::SourceRunStatus::Applied as i32);
    let renamed = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::DataProduct as i32,
            id: Some(eid(&world.namespace, "offering")),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let renamed_product = product_of(&renamed);
    assert_eq!(renamed_product.title, "Renamed");
    assert_eq!(renamed_product.resource_associations, preserved);
    let absent = eid(&world.namespace, "fresh");
    let fresh = pb::Entity {
        kind: Some(pb::entity::Kind::DataProduct(pb::DataProduct {
            id: Some(absent.clone()),
            title: "Fresh".into(),
            outputs: product_of(&renamed).outputs.clone(),
            resource_associations: preserved.clone(),
            ..Default::default()
        })),
        ..Default::default()
    };
    service
        .create_source_mapping(Request::new(pb::CreateSourceMappingRequest {
            registration_revision: registration.revision.clone(),
            mapping: Some(pb::SourceMapping {
                id: unique("map"),
                source: Some(source_key(&registration.id, "fresh")),
                selection: Some(pb::SourceSelection {
                    selection: Some(pb::source_selection::Selection::Entity(pb::EntityRef {
                        kind: pb::EntityKind::DataProduct as i32,
                        id: Some(absent),
                    })),
                }),
                ownership: Some(title_policy()),
                ..Default::default()
            }),
        }))
        .await
        .unwrap();
    let snapshot = source_snapshot(&service, &registration, "fresh").await;
    let created = source_preview(&service, &registration, &snapshot, "fresh", fresh)
        .await
        .unwrap_err();
    assert_eq!(created.code(), Code::InvalidArgument, "{created}");
    assert!(
        created.message().contains("native authoring only"),
        "{created}"
    );
}

#[tokio::test]
async fn bound_callers_get_one_generic_failure_for_hidden_and_foreign_branches() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let registry = trogon_atlas_server::auth::TokenRegistry::from_file_contents(
        toml::from_str(
            r#"
[principals.admin]
role = "admin"
tokens = ["assoc-admin"]
[principals.reader]
role = "reader"
tokens = ["assoc-reader"]
[principals.limited]
role = "reader"
parent = "assoc-visible"
tokens = ["assoc-limited"]
"#,
        )
        .unwrap(),
    )
    .unwrap();
    let direct = service().await;
    let hidden_id = direct
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: unique("assoc-hid"),
            parent: "assoc-hidden".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let visible_id = direct
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: unique("assoc-vis"),
            parent: "assoc-visible".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let loaded = world_in(&direct, &hidden_id).await;
    let implementation = service().await;
    let grpc = pb::event_model_service_server::EventModelServiceServer::new(implementation);
    let plane = trogon_atlas_server::mcp::data_plane(
        grpc,
        trogon_atlas_server::auth::BearerAuth::new(Arc::new(registry)),
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(plane)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let mut client = pb::event_model_service_client::EventModelServiceClient::connect(format!(
        "http://{address}"
    ))
    .await
    .unwrap();
    fn admin<T>(request: Request<T>) -> Request<T> {
        let mut request = request;
        request
            .metadata_mut()
            .insert("authorization", "Bearer assoc-admin".parse().unwrap());
        request
    }
    let secret = loaded.channel.clone();
    let limited = |body, branch: Option<&str>| {
        let mut request = Request::new(body);
        request
            .metadata_mut()
            .insert("authorization", "Bearer assoc-limited".parse().unwrap());
        if let Some(branch) = branch {
            request
                .metadata_mut()
                .insert("x-trogon-atlas-branch", branch.parse().unwrap());
        }
        request
    };
    let hidden_status = client
        .list_channel_messages(limited(
            pb::ListChannelMessagesRequest {
                channel: Some(pb::ChannelRef {
                    id: Some(secret.clone()),
                }),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap_err();
    let missing_status = client
        .list_channel_messages(limited(
            pb::ListChannelMessagesRequest {
                channel: Some(pb::ChannelRef {
                    id: Some(eid(&visible_id, "absent")),
                }),
                ..Default::default()
            },
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(hidden_status.code(), missing_status.code());
    assert_eq!(hidden_status.message(), missing_status.message());
    assert_eq!(hidden_status.message(), "resource unavailable");
    assert!(!hidden_status.message().contains("Orders"));
    let branch = "@assoc-hidden:work";
    client
        .create_branch(admin(Request::new(pb::CreateBranchRequest {
            name: branch.into(),
            ..Default::default()
        })))
        .await
        .unwrap();
    let foreign = client
        .list_channel_messages(limited(
            pb::ListChannelMessagesRequest {
                channel: Some(pb::ChannelRef { id: Some(secret) }),
                ..Default::default()
            },
            Some(branch),
        ))
        .await
        .unwrap_err();
    assert_eq!(foreign.code(), Code::NotFound);
    assert_eq!(foreign.message(), "resource unavailable");
    assert!(!foreign.message().contains("Orders"));
}

#[tokio::test]
async fn message_selector_rejects_stale_uid_and_missing_message() {
    let service = service().await;
    let world = world(&service).await;
    let stored = product_of(&world.product).clone();
    let mut stale = stored.clone();
    stale.id.as_mut().unwrap().slug = unique("stale-message");
    let pb::product_resource_association::Target::Message(message) =
        stale.resource_associations[3].target.as_mut().unwrap()
    else {
        panic!("message selector");
    };
    message.owner_uid = "stale-owner".into();
    let error = put_expect(&service, pb::entity::Kind::DataProduct(stale)).await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    assert!(
        error
            .message()
            .contains("DATA_PRODUCT_REFERENCE_UNRESOLVED"),
        "{error}"
    );
    let mut missing = stored;
    missing.id.as_mut().unwrap().slug = unique("missing-message");
    let pb::product_resource_association::Target::Message(message) =
        missing.resource_associations[3].target.as_mut().unwrap()
    else {
        panic!("message selector");
    };
    message.member_id = Some(member("99999999-9999-4999-8999-999999999999"));
    let error = put_expect(&service, pb::entity::Kind::DataProduct(missing)).await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    assert!(
        error
            .message()
            .contains("DATA_PRODUCT_REFERENCE_UNRESOLVED"),
        "{error}"
    );
}

#[tokio::test]
async fn title_edit_keeps_unchanged_dangling_association_and_rejects_a_new_target() {
    let service = service().await;
    let world = world(&service).await;
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(world.event.clone()),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut product = product_of(&world.product).clone();
    let preserved = product.resource_associations.clone();
    product.title = "Renamed".into();
    let updated = put(
        &service,
        pb::entity::Kind::DataProduct(product.clone()),
        None,
    )
    .await;
    let updated_entity = updated.entity.unwrap();
    let updated_product = product_of(&updated_entity);
    assert_eq!(updated_product.title, "Renamed");
    assert_eq!(updated_product.resource_associations, preserved);
    product.resource_associations[1].connection_id = None;
    product.resource_associations[1].target = Some(
        pb::product_resource_association::Target::Event(pb::EventRef {
            id: Some(eid(&world.namespace, "missing-event")),
        }),
    );
    let error = put_expect(&service, pb::entity::Kind::DataProduct(product)).await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    assert!(
        error
            .message()
            .contains("DATA_PRODUCT_REFERENCE_UNRESOLVED"),
        "{error}"
    );
}

#[tokio::test]
async fn title_edit_keeps_unchanged_dangling_message_channel_and_rejects_a_new_channel() {
    let service = service().await;
    let world = world(&service).await;
    let mut specification = spec_of(&world.specification).clone();
    specification.operations.clear();
    let cleared = put(
        &service,
        pb::entity::Kind::Specification(specification),
        None,
    )
    .await;
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Channel as i32,
            id: Some(world.channel.clone()),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut specification = spec_of(&cleared.entity.unwrap()).clone();
    let preserved = specification.message_channels.clone();
    specification.title = "Renamed contract".into();
    let updated = put(
        &service,
        pb::entity::Kind::Specification(specification.clone()),
        None,
    )
    .await;
    let updated_entity = updated.entity.unwrap();
    let updated_specification = spec_of(&updated_entity);
    assert_eq!(updated_specification.title, "Renamed contract");
    assert_eq!(updated_specification.message_channels, preserved);
    specification.message_channels[0].connection_id = None;
    specification.message_channels[0].channel = Some(pb::ChannelRef {
        id: Some(eid(&world.namespace, "missing-channel")),
    });
    let error = put_expect(&service, pb::entity::Kind::Specification(specification)).await;
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    assert!(
        error
            .message()
            .contains("SPECIFICATION_REFERENCE_UNRESOLVED"),
        "{error}"
    );
}

#[tokio::test]
async fn channel_page_token_rejects_a_second_caller_with_the_same_scope() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let registry = trogon_atlas_server::auth::TokenRegistry::from_file_contents(
        toml::from_str(
            r#"
[principals.reader-a]
role = "reader"
tokens = ["assoc-reader-a"]
[principals.reader-b]
role = "reader"
tokens = ["assoc-reader-b"]
"#,
        )
        .unwrap(),
    )
    .unwrap();
    let store: Arc<dyn Store> = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let direct = EventModelServiceImpl::try_new(Arc::clone(&store), ServerMetrics::default())
        .unwrap()
        .with_snapshot_cap(0);
    let world = world(&direct).await;
    let grpc = pb::event_model_service_server::EventModelServiceServer::new(
        EventModelServiceImpl::try_new(store, ServerMetrics::default())
            .unwrap()
            .with_snapshot_cap(0),
    );
    let plane = trogon_atlas_server::mcp::data_plane(
        grpc,
        trogon_atlas_server::auth::BearerAuth::new(Arc::new(registry)),
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(plane)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let mut client = pb::event_model_service_client::EventModelServiceClient::connect(format!(
        "http://{address}"
    ))
    .await
    .unwrap();
    let authed = |token: &'static str, body: pb::ListChannelMessagesRequest| {
        let mut request = Request::new(body);
        request
            .metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        request
    };
    let channel = pb::ChannelRef {
        id: Some(world.channel.clone()),
    };
    let first = client
        .list_channel_messages(authed(
            "assoc-reader-a",
            pb::ListChannelMessagesRequest {
                channel: Some(channel.clone()),
                page_size: 1,
                ..Default::default()
            },
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.records.len(), 1, "{first:?}");
    assert!(!first.next_page_token.is_empty());
    let continued = client
        .list_channel_messages(authed(
            "assoc-reader-a",
            pb::ListChannelMessagesRequest {
                channel: Some(channel.clone()),
                page_size: 1,
                page_token: first.next_page_token.clone(),
            },
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(continued.records.len(), 1, "{continued:?}");
    let rejected = client
        .list_channel_messages(authed(
            "assoc-reader-b",
            pb::ListChannelMessagesRequest {
                channel: Some(channel),
                page_size: 1,
                page_token: first.next_page_token,
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument, "{rejected}");
    assert_eq!(rejected.message(), "page token does not match this query");
}

#[tokio::test]
async fn channel_page_scans_the_full_scope_before_paging() {
    let service = service().await;
    let namespace = unique("assoc-budget");
    let channel_id = eid(&namespace, "orders");
    service
        .store
        .put(
            pb::EntityKind::Channel,
            &entity(pb::entity::Kind::Channel(pb::Channel {
                id: Some(channel_id.clone()),
                title: "Orders".into(),
                ..Default::default()
            })),
            false,
            trogon_atlas_store::WriteContext::baseline(),
        )
        .await
        .unwrap();
    let mut ops = Vec::with_capacity(5_000);
    for index in 0..5_000 {
        ops.push(trogon_atlas_store::store::MutationOp::Put {
            kind: pb::EntityKind::Specification,
            entity: entity(pb::entity::Kind::Specification(pb::Specification {
                id: Some(eid(&namespace, &format!("s{index:04}"))),
                title: "Declared".into(),
                message_channels: vec![pb::MessageChannelDeclaration {
                    channel: Some(pb::ChannelRef {
                        id: Some(channel_id.clone()),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            if_match: None,
            force: false,
        });
    }
    for chunk in ops.chunks(500) {
        service
            .store
            .batch_apply(chunk, trogon_atlas_store::WriteContext::baseline())
            .await
            .unwrap();
    }
    let error = service
        .list_channel_messages(Request::new(pb::ListChannelMessagesRequest {
            channel: Some(pb::ChannelRef {
                id: Some(channel_id),
            }),
            page_size: 1,
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::ResourceExhausted, "{error}");
    assert_eq!(error.message(), "channel message budget exhausted");
}
