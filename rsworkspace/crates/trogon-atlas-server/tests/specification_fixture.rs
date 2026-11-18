#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{collections::HashMap, sync::Arc};

use pb::event_model_service_server::EventModelService;
use tonic::Request;
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

fn reference(kind: pb::EntityKind, slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: kind as i32,
        id: Some(id(slug)),
    }
}

async fn put(service: &EventModelServiceImpl, kind: pb::entity::Kind) -> pb::Entity {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(kind),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

async fn save(service: &EventModelServiceImpl, spec: pb::Specification) -> pb::Entity {
    put(service, pb::entity::Kind::Specification(spec)).await
}

fn specification(entity: &pb::Entity) -> pb::Specification {
    match entity.kind.as_ref().unwrap() {
        pb::entity::Kind::Specification(value) => value.clone(),
        _ => panic!("expected Specification"),
    }
}

async fn contract(service: &EventModelServiceImpl, slug: &str) -> pb::Entity {
    let content = "type Order { id: ID! }\n";
    let first = save(
        service,
        pb::Specification {
            id: Some(id(slug)),
            title: slug.into(),
            artifacts: vec![pb::ContractArtifact {
                name: Some(name("Order source")),
                path: Some(pb::ArtifactPath {
                    value: "order.graphql".into(),
                }),
                format: Some(pb::ContractFormat {
                    kind: Some(pb::contract_format::Kind::Known(
                        pb::contract_format::Known::Graphql as i32,
                    )),
                }),
                media_type: Some(pb::MediaType {
                    value: "application/graphql".into(),
                }),
                content: content.into(),
                digest: Some(pb::ContentDigest {
                    sha256: "ead4699b692541fca1d93a621700779902c58f2eec55ed73c3d32597a00ec136"
                        .into(),
                }),
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .await;
    let mut spec = specification(&first);
    spec.schemas.push(pb::ContractSchema {
        name: Some(name("Order shape")),
        definition: Some(pb::SchemaDefinition {
            target: Some(pb::schema_definition::Target::Artifact(
                pb::LocalArtifactSelection {
                    artifact_id: spec.artifacts[0].member_id.clone(),
                    expected_digest: spec.artifacts[0].digest.clone(),
                },
            )),
            selector: Some(pb::OpaqueTypeSelector {
                syntax: "graphql-type".into(),
                value: "Order".into(),
            }),
            ..Default::default()
        }),
        ..Default::default()
    });
    save(service, spec).await
}

async fn outgoing(
    service: &EventModelServiceImpl,
    kind: pb::EntityKind,
    slug: &str,
) -> Vec<pb::Reference> {
    service
        .get_outgoing_references(Request::new(pb::GetReferencesRequest {
            kind: kind as i32,
            id: Some(id(slug)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references
}

#[tokio::test(flavor = "multi_thread")]
async fn connected_specification_fixture_preserves_roles_selections_and_native_export() {
    let nats = trogon_atlas_testsupport::shared().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default())
            .unwrap();
    let mut expected = HashMap::new();
    for slug in ["order-request", "order-receipt"] {
        let stored = put(
            &service,
            pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            }),
        )
        .await;
        expected.insert(
            pb::EntityKey::new(pb::EntityKind::ReadModel, &id(slug)),
            stored,
        );
    }
    for slug in ["public-gateway", "partner-gateway", "checkout"] {
        let stored = put(
            &service,
            pb::entity::Kind::Component(pb::Component {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            }),
        )
        .await;
        expected.insert(
            pb::EntityKey::new(pb::EntityKind::Component, &id(slug)),
            stored,
        );
    }
    for (kind, slug, value) in [
        (
            pb::EntityKind::Processor,
            "order-adapter",
            pb::entity::Kind::Processor(pb::Processor {
                id: Some(id("order-adapter")),
                title: "Order adapter".into(),
                ..Default::default()
            }),
        ),
        (
            pb::EntityKind::Command,
            "place-order",
            pb::entity::Kind::Command(pb::Command {
                id: Some(id("place-order")),
                title: "Place order".into(),
                ..Default::default()
            }),
        ),
        (
            pb::EntityKind::Event,
            "order-placed",
            pb::entity::Kind::Event(pb::Event {
                id: Some(id("order-placed")),
                title: "Order placed".into(),
                ..Default::default()
            }),
        ),
        (
            pb::EntityKind::Schema,
            "native-order-shape",
            pb::entity::Kind::Schema(pb::Schema {
                id: Some(id("native-order-shape")),
                title: "Native order shape".into(),
                ..Default::default()
            }),
        ),
    ] {
        expected.insert(
            pb::EntityKey::new(kind, &id(slug)),
            put(&service, value).await,
        );
    }
    let automation = put(
        &service,
        pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id("order-intake")),
            title: "Order intake".into(),
            source_read_models: vec![pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id("order-request")),
                }),
                ..Default::default()
            }],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(id("order-adapter")),
                }),
                ..Default::default()
            }),
            emitted_command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(id("place-order")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await;
    let observation = match automation.kind.as_ref().unwrap() {
        pb::entity::Kind::AutomationSlice(value) => pb::ConnectionRef {
            owner: Some(reference(pb::EntityKind::AutomationSlice, "order-intake")),
            owner_uid: automation.system.as_ref().unwrap().uid.clone(),
            id: value.source_read_models[0].connection_id.clone(),
        },
        _ => panic!(),
    };
    expected.insert(
        pb::EntityKey::new(pb::EntityKind::AutomationSlice, &id("order-intake")),
        automation,
    );
    for (kind, slug, value) in [
        (
            pb::EntityKind::CommandSlice,
            "handle-order",
            pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id("handle-order")),
                title: "Handle order".into(),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef {
                        id: Some(id("place-order")),
                    }),
                    ..Default::default()
                }),
                emitted_events: vec![pb::EventEdge {
                    event: Some(pb::EventRef {
                        id: Some(id("order-placed")),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
        ),
        (
            pb::EntityKind::ReadModelSlice,
            "receipt-projection",
            pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
                id: Some(id("receipt-projection")),
                title: "Receipt projection".into(),
                source_events: vec![pb::EventEdge {
                    event: Some(pb::EventRef {
                        id: Some(id("order-placed")),
                    }),
                    ..Default::default()
                }],
                read_model: Some(pb::ReadModelEdge {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(id("order-receipt")),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        ),
        (
            pb::EntityKind::Channel,
            "order-transport",
            pb::entity::Kind::Channel(pb::Channel {
                id: Some(id("order-transport")),
                title: "Order transport".into(),
                address: Some(pb::ChannelAddress {
                    template: "orders".into(),
                    ..Default::default()
                }),
                bindings: vec![pb::ChannelBinding {
                    observation: Some(observation),
                    ..Default::default()
                }],
                ..Default::default()
            }),
        ),
    ] {
        expected.insert(
            pb::EntityKey::new(kind, &id(slug)),
            put(&service, value).await,
        );
    }
    let library = put(&service, pb::entity::Kind::TypeLibrary(pb::TypeLibrary {
        id: Some(id("fixture.contract")), title: "Order types".into(),
        files: vec![pb::ProtoSourceFile { path: "orders.proto".into(), content: "syntax = \"proto3\"; package fixture.contract; message Order { string id = 1; }".into() }], ..Default::default()
    })).await;
    expected.insert(
        pb::EntityKey::new(pb::EntityKind::TypeLibrary, &id("fixture.contract")),
        library,
    );
    let shared = contract(&service, "shared-contract").await;
    let shared_spec = specification(&shared);
    let external_type = pb::ContractTypeRef {
        schema: Some(pb::ContractMemberRef {
            specification: Some(pb::SpecificationRef {
                id: Some(id("shared-contract")),
            }),
            owner_uid: shared.system.as_ref().unwrap().uid.clone(),
            kind: pb::ContractMemberKind::Schema as i32,
            member_id: shared_spec.schemas[0].member_id.clone(),
            ..Default::default()
        }),
        expected_artifact_id: shared_spec.artifacts[0].member_id.clone(),
        expected_digest: shared_spec.artifacts[0].digest.clone(),
        expected_selector: shared_spec.schemas[0]
            .definition
            .as_ref()
            .unwrap()
            .selector
            .clone(),
    };
    expected.insert(
        pb::EntityKey::new(pb::EntityKind::Specification, &id("shared-contract")),
        shared,
    );
    let initial = contract(&service, "order-api").await;
    let mut spec = specification(&initial);
    for (label, target) in [
        ("Submission", "order-request"),
        ("Receipt", "order-receipt"),
    ] {
        spec.messages.push(pb::ContractMessage {
            name: Some(name(label)),
            schemas: vec![pb::SchemaBinding {
                role: pb::schema_binding::Role::Payload as i32,
                r#type: Some(pb::schema_binding::Type::LocalSchema(
                    spec.schemas[0].member_id.clone().unwrap(),
                )),
                ..Default::default()
            }],
            mapping: Some(pb::MessageReadModelMapping {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id(target)),
                }),
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    for target in [
        pb::schema_binding::Type::ContractType(external_type),
        pb::schema_binding::Type::Schema(pb::SchemaRef {
            id: Some(id("native-order-shape")),
        }),
        pb::schema_binding::Type::LibraryType(pb::TypeLibraryTypeRef {
            library: Some(pb::TypeLibraryRef {
                id: Some(id("fixture.contract")),
            }),
            message: Some(pb::QualifiedProtoName {
                value: "fixture.contract.Order".into(),
            }),
        }),
    ] {
        spec.messages[0].schemas.push(pb::SchemaBinding {
            role: pb::schema_binding::Role::Payload as i32,
            r#type: Some(target),
            ..Default::default()
        });
    }
    let stored = save(&service, spec).await;
    let mut spec = specification(&stored);
    let request = spec.messages[0].member_id.clone();
    let response = spec.messages[1].member_id.clone();
    for (label, binding, occurrences) in [
        (
            "Submit",
            pb::operation_binding::Kind::Http(pb::HttpOperationBinding {
                method: "POST".into(),
                path: "/orders".into(),
                response_status_codes: vec![201],
            }),
            vec![
                (request, pb::operation_message::Role::Request),
                (response.clone(), pb::operation_message::Role::Response),
            ],
        ),
        (
            "Read receipt",
            pb::operation_binding::Kind::Http(pb::HttpOperationBinding {
                method: "GET".into(),
                path: "/orders/{id}".into(),
                response_status_codes: vec![200],
            }),
            vec![(response.clone(), pb::operation_message::Role::Response)],
        ),
        (
            "Receipt notice",
            pb::operation_binding::Kind::Channel(pb::ChannelOperationBinding {
                channel: Some(pb::ChannelRef {
                    id: Some(id("order-transport")),
                }),
                ..Default::default()
            }),
            vec![(response, pb::operation_message::Role::Notification)],
        ),
    ] {
        spec.operations.push(pb::ContractOperation {
            name: Some(name(label)),
            binding: Some(pb::OperationBinding {
                kind: Some(binding),
            }),
            messages: occurrences
                .into_iter()
                .map(|(message_id, role)| pb::OperationMessage {
                    message_id,
                    role: role as i32,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        });
    }
    let stored = save(&service, spec).await;
    let mut spec = specification(&stored);
    for provider in ["public-gateway", "partner-gateway"] {
        spec.exposures.push(pb::ProviderExposure {
            provider: Some(pb::ProviderTarget {
                target: Some(pb::provider_target::Target::Component(pb::ComponentRef {
                    id: Some(id(provider)),
                })),
            }),
            operation_id: spec.operations[0].member_id.clone(),
            ..Default::default()
        });
    }
    let stored = save(&service, spec).await;
    let mut spec = specification(&stored);
    for exposure in [spec.exposures[0].connection_id.clone(), None] {
        spec.usages.push(pb::ApiUsage {
            caller: Some(pb::ProviderTarget {
                target: Some(pb::provider_target::Target::Component(pb::ComponentRef {
                    id: Some(id("checkout")),
                })),
            }),
            operation_id: spec.operations[0].member_id.clone(),
            exposure_id: exposure,
            ..Default::default()
        });
    }
    let stored = save(&service, spec).await;
    let spec = specification(&stored);
    assert_eq!(spec.usages[0].exposure_id, spec.exposures[0].connection_id);
    assert_ne!(spec.usages[0].exposure_id, spec.exposures[1].connection_id);
    assert!(spec.usages[1].exposure_id.is_none());
    let submit = &spec.operations[0];
    assert_eq!(
        submit.messages[0].role,
        pb::operation_message::Role::Request as i32
    );
    assert_eq!(
        submit.messages[1].role,
        pb::operation_message::Role::Response as i32
    );
    assert_eq!(submit.messages[0].message_id, spec.messages[0].member_id);
    assert_eq!(submit.messages[1].message_id, spec.messages[1].member_id);
    assert_ne!(
        submit.messages[1].connection_id,
        spec.operations[1].messages[0].connection_id
    );
    assert_ne!(
        spec.messages[0].mapping.as_ref().unwrap().read_model,
        spec.messages[1].mapping.as_ref().unwrap().read_model
    );
    assert_eq!(
        spec.messages[0].schemas[0].r#type,
        spec.messages[1].schemas[0].r#type
    );
    expected.insert(
        pb::EntityKey::new(pb::EntityKind::Specification, &id("order-api")),
        stored,
    );
    let refs = outgoing(&service, pb::EntityKind::Specification, "order-api").await;
    for (kind, slug) in [
        (pb::EntityKind::ReadModel, "order-request"),
        (pb::EntityKind::ReadModel, "order-receipt"),
        (pb::EntityKind::Component, "public-gateway"),
        (pb::EntityKind::Component, "partner-gateway"),
        (pb::EntityKind::Component, "checkout"),
        (pb::EntityKind::Specification, "shared-contract"),
        (pb::EntityKind::Schema, "native-order-shape"),
        (pb::EntityKind::TypeLibrary, "fixture.contract"),
        (pb::EntityKind::Channel, "order-transport"),
    ] {
        assert!(
            refs.iter()
                .any(|r| r.to.as_ref() == Some(&reference(kind, slug))),
            "missing {kind:?}/{slug}"
        );
    }
    assert!(!refs.iter().any(|r| r.to.as_ref().is_some_and(|r| [
        pb::EntityKind::Command as i32,
        pb::EntityKind::Event as i32,
        pb::EntityKind::Processor as i32
    ]
    .contains(&r.kind))));
    assert_eq!(
        outgoing(&service, pb::EntityKind::Component, "checkout").await,
        [] as [pb::Reference; 0]
    );
    let model = put(
        &service,
        pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id("order-contracts")),
            title: "Order contracts".into(),
            members: expected
                .keys()
                .map(|key| pb::EntityRef {
                    kind: key.kind as i32,
                    id: Some(key.id()),
                })
                .collect(),
            ..Default::default()
        }),
    )
    .await;
    expected.insert(
        pb::EntityKey::new(pb::EntityKind::EventModel, &id("order-contracts")),
        model,
    );
    let exported = service
        .export_event_model_manifests(Request::new(pb::ExportEventModelManifestsRequest {
            id: Some(id("order-contracts")),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!exported.truncated);
    assert_eq!(exported.skipped, [] as [pb::SkippedEntity; 0]);
    let manifests = trogon_atlas_client::manifest::parse_str(
        &exported.manifests,
        "specification-fixture-export",
    )
    .unwrap();
    assert_eq!(manifests.len(), expected.len());
    for manifest in manifests {
        let key = pb::EntityKey::new(manifest.kind, &manifest.id);
        assert_eq!(
            manifest.entity.kind, expected[&key].kind,
            "export changed {key}"
        );
    }
}
