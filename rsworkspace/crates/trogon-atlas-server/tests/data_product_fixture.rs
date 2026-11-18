#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "product-fixture".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn port_name(value: &str) -> pb::PortName {
    pb::PortName {
        value: value.into(),
    }
}
fn member_name(value: &str) -> pb::MemberName {
    pb::MemberName {
        value: value.into(),
    }
}
fn read_model(slug: &str) -> pb::ReadModelRef {
    pb::ReadModelRef { id: Some(id(slug)) }
}
fn rm_edge(slug: &str) -> pb::ReadModelEdge {
    pb::ReadModelEdge {
        read_model: Some(read_model(slug)),
        ..Default::default()
    }
}
fn event_edge() -> pb::EventEdge {
    pb::EventEdge {
        event: Some(pb::EventRef {
            id: Some(id("placed")),
        }),
        ..Default::default()
    }
}
fn command_edge() -> pb::CommandEdge {
    pb::CommandEdge {
        command: Some(pb::CommandRef {
            id: Some(id("place")),
        }),
        ..Default::default()
    }
}
fn connection(owner: &pb::Entity, occurrence: Option<pb::ConnectionId>) -> pb::ConnectionRef {
    pb::ConnectionRef {
        owner: Some(trogon_atlas_server::conv::entity_to_ref(owner).unwrap()),
        owner_uid: owner.system.as_ref().unwrap().uid.clone(),
        id: occurrence,
    }
}
async fn put(service: &EventModelServiceImpl, kind: pb::entity::Kind) -> pb::Entity {
    let description = format!("{kind:?}");
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(kind),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap_or_else(|error| panic!("fixture put failed for {description}: {error}"))
        .into_inner()
        .entity
        .unwrap()
}
fn spec(value: &pb::Entity) -> pb::Specification {
    match value.kind.as_ref().unwrap() {
        pb::entity::Kind::Specification(v) => v.clone(),
        _ => panic!("Specification required"),
    }
}
fn product(value: &pb::Entity) -> pb::DataProduct {
    match value.kind.as_ref().unwrap() {
        pb::entity::Kind::DataProduct(v) => v.clone(),
        _ => panic!("DataProduct required"),
    }
}
fn output(name: &str) -> pb::DataOutputPort {
    pb::DataOutputPort {
        name: Some(port_name(name)),
        data: Some(pb::OutputDataSelection {
            read_model: Some(read_model("receipt")),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn connected_product_inspection_preserves_selected_access_and_independent_lineage() {
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap()
    .with_snapshot_cap(0);
    for slug in ["submitted", "receipt"] {
        put(
            &service,
            pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            }),
        )
        .await;
    }
    put(
        &service,
        pb::entity::Kind::Event(pb::Event {
            id: Some(id("placed")),
            title: "Placed".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &service,
        pb::entity::Kind::Command(pb::Command {
            id: Some(id("place")),
            title: "Place".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &service,
        pb::entity::Kind::Processor(pb::Processor {
            id: Some(id("adapter")),
            title: "Adapter".into(),
            ..Default::default()
        }),
    )
    .await;
    for slug in ["public", "partner"] {
        put(
            &service,
            pb::entity::Kind::Component(pb::Component {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            }),
        )
        .await;
    }
    put(
        &service,
        pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id("intake")),
            title: "Intake".into(),
            source_read_models: vec![rm_edge("submitted")],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(id("adapter")),
                }),
                ..Default::default()
            }),
            emitted_command: Some(command_edge()),
            ..Default::default()
        }),
    )
    .await;
    let emitted = put(
        &service,
        pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id("handle")),
            title: "Handle".into(),
            command: Some(command_edge()),
            emitted_events: vec![event_edge()],
            ..Default::default()
        }),
    )
    .await;
    put(
        &service,
        pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
            id: Some(id("project")),
            title: "Project".into(),
            source_events: vec![event_edge()],
            read_model: Some(rm_edge("receipt")),
            ..Default::default()
        }),
    )
    .await;
    let observed = put(
        &service,
        pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id("observe")),
            title: "Observe".into(),
            source_read_models: vec![rm_edge("receipt"), rm_edge("receipt")],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(id("adapter")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await;
    let pb::entity::Kind::AutomationSlice(observation) = observed.kind.as_ref().unwrap() else {
        panic!()
    };
    let observation_refs: Vec<_> = observation
        .source_read_models
        .iter()
        .map(|edge| connection(&observed, edge.connection_id.clone()))
        .collect();
    let channel = put(
        &service,
        pb::entity::Kind::Channel(pb::Channel {
            id: Some(id("delivery")),
            title: "Delivery".into(),
            bindings: observation_refs
                .iter()
                .map(|r| pb::ChannelBinding {
                    observation: Some(r.clone()),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }),
    )
    .await;
    let store = put(
        &service,
        pb::entity::Kind::DataStore(pb::DataStore {
            id: Some(id("warehouse")),
            title: "Warehouse".into(),
            store_class: Some(pb::StoreClass {
                kind: Some(pb::store_class::Kind::Other("warehouse".into())),
            }),
            attachments: vec![pb::ReadModelAttachment {
                read_model: Some(read_model("receipt")),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;

    let mut api = spec(
        &put(
            &service,
            pb::entity::Kind::Specification(pb::Specification {
                id: Some(id("api")),
                title: "API".into(),
                messages: ["submitted", "receipt"]
                    .into_iter()
                    .map(|slug| pb::ContractMessage {
                        name: Some(member_name(slug)),
                        mapping: Some(pb::MessageReadModelMapping {
                            read_model: Some(read_model(slug)),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }),
        )
        .await,
    );
    api.operations.push(pb::ContractOperation {
        name: Some(member_name("Submit")),
        messages: api
            .messages
            .iter()
            .enumerate()
            .map(|(i, m)| pb::OperationMessage {
                message_id: m.member_id.clone(),
                role: if i == 0 {
                    pb::operation_message::Role::Request
                } else {
                    pb::operation_message::Role::Response
                } as i32,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    });
    api = spec(&put(&service, pb::entity::Kind::Specification(api)).await);
    for slug in ["public", "partner"] {
        api.exposures.push(pb::ProviderExposure {
            provider: Some(pb::ProviderTarget {
                target: Some(pb::provider_target::Target::Component(pb::ComponentRef {
                    id: Some(id(slug)),
                })),
            }),
            operation_id: api.operations[0].member_id.clone(),
            ..Default::default()
        });
    }
    let api_owner = put(&service, pb::entity::Kind::Specification(api)).await;
    let api = spec(&api_owner);
    let response = connection(
        &api_owner,
        api.operations[0].messages[1].connection_id.clone(),
    );
    let exposure = connection(&api_owner, api.exposures[0].connection_id.clone());
    let operation = pb::ContractMemberRef {
        specification: Some(pb::SpecificationRef {
            id: Some(id("api")),
        }),
        owner_uid: api_owner.system.as_ref().unwrap().uid.clone(),
        kind: pb::ContractMemberKind::Operation as i32,
        member_id: api.operations[0].member_id.clone(),
        ..Default::default()
    };
    let api_access = |provider| pb::OutputAccess {
        selection: Some(pb::output_access::Selection::Operation(
            pb::OperationAccess {
                operation: Some(operation.clone()),
                response: Some(response.clone()),
                provider_exposure: provider,
            },
        )),
        ..Default::default()
    };
    let pb::entity::Kind::DataStore(store_value) = store.kind.as_ref().unwrap() else {
        panic!()
    };
    let pb::entity::Kind::Channel(channel_value) = channel.kind.as_ref().unwrap() else {
        panic!()
    };
    let mut receipt = output("Receipt");
    receipt.access.push(pb::OutputAccess {
        selection: Some(pb::output_access::Selection::Store(pb::StoreAccess {
            store: Some(pb::DataStoreRef {
                id: Some(id("warehouse")),
            }),
            attachment: Some(connection(
                &store,
                store_value.attachments[0].connection_id.clone(),
            )),
        })),
        ..Default::default()
    });
    for (binding, observation) in channel_value.bindings.iter().zip(&observation_refs) {
        receipt.access.push(pb::OutputAccess {
            selection: Some(pb::output_access::Selection::Channel(pb::ChannelAccess {
                channel: Some(pb::ChannelRef {
                    id: Some(id("delivery")),
                }),
                binding: Some(connection(&channel, binding.connection_id.clone())),
                observation: Some(observation.clone()),
            })),
            ..Default::default()
        });
    }
    receipt.access.extend([
        api_access(Some(exposure.clone())),
        api_access(None),
        api_access(Some(exposure)),
    ]);
    let authored = pb::DataProduct {
        id: Some(id("orders")),
        title: "Order data".into(),
        inputs: vec![
            pb::DataInputPort {
                name: Some(port_name("Submitted")),
                data: Some(pb::InputDataSelection {
                    target: Some(pb::input_data_selection::Target::ReadModel(read_model(
                        "submitted",
                    ))),
                    ..Default::default()
                }),
                ..Default::default()
            },
            pb::DataInputPort {
                name: Some(port_name("Placed")),
                data: Some(pb::InputDataSelection {
                    target: Some(pb::input_data_selection::Target::Event(pb::EventRef {
                        id: Some(id("placed")),
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ],
        outputs: vec![
            receipt,
            output("Receipt again"),
            pb::DataOutputPort {
                name: Some(port_name("Undeclared")),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let first = put(&service, pb::entity::Kind::DataProduct(authored)).await;
    let mut saved = product(&first);
    saved.outputs[0].derived_from = Some(pb::DeclaredLineage {
        inputs: vec![pb::LineageInput {
            input_port_id: saved.inputs[1].port_id.clone(),
            ..Default::default()
        }],
        ..Default::default()
    });
    saved.outputs[1].derived_from = Some(pb::DeclaredLineage::default());
    let persisted = put(&service, pb::entity::Kind::DataProduct(saved)).await;
    let saved = product(&persisted);
    let inspection = service
        .inspect_data_product(Request::new(pb::InspectDataProductRequest {
            product: Some(pb::DataProductRef {
                id: Some(id("orders")),
            }),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(inspection.product.as_ref(), Some(&saved));
    assert_eq!(
        inspection.offering,
        pb::DataProductOfferingState::Draft as i32
    );
    assert!(!inspection.incomplete, "{inspection:?}");
    assert_eq!(inspection.emissions.len(), 1);
    assert_eq!(
        inspection.emissions[0].emission.as_ref().unwrap().owner_uid,
        emitted.system.as_ref().unwrap().uid
    );
    let state = inspection
        .ports
        .iter()
        .find(|s| s.port.as_ref().unwrap().port_id == saved.outputs[0].port_id)
        .unwrap();
    assert_eq!(state.access.len(), 6);
    assert!(state
        .access
        .iter()
        .all(|a| a.state == pb::DataProductSelectionState::Resolved as i32));
    assert_eq!(
        state.access[3].provider,
        pb::DataProductSelectionState::Resolved as i32
    );
    assert_eq!(
        state.access[4].provider,
        pb::DataProductSelectionState::Undeclared as i32
    );
    assert_ne!(
        saved.outputs[0].access[3].connection_id,
        saved.outputs[0].access[5].connection_id
    );
    assert_ne!(saved.outputs[0].port_id, saved.outputs[1].port_id);
    for input in &saved.inputs {
        assert!(
            inspection
                .producing
                .iter()
                .any(|p| p.input.as_ref().unwrap().port_id == input.port_id
                    && p.output.as_ref().unwrap().port_id == saved.outputs[0].port_id
                    && !p.path.is_empty()),
            "{inspection:?}"
        );
    }
    assert_eq!(
        saved.outputs[0].derived_from.as_ref().unwrap().inputs.len(),
        1
    );
    assert_eq!(
        saved.outputs[1].derived_from.as_ref().unwrap().inputs,
        Vec::<pb::LineageInput>::new()
    );
    assert_ne!(inspection.scope.as_ref().unwrap().revision, "");
    let repeated = service
        .inspect_data_product(Request::new(pb::InspectDataProductRequest {
            product: Some(pb::DataProductRef {
                id: Some(id("orders")),
            }),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(inspection.scope, repeated.scope);
    let outbound = trogon_atlas_core::refs::outbound_refs(&persisted);
    assert!(!outbound
        .iter()
        .any(|r| r.to_kind == Some(pb::EntityKind::Command)
            || r.to_kind == Some(pb::EntityKind::Processor)));
    put(
        &service,
        pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id("model")),
            title: "Offering model".into(),
            members: vec![trogon_atlas_server::conv::entity_to_ref(&persisted).unwrap()],
            ..Default::default()
        }),
    )
    .await;
    let service = service.with_snapshot_cap(5_000);
    let exported = service
        .export_event_model_manifests(Request::new(pb::ExportEventModelManifestsRequest {
            id: Some(id("model")),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!exported.truncated);
    assert_eq!(exported.skipped, Vec::<pb::SkippedEntity>::new());
    let manifests =
        trogon_atlas_client::manifest::parse_str(&exported.manifests, "connected-product-export")
            .unwrap();
    let round_trip = manifests
        .iter()
        .find(|m| m.kind == pb::EntityKind::DataProduct)
        .unwrap();
    assert_eq!(round_trip.entity.kind, persisted.kind);
    let reapplied = put(&service, round_trip.entity.kind.clone().unwrap()).await;
    assert_eq!(reapplied.kind, persisted.kind);
    assert_eq!(
        reapplied.system.as_ref().unwrap().uid,
        persisted.system.as_ref().unwrap().uid
    );
}
