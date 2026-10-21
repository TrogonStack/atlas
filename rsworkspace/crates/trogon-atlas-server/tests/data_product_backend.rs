#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use tokio::net::TcpListener;
use tonic::{service::interceptor::InterceptedService, transport::Channel, Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

const ADMIN: &str = "data-product-admin";
const READER: &str = "data-product-reader";
const LIMITED: &str = "data-product-limited";
type Client = EventModelServiceClient<Channel>;
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
async fn client() -> Client {
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
tokens = ["{READER}"]
[principals.limited]
role = "writer"
parent = "visible"
tokens = ["{LIMITED}"]
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
fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "offering".into(),
        slug: slug.into(),
        version: 1,
    }
}
fn entity(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}
fn output(name: &str) -> pb::DataOutputPort {
    pb::DataOutputPort {
        name: Some(pb::PortName { value: name.into() }),
        data: Some(pb::OutputDataSelection {
            read_model: Some(pb::ReadModelRef {
                id: Some(id("result")),
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn product(slug: &str) -> pb::DataProduct {
    pb::DataProduct {
        id: Some(id(slug)),
        title: slug.into(),
        outputs: vec![output("Result")],
        ..Default::default()
    }
}
async fn put(client: &mut Client, value: pb::entity::Kind) -> pb::Entity {
    client
        .put_entity(request(
            pb::PutEntityRequest {
                entity: Some(entity(value)),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}
async fn inspect(
    client: &mut Client,
    slug: &str,
    branch: Option<&str>,
) -> pb::InspectDataProductResponse {
    client
        .inspect_data_product(request(
            pb::InspectDataProductRequest {
                product: Some(pb::DataProductRef { id: Some(id(slug)) }),
            },
            READER,
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
}
fn connection(owner: &pb::Entity, id: Option<pb::ConnectionId>) -> pb::ConnectionRef {
    pb::ConnectionRef {
        owner: Some(trogon_atlas_server::conv::entity_to_ref(owner).unwrap()),
        owner_uid: owner.system.as_ref().unwrap().uid.clone(),
        id,
    }
}
async fn anchors(client: &mut Client) -> (pb::Entity, pb::Entity, pb::Entity) {
    for slug in ["result", "other"] {
        put(
            client,
            pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            }),
        )
        .await;
    }
    put(
        client,
        pb::entity::Kind::Event(pb::Event {
            id: Some(id("fact")),
            title: "Fact".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        client,
        pb::entity::Kind::Command(pb::Command {
            id: Some(id("produce")),
            title: "Produce".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        client,
        pb::entity::Kind::Processor(pb::Processor {
            id: Some(id("worker")),
            title: "Worker".into(),
            ..Default::default()
        }),
    )
    .await;
    let emission = put(
        client,
        pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id("emit")),
            title: "Emit".into(),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(id("produce")),
                }),
                ..Default::default()
            }),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id("fact")),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let projection = put(
        client,
        pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
            id: Some(id("project")),
            title: "Project".into(),
            source_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id("fact")),
                }),
                ..Default::default()
            }],
            read_model: Some(pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id("result")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await;
    let automation = put(
        client,
        pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id("observe")),
            title: "Observe".into(),
            source_read_models: vec![pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id("result")),
                }),
                ..Default::default()
            }],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(id("worker")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await;
    (emission, projection, automation)
}
async fn reject_all(client: &mut Client, value: pb::DataProduct) {
    for preview in [false, true] {
        let body = pb::PutEntityRequest {
            entity: Some(entity(pb::entity::Kind::DataProduct(value.clone()))),
            validate_only: preview,
            ..Default::default()
        };
        let error = client
            .put_entity(request(body.clone(), ADMIN, None))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{error}");
        let error = client
            .batch_mutate(request(
                pb::BatchMutateRequest {
                    ops: vec![pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(body)),
                    }],
                    validate_only: preview,
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn authenticated_inspection_checks_real_paths_repeated_ports_and_branch_tombstones() {
    let mut client = client().await;
    let (_, _, _) = anchors(&mut client).await;
    let mut value = product("checked");
    value.inputs.push(pb::DataInputPort {
        name: Some(pb::PortName {
            value: "Fact".into(),
        }),
        data: Some(pb::InputDataSelection {
            target: Some(pb::input_data_selection::Target::Event(pb::EventRef {
                id: Some(id("fact")),
            })),
            ..Default::default()
        }),
        ..Default::default()
    });
    value.outputs.push(output("Again"));
    let saved = put(&mut client, pb::entity::Kind::DataProduct(value)).await;
    let pb::entity::Kind::DataProduct(saved) = saved.kind.unwrap() else {
        panic!()
    };
    assert_ne!(saved.outputs[0].port_id, saved.outputs[1].port_id);
    let first = inspect(&mut client, "checked", None).await;
    assert_eq!(first.offering, pb::DataProductOfferingState::Mapped as i32);
    assert_eq!(first.emissions.len(), 1);
    assert_eq!(first.producing.len(), 2);
    assert!(first.producing.iter().all(|p| p.path.len() == 2));
    assert!(!first.incomplete, "{first:?}");
    let again = inspect(&mut client, "checked", None).await;
    assert_eq!(
        first.scope.as_ref().unwrap().revision,
        again.scope.as_ref().unwrap().revision
    );
    assert!(first
        .scope
        .as_ref()
        .unwrap()
        .visited
        .iter()
        .any(|r| r.kind == pb::EntityKind::ReadModelSlice as i32));
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: "inspection-work".into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::ReadModel as i32,
                id: Some(id("result")),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ADMIN,
            Some("inspection-work"),
        ))
        .await
        .unwrap();
    let branch = inspect(&mut client, "checked", Some("inspection-work")).await;
    assert_eq!(
        branch.offering,
        pb::DataProductOfferingState::Unresolved as i32
    );
    assert_eq!(
        branch.producing,
        Vec::<pb::DataProductProducingEvidence>::new()
    );
    assert_eq!(
        branch.scope.as_ref().unwrap().branch.as_deref(),
        Some("inspection-work")
    );
    assert_ne!(
        first.scope.unwrap().revision,
        branch.scope.unwrap().revision
    );
    assert_eq!(
        inspect(&mut client, "checked", None).await.offering,
        pb::DataProductOfferingState::Mapped as i32
    );
    let denied = client
        .inspect_data_product(pb::InspectDataProductRequest {
            product: Some(pb::DataProductRef {
                id: Some(id("checked")),
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::Unauthenticated);
    let denied = client
        .inspect_data_product(request(
            pb::InspectDataProductRequest {
                product: Some(pb::DataProductRef {
                    id: Some(id("checked")),
                }),
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        denied.code(),
        Code::PermissionDenied | Code::NotFound
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn store_and_channel_access_are_exact_occurrences_and_stale_products_remain_inspectable() {
    let mut client = client().await;
    let (_, _, auto) = anchors(&mut client).await;
    let pb::entity::Kind::AutomationSlice(slice) = auto.kind.as_ref().unwrap() else {
        panic!()
    };
    let observation = connection(&auto, slice.source_read_models[0].connection_id.clone());
    let channel = put(
        &mut client,
        pb::entity::Kind::Channel(pb::Channel {
            id: Some(id("transport")),
            title: "Transport".into(),
            bindings: vec![pb::ChannelBinding {
                observation: Some(observation.clone()),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let pb::entity::Kind::Channel(channel_value) = channel.kind.as_ref().unwrap() else {
        panic!()
    };
    let store = put(
        &mut client,
        pb::entity::Kind::DataStore(pb::DataStore {
            id: Some(id("storage")),
            title: "Storage".into(),
            store_class: Some(pb::StoreClass {
                kind: Some(pb::store_class::Kind::Known(
                    pb::store_class::Known::Cache as i32,
                )),
            }),
            attachments: vec![pb::ReadModelAttachment {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id("result")),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let pb::entity::Kind::DataStore(store_value) = store.kind.as_ref().unwrap() else {
        panic!()
    };
    let mut value = product("access");
    value.outputs[0].access = vec![
        pb::OutputAccess {
            selection: Some(pb::output_access::Selection::Store(pb::StoreAccess {
                store: Some(pb::DataStoreRef {
                    id: Some(id("storage")),
                }),
                attachment: Some(connection(
                    &store,
                    store_value.attachments[0].connection_id.clone(),
                )),
            })),
            ..Default::default()
        },
        pb::OutputAccess {
            selection: Some(pb::output_access::Selection::Channel(pb::ChannelAccess {
                channel: Some(pb::ChannelRef {
                    id: Some(id("transport")),
                }),
                binding: Some(connection(
                    &channel,
                    channel_value.bindings[0].connection_id.clone(),
                )),
                observation: Some(observation),
            })),
            ..Default::default()
        },
    ];
    let repeated = value.outputs[0].access[0].clone();
    value.outputs[0].access.push(repeated);
    let mut mismatch = value.clone();
    mismatch.id = Some(id("mismatch"));
    mismatch.outputs[0].data.as_mut().unwrap().read_model = Some(pb::ReadModelRef {
        id: Some(id("other")),
    });
    reject_all(&mut client, mismatch).await;
    put(&mut client, pb::entity::Kind::DataProduct(value)).await;
    let checked = inspect(&mut client, "access", None).await;
    assert_eq!(checked.ports[0].access.len(), 3);
    assert!(checked.ports[0]
        .access
        .iter()
        .all(|a| a.state == pb::DataProductSelectionState::Resolved as i32));
    let mut changed = store_value.clone();
    changed.attachments.clear();
    changed.attachments.push(pb::ReadModelAttachment {
        read_model: Some(pb::ReadModelRef {
            id: Some(id("other")),
        }),
        ..Default::default()
    });
    put(&mut client, pb::entity::Kind::DataStore(changed)).await;
    let stale = inspect(&mut client, "access", None).await;
    assert_eq!(
        stale.offering,
        pb::DataProductOfferingState::Unresolved as i32
    );
    assert_eq!(
        stale.ports[0].access[0].state,
        pb::DataProductSelectionState::Unresolved as i32
    );
    assert_eq!(
        stale.ports[0].access[1].state,
        pb::DataProductSelectionState::Resolved as i32
    );
    let error = client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Channel as i32,
                id: Some(id("transport")),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
}

fn member_name(value: &str) -> pb::MemberName {
    pb::MemberName {
        value: value.into(),
    }
}
async fn operation_contract(client: &mut Client) -> pb::Entity {
    let content = "opaque schema";
    let first = put(
        client,
        pb::entity::Kind::Specification(pb::Specification {
            id: Some(id("interface")),
            title: "Interface".into(),
            artifacts: vec![pb::ContractArtifact {
                name: Some(member_name("Artifact")),
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
                digest: Some(trogon_atlas_core::specification::content_digest(content)),
                ..Default::default()
            }],
            messages: vec![pb::ContractMessage {
                name: Some(member_name("Payload")),
                mapping: Some(pb::MessageReadModelMapping {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(id("result")),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let pb::entity::Kind::Specification(mut spec) = first.kind.unwrap() else {
        panic!()
    };
    spec.schemas.push(pb::ContractSchema {
        name: Some(member_name("Shape")),
        definition: Some(pb::SchemaDefinition {
            target: Some(pb::schema_definition::Target::Artifact(
                pb::LocalArtifactSelection {
                    artifact_id: spec.artifacts[0].member_id.clone(),
                    expected_digest: spec.artifacts[0].digest.clone(),
                },
            )),
            ..Default::default()
        }),
        ..Default::default()
    });
    spec.operations.push(pb::ContractOperation {
        name: Some(member_name("Read")),
        binding: Some(pb::OperationBinding {
            kind: Some(pb::operation_binding::Kind::Rpc(pb::RpcOperationBinding {
                service: "catalog.Reader".into(),
                method: "Get".into(),
            })),
        }),
        messages: vec![
            pb::OperationMessage {
                message_id: spec.messages[0].member_id.clone(),
                role: pb::operation_message::Role::Response as i32,
                ..Default::default()
            },
            pb::OperationMessage {
                message_id: spec.messages[0].member_id.clone(),
                role: pb::operation_message::Role::Request as i32,
                ..Default::default()
            },
        ],
        ..Default::default()
    });
    put(client, pb::entity::Kind::Specification(spec)).await
}
#[tokio::test(flavor = "multi_thread")]
async fn operation_access_rejects_request_and_stale_type_pins_and_keeps_provider_undeclared() {
    let mut client = client().await;
    anchors(&mut client).await;
    let owner = operation_contract(&mut client).await;
    let pb::entity::Kind::Specification(spec) = owner.kind.as_ref().unwrap() else {
        panic!()
    };
    let operation = pb::ContractMemberRef {
        specification: Some(pb::SpecificationRef {
            id: spec.id.clone(),
        }),
        owner_uid: owner.system.as_ref().unwrap().uid.clone(),
        kind: pb::ContractMemberKind::Operation as i32,
        member_id: spec.operations[0].member_id.clone(),
        ..Default::default()
    };
    let pin = pb::ContractTypeRef {
        schema: Some(pb::ContractMemberRef {
            kind: pb::ContractMemberKind::Schema as i32,
            member_id: spec.schemas[0].member_id.clone(),
            ..operation.clone()
        }),
        expected_artifact_id: spec.artifacts[0].member_id.clone(),
        expected_digest: spec.artifacts[0].digest.clone(),
        expected_selector: None,
    };
    let mut value = product("operation");
    value.outputs[0].contract = Some(pb::OutputContract {
        r#type: Some(pb::output_contract::Type::ContractType(pin)),
        ..Default::default()
    });
    value.outputs[0].access.push(pb::OutputAccess {
        selection: Some(pb::output_access::Selection::Operation(
            pb::OperationAccess {
                operation: Some(operation),
                response: Some(connection(
                    &owner,
                    spec.operations[0].messages[0].connection_id.clone(),
                )),
                provider_exposure: None,
            },
        )),
        ..Default::default()
    });
    let mut invalid = value.clone();
    invalid.id = Some(id("request-access"));
    if let Some(pb::output_access::Selection::Operation(access)) =
        &mut invalid.outputs[0].access[0].selection
    {
        access.response = Some(connection(
            &owner,
            spec.operations[0].messages[1].connection_id.clone(),
        ));
    }
    reject_all(&mut client, invalid).await;
    let mut stale = value.clone();
    stale.id = Some(id("stale-type"));
    if let Some(pb::output_contract::Type::ContractType(pin)) =
        stale.outputs[0].contract.as_mut().unwrap().r#type.as_mut()
    {
        pin.expected_selector = Some(pb::OpaqueTypeSelector {
            syntax: "json-pointer".into(),
            value: "/wrong".into(),
        });
    }
    reject_all(&mut client, stale).await;
    put(&mut client, pb::entity::Kind::DataProduct(value)).await;
    let checked = inspect(&mut client, "operation", None).await;
    assert_eq!(
        checked.offering,
        pb::DataProductOfferingState::Mapped as i32
    );
    assert_eq!(
        checked.ports[0].contract,
        pb::DataProductSelectionState::Opaque as i32
    );
    assert_eq!(
        checked.ports[0].access[0].provider,
        pb::DataProductSelectionState::Undeclared as i32
    );
    assert_eq!(checked.ports[0].access[0].checked_connections.len(), 1);
    let mut changed = spec.clone();
    changed.operations[0].messages.remove(0);
    put(&mut client, pb::entity::Kind::Specification(changed)).await;
    assert_eq!(
        inspect(&mut client, "operation", None).await.offering,
        pb::DataProductOfferingState::Unresolved as i32
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn draft_missing_emission_and_final_batch_graph_are_distinct() {
    let mut client = client().await;
    let mut draft = product("draft");
    draft.outputs[0].data = None;
    put(&mut client, pb::entity::Kind::DataProduct(draft)).await;
    assert_eq!(
        inspect(&mut client, "draft", None).await.offering,
        pb::DataProductOfferingState::Draft as i32
    );
    let mut missing = product("missing");
    missing.inputs.push(pb::DataInputPort {
        name: Some(pb::PortName {
            value: "Unknown".into(),
        }),
        data: Some(pb::InputDataSelection {
            target: Some(pb::input_data_selection::Target::Event(pb::EventRef {
                id: Some(id("unemitted")),
            })),
            ..Default::default()
        }),
        ..Default::default()
    });
    put(
        &mut client,
        pb::entity::Kind::Event(pb::Event {
            id: Some(id("unemitted")),
            title: "Unemitted".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id("result")),
            title: "Result".into(),
            ..Default::default()
        }),
    )
    .await;
    reject_all(&mut client, missing.clone()).await;
    missing.id = Some(id("batch"));
    let operations = vec![
        entity(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id("new-emitter")),
            title: "Emitter".into(),
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id("unemitted")),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        entity(pb::entity::Kind::DataProduct(missing)),
    ];
    client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops: operations
                    .into_iter()
                    .map(|entity| pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(entity),
                            ..Default::default()
                        })),
                    })
                    .collect(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(inspect(&mut client, "batch", None).await.emissions.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn event_occurrence_budget_is_shared_across_batch_products_and_resets_per_rpc() {
    let mut client = client().await;
    put(
        &mut client,
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id("result")),
            title: "Result".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::Event(pb::Event {
            id: Some(id("fact")),
            title: "Fact".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id("many-emissions")),
            title: "Many emissions".into(),
            emitted_events: vec![
                pb::EventEdge {
                    event: Some(pb::EventRef {
                        id: Some(id("fact"))
                    }),
                    ..Default::default()
                };
                2_500
            ],
            ..Default::default()
        }),
    )
    .await;
    let product = |slug: &str| {
        let mut product = product(slug);
        product.inputs.push(pb::DataInputPort {
            name: Some(pb::PortName {
                value: "Fact".into(),
            }),
            data: Some(pb::InputDataSelection {
                target: Some(pb::input_data_selection::Target::Event(pb::EventRef {
                    id: Some(id("fact")),
                })),
                ..Default::default()
            }),
            ..Default::default()
        });
        product
    };
    for preview in [true, false] {
        let error = client
            .batch_mutate(request(
                pb::BatchMutateRequest {
                    validate_only: preview,
                    ops: ["first", "second", "third"]
                        .into_iter()
                        .map(|slug| pb::BatchMutateOp {
                            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                                entity: Some(entity(pb::entity::Kind::DataProduct(product(slug)))),
                                ..Default::default()
                            })),
                        })
                        .collect(),
                    ..Default::default()
                },
                ADMIN,
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::ResourceExhausted, "{error}");
        let absent = client
            .get_entity(request(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::DataProduct as i32,
                    id: Some(id("first")),
                },
                READER,
                None,
            ))
            .await
            .unwrap_err();
        assert_eq!(absent.code(), Code::NotFound);
    }
    client
        .batch_mutate(request(
            pb::BatchMutateRequest {
                ops: ["first", "second"]
                    .into_iter()
                    .map(|slug| pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(entity(pb::entity::Kind::DataProduct(product(slug)))),
                            ..Default::default()
                        })),
                    })
                    .collect(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        inspect(&mut client, "first", None).await.emissions.len(),
        2_500
    );
    put(&mut client, pb::entity::Kind::DataProduct(product("third"))).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restricted_inspection_redacts_hidden_targets_and_hidden_referrers_block_safe_delete() {
    let mut client = client().await;
    let mut namespace_ids = Vec::new();
    for (name, parent) in [("offering", "visible"), ("private-data", "hidden")] {
        let record = client
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
            .unwrap();
        namespace_ids.push(record.id);
    }
    let visible_id = namespace_ids[0].clone();
    let hidden_id = namespace_ids[1].clone();
    let hidden = pb::Id {
        namespace: hidden_id.clone(),
        slug: "private-result".into(),
        version: 1,
    };
    put(
        &mut client,
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(hidden.clone()),
            title: "Private result".into(),
            ..Default::default()
        }),
    )
    .await;
    let mut value = product("restricted");
    value.id.as_mut().unwrap().namespace = visible_id.clone();
    value.outputs[0].data.as_mut().unwrap().read_model =
        Some(pb::ReadModelRef { id: Some(hidden) });
    put(&mut client, pb::entity::Kind::DataProduct(value)).await;
    let report = client
        .inspect_data_product(request(
            pb::InspectDataProductRequest {
                product: Some(pb::DataProductRef {
                    id: Some(pb::Id {
                        namespace: visible_id.clone(),
                        ..id("restricted")
                    }),
                }),
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        report.offering,
        pb::DataProductOfferingState::Unresolved as i32
    );
    assert_eq!(
        report.ports[0].data,
        pb::DataProductSelectionState::Unresolved as i32
    );
    let report = format!("{report:?}");
    for private in [
        "private-data",
        "private-result",
        "Private result",
        hidden_id.as_str(),
    ] {
        assert!(!report.contains(private), "{report}");
    }
    put(
        &mut client,
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(pb::Id {
                namespace: visible_id.clone(),
                ..id("result")
            }),
            title: "Result".into(),
            ..Default::default()
        }),
    )
    .await;
    let mut hidden_referrer = product("hidden-referrer");
    hidden_referrer.id.as_mut().unwrap().namespace = hidden_id;
    hidden_referrer.outputs[0]
        .data
        .as_mut()
        .unwrap()
        .read_model
        .as_mut()
        .unwrap()
        .id
        .as_mut()
        .unwrap()
        .namespace = visible_id.clone();
    put(&mut client, pb::entity::Kind::DataProduct(hidden_referrer)).await;
    let error = client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::ReadModel as i32,
                id: Some(pb::Id {
                    namespace: visible_id.clone(),
                    ..id("result")
                }),
                ..Default::default()
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition);
    assert!(!format!("{error:?}").contains("hidden-referrer"));
    client
        .create_branch(request(
            pb::CreateBranchRequest {
                name: "global-inspection".into(),
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let denied = client
        .inspect_data_product(request(
            pb::InspectDataProductRequest {
                product: Some(pb::DataProductRef {
                    id: Some(pb::Id {
                        namespace: visible_id,
                        ..id("restricted")
                    }),
                }),
            },
            LIMITED,
            Some("global-inspection"),
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn native_and_compiled_contracts_select_exact_versions_and_reject_unknown_messages() {
    let mut client = client().await;
    put(
        &mut client,
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id("result")),
            title: "Result".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::Schema(pb::Schema {
            id: Some(id("shape")),
            title: "Shape".into(),
            ..Default::default()
        }),
    )
    .await;
    put(
        &mut client,
        pb::entity::Kind::TypeLibrary(pb::TypeLibrary {
            id: Some(id("types")),
            title: "Types".into(),
            files: vec![pb::ProtoSourceFile {
                path: "types.proto".into(),
                content: "syntax = \"proto3\"; package types.v1; message Result {}".into(),
            }],
            ..Default::default()
        }),
    )
    .await;
    let mut value = product("typed");
    value.outputs[0].contract = Some(pb::OutputContract {
        r#type: Some(pb::output_contract::Type::Schema(pb::SchemaRef {
            id: Some(id("shape")),
        })),
        ..Default::default()
    });
    let mut compiled = output("Compiled");
    compiled.contract = Some(pb::OutputContract {
        r#type: Some(pb::output_contract::Type::LibraryType(
            pb::TypeLibraryTypeRef {
                library: Some(pb::TypeLibraryRef {
                    id: Some(id("types")),
                }),
                message: Some(pb::QualifiedProtoName {
                    value: "types.v1.Result".into(),
                }),
            },
        )),
        ..Default::default()
    });
    value.outputs.push(compiled.clone());
    let mut repeated = compiled;
    repeated.name = Some(pb::PortName {
        value: "Repeated selection".into(),
    });
    value.outputs.push(repeated);
    put(&mut client, pb::entity::Kind::DataProduct(value.clone())).await;
    let checked = inspect(&mut client, "typed", None).await;
    assert_eq!(
        checked.offering,
        pb::DataProductOfferingState::Mapped as i32
    );
    assert!(checked
        .ports
        .iter()
        .all(|p| p.contract == pb::DataProductSelectionState::Resolved as i32));
    value.id = Some(id("unknown-message"));
    if let Some(pb::output_contract::Type::LibraryType(r)) =
        &mut value.outputs[1].contract.as_mut().unwrap().r#type
    {
        r.message.as_mut().unwrap().value = "types.v1.Missing".into();
    }
    reject_all(&mut client, value).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn retained_event_confirmation_budget_failure_is_unknown_not_missing() {
    let mut client = client().await;
    let (emitter, _, _) = anchors(&mut client).await;
    let mut value = product("incomplete");
    value.inputs = ["First", "Second", "Third"]
        .into_iter()
        .map(|name| pb::DataInputPort {
            name: Some(pb::PortName { value: name.into() }),
            data: Some(pb::InputDataSelection {
                target: Some(pb::input_data_selection::Target::Event(pb::EventRef {
                    id: Some(id("fact")),
                })),
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect();
    put(&mut client, pb::entity::Kind::DataProduct(value)).await;
    let pb::entity::Kind::CommandSlice(mut slice) = emitter.kind.unwrap() else {
        panic!()
    };
    let mut added = slice.emitted_events[0].clone();
    added.connection_id = None;
    slice.emitted_events.extend(vec![added; 2_499]);
    put(&mut client, pb::entity::Kind::CommandSlice(slice)).await;
    let checked = inspect(&mut client, "incomplete", None).await;
    assert!(checked.incomplete);
    assert_eq!(
        checked.ports[2].data,
        pb::DataProductSelectionState::Unknown as i32
    );
    assert!(checked.ports[2].incomplete);
    assert_eq!(
        checked.offering,
        pb::DataProductOfferingState::Unknown as i32
    );
    assert_eq!(checked.emissions.len(), 5_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn operation_provider_selects_only_its_matching_exposure() {
    let mut client = client().await;
    anchors(&mut client).await;
    let first = operation_contract(&mut client).await;
    let pb::entity::Kind::Specification(mut spec) = first.kind.unwrap() else {
        panic!()
    };
    for name in ["provider", "other-provider"] {
        put(
            &mut client,
            pb::entity::Kind::Component(pb::Component {
                id: Some(id(name)),
                title: name.into(),
                ..Default::default()
            }),
        )
        .await;
    }
    let mut second = spec.operations[0].clone();
    second.member_id = None;
    second.name = Some(member_name("Other operation"));
    for edge in &mut second.messages {
        edge.connection_id = None;
    }
    spec.operations.push(second);
    let second = put(&mut client, pb::entity::Kind::Specification(spec)).await;
    let pb::entity::Kind::Specification(mut spec) = second.kind.unwrap() else {
        panic!()
    };
    spec.exposures = vec![
        pb::ProviderExposure {
            operation_id: spec.operations[0].member_id.clone(),
            provider: Some(pb::ProviderTarget {
                target: Some(pb::provider_target::Target::Component(pb::ComponentRef {
                    id: Some(id("provider")),
                })),
            }),
            ..Default::default()
        },
        pb::ProviderExposure {
            operation_id: spec.operations[1].member_id.clone(),
            provider: Some(pb::ProviderTarget {
                target: Some(pb::provider_target::Target::Component(pb::ComponentRef {
                    id: Some(id("other-provider")),
                })),
            }),
            ..Default::default()
        },
    ];
    let owner = put(&mut client, pb::entity::Kind::Specification(spec)).await;
    let pb::entity::Kind::Specification(spec) = owner.kind.as_ref().unwrap() else {
        panic!()
    };
    let selected = connection(&owner, spec.exposures[0].connection_id.clone());
    let mut value = product("provider");
    value.outputs[0].access.push(pb::OutputAccess {
        selection: Some(pb::output_access::Selection::Operation(
            pb::OperationAccess {
                operation: Some(pb::ContractMemberRef {
                    specification: Some(pb::SpecificationRef {
                        id: spec.id.clone(),
                    }),
                    owner_uid: owner.system.as_ref().unwrap().uid.clone(),
                    kind: pb::ContractMemberKind::Operation as i32,
                    member_id: spec.operations[0].member_id.clone(),
                    ..Default::default()
                }),
                response: Some(connection(
                    &owner,
                    spec.operations[0].messages[0].connection_id.clone(),
                )),
                provider_exposure: Some(selected.clone()),
            },
        )),
        ..Default::default()
    });
    let mut invalid = value.clone();
    invalid.id = Some(id("cross-operation"));
    if let Some(pb::output_access::Selection::Operation(access)) =
        &mut invalid.outputs[0].access[0].selection
    {
        access.provider_exposure =
            Some(connection(&owner, spec.exposures[1].connection_id.clone()));
    }
    reject_all(&mut client, invalid).await;
    put(&mut client, pb::entity::Kind::DataProduct(value)).await;
    let checked = inspect(&mut client, "provider", None).await;
    assert_eq!(
        checked.ports[0].access[0].provider,
        pb::DataProductSelectionState::Resolved as i32
    );
    assert_eq!(checked.ports[0].access[0].checked_connections.len(), 2);
    assert_eq!(checked.ports[0].access[0].checked_connections[1], selected);
}

async fn retained_access_fixture(
    client: &mut Client,
    owner_namespace: &str,
    target_namespace: &str,
) -> pb::DataProductRef {
    let target = pb::Id {
        namespace: target_namespace.into(),
        ..id("retained-output")
    };
    put(
        client,
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(target.clone()),
            title: "Retained output".into(),
            ..Default::default()
        }),
    )
    .await;
    let store_id = pb::Id {
        namespace: owner_namespace.into(),
        ..id("retained-storage")
    };
    let owner = put(
        client,
        pb::entity::Kind::DataStore(pb::DataStore {
            id: Some(store_id.clone()),
            title: "Retained storage".into(),
            store_class: Some(pb::StoreClass {
                kind: Some(pb::store_class::Kind::Known(
                    pb::store_class::Known::Cache as i32,
                )),
            }),
            attachments: vec![pb::ReadModelAttachment {
                read_model: Some(pb::ReadModelRef {
                    id: Some(target.clone()),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let pb::entity::Kind::DataStore(store) = owner.kind.as_ref().unwrap() else {
        panic!("DataStore")
    };
    let mut product = product("retained-access");
    product.id.as_mut().unwrap().namespace = owner_namespace.into();
    product.outputs[0].data.as_mut().unwrap().read_model =
        Some(pb::ReadModelRef { id: Some(target) });
    product.outputs[0].access.push(pb::OutputAccess {
        selection: Some(pb::output_access::Selection::Store(pb::StoreAccess {
            store: Some(pb::DataStoreRef { id: Some(store_id) }),
            attachment: Some(connection(
                &owner,
                store.attachments[0].connection_id.clone(),
            )),
        })),
        ..Default::default()
    });
    let reference = pb::DataProductRef {
        id: product.id.clone(),
    };
    put(client, pb::entity::Kind::DataProduct(product)).await;
    reference
}

fn assert_retained_access_unresolved(report: &pb::InspectDataProductResponse) {
    assert_eq!(
        report.offering,
        pb::DataProductOfferingState::Unresolved as i32
    );
    assert_eq!(
        report.ports[0].data,
        pb::DataProductSelectionState::Unresolved as i32
    );
    assert_eq!(
        report.ports[0].access[0].state,
        pb::DataProductSelectionState::Unresolved as i32,
        "an unresolved output cannot confirm selected access: {report:?}"
    );
    assert_eq!(
        report.ports[0].access[0].checked_connections,
        Vec::<pb::ConnectionRef>::new()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn retained_access_cannot_confirm_a_force_deleted_output() {
    let mut client = client().await;
    let product = retained_access_fixture(&mut client, "offering", "offering").await;
    client
        .delete_entity(request(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::ReadModel as i32,
                id: Some(id("retained-output")),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ADMIN,
            None,
        ))
        .await
        .unwrap();
    let report = client
        .inspect_data_product(request(
            pb::InspectDataProductRequest {
                product: Some(product),
            },
            READER,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_retained_access_unresolved(&report);
}

#[tokio::test(flavor = "multi_thread")]
async fn retained_access_cannot_confirm_an_inaccessible_output() {
    let mut client = client().await;
    let mut namespaces = Vec::new();
    for (name, parent) in [("offering", "visible"), ("private-data", "hidden")] {
        namespaces.push(
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
    let product = retained_access_fixture(&mut client, &namespaces[0], &namespaces[1]).await;
    let report = client
        .inspect_data_product(request(
            pb::InspectDataProductRequest {
                product: Some(product),
            },
            LIMITED,
            None,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_retained_access_unresolved(&report);
    assert!(!format!("{report:?}").contains(&namespaces[1]));
}
