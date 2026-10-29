#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
#[path = "support/observed_claim_store.rs"]
mod observed_claim_store;
use std::sync::Arc;

use support::two_tenants::{authed, make_registry, serve_with_registry, ROOT_TOKEN};
use tonic::{transport::Channel, Code};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};
type Client = pb::event_model_service_client::EventModelServiceClient<Channel>;
fn id(namespace: &str, definition: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: definition.into(),
        version: 1,
    }
}
fn owner(namespace: &str, kind: pb::EntityKind, message: pb::ContractMemberRef) -> pb::Entity {
    let claims = Some(pb::MessageClaims {
        claims: vec![pb::MessageClaim {
            direction: pb::CommunicationDirection::Receive as i32,
            message: Some(message),
            ..Default::default()
        }],
        ..Default::default()
    });
    pb::Entity {
        kind: Some(match kind {
            pb::EntityKind::Component => pb::entity::Kind::Component(pb::Component {
                id: Some(id(namespace, "owner")),
                title: "Owner".into(),
                message_claims: claims,
                ..Default::default()
            }),
            pb::EntityKind::ExternalSystem => {
                pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
                    id: Some(id(namespace, "owner")),
                    title: "Owner".into(),
                    message_claims: claims,
                    ..Default::default()
                })
            }
            _ => pb::entity::Kind::Domain(pb::Domain {
                id: Some(id(namespace, "owner")),
                title: "Owner".into(),
                message_claims: claims,
                ..Default::default()
            }),
        }),
        ..Default::default()
    }
}
async fn fixture_with_store() -> (Client, String, Arc<trogon_atlas_store::NatsStore>) {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
    let mut client = serve_with_registry(service, make_registry()).await;
    let namespace = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "claims-acceptance".into(),
                parent: "commerce".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    (client, namespace, store)
}
async fn fixture() -> (Client, String) {
    let (client, namespace, _store) = fixture_with_store().await;
    (client, namespace)
}
async fn put(
    client: &mut Client,
    owner_entity: pb::Entity,
    etag: &str,
) -> Result<pb::PutEntityResponse, tonic::Status> {
    client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(owner_entity),
                if_match: etag.into(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .map(tonic::Response::into_inner)
}
fn unavailable_message(namespace: &str) -> pb::ContractMemberRef {
    pb::ContractMemberRef {
        specification: Some(pb::SpecificationRef {
            id: Some(id(namespace, "missing")),
        }),
        owner_uid: "01930000-0000-7000-8000-000000000001".into(),
        kind: pb::ContractMemberKind::Message as i32,
        member_id: Some(pb::ContractMemberId {
            value: "01930000-0000-7000-8000-000000000002".into(),
        }),
        expected_digest: None,
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn every_native_claim_owner_rejects_unavailable_exact_message() {
    let (mut client, namespace) = fixture().await;
    for kind in [
        pb::EntityKind::Component,
        pb::EntityKind::ExternalSystem,
        pb::EntityKind::Domain,
    ] {
        let result = put(
            &mut client,
            owner(&namespace, kind, unavailable_message(&namespace)),
            "",
        )
        .await;
        assert!(
            result.is_err(),
            "{kind:?} accepted unavailable claim target"
        );
        assert_eq!(result.unwrap_err().code(), Code::InvalidArgument);
    }
}

fn entity(kind: pb::entity::Kind) -> pb::Entity {
    pb::Entity {
        kind: Some(kind),
        ..Default::default()
    }
}
fn reference(owner_entity: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(owner_entity).unwrap()
}
async fn targets(
    client: &mut Client,
    namespace: &str,
) -> (pb::Entity, pb::ContractMemberRef, pb::Entity) {
    let schema = put(
        client,
        entity(pb::entity::Kind::Schema(pb::Schema {
            id: Some(id(namespace, "payload")),
            title: "Payload".into(),
            fields: vec![pb::FieldSpec {
                name: "name".into(),
                r#type: Some(pb::FieldType {
                    kind: Some(pb::field_type::Kind::String(pb::field_type::StringType {})),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        "",
    )
    .await
    .unwrap()
    .entity
    .unwrap();
    let spec = put(
        client,
        entity(pb::entity::Kind::Specification(pb::Specification {
            id: Some(id(namespace, "messages")),
            title: "Messages".into(),
            messages: ["Received", "Triggered"]
                .into_iter()
                .map(|n| pb::ContractMessage {
                    name: Some(pb::MemberName { value: n.into() }),
                    schemas: vec![pb::SchemaBinding {
                        role: pb::schema_binding::Role::Payload as i32,
                        r#type: Some(pb::schema_binding::Type::Schema(pb::SchemaRef {
                            id: reference(&schema).id,
                        })),
                        ..Default::default()
                    }],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })),
        "",
    )
    .await
    .unwrap()
    .entity
    .unwrap();
    let Some(pb::entity::Kind::Specification(definition)) = &spec.kind else {
        panic!()
    };
    let message = pb::ContractMemberRef {
        specification: Some(pb::SpecificationRef {
            id: definition.id.clone(),
        }),
        owner_uid: spec.system.as_ref().unwrap().uid.clone(),
        kind: pb::ContractMemberKind::Message as i32,
        member_id: definition.messages[0].member_id.clone(),
        expected_digest: None,
    };
    let model = put(
        client,
        entity(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id(namespace, "observed")),
            title: "Observed".into(),
            ..Default::default()
        })),
        "",
    )
    .await
    .unwrap()
    .entity
    .unwrap();
    put(
        client,
        entity(pb::entity::Kind::Processor(pb::Processor {
            id: Some(id(namespace, "processor")),
            title: "Processor".into(),
            ..Default::default()
        })),
        "",
    )
    .await
    .unwrap();
    let slice = put(
        client,
        entity(pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id(namespace, "slice")),
            title: "Slice".into(),
            source_read_models: vec![pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: reference(&model).id,
                }),
                ..Default::default()
            }],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(id(namespace, "processor")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        })),
        "",
    )
    .await
    .unwrap()
    .entity
    .unwrap();
    let Some(pb::entity::Kind::AutomationSlice(definition)) = &slice.kind else {
        panic!()
    };
    let channel = put(
        client,
        entity(pb::entity::Kind::Channel(pb::Channel {
            id: Some(id(namespace, "channel")),
            title: "Channel".into(),
            address: Some(pb::ChannelAddress {
                template: "{tenant}".into(),
                parameters: vec![pb::AddressParameter {
                    name: Some(pb::ParameterName {
                        value: "tenant".into(),
                    }),
                    ..Default::default()
                }],
            }),
            bindings: vec![pb::ChannelBinding {
                parameter_values: [("tenant".into(), String::new())].into_iter().collect(),
                observation: Some(pb::ConnectionRef {
                    owner: Some(reference(&slice)),
                    owner_uid: slice.system.as_ref().unwrap().uid.clone(),
                    id: definition.source_read_models[0].connection_id.clone(),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        "",
    )
    .await
    .unwrap()
    .entity
    .unwrap();
    (spec, message, channel)
}
#[tokio::test(flavor = "multi_thread")]
async fn authenticated_native_claim_authoring_keeps_flat_identity_and_exact_field_validation() {
    use trogon_atlas_core::message_claims as claims;
    let (mut client, namespace) = fixture().await;
    let (mut spec, message, channel) = targets(&mut client, &namespace).await;
    let current = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Specification as i32,
                id: reference(&spec).id,
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    if let Some(pb::entity::Kind::Specification(value)) = &mut spec.kind {
        let mut header = value.messages[0].schemas[0].clone();
        header.role = pb::schema_binding::Role::Headers as i32;
        header.connection_id = None;
        value.messages[0].schemas.push(header);
    }
    spec = put(&mut client, spec, &current.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    let Some(pb::entity::Kind::Specification(definition)) = &spec.kind else {
        panic!()
    };
    for kind in [
        pb::EntityKind::Component,
        pb::EntityKind::ExternalSystem,
        pb::EntityKind::Domain,
    ] {
        let mut proposed = owner(&namespace, kind, message.clone());
        let declarations = claims::declarations_mut(&mut proposed).unwrap();
        declarations.claims.push(declarations.claims[0].clone());
        let first = put(&mut client, proposed, "").await.unwrap();
        let first_entity = first.entity.unwrap();
        let mut next = first_entity.clone();
        let declarations = claims::declarations_mut(&mut next).unwrap();
        assert_ne!(
            declarations.claims[0].connection_id,
            declarations.claims[1].connection_id
        );
        declarations.routes.push(pb::ClaimRoute {
            claim_id: declarations.claims[0].connection_id.clone(),
            channel: Some(pb::ChannelRef {
                id: reference(&channel).id,
            }),
            ..Default::default()
        });
        assert_eq!(
            put(&mut client, next.clone(), &first.etag)
                .await
                .unwrap_err()
                .code(),
            Code::InvalidArgument
        );
        let declarations = claims::declarations_mut(&mut next).unwrap();
        declarations.routes[0]
            .parameter_values
            .insert("tenant".into(), String::new());
        let mut triggered = message.clone();
        triggered.member_id = definition.messages[1].member_id.clone();
        declarations.triggers.push(pb::ClaimTrigger {
            claim_id: declarations.claims[0].connection_id.clone(),
            message: Some(triggered),
            ..Default::default()
        });
        declarations.field_selections.push(pb::ClaimFieldSelection {
            claim_id: declarations.claims[0].connection_id.clone(),
            binding: Some(pb::ConnectionRef {
                owner: Some(reference(&spec)),
                owner_uid: spec.system.as_ref().unwrap().uid.clone(),
                id: definition.messages[0].schemas[0].connection_id.clone(),
            }),
            selectors: vec![pb::ClaimFieldSelector {
                selector: Some(pb::claim_field_selector::Selector::Path(pb::FieldPath {
                    segments: vec!["name".into()],
                })),
            }],
            ..Default::default()
        });
        let valid = put(&mut client, next, &first.etag).await.unwrap();
        let owner_entity = valid.entity.unwrap();
        let declarations = claims::declarations(&owner_entity).unwrap();
        assert_eq!(declarations.routes[0].delivery_mode, None);
        let refs = client
            .get_outgoing_references(authed(
                pb::GetReferencesRequest {
                    kind: kind as i32,
                    id: reference(&owner_entity).id,
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(refs.references.len(), 5);
        assert!(refs.references.iter().all(|r| r.connection_id.is_some()
            && r.from_uid == owner_entity.system.as_ref().unwrap().uid));
        assert_eq!(
            refs.references
                .iter()
                .find(|r| r.from_field == "message_claims.routes[0].channel")
                .unwrap()
                .connection_id,
            declarations.routes[0].connection_id
        );
        let mut wrong = owner_entity.clone();
        claims::declarations_mut(&mut wrong)
            .unwrap()
            .field_selections[0]
            .selectors[0]
            .selector = Some(pb::claim_field_selector::Selector::Path(pb::FieldPath {
            segments: vec!["missing".into()],
        }));
        assert_eq!(
            put(&mut client, wrong, &valid.etag)
                .await
                .unwrap_err()
                .code(),
            Code::InvalidArgument
        );
        let mut wrong = owner_entity.clone();
        let declarations = claims::declarations_mut(&mut wrong).unwrap();
        declarations.field_selections[0]
            .binding
            .as_mut()
            .unwrap()
            .id = definition.messages[1].schemas[0].connection_id.clone();
        declarations.field_selections[0].connection_id = None;
        assert_eq!(
            put(&mut client, wrong, &valid.etag)
                .await
                .unwrap_err()
                .code(),
            Code::InvalidArgument
        );
        let mut wrong_role = owner_entity.clone();
        let declarations = claims::declarations_mut(&mut wrong_role).unwrap();
        declarations.field_selections[0]
            .binding
            .as_mut()
            .unwrap()
            .id = definition.messages[0].schemas[1].connection_id.clone();
        declarations.field_selections[0].connection_id = None;
        assert_eq!(
            put(&mut client, wrong_role, &valid.etag)
                .await
                .unwrap_err()
                .code(),
            Code::InvalidArgument
        );
        let mut sending = owner(&namespace, kind, message.clone());
        match sending.kind.as_mut().unwrap() {
            pb::entity::Kind::Component(value) => value.id.as_mut().unwrap().slug = "sender".into(),
            pb::entity::Kind::ExternalSystem(value) => {
                value.id.as_mut().unwrap().slug = "sender".into();
            }
            pb::entity::Kind::Domain(value) => value.id.as_mut().unwrap().slug = "sender".into(),
            _ => unreachable!(),
        }
        let declarations = claims::declarations_mut(&mut sending).unwrap();
        declarations.claims[0].direction = pb::CommunicationDirection::Send as i32;
        declarations.claims[0].connection_id = Some(pb::ConnectionId {
            value: "01930000-0000-7000-8000-000000000777".into(),
        });
        let mut triggered = message.clone();
        triggered.member_id = definition.messages[1].member_id.clone();
        declarations.triggers.push(pb::ClaimTrigger {
            claim_id: declarations.claims[0].connection_id.clone(),
            message: Some(triggered),
            ..Default::default()
        });
        assert_eq!(
            put(&mut client, sending, "").await.unwrap_err().code(),
            Code::InvalidArgument
        );
        let mut opaque = owner_entity.clone();
        claims::declarations_mut(&mut opaque)
            .unwrap()
            .field_selections[0]
            .selectors[0]
            .selector = Some(pb::claim_field_selector::Selector::Opaque(
            pb::OpaqueTypeSelector {
                syntax: "json-pointer".into(),
                value: "/not-verified".into(),
            },
        ));
        put(&mut client, opaque, &valid.etag).await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn channel_default_supplies_an_omitted_claim_route_parameter() {
    use trogon_atlas_core::message_claims as claims;
    let (mut client, namespace) = fixture().await;
    let (_, message, mut channel) = targets(&mut client, &namespace).await;
    let current = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Channel as i32,
                id: reference(&channel).id,
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    if let Some(pb::entity::Kind::Channel(value)) = &mut channel.kind {
        value.address.as_mut().unwrap().parameters[0].default = Some("default-tenant".into());
    }
    put(&mut client, channel.clone(), &current.etag)
        .await
        .unwrap();
    let first = put(
        &mut client,
        owner(&namespace, pb::EntityKind::Domain, message),
        "",
    )
    .await
    .unwrap();
    let mut proposed = first.entity.unwrap();
    let declarations = claims::declarations_mut(&mut proposed).unwrap();
    declarations.routes.push(pb::ClaimRoute {
        claim_id: declarations.claims[0].connection_id.clone(),
        channel: Some(pb::ChannelRef {
            id: reference(&channel).id,
        }),
        ..Default::default()
    });
    let result = put(&mut client, proposed, &first.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    assert!(claims::declarations(&result).unwrap().routes[0]
        .parameter_values
        .is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn selected_native_fields_become_unavailable_without_rebinding_or_vetoing_title_edits() {
    use trogon_atlas_core::message_claims as claims;
    let (mut client, namespace) = fixture().await;
    let (spec, message, _) = targets(&mut client, &namespace).await;
    let Some(pb::entity::Kind::Specification(definition)) = &spec.kind else {
        panic!()
    };
    let first = put(
        &mut client,
        owner(&namespace, pb::EntityKind::Domain, message),
        "",
    )
    .await
    .unwrap();
    let mut owner_entity = first.entity.unwrap();
    let declarations = claims::declarations_mut(&mut owner_entity).unwrap();
    declarations.field_selections.push(pb::ClaimFieldSelection {
        claim_id: declarations.claims[0].connection_id.clone(),
        binding: Some(pb::ConnectionRef {
            owner: Some(reference(&spec)),
            owner_uid: spec.system.as_ref().unwrap().uid.clone(),
            id: definition.messages[0].schemas[0].connection_id.clone(),
        }),
        selectors: vec![pb::ClaimFieldSelector {
            selector: Some(pb::claim_field_selector::Selector::Path(pb::FieldPath {
                segments: vec!["name".into()],
            })),
        }],
        ..Default::default()
    });
    let stored = put(&mut client, owner_entity, &first.etag).await.unwrap();
    let owner_entity = stored.entity.unwrap();
    let selected = pb::ConnectionRef {
        owner: Some(reference(&owner_entity)),
        owner_uid: owner_entity.system.as_ref().unwrap().uid.clone(),
        id: claims::declarations(&owner_entity)
            .unwrap()
            .field_selections[0]
            .connection_id
            .clone(),
    };
    let request = || {
        authed(
            pb::ListSubjectAssignmentsRequest {
                subject: Some(pb::ResponsibilitySubject {
                    selection: Some(pb::responsibility_subject::Selection::Connection(
                        selected.clone(),
                    )),
                }),
                ..Default::default()
            },
            ROOT_TOKEN,
        )
    };
    assert_eq!(
        client
            .list_subject_assignments(request())
            .await
            .unwrap()
            .into_inner()
            .subject_state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );
    let schema = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(&namespace, "payload")),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    let mut changed = schema.entity.unwrap();
    if let Some(pb::entity::Kind::Schema(definition)) = &mut changed.kind {
        definition.fields.clear();
    }
    put(&mut client, changed, &schema.etag).await.unwrap();
    assert_eq!(
        client
            .list_subject_assignments(request())
            .await
            .unwrap()
            .into_inner()
            .subject_state,
        pb::ResponsibilityResolutionState::Unresolved as i32
    );
    let mut title = owner_entity.clone();
    if let Some(pb::entity::Kind::Domain(domain)) = &mut title.kind {
        domain.title = "Unrelated title".into();
    }
    let title = put(&mut client, title, &stored.etag).await.unwrap();
    assert_eq!(
        claims::declarations(title.entity.as_ref().unwrap()),
        claims::declarations(&owner_entity)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_targets_and_hidden_referrers_do_not_leak_or_bypass_safe_delete() {
    use support::two_tenants::{ACME_TOKEN, BETA_TOKEN};
    let (mut client, _) = fixture().await;
    let acme = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "claims-a".into(),
                parent: String::new(),
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let beta = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "claims-b".into(),
                parent: String::new(),
            },
            BETA_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let (spec, message, _) = targets(&mut client, &beta).await;
    let request = |message| {
        authed(
            pb::PutEntityRequest {
                entity: Some(owner(&acme, pb::EntityKind::Component, message)),
                ..Default::default()
            },
            ACME_TOKEN,
        )
    };
    let hidden = client
        .put_entity(request(message.clone()))
        .await
        .unwrap_err();
    let mut missing = message.clone();
    missing.specification.as_mut().unwrap().id = Some(id(&beta, "nonexistent"));
    let absent = client.put_entity(request(missing)).await.unwrap_err();
    assert_eq!(hidden.code(), absent.code());
    assert_eq!(hidden.message(), absent.message());
    assert!(!hidden.message().contains(&beta));
    let authored = put(
        &mut client,
        owner(&acme, pb::EntityKind::Domain, message),
        "",
    )
    .await
    .unwrap();
    let deletion = pb::DeleteEntityRequest {
        kind: pb::EntityKind::Specification as i32,
        id: reference(&spec).id,
        mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
        ..Default::default()
    };
    assert_eq!(
        client
            .delete_entity(authed(deletion.clone(), BETA_TOKEN))
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..deletion
            },
            BETA_TOKEN,
        ))
        .await
        .unwrap();
    let owner_entity = authored.entity.unwrap();
    let after = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Domain as i32,
                id: reference(&owner_entity).id,
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(after.entity, Some(owner_entity.clone()));
    assert_eq!(after.etag, authored.etag);
    let selected = pb::ConnectionRef {
        owner: Some(reference(&owner_entity)),
        owner_uid: owner_entity.system.as_ref().unwrap().uid.clone(),
        id: trogon_atlas_core::message_claims::declarations(&owner_entity)
            .unwrap()
            .claims[0]
            .connection_id
            .clone(),
    };
    let state = client
        .list_subject_assignments(authed(
            pb::ListSubjectAssignmentsRequest {
                subject: Some(pb::ResponsibilitySubject {
                    selection: Some(pb::responsibility_subject::Selection::Connection(selected)),
                }),
                ..Default::default()
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        state.subject_state,
        pb::ResponsibilityResolutionState::Unresolved as i32
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn final_batch_tombstones_and_validate_only_refuse_partial_claim_publication() {
    let (mut client, namespace) = fixture().await;
    let (spec, message, _) = targets(&mut client, &namespace).await;
    let proposed = owner(&namespace, pb::EntityKind::Domain, message.clone());
    let result = client
        .batch_mutate(authed(
            pb::BatchMutateRequest {
                ops: vec![
                    pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(proposed),
                            ..Default::default()
                        })),
                    },
                    pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                            kind: pb::EntityKind::Specification as i32,
                            id: reference(&spec).id,
                            mode: pb::delete_entity_request::Mode::Force as i32,
                            ..Default::default()
                        })),
                    },
                ],
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(result.code(), Code::InvalidArgument);
    assert!(client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Specification as i32,
                id: reference(&spec).id
            },
            ROOT_TOKEN
        ))
        .await
        .is_ok());
    assert_eq!(
        client
            .get_entity(authed(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Domain as i32,
                    id: Some(id(&namespace, "owner"))
                },
                ROOT_TOKEN
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let preview = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(owner(&namespace, pb::EntityKind::Domain, message)),
                validate_only: true,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(preview.etag, "");
    assert_eq!(
        client
            .get_entity(authed(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Domain as i32,
                    id: Some(id(&namespace, "owner"))
                },
                ROOT_TOKEN
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn foreign_branch_is_refused_before_claim_target_hydration() {
    use support::two_tenants::{authed_branch, ACME_TOKEN};
    let (mut client, _) = fixture().await;
    let namespace = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "claims-private-test".into(),
                parent: String::new(),
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    client
        .create_branch(authed(
            pb::CreateBranchRequest {
                name: "claims-private".into(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let error = client
        .put_entity(authed_branch(
            pb::PutEntityRequest {
                entity: Some(owner(
                    &namespace,
                    pb::EntityKind::Domain,
                    unavailable_message(&namespace),
                )),
                ..Default::default()
            },
            ACME_TOKEN,
            "claims-private",
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn selected_claim_inventories_share_exact_work_budget_without_charging_unselected_claims() {
    use trogon_atlas_core::message_claims as claims;
    use trogon_atlas_store::Store;
    let (mut client, namespace, store) = fixture_with_store().await;
    let (spec, message, _) = targets(&mut client, &namespace).await;
    let mut owners = Vec::new();
    for index in 0..11 {
        let mut owner_entity = owner(&namespace, pb::EntityKind::Domain, message.clone());
        if let Some(pb::entity::Kind::Domain(domain)) = &mut owner_entity.kind {
            domain.id.as_mut().unwrap().slug = format!("budget-{index}");
        }
        let declarations = claims::declarations_mut(&mut owner_entity).unwrap();
        let count = if index == 9 { 772 } else { 1024 };
        declarations.claims = vec![declarations.claims[0].clone(); count];
        owners.push(put(&mut client, owner_entity, "").await.unwrap());
    }
    let selected = owners
        .iter()
        .take(10)
        .map(|r| {
            let owner_entity = r.entity.as_ref().unwrap();
            pb::SubjectAssociation {
                subject: Some(pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Connection(
                        pb::ConnectionRef {
                            owner: Some(reference(owner_entity)),
                            owner_uid: owner_entity.system.as_ref().unwrap().uid.clone(),
                            id: claims::declarations(owner_entity).unwrap().claims[0]
                                .connection_id
                                .clone(),
                        },
                    )),
                }),
                ..Default::default()
            }
        })
        .collect();
    let content = "Budget fixture";
    let doc = entity(pb::entity::Kind::Document(pb::Document {
        id: Some(id(&namespace, "budget-document")),
        title: Some(pb::CatalogTitle {
            value: "Budget".into(),
        }),
        subjects: selected,
        body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
            media_type: Some(pb::MediaType {
                value: "text/markdown".into(),
            }),
            content: content.into(),
            digest: Some(trogon_atlas_core::specification::content_digest(content)),
        })),
        ..Default::default()
    }));
    store
        .put(
            pb::EntityKind::Document,
            &doc,
            false,
            trogon_atlas_store::WriteContext::baseline(),
        )
        .await
        .unwrap();
    let request = || {
        authed(
            pb::GetDocumentDetailRequest {
                document: Some(pb::DocumentSelection {
                    selection: Some(pb::document_selection::Selection::Document(
                        pb::DocumentRef {
                            id: Some(id(&namespace, "budget-document")),
                        },
                    )),
                }),
            },
            ROOT_TOKEN,
        )
    };
    let response = client
        .get_document_detail(request())
        .await
        .unwrap()
        .into_inner();
    assert!(!response.incomplete);
    let unrelated = client
        .list_subject_documents(authed(
            pb::ListSubjectDocumentsRequest {
                subject: Some(pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Entity(reference(&spec))),
                }),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!unrelated.incomplete);
    assert_eq!(unrelated.declarations, []);
    let old = owners.remove(9);
    let mut added = old.entity.unwrap();
    let declarations = claims::declarations_mut(&mut added).unwrap();
    let mut next = declarations.claims[0].clone();
    next.connection_id = None;
    declarations.claims.push(next);
    put(&mut client, added, &old.etag).await.unwrap();
    assert_eq!(
        client
            .get_document_detail(request())
            .await
            .unwrap_err()
            .code(),
        Code::ResourceExhausted
    );
}

fn note(namespace: &str, slug: &str, selected: pb::ConnectionRef, count: usize) -> pb::Entity {
    let content = "Native claim rationale";
    entity(pb::entity::Kind::Document(pb::Document {
        id: Some(id(namespace, slug)),
        title: Some(pb::CatalogTitle {
            value: "Rationale".into(),
        }),
        subjects: vec![
            pb::SubjectAssociation {
                subject: Some(pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Connection(selected))
                }),
                ..Default::default()
            };
            count
        ],
        body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
            media_type: Some(pb::MediaType {
                value: "text/markdown".into(),
            }),
            content: content.into(),
            digest: Some(trogon_atlas_core::specification::content_digest(content)),
        })),
        ..Default::default()
    }))
}
#[tokio::test(flavor = "multi_thread")]
async fn exact_claim_subject_output_boundary_keeps_equal_siblings_separate() {
    use trogon_atlas_core::message_claims as claims;
    use trogon_atlas_store::{Store, WriteContext};
    let (mut client, namespace, store) = fixture_with_store().await;
    let (_, message, _) = targets(&mut client, &namespace).await;
    let mut raw = owner(&namespace, pb::EntityKind::Domain, message);
    let declarations = claims::declarations_mut(&mut raw).unwrap();
    declarations.claims.push(declarations.claims[0].clone());
    let owner_entity = put(&mut client, raw, "").await.unwrap().entity.unwrap();
    let declarations = claims::declarations(&owner_entity).unwrap();
    let selected = pb::ConnectionRef {
        owner: Some(reference(&owner_entity)),
        owner_uid: owner_entity.system.as_ref().unwrap().uid.clone(),
        id: declarations.claims[0].connection_id.clone(),
    };
    for index in 0..4 {
        store
            .put(
                pb::EntityKind::Document,
                &note(&namespace, &format!("note-{index}"), selected.clone(), 1024),
                false,
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }
    let request = |selected| {
        authed(
            pb::ListSubjectDocumentsRequest {
                subject: Some(pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Connection(selected)),
                }),
            },
            ROOT_TOKEN,
        )
    };
    assert_eq!(
        client
            .list_subject_documents(request(selected.clone()))
            .await
            .unwrap_err()
            .code(),
        Code::ResourceExhausted
    );
    let sibling = pb::ConnectionRef {
        id: declarations.claims[1].connection_id.clone(),
        ..selected.clone()
    };
    let sibling_page = client
        .list_subject_documents(request(sibling.clone()))
        .await
        .unwrap()
        .into_inner();
    assert!(sibling_page.declarations.is_empty() && !sibling_page.incomplete);
    let scope = sibling_page.scope.unwrap();
    let receipt_records =
        scope.visited.len() + scope.visited_subjects.len() + scope.visited_documents.len();
    assert!(receipt_records > 0 && receipt_records < 1024);
    let current = store
        .get(pb::EntityKind::Document, &id(&namespace, "note-0"), None)
        .await
        .unwrap();
    let mut trimmed = current.entity;
    let Some(pb::entity::Kind::Document(trimmed_note)) = &mut trimmed.kind else {
        panic!()
    };
    trimmed_note.subjects.truncate(1024 - receipt_records);
    store
        .update(
            pb::EntityKind::Document,
            &trimmed,
            &current.etag,
            true,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    let boundary = client
        .list_subject_documents(request(selected.clone()))
        .await
        .unwrap()
        .into_inner();
    let checked = boundary.scope.as_ref().unwrap();
    assert_eq!(
        boundary.declarations.len()
            + checked.visited.len()
            + checked.visited_subjects.len()
            + checked.visited_documents.len(),
        4096
    );
    assert_eq!(boundary.declarations.len(), 4096 - receipt_records);
    assert!(!boundary.incomplete);
    let sibling_page = client
        .list_subject_documents(request(sibling))
        .await
        .unwrap()
        .into_inner();
    assert!(sibling_page.declarations.is_empty() && !sibling_page.incomplete);
    store
        .put(
            pb::EntityKind::Document,
            &note(&namespace, "overflow", selected.clone(), 1),
            false,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    assert_eq!(
        client
            .list_subject_documents(request(selected))
            .await
            .unwrap_err()
            .code(),
        Code::ResourceExhausted
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn independent_domain_claim_rows_obey_exact_shared_discovery_row_boundary() {
    use trogon_atlas_store::{store::MutationOp, Store, WriteContext};
    let (mut client, namespace, store) = fixture_with_store().await;
    let (spec, message, _) = targets(&mut client, &namespace).await;
    for start in (0..4998).step_by(128) {
        let ops: Vec<_> = (start..(start + 128).min(4998))
            .map(|index| {
                let mut owner_entity = owner(&namespace, pb::EntityKind::Domain, message.clone());
                if let Some(pb::entity::Kind::Domain(domain)) = &mut owner_entity.kind {
                    domain.id.as_mut().unwrap().slug = format!("row-{index}");
                }
                MutationOp::Create {
                    kind: pb::EntityKind::Domain,
                    entity: owner_entity,
                }
            })
            .collect();
        store
            .batch_apply(&ops, WriteContext::baseline())
            .await
            .unwrap();
    }
    let request = || {
        authed(
            pb::ListSubjectDocumentsRequest {
                subject: Some(pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Entity(reference(&spec))),
                }),
            },
            ROOT_TOKEN,
        )
    };
    let boundary = client.list_subject_documents(request()).await.unwrap_err();
    assert_eq!(boundary.code(), Code::ResourceExhausted);
    assert!(boundary.message().contains("output budget"));
    let current = store
        .get(pb::EntityKind::Domain, &id(&namespace, "row-0"), None)
        .await
        .unwrap();
    let mut selected_owner = current.entity;
    let Some(pb::entity::Kind::Specification(definition)) = &spec.kind else {
        panic!()
    };
    let declarations =
        trogon_atlas_core::message_claims::declarations_mut(&mut selected_owner).unwrap();
    declarations.field_selections.push(pb::ClaimFieldSelection {
        claim_id: declarations.claims[0].connection_id.clone(),
        binding: Some(pb::ConnectionRef {
            owner: Some(reference(&spec)),
            owner_uid: spec.system.as_ref().unwrap().uid.clone(),
            id: definition.messages[0].schemas[0].connection_id.clone(),
        }),
        selectors: vec![pb::ClaimFieldSelector {
            selector: Some(pb::claim_field_selector::Selector::Path(pb::FieldPath {
                segments: vec!["name".into()],
            })),
        }],
        ..Default::default()
    });
    let selected_owner = put(&mut client, selected_owner, &current.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    let native_field = pb::ConnectionRef {
        owner: Some(reference(&selected_owner)),
        owner_uid: selected_owner.system.as_ref().unwrap().uid.clone(),
        id: trogon_atlas_core::message_claims::declarations(&selected_owner)
            .unwrap()
            .field_selections[0]
            .connection_id
            .clone(),
    };
    let native_request = || {
        authed(
            pb::ListSubjectAssignmentsRequest {
                subject: Some(pb::ResponsibilitySubject {
                    selection: Some(pb::responsibility_subject::Selection::Connection(
                        native_field.clone(),
                    )),
                }),
                ..Default::default()
            },
            ROOT_TOKEN,
        )
    };
    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Domain as i32,
                id: Some(id(&namespace, "row-4997")),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let schema_boundary = client
        .list_subject_assignments(native_request())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        schema_boundary.subject_state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );
    let mut restored = owner(&namespace, pb::EntityKind::Domain, message.clone());
    if let Some(pb::entity::Kind::Domain(value)) = &mut restored.kind {
        value.id.as_mut().unwrap().slug = "row-4997".into();
    }
    put(&mut client, restored, "").await.unwrap();
    let failure = client
        .list_subject_assignments(native_request())
        .await
        .unwrap_err();
    assert_eq!(failure.code(), Code::ResourceExhausted);
    let mut owner_entity = owner(&namespace, pb::EntityKind::Domain, message);
    if let Some(pb::entity::Kind::Domain(domain)) = &mut owner_entity.kind {
        domain.id.as_mut().unwrap().slug = "row-overflow".into();
    }
    store
        .put(
            pb::EntityKind::Domain,
            &owner_entity,
            false,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    assert_eq!(
        client
            .list_subject_documents(request())
            .await
            .unwrap_err()
            .code(),
        Code::ResourceExhausted
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn decode_and_store_outages_remain_operational_errors_and_never_authorize_deletion() {
    let nats = trogon_atlas_testsupport::TestNats::start().await;
    let config = trogon_atlas_store::NatsStoreConfig::new(
        &nats.url,
        trogon_atlas_store::StoreMetrics::default(),
    );
    let bucket = config.bucket.clone();
    let store = Arc::new(
        trogon_atlas_store::NatsStore::connect_with(config)
            .await
            .unwrap(),
    );
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    let mut client = serve_with_registry(service, make_registry()).await;
    let namespace = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "claims-faults".into(),
                parent: "commerce".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let (spec, message, _) = targets(&mut client, &namespace).await;
    let nats_client = async_nats::connect(&nats.url).await.unwrap();
    let js = async_nats::jetstream::new(nats_client);
    let kv = js.get_key_value(bucket).await.unwrap();
    let key = trogon_atlas_store::key::entity_key(
        pb::EntityKind::Specification,
        reference(&spec).id.as_ref().unwrap(),
    );
    kv.put(key, vec![255, 255, 255].into()).await.unwrap();
    let error = put(
        &mut client,
        owner(&namespace, pb::EntityKind::Domain, message.clone()),
        "",
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), Code::Internal);
    assert_eq!(error.message(), "storage error");
    let error = client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(&namespace, "payload")),
                mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Internal);
    assert!(client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(&namespace, "payload"))
            },
            ROOT_TOKEN
        ))
        .await
        .is_ok());
    nats.stop().await.unwrap();
    let error = put(
        &mut client,
        owner(&namespace, pb::EntityKind::Domain, message),
        "",
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error.code(),
        Code::Unavailable | Code::DeadlineExceeded | Code::Internal
    ));
    assert_ne!(error.code(), Code::InvalidArgument);
}

#[derive(Debug, Default)]
struct FaultAuthorizer {
    target: std::sync::Mutex<Option<String>>,
    calls: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl trogon_atlas_server::ownership::NamespaceAuthorizer for FaultAuthorizer {
    async fn admits(
        &self,
        _visibility: &trogon_atlas_server::ownership::Visibility,
        namespace: &trogon_atlas_core::NamespaceId,
    ) -> Result<bool, tonic::Status> {
        if self.target.lock().unwrap().as_deref() == Some(namespace.as_str()) {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(tonic::Status::unavailable(
                "authorization service unavailable",
            ))
        } else {
            Ok(true)
        }
    }
    async fn lens(
        &self,
        _visibility: &trogon_atlas_server::ownership::Visibility,
    ) -> Result<trogon_atlas_server::ownership::Lens, tonic::Status> {
        Ok(trogon_atlas_server::ownership::Lens::Everything)
    }
    async fn may_write(
        &self,
        _visibility: &trogon_atlas_server::ownership::Visibility,
        _namespace: &trogon_atlas_core::NamespaceId,
    ) -> Result<trogon_atlas_server::ownership::WriteVerdict, tonic::Status> {
        Ok(trogon_atlas_server::ownership::WriteVerdict::Allowed)
    }
    async fn on_registered(
        &self,
        _record: &trogon_atlas_store::NamespaceRecord,
    ) -> Result<(), tonic::Status> {
        Ok(())
    }
    async fn on_moved(
        &self,
        _record: &trogon_atlas_store::NamespaceRecord,
        _from: &trogon_atlas_core::OwnerId,
    ) -> Result<(), tonic::Status> {
        Ok(())
    }
    async fn on_released(
        &self,
        _record: &trogon_atlas_store::NamespaceRecord,
    ) -> Result<(), tonic::Status> {
        Ok(())
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn authorization_outage_is_not_hidden_absence_and_foreign_branch_stops_before_authorizer() {
    use support::two_tenants::{authed_branch, ACME_TOKEN};
    let fault = Arc::new(FaultAuthorizer::default());
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap()
    .with_authorizer(fault.clone());
    let mut client = serve_with_registry(service, make_registry()).await;
    let namespace = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "claims-fault-owner".into(),
                parent: String::new(),
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let target_ns = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "claims-fault-target".into(),
                parent: "other".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let (_, message, _) = targets(&mut client, &target_ns).await;
    client
        .create_branch(authed(
            pb::CreateBranchRequest {
                name: "claims-fault-branch".into(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    *fault.target.lock().unwrap() = Some(target_ns);
    let proposed = owner(&namespace, pb::EntityKind::Domain, message);
    let error = client
        .put_entity(authed_branch(
            pb::PutEntityRequest {
                entity: Some(proposed.clone()),
                ..Default::default()
            },
            ACME_TOKEN,
            "claims-fault-branch",
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::NotFound);
    assert_eq!(fault.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    let error = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(proposed),
                ..Default::default()
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Unavailable);
    assert_eq!(fault.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        client
            .get_entity(authed(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Domain as i32,
                    id: Some(id(&namespace, "owner"))
                },
                ROOT_TOKEN
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn validation_reports_dangling_independent_domain_claims() {
    let (mut client, namespace) = fixture().await;
    let (spec, message, _) = targets(&mut client, &namespace).await;
    put(
        &mut client,
        owner(&namespace, pb::EntityKind::Domain, message),
        "",
    )
    .await
    .unwrap();
    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Specification as i32,
                id: reference(&spec).id,
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let report = client
        .validate_project(authed(
            pb::ValidateProjectRequest {
                scope: Some(pb::validate_project_request::Scope::Namespace(namespace)),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(report
        .reports
        .iter()
        .flat_map(|r| &r.issues)
        .any(|i| i.code == "MESSAGE_CLAIMS_REFERENCE_UNRESOLVED"));
}

#[tokio::test(flavor = "multi_thread")]
async fn explicit_branch_restoration_renews_retired_claims_and_revalidates_remote_targets() {
    use support::two_tenants::authed_branch;
    use trogon_atlas_core::message_claims as claims;
    for delete_target in [false, true] {
        let (mut client, namespace) = fixture().await;
        let (spec, message, channel) = targets(&mut client, &namespace).await;
        let initial = put(
            &mut client,
            owner(&namespace, pb::EntityKind::Domain, message),
            "",
        )
        .await
        .unwrap();
        let mut raw = initial.entity.unwrap();
        let declarations = claims::declarations_mut(&mut raw).unwrap();
        declarations.routes.push(pb::ClaimRoute {
            claim_id: declarations.claims[0].connection_id.clone(),
            channel: Some(pb::ChannelRef {
                id: reference(&channel).id,
            }),
            parameter_values: [("tenant".into(), String::new())].into_iter().collect(),
            ..Default::default()
        });
        let base = put(&mut client, raw, &initial.etag).await.unwrap();
        let base_entity = base.entity.unwrap();
        let old = claims::declarations(&base_entity).unwrap().claims[0]
            .connection_id
            .clone();
        client
            .create_branch(authed(
                pb::CreateBranchRequest {
                    name: "claims-restore".into(),
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap();
        let mut branch = base_entity.clone();
        if let Some(pb::entity::Kind::Domain(domain)) = &mut branch.kind {
            domain.title = "Branch choice".into();
        }
        client
            .put_entity(authed_branch(
                pb::PutEntityRequest {
                    entity: Some(branch),
                    ..Default::default()
                },
                ROOT_TOKEN,
                "claims-restore",
            ))
            .await
            .unwrap();
        let mut removed = base_entity.clone();
        *claims::declarations_mut(&mut removed).unwrap() = pb::MessageClaims::default();
        put(&mut client, removed, &base.etag).await.unwrap();
        if delete_target {
            client
                .delete_entity(authed(
                    pb::DeleteEntityRequest {
                        kind: pb::EntityKind::Specification as i32,
                        id: reference(&spec).id,
                        mode: pb::delete_entity_request::Mode::Force as i32,
                        ..Default::default()
                    },
                    ROOT_TOKEN,
                ))
                .await
                .unwrap();
        }
        let result = client
            .resolve_branch_entry(authed(
                pb::ResolveBranchEntryRequest {
                    name: "claims-restore".into(),
                    r#ref: Some(reference(&base_entity)),
                    resolution: pb::resolve_branch_entry_request::Resolution::KeepOurs as i32,
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await;
        if delete_target {
            assert_eq!(result.unwrap_err().code(), Code::InvalidArgument);
            continue;
        }
        result.unwrap();
        let restored = client
            .get_entity(authed_branch(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Domain as i32,
                    id: reference(&base_entity).id,
                },
                ROOT_TOKEN,
                "claims-restore",
            ))
            .await
            .unwrap()
            .into_inner()
            .entity
            .unwrap();
        let declarations = claims::declarations(&restored).unwrap();
        assert_ne!(declarations.claims[0].connection_id, old);
        assert_eq!(
            declarations.routes[0].claim_id,
            declarations.claims[0].connection_id
        );
        let merged = client
            .merge_branch(authed(
                pb::MergeBranchRequest {
                    name: "claims-restore".into(),
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            merged.status,
            pb::merge_branch_response::Status::Applied as i32
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn selected_claim_budget_includes_identified_resource_group_any_leaves() {
    use prost::Message;
    use trogon_atlas_core::message_claims as claims;
    use trogon_atlas_store::{Store, WriteContext};
    let (mut client, namespace, store) = fixture_with_store().await;
    let (spec, message, _) = targets(&mut client, &namespace).await;
    let groups = pb::ResourceGroupsAnnotation {
        groups: vec![pb::ResourceGroup {
            items: vec![
                pb::ResourceGroupItem {
                    target: Some(reference(&spec)),
                    ..Default::default()
                };
                256
            ],
            ..Default::default()
        }],
    };
    let mut subjects = Vec::new();
    for index in 0..40 {
        let mut raw = owner(&namespace, pb::EntityKind::Domain, message.clone());
        if let Some(pb::entity::Kind::Domain(domain)) = &mut raw.kind {
            domain.id.as_mut().unwrap().slug = format!("group-owner-{index}");
            domain.metadata.push(prost_types::Any {
                type_url:
                    "type.googleapis.com/trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation"
                        .into(),
                value: groups.encode_to_vec(),
            });
        }
        let authored = put(&mut client, raw, "").await.unwrap().entity.unwrap();
        subjects.push(pb::SubjectAssociation {
            subject: Some(pb::CatalogSubject {
                selection: Some(pb::catalog_subject::Selection::Connection(
                    pb::ConnectionRef {
                        owner: Some(reference(&authored)),
                        owner_uid: authored.system.as_ref().unwrap().uid.clone(),
                        id: claims::declarations(&authored).unwrap().claims[0]
                            .connection_id
                            .clone(),
                    },
                )),
            }),
            ..Default::default()
        });
    }
    let mut doc = note(&namespace, "group-budget", pb::ConnectionRef::default(), 1);
    if let Some(pb::entity::Kind::Document(document)) = &mut doc.kind {
        document.subjects = subjects;
    }
    store
        .put(
            pb::EntityKind::Document,
            &doc,
            false,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    let error = client
        .get_document_detail(authed(
            pb::GetDocumentDetailRequest {
                document: Some(pb::DocumentSelection {
                    selection: Some(pb::document_selection::Selection::Document(
                        pb::DocumentRef {
                            id: Some(id(&namespace, "group-budget")),
                        },
                    )),
                }),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::ResourceExhausted);
}

#[tokio::test(flavor = "multi_thread")]
async fn reader_cannot_publish_claims_or_obtain_authority_from_a_claim() {
    let store = Arc::new(trogon_atlas_testsupport::shared().await.store().await);
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    let registry=trogon_atlas_server::auth::TokenRegistry::from_file_contents(toml::from_str::<trogon_atlas_server::auth::RegistryFile>(&format!("[principals.root]\nrole=\"admin\"\ntokens=[\"{ROOT_TOKEN}\"]\n[principals.reader]\nrole=\"reader\"\ntokens=[\"claims-reader-token\"]\n")).unwrap()).unwrap();
    let mut client = serve_with_registry(service, registry).await;
    let namespace = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "claims-reader".into(),
                parent: "commerce".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let (_, message, _) = targets(&mut client, &namespace).await;
    let error = client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(owner(&namespace, pb::EntityKind::Component, message)),
                ..Default::default()
            },
            "claims-reader-token",
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
    assert_eq!(
        client
            .get_entity(authed(
                pb::GetEntityRequest {
                    kind: pb::EntityKind::Component as i32,
                    id: Some(id(&namespace, "owner"))
                },
                ROOT_TOKEN
            ))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn independent_domain_routes_support_exact_references_impact_and_guarded_retarget() {
    use trogon_atlas_core::message_claims as claims;
    let (mut client, namespace) = fixture().await;
    let (_, message, channel) = targets(&mut client, &namespace).await;
    let first = put(
        &mut client,
        owner(&namespace, pb::EntityKind::Domain, message),
        "",
    )
    .await
    .unwrap();
    let mut proposed = first.entity.unwrap();
    let declarations = claims::declarations_mut(&mut proposed).unwrap();
    declarations.routes.push(pb::ClaimRoute {
        claim_id: declarations.claims[0].connection_id.clone(),
        channel: Some(pb::ChannelRef {
            id: reference(&channel).id,
        }),
        parameter_values: [("tenant".into(), "selected".into())].into_iter().collect(),
        ..Default::default()
    });
    let before = put(&mut client, proposed, &first.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    let listing = client
        .list_entities_by_domain(authed(
            pb::ListEntitiesByDomainRequest {
                domain_id: reference(&before).id,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        listing.entities,
        vec![before.clone()],
        "authored claims must not become derived membership"
    );
    let incoming = client
        .get_incoming_references(authed(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::Channel as i32,
                id: reference(&channel).id,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    let route_ref = incoming
        .references
        .iter()
        .find(|r| r.from == Some(reference(&before)))
        .unwrap();
    assert_eq!(
        route_ref.connection_id,
        claims::declarations(&before).unwrap().routes[0].connection_id
    );
    assert_eq!(route_ref.from_field, "message_claims.routes[0].channel");
    let impact = client
        .get_impact(authed(
            pb::GetImpactRequest {
                root: Some(reference(&channel)),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(impact
        .nodes
        .iter()
        .any(|node| node.entity == Some(reference(&before))));
    let mut replacement = channel.clone();
    replacement.system = None;
    if let Some(pb::entity::Kind::Channel(value)) = &mut replacement.kind {
        value.id.as_mut().unwrap().slug = "replacement-channel".into();
        value.bindings[0].connection_id = None;
    }
    let replacement = put(&mut client, replacement, "")
        .await
        .unwrap()
        .entity
        .unwrap();
    let preview = client
        .retarget_references(authed(
            pb::RetargetReferencesRequest {
                from: Some(reference(&channel)),
                to: Some(reference(&replacement)),
                dry_run: true,
                filter_kinds: vec![pb::EntityKind::Domain as i32],
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(preview.rewritten, 1);
    let unchanged = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Domain as i32,
                id: reference(&before).id,
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(unchanged, before);
    let applied = client
        .retarget_references(authed(
            pb::RetargetReferencesRequest {
                from: Some(reference(&channel)),
                to: Some(reference(&replacement)),
                filter_kinds: vec![pb::EntityKind::Domain as i32],
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(applied.rewritten, 1);
    let after = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Domain as i32,
                id: reference(&before).id,
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let old = claims::declarations(&before).unwrap();
    let new = claims::declarations(&after).unwrap();
    assert_eq!(old.claims, new.claims);
    assert_eq!(old.routes[0].claim_id, new.routes[0].claim_id);
    assert_ne!(old.routes[0].connection_id, new.routes[0].connection_id);
    assert_eq!(
        new.routes[0].channel.as_ref().unwrap().id,
        reference(&replacement).id
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_native_owner_inventory_cannot_authorize_safe_delete() {
    use prost::Message;
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let mut config = trogon_atlas_store::NatsStoreConfig::new(
        &nats.url,
        trogon_atlas_store::StoreMetrics::default(),
    );
    config.bucket = format!("claims-corrupt-{suffix}");
    config.branches_bucket = format!("claims-corrupt-branches-{suffix}");
    config.changesets_bucket = format!("claims-corrupt-changesets-{suffix}");
    config.revisions_bucket = format!("claims-corrupt-revisions-{suffix}");
    config.batches_bucket = format!("claims-corrupt-batches-{suffix}");
    config.namespaces_bucket = format!("claims-corrupt-namespaces-{suffix}");
    config.lease_bucket = format!("claims-corrupt-lease-{suffix}");
    config.stream = format!("CLAIMS_CORRUPT_{}", suffix.to_uppercase());
    config.subject_root = format!("claims.corrupt.{suffix}");
    let bucket = config.bucket.clone();
    let store = Arc::new(
        trogon_atlas_store::NatsStore::connect_with(config)
            .await
            .unwrap(),
    );
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    let mut client = serve_with_registry(service, make_registry()).await;
    let namespace = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "claims-inventory-fault".into(),
                parent: "commerce".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let (_, message, channel) = targets(&mut client, &namespace).await;
    let mut authored = put(
        &mut client,
        owner(&namespace, pb::EntityKind::Domain, message),
        "",
    )
    .await
    .unwrap()
    .entity
    .unwrap();
    if let Some(pb::entity::Kind::Domain(value)) = &mut authored.kind {
        value.metadata.push(prost_types::Any {
            type_url:
                "type.googleapis.com/trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation"
                    .into(),
            value: vec![255, 255, 255],
        });
    }
    let nats_client = async_nats::connect(&nats.url).await.unwrap();
    let kv = async_nats::jetstream::new(nats_client)
        .get_key_value(bucket)
        .await
        .unwrap();
    kv.put(
        trogon_atlas_store::key::entity_key(
            pb::EntityKind::Domain,
            reference(&authored).id.as_ref().unwrap(),
        ),
        authored.encode_to_vec().into(),
    )
    .await
    .unwrap();
    let error = client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Channel as i32,
                id: reference(&channel).id,
                mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Internal);
    assert!(client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Channel as i32,
                id: reference(&channel).id
            },
            ROOT_TOKEN
        ))
        .await
        .is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn opaque_selector_still_requires_its_exact_native_schema_target() {
    use trogon_atlas_core::message_claims as claims;
    let (mut client, namespace) = fixture().await;
    let (spec, message, _) = targets(&mut client, &namespace).await;
    let Some(pb::entity::Kind::Specification(definition)) = &spec.kind else {
        panic!()
    };
    let first = put(
        &mut client,
        owner(&namespace, pb::EntityKind::Domain, message),
        "",
    )
    .await
    .unwrap();
    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(&namespace, "payload")),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let mut proposed = first.entity.unwrap();
    let declarations = claims::declarations_mut(&mut proposed).unwrap();
    declarations.field_selections.push(pb::ClaimFieldSelection {
        claim_id: declarations.claims[0].connection_id.clone(),
        binding: Some(pb::ConnectionRef {
            owner: Some(reference(&spec)),
            owner_uid: spec.system.as_ref().unwrap().uid.clone(),
            id: definition.messages[0].schemas[0].connection_id.clone(),
        }),
        selectors: vec![pb::ClaimFieldSelector {
            selector: Some(pb::claim_field_selector::Selector::Opaque(
                pb::OpaqueTypeSelector {
                    syntax: "json-pointer".into(),
                    value: "/unverified".into(),
                },
            )),
        }],
        ..Default::default()
    });
    let error = put(&mut client, proposed, &first.etag).await.unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert_eq!(
        error.message(),
        "selected message claim target is unavailable or invalid"
    );
}

async fn native_field_leaf(client: &mut Client, namespace: &str) -> pb::ConnectionRef {
    use trogon_atlas_core::message_claims as claims;
    let (spec, message, _) = targets(client, namespace).await;
    let Some(pb::entity::Kind::Specification(definition)) = &spec.kind else {
        panic!()
    };
    let first = put(
        client,
        owner(namespace, pb::EntityKind::Domain, message),
        "",
    )
    .await
    .unwrap();
    let mut authored = first.entity.unwrap();
    let declarations = claims::declarations_mut(&mut authored).unwrap();
    declarations.field_selections.push(pb::ClaimFieldSelection {
        claim_id: declarations.claims[0].connection_id.clone(),
        binding: Some(pb::ConnectionRef {
            owner: Some(reference(&spec)),
            owner_uid: spec.system.as_ref().unwrap().uid.clone(),
            id: definition.messages[0].schemas[0].connection_id.clone(),
        }),
        selectors: vec![pb::ClaimFieldSelector {
            selector: Some(pb::claim_field_selector::Selector::Path(pb::FieldPath {
                segments: vec!["name".into()],
            })),
        }],
        ..Default::default()
    });
    let authored = put(client, authored, &first.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    pb::ConnectionRef {
        owner: Some(reference(&authored)),
        owner_uid: authored.system.as_ref().unwrap().uid.clone(),
        id: claims::declarations(&authored).unwrap().field_selections[0]
            .connection_id
            .clone(),
    }
}

fn standalone_field_association(
    namespace: &str,
    slug: &str,
    kind: pb::EntityKind,
    leaf: pb::ConnectionRef,
) -> pb::Entity {
    let catalog_subject = pb::CatalogSubject {
        selection: Some(pb::catalog_subject::Selection::Connection(leaf.clone())),
    };
    let subjects = vec![pb::SubjectAssociation {
        subject: Some(catalog_subject),
        ..Default::default()
    }];
    let responsibilities = vec![pb::Responsibility {
        kind: pb::ResponsibilityKind::Owner as i32,
        subject: Some(pb::ResponsibilitySubject {
            selection: Some(pb::responsibility_subject::Selection::Connection(leaf)),
        }),
        ..Default::default()
    }];
    match kind {
        pb::EntityKind::Document => note(
            namespace,
            slug,
            match subjects[0]
                .subject
                .as_ref()
                .unwrap()
                .selection
                .as_ref()
                .unwrap()
            {
                pb::catalog_subject::Selection::Connection(value) => value.clone(),
                _ => unreachable!(),
            },
            1,
        ),
        pb::EntityKind::Decision => entity(pb::entity::Kind::Decision(pb::Decision {
            id: Some(id(namespace, slug)),
            title: Some(pb::CatalogTitle {
                value: "Decision".into(),
            }),
            status: pb::DecisionStatus::Proposed as i32,
            subjects,
            context: Some(pb::CatalogText {
                value: "Native field selection".into(),
            }),
            rationale: Some(pb::CatalogText {
                value: "Retain exact selected context".into(),
            }),
            options: vec![pb::DecisionOption {
                title: Some(pb::CatalogTitle {
                    value: "Option".into(),
                }),
                description: Some(pb::CatalogText {
                    value: "Use the selected field".into(),
                }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        pb::EntityKind::Team => entity(pb::entity::Kind::Team(pb::Team {
            id: Some(id(namespace, slug)),
            title: Some(pb::DirectoryDisplayName {
                value: "Team".into(),
            }),
            responsibilities,
            ..Default::default()
        })),
        pb::EntityKind::Person => entity(pb::entity::Kind::Person(pb::Person {
            id: Some(id(namespace, slug)),
            title: Some(pb::DirectoryDisplayName {
                value: "Person".into(),
            }),
            responsibilities,
            ..Default::default()
        })),
        _ => unreachable!(),
    }
}

fn catalog_leaf(association: &pb::SubjectAssociation) -> &pb::ConnectionRef {
    let Some(pb::catalog_subject::Selection::Connection(value)) =
        association.subject.as_ref().unwrap().selection.as_ref()
    else {
        panic!()
    };
    value
}
fn responsibility_leaf(assignment: &pb::Responsibility) -> &pb::ConnectionRef {
    let Some(pb::responsibility_subject::Selection::Connection(value)) =
        assignment.subject.as_ref().unwrap().selection.as_ref()
    else {
        panic!()
    };
    value
}

async fn standalone_native_field_admission(kind: pb::EntityKind) {
    let (mut client, namespace) = fixture().await;
    let leaf = native_field_leaf(&mut client, &namespace).await;
    let proposed = standalone_field_association(&namespace, "standalone", kind, leaf.clone());
    let saved = put(&mut client, proposed.clone(), "")
        .await
        .expect("valid standalone native field association must hydrate its Schema");
    let references = client
        .get_outgoing_references(authed(
            pb::GetReferencesRequest {
                kind: kind as i32,
                id: Some(id(&namespace, "standalone")),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(references
        .references
        .iter()
        .any(|reference| reference.to == leaf.owner));
    let saved = saved.entity.unwrap();
    let selected = match &saved.kind {
        Some(pb::entity::Kind::Document(value)) => catalog_leaf(&value.subjects[0]),
        Some(pb::entity::Kind::Decision(value)) => catalog_leaf(&value.subjects[0]),
        Some(pb::entity::Kind::Team(value)) => responsibility_leaf(&value.responsibilities[0]),
        Some(pb::entity::Kind::Person(value)) => responsibility_leaf(&value.responsibilities[0]),
        _ => unreachable!(),
    };
    assert_eq!(selected, &leaf);
    let mut stale = leaf.clone();
    stale.owner_uid = "01930000-0000-7000-8000-000000009999".into();
    assert_eq!(
        put(
            &mut client,
            standalone_field_association(&namespace, "stale", kind, stale),
            ""
        )
        .await
        .unwrap_err()
        .code(),
        Code::InvalidArgument
    );
    let schema = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(&namespace, "payload")),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    let batch = client
        .batch_mutate(authed(
            pb::BatchMutateRequest {
                ops: vec![
                    pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                            kind: pb::EntityKind::Schema as i32,
                            id: Some(id(&namespace, "payload")),
                            mode: pb::delete_entity_request::Mode::Force as i32,
                            ..Default::default()
                        })),
                    },
                    pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(standalone_field_association(
                                &namespace,
                                "tombstone",
                                kind,
                                leaf.clone(),
                            )),
                            ..Default::default()
                        })),
                    },
                ],
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(batch.code(), Code::InvalidArgument);
    assert!(client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(&namespace, "payload"))
            },
            ROOT_TOKEN
        ))
        .await
        .is_ok());
    client
        .create_branch(authed(
            pb::CreateBranchRequest {
                name: "standalone-fields".into(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let mut branch_schema = schema.entity.clone().unwrap();
    if let Some(pb::entity::Kind::Schema(value)) = &mut branch_schema.kind {
        value.fields.clear();
    }
    client
        .put_entity(support::two_tenants::authed_branch(
            pb::PutEntityRequest {
                entity: Some(branch_schema),
                if_match: schema.etag.clone(),
                ..Default::default()
            },
            ROOT_TOKEN,
            "standalone-fields",
        ))
        .await
        .unwrap();
    let error = client
        .put_entity(support::two_tenants::authed_branch(
            pb::PutEntityRequest {
                entity: Some(standalone_field_association(
                    &namespace,
                    "branch-invalid",
                    kind,
                    leaf.clone(),
                )),
                ..Default::default()
            },
            ROOT_TOKEN,
            "standalone-fields",
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    put(
        &mut client,
        standalone_field_association(&namespace, "baseline-valid", kind, leaf.clone()),
        "",
    )
    .await
    .unwrap();
    let mut changed = schema.entity.unwrap();
    if let Some(pb::entity::Kind::Schema(value)) = &mut changed.kind {
        value.fields.clear();
    }
    put(&mut client, changed, &schema.etag).await.unwrap();
    assert_eq!(
        put(
            &mut client,
            standalone_field_association(&namespace, "invalid-path", kind, leaf),
            ""
        )
        .await
        .unwrap_err()
        .code(),
        Code::InvalidArgument
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_document_native_field_admission() {
    Box::pin(standalone_native_field_admission(pb::EntityKind::Document)).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn standalone_decision_native_field_admission() {
    Box::pin(standalone_native_field_admission(pb::EntityKind::Decision)).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn standalone_team_native_field_admission() {
    Box::pin(standalone_native_field_admission(pb::EntityKind::Team)).await;
}
#[tokio::test(flavor = "multi_thread")]
async fn standalone_person_native_field_admission() {
    Box::pin(standalone_native_field_admission(pb::EntityKind::Person)).await;
}

async fn foreign_native_schema_leaf(
    client: &mut Client,
    namespace: &str,
) -> (pb::ConnectionRef, pb::Id) {
    let mut leaf = native_field_leaf(client, namespace).await;
    let foreign_namespace = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "standalone-native-schema".into(),
                parent: "other".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let schema = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "payload")),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let mut foreign = schema;
    foreign.system = None;
    let foreign_id = id(&foreign_namespace, "foreign-payload");
    if let Some(pb::entity::Kind::Schema(value)) = &mut foreign.kind {
        value.id = Some(foreign_id.clone());
    }
    put(client, foreign, "").await.unwrap();
    let current = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Specification as i32,
                id: Some(id(namespace, "messages")),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    let mut spec = current.entity.unwrap();
    if let Some(pb::entity::Kind::Specification(value)) = &mut spec.kind {
        value.messages[0].schemas[0].r#type =
            Some(pb::schema_binding::Type::Schema(pb::SchemaRef {
                id: Some(foreign_id.clone()),
            }));
        value.messages[0].schemas[0].connection_id = None;
    }
    let spec = put(client, spec, &current.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    let Some(pb::entity::Kind::Specification(definition)) = &spec.kind else {
        panic!()
    };
    let current = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Domain as i32,
                id: leaf.owner.as_ref().unwrap().id.clone(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    let mut owner = current.entity.unwrap();
    let declarations = trogon_atlas_core::message_claims::declarations_mut(&mut owner).unwrap();
    declarations.field_selections[0]
        .binding
        .as_mut()
        .unwrap()
        .id
        .clone_from(&definition.messages[0].schemas[0].connection_id);
    declarations.field_selections[0].connection_id = None;
    let owner = put(client, owner, &current.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    leaf.id.clone_from(
        &trogon_atlas_core::message_claims::declarations(&owner)
            .unwrap()
            .field_selections[0]
            .connection_id,
    );
    (leaf, foreign_id)
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_native_schema_hidden_and_missing_are_identical() {
    use support::two_tenants::ACME_TOKEN;
    let (mut client, _) = fixture().await;
    let namespace = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "standalone-acme".into(),
                parent: "acme".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let (leaf, foreign_id) = foreign_native_schema_leaf(&mut client, &namespace).await;
    let mut hidden = Vec::new();
    for kind in [
        pb::EntityKind::Document,
        pb::EntityKind::Decision,
        pb::EntityKind::Team,
        pb::EntityKind::Person,
    ] {
        let error = client
            .put_entity(authed(
                pb::PutEntityRequest {
                    entity: Some(standalone_field_association(
                        &namespace,
                        "hidden",
                        kind,
                        leaf.clone(),
                    )),
                    ..Default::default()
                },
                ACME_TOKEN,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument);
        hidden.push((error.code(), error.message().to_owned()));
    }
    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(foreign_id),
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    for (index, kind) in [
        pb::EntityKind::Document,
        pb::EntityKind::Decision,
        pb::EntityKind::Team,
        pb::EntityKind::Person,
    ]
    .into_iter()
    .enumerate()
    {
        let error = client
            .put_entity(authed(
                pb::PutEntityRequest {
                    entity: Some(standalone_field_association(
                        &namespace,
                        "missing",
                        kind,
                        leaf.clone(),
                    )),
                    ..Default::default()
                },
                ACME_TOKEN,
            ))
            .await
            .unwrap_err();
        assert_eq!((error.code(), error.message().to_owned()), hidden[index]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_native_schema_authorization_and_decode_faults_remain_errors() {
    use prost::Message;
    use support::two_tenants::ACME_TOKEN;
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let mut config = trogon_atlas_store::NatsStoreConfig::new(
        &nats.url,
        trogon_atlas_store::StoreMetrics::default(),
    );
    config.bucket = format!("standalone-fault-{suffix}");
    config.branches_bucket = format!("standalone-branches-{suffix}");
    config.changesets_bucket = format!("standalone-changesets-{suffix}");
    config.revisions_bucket = format!("standalone-revisions-{suffix}");
    config.batches_bucket = format!("standalone-batches-{suffix}");
    config.namespaces_bucket = format!("standalone-namespaces-{suffix}");
    config.lease_bucket = format!("standalone-lease-{suffix}");
    config.stream = format!("STANDALONE_{}", suffix.to_uppercase());
    config.subject_root = format!("standalone.{suffix}");
    let bucket = config.bucket.clone();
    let fault = Arc::new(FaultAuthorizer::default());
    let service = EventModelServiceImpl::try_new(
        Arc::new(
            trogon_atlas_store::NatsStore::connect_with(config)
                .await
                .unwrap(),
        ),
        ServerMetrics::default(),
    )
    .unwrap()
    .with_authorizer(fault.clone());
    let mut client = serve_with_registry(service, make_registry()).await;
    let namespace = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "standalone-fault-owner".into(),
                parent: "commerce".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let (leaf, foreign_id) = foreign_native_schema_leaf(&mut client, &namespace).await;
    *fault.target.lock().unwrap() = Some(foreign_id.namespace.clone());
    for kind in [
        pb::EntityKind::Document,
        pb::EntityKind::Decision,
        pb::EntityKind::Team,
        pb::EntityKind::Person,
    ] {
        let error = client
            .put_entity(authed(
                pb::PutEntityRequest {
                    entity: Some(standalone_field_association(
                        &namespace,
                        "auth-fault",
                        kind,
                        leaf.clone(),
                    )),
                    ..Default::default()
                },
                ACME_TOKEN,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::Unavailable);
    }
    assert_eq!(fault.calls.load(std::sync::atomic::Ordering::SeqCst), 4);
    *fault.target.lock().unwrap() = None;
    let kv = async_nats::jetstream::new(async_nats::connect(&nats.url).await.unwrap())
        .get_key_value(bucket)
        .await
        .unwrap();
    let key = trogon_atlas_store::key::entity_key(pb::EntityKind::Schema, &foreign_id);
    let valid_schema = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(foreign_id),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    kv.put(key.clone(), vec![255, 255, 255].into())
        .await
        .unwrap();
    for kind in [
        pb::EntityKind::Document,
        pb::EntityKind::Decision,
        pb::EntityKind::Team,
        pb::EntityKind::Person,
    ] {
        let error = put(
            &mut client,
            standalone_field_association(&namespace, "decode-fault", kind, leaf.clone()),
            "",
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), Code::Internal);
    }
    kv.put(key, valid_schema.encode_to_vec().into())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_native_associations_share_request_work_budget() {
    let (mut client, namespace) = fixture().await;
    let leaf = native_field_leaf(&mut client, &namespace).await;
    let ops = (0..10)
        .map(|index| pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                entity: Some(note(
                    &namespace,
                    &format!("standalone-budget-{index}"),
                    leaf.clone(),
                    1000,
                )),
                ..Default::default()
            })),
        })
        .collect();
    let error = client
        .batch_mutate(authed(
            pb::BatchMutateRequest {
                ops,
                validate_only: true,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::ResourceExhausted);
    assert!(error.message().contains("work budget"));
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_retired_native_field_never_substitutes_equal_sibling() {
    use trogon_atlas_core::message_claims as claims;
    let (mut client, namespace) = fixture().await;
    let leaf = native_field_leaf(&mut client, &namespace).await;
    let current = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Domain as i32,
                id: leaf.owner.as_ref().unwrap().id.clone(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    let mut owner = current.entity.unwrap();
    let declarations = claims::declarations_mut(&mut owner).unwrap();
    let mut parallel = declarations.claims[0].clone();
    parallel.connection_id = None;
    declarations.claims.push(parallel);
    let staged = put(&mut client, owner, &current.etag).await.unwrap();
    let mut owner = staged.entity.unwrap();
    let declarations = claims::declarations_mut(&mut owner).unwrap();
    let mut sibling = declarations.field_selections[0].clone();
    sibling.connection_id = None;
    sibling
        .claim_id
        .clone_from(&declarations.claims[1].connection_id);
    declarations.field_selections.push(sibling);
    let saved = put(&mut client, owner, &staged.etag).await.unwrap();
    let mut owner = saved.entity.unwrap();
    let declarations = claims::declarations_mut(&mut owner).unwrap();
    let mut sibling = leaf.clone();
    sibling.id = declarations.field_selections[1].connection_id.clone();
    assert_ne!(leaf.id, sibling.id);
    declarations.field_selections.remove(0);
    put(&mut client, owner, &saved.etag).await.unwrap();
    for kind in [
        pb::EntityKind::Document,
        pb::EntityKind::Decision,
        pb::EntityKind::Team,
        pb::EntityKind::Person,
    ] {
        assert_eq!(
            put(
                &mut client,
                standalone_field_association(&namespace, "retired", kind, leaf.clone()),
                ""
            )
            .await
            .unwrap_err()
            .code(),
            Code::InvalidArgument
        );
        put(
            &mut client,
            standalone_field_association(&namespace, "sibling", kind, sibling.clone()),
            "",
        )
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_document_diagnostics_hydrate_native_schema_outside_owner_scope() {
    let (mut client, namespace) = fixture().await;
    let leaf = native_field_leaf(&mut client, &namespace).await;
    let documents_namespace = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "standalone-diagnostics".into(),
                parent: "commerce".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    put(
        &mut client,
        note(&documents_namespace, "standalone", leaf, 1),
        "",
    )
    .await
    .unwrap();
    let report = client
        .validate_project(authed(
            pb::ValidateProjectRequest {
                scope: Some(pb::validate_project_request::Scope::Namespace(
                    documents_namespace,
                )),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    let issues: Vec<_> = report
        .reports
        .iter()
        .flat_map(|report| &report.issues)
        .collect();
    assert!(
        !issues
            .iter()
            .any(|issue| issue.code == "CATALOG_REFERENCE_UNRESOLVED"),
        "{issues:?}"
    );
}

#[tokio::test]
async fn channel_inverse_keeps_claim_routes_operations_and_independent_declarations_distinct() {
    let (mut client, namespace) = fixture().await;
    let (mut spec, message, channel_entity) = targets(&mut client, &namespace).await;
    let channel = pb::ChannelRef {
        id: reference(&channel_entity).id,
    };
    if let Some(pb::entity::Kind::Specification(value)) = &mut spec.kind {
        value.message_channels.push(pb::MessageChannelDeclaration {
            member_id: message.member_id.clone(),
            channel: Some(channel.clone()),
            parameter_values: [("tenant".into(), String::new())].into_iter().collect(),
            ..Default::default()
        });
        value.operations.push(pb::ContractOperation {
            name: Some(pb::MemberName {
                value: "Consume".into(),
            }),
            messages: vec![pb::OperationMessage {
                message_id: message.member_id.clone(),
                role: pb::operation_message::Role::Request as i32,
                ..Default::default()
            }],
            binding: Some(pb::OperationBinding {
                kind: Some(pb::operation_binding::Kind::Channel(
                    pb::ChannelOperationBinding {
                        channel: Some(channel.clone()),
                        ..Default::default()
                    },
                )),
            }),
            ..Default::default()
        });
    }
    put(&mut client, spec, "").await.unwrap();
    let mut proposal = owner(&namespace, pb::EntityKind::Component, message.clone());
    if let Some(pb::entity::Kind::Component(value)) = &mut proposal.kind {
        value
            .message_claims
            .as_mut()
            .unwrap()
            .claims
            .push(pb::MessageClaim {
                direction: pb::CommunicationDirection::Send as i32,
                message: Some(message.clone()),
                group: Some(pb::CatalogTitle {
                    value: "Orders".into(),
                }),
                ..Default::default()
            });
    }
    let mut stored = put(&mut client, proposal, "")
        .await
        .unwrap()
        .entity
        .unwrap();
    if let Some(pb::entity::Kind::Component(value)) = &mut stored.kind {
        let claims = value.message_claims.as_mut().unwrap();
        claims.routes = claims
            .claims
            .iter()
            .map(|claim| pb::ClaimRoute {
                claim_id: claim.connection_id.clone(),
                channel: Some(channel.clone()),
                parameter_values: [("tenant".into(), String::new())].into_iter().collect(),
                ..Default::default()
            })
            .collect();
    }
    put(&mut client, stored, "").await.unwrap();
    let result = client
        .list_channel_messages(authed(
            pb::ListChannelMessagesRequest {
                channel: Some(channel),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(result.records.len(), 4);
    assert!(result.complete && !result.incomplete && result.next_page_token.is_empty());
    let mut ids = std::collections::HashSet::new();
    for row in &result.records {
        assert_ne!(row.owner_uid, "");
        assert!(ids.insert(row.connection_id.as_ref().unwrap().value.clone()));
        assert_eq!(row.message.as_ref(), Some(&message));
        assert_eq!(row.state, pb::CatalogResolutionState::Resolved as i32);
        match pb::ChannelMessageAuthority::try_from(row.authority).unwrap() {
            pb::ChannelMessageAuthority::ClaimRoute => {
                assert_eq!(
                    row.route.as_ref().unwrap().claim_id,
                    row.claim.as_ref().unwrap().connection_id
                );
                assert_eq!(row.route.as_ref().unwrap().connection_id, row.connection_id);
            }
            pb::ChannelMessageAuthority::OperationBinding => {
                assert_eq!(
                    row.operation.as_ref().unwrap().kind,
                    pb::ContractMemberKind::Operation as i32
                );
                assert!(row
                    .operation_message
                    .as_ref()
                    .unwrap()
                    .connection_id
                    .is_some());
            }
            pb::ChannelMessageAuthority::SpecificationDeclaration => {
                assert!(row.claim.is_none() && row.operation.is_none());
            }
            pb::ChannelMessageAuthority::Unspecified => panic!("native authority required"),
        }
    }
}

#[tokio::test]
async fn channel_inverse_pages_all_matches_and_rejects_revision_and_branch_replays() {
    use trogon_atlas_store::{store::MutationOp, Store, WriteContext};
    let (mut client, namespace, store) = fixture_with_store().await;
    let (template, message, channel) = targets(&mut client, &namespace).await;
    let channel_ref = pb::ChannelRef {
        id: reference(&channel).id,
    };
    let mut ops = Vec::new();
    for index in 0..4097 {
        let mut entity = template.clone();
        entity.system = None;
        let Some(pb::entity::Kind::Specification(spec)) = &mut entity.kind else {
            panic!()
        };
        spec.id = Some(id(&namespace, &format!("inverse-{index:04}")));
        spec.messages.truncate(1);
        spec.message_channels.push(pb::MessageChannelDeclaration {
            member_id: message.member_id.clone(),
            channel: Some(channel_ref.clone()),
            parameter_values: [("tenant".into(), String::new())].into_iter().collect(),
            ..Default::default()
        });
        ops.push(MutationOp::Put {
            kind: pb::EntityKind::Specification,
            entity,
            if_match: None,
            force: false,
        });
    }
    for chunk in ops.chunks(500) {
        store
            .batch_apply(chunk, WriteContext::baseline())
            .await
            .unwrap();
    }
    let query = |token| pb::ListChannelMessagesRequest {
        channel: Some(channel_ref.clone()),
        page_size: 500,
        page_token: token,
    };
    let first = client
        .list_channel_messages(authed(query(String::new()), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.records.len(), 500);
    assert!(!first.incomplete && !first.complete);
    let mut token = first.next_page_token.clone();
    let mut seen: std::collections::HashSet<_> = first
        .records
        .iter()
        .map(|r| r.connection_id.clone().unwrap().value)
        .collect();
    loop {
        let page = client
            .list_channel_messages(authed(query(token), ROOT_TOKEN))
            .await
            .unwrap()
            .into_inner();
        assert!(!page.incomplete);
        assert_eq!(page.scope_digest, first.scope_digest);
        for row in &page.records {
            assert!(seen.insert(row.connection_id.clone().unwrap().value));
        }
        token = page.next_page_token;
        if token.is_empty() {
            assert!(page.complete);
            break;
        }
        assert!(!page.complete);
    }
    assert_eq!(seen.len(), 4097);
    client
        .create_branch(authed(
            pb::CreateBranchRequest {
                name: "inverse-page-work".into(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let mut branch_request = authed(query(first.next_page_token.clone()), ROOT_TOKEN);
    branch_request.metadata_mut().insert(
        "x-trogon-atlas-branch",
        "inverse-page-work".parse().unwrap(),
    );
    assert_eq!(
        client
            .list_channel_messages(branch_request)
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    put(
        &mut client,
        entity(pb::entity::Kind::Component(pb::Component {
            id: Some(id(&namespace, "revision-filler")),
            title: "Revision".into(),
            ..Default::default()
        })),
        "",
    )
    .await
    .unwrap();
    assert_eq!(
        client
            .list_channel_messages(authed(query(first.next_page_token), ROOT_TOKEN))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    let fresh = client
        .list_channel_messages(authed(query(String::new()), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(fresh.scope_digest, first.scope_digest);
    assert_eq!(fresh.records, first.records);
}

#[tokio::test]
async fn channel_inverse_preserves_authored_routes_when_the_exact_message_is_hidden_or_missing() {
    use support::two_tenants::{ACME_TOKEN, BETA_TOKEN};
    let (mut client, _) = fixture().await;
    let mut namespaces = Vec::new();
    for (name, token) in [
        ("inverse-visible", ACME_TOKEN),
        ("inverse-hidden", BETA_TOKEN),
    ] {
        namespaces.push(
            client
                .register_namespace(authed(
                    pb::RegisterNamespaceRequest {
                        name: name.into(),
                        ..Default::default()
                    },
                    token,
                ))
                .await
                .unwrap()
                .into_inner()
                .namespace
                .unwrap()
                .id,
        );
    }
    let (visible, hidden) = (&namespaces[0], &namespaces[1]);
    let (_, _, channel) = targets(&mut client, visible).await;
    let (mut spec, message, _) = targets(&mut client, hidden).await;
    if let Some(pb::entity::Kind::Specification(value)) = &mut spec.kind {
        value.title = "Hidden native message canary".into();
    }
    put(&mut client, spec.clone(), "").await.unwrap();
    let authored = put(
        &mut client,
        owner(visible, pb::EntityKind::Domain, message),
        "",
    )
    .await
    .unwrap();
    let mut routed = authored.entity.unwrap();
    let claims = trogon_atlas_core::message_claims::declarations_mut(&mut routed).unwrap();
    claims.routes.push(pb::ClaimRoute {
        claim_id: claims.claims[0].connection_id.clone(),
        channel: Some(pb::ChannelRef {
            id: reference(&channel).id,
        }),
        parameter_values: [("tenant".into(), String::new())].into_iter().collect(),
        ..Default::default()
    });
    let routed = put(&mut client, routed, &authored.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    let query = || pb::ListChannelMessagesRequest {
        channel: Some(pb::ChannelRef {
            id: reference(&channel).id,
        }),
        ..Default::default()
    };
    let full = client
        .list_channel_messages(authed(query(), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(full.records.len(), 1);
    assert_eq!(
        full.records[0].state,
        pb::CatalogResolutionState::Resolved as i32
    );
    let limited = client
        .list_channel_messages(authed(query(), ACME_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(limited.records.len(), 1);
    assert!(limited.complete && !limited.incomplete);
    assert_eq!(
        limited.records[0].state,
        pb::CatalogResolutionState::Unavailable as i32
    );
    assert_eq!(limited.records[0].claim, full.records[0].claim);
    assert_eq!(limited.records[0].route, full.records[0].route);
    assert_eq!(
        limited.records[0].owner_uid,
        routed.system.as_ref().unwrap().uid
    );
    assert!(!format!("{limited:?}").contains("Hidden native message canary"));
    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Specification as i32,
                id: reference(&spec).id,
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let missing = client
        .list_channel_messages(authed(query(), ROOT_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(missing.records.len(), 1);
    assert_eq!(
        missing.records[0].state,
        pb::CatalogResolutionState::Unavailable as i32
    );
    assert_eq!(missing.records[0].claim, full.records[0].claim);
    assert_eq!(missing.records[0].route, full.records[0].route);
}

async fn read_owner(client: &mut Client, value: &pb::Entity) -> pb::GetEntityResponse {
    let target = reference(value);
    client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: target.kind,
                id: target.id,
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
}

async fn refused_unchanged(
    client: &mut Client,
    current: &pb::GetEntityResponse,
    proposed: pb::Entity,
    reason: &str,
) {
    let error = put(client, proposed, &current.etag).await.unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument, "{reason}: {error}");
    let after = read_owner(client, current.entity.as_ref().unwrap()).await;
    assert_eq!(
        after.entity, current.entity,
        "{reason}: changed stored owner"
    );
    assert_eq!(after.etag, current.etag, "{reason}: changed etag");
}

#[tokio::test(flavor = "multi_thread")]
async fn native_claim_direction_message_and_utf8_matrix_is_atomic_on_every_owner() {
    use trogon_atlas_core::message_claims as claims;
    let (mut client, namespace) = fixture().await;
    let (mut alternate_spec, message, _) = targets(&mut client, &namespace).await;
    let pb::entity::Kind::Specification(alternate) = alternate_spec.kind.as_mut().unwrap() else {
        panic!()
    };
    alternate.id.as_mut().unwrap().version = 2;
    put(&mut client, alternate_spec, "").await.unwrap();
    for kind in [
        pb::EntityKind::Component,
        pb::EntityKind::ExternalSystem,
        pb::EntityKind::Domain,
    ] {
        let created = put(&mut client, owner(&namespace, kind, message.clone()), "")
            .await
            .unwrap()
            .entity
            .unwrap();
        let mut current = read_owner(&mut client, &created).await;
        for direction in [0, -1, 99] {
            let mut proposed = current.entity.clone().unwrap();
            let c = &mut claims::declarations_mut(&mut proposed).unwrap().claims[0];
            c.direction = direction;
            c.connection_id = None;
            refused_unchanged(&mut client, &current, proposed, "invalid direction").await;
        }
        for case in 0..20 {
            let mut proposed = current.entity.clone().unwrap();
            let c = &mut claims::declarations_mut(&mut proposed).unwrap().claims[0];
            c.connection_id = None;
            let m = c.message.as_mut().unwrap();
            match case {
                0 => m.owner_uid.clear(),
                1 => m.owner_uid = "01930000-0000-7000-8000-000000000999".into(),
                2 => m.member_id = None,
                3 => m.member_id.as_mut().unwrap().value.clear(),
                4 => {
                    m.member_id.as_mut().unwrap().value =
                        "01930000-0000-7000-8000-000000000999".into();
                }
                5 => m.kind = pb::ContractMemberKind::Unspecified as i32,
                6 => m.kind = 99,
                7 => {
                    m.expected_digest = Some(pb::ContentDigest {
                        sha256: "a".repeat(64),
                    });
                }
                8 => m.specification = None,
                9 => m.specification.as_mut().unwrap().id = None,
                10 => {
                    m.specification
                        .as_mut()
                        .unwrap()
                        .id
                        .as_mut()
                        .unwrap()
                        .version = 0;
                }
                11 => {
                    m.specification
                        .as_mut()
                        .unwrap()
                        .id
                        .as_mut()
                        .unwrap()
                        .version = 2;
                }
                12 => c.message = None,
                13 => m.kind = pb::ContractMemberKind::Artifact as i32,
                14 => m.kind = pb::ContractMemberKind::Schema as i32,
                15 => m.kind = pb::ContractMemberKind::Operation as i32,
                16 => m
                    .specification
                    .as_mut()
                    .unwrap()
                    .id
                    .as_mut()
                    .unwrap()
                    .namespace
                    .clear(),
                17 => m
                    .specification
                    .as_mut()
                    .unwrap()
                    .id
                    .as_mut()
                    .unwrap()
                    .slug
                    .clear(),
                18 => m.member_id.as_mut().unwrap().value = "malformed".into(),
                _ => m.owner_uid = "malformed".into(),
            }
            refused_unchanged(
                &mut client,
                &current,
                proposed,
                &format!("selector case {case}"),
            )
            .await;
        }
        for label in ["é".repeat(128), "a".repeat(256)] {
            let mut proposed = current.entity.clone().unwrap();
            claims::declarations_mut(&mut proposed).unwrap().claims[0].group =
                Some(pb::CatalogTitle {
                    value: label.clone(),
                });
            let accepted = put(&mut client, proposed, &current.etag).await.unwrap();
            let after = read_owner(&mut client, accepted.entity.as_ref().unwrap()).await;
            assert_eq!(
                claims::declarations(after.entity.as_ref().unwrap())
                    .unwrap()
                    .claims[0]
                    .group
                    .as_ref()
                    .unwrap()
                    .value,
                label
            );
            assert_eq!(
                claims::declarations(after.entity.as_ref().unwrap())
                    .unwrap()
                    .claims[0]
                    .connection_id,
                claims::declarations(current.entity.as_ref().unwrap())
                    .unwrap()
                    .claims[0]
                    .connection_id
            );
            current = after;
        }
        for label in [String::new(), " ".into(), format!("{}x", "é".repeat(128))] {
            let mut proposed = current.entity.clone().unwrap();
            claims::declarations_mut(&mut proposed).unwrap().claims[0].group =
                Some(pb::CatalogTitle { value: label });
            refused_unchanged(&mut client, &current, proposed, "invalid UTF8 group length").await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_claims_and_peer_declarations_coexist_without_inferred_peer_references() {
    use trogon_atlas_core::message_claims as claims;
    let (mut client, namespace) = fixture().await;
    let (_, message, _) = targets(&mut client, &namespace).await;
    let peer = put(
        &mut client,
        entity(pb::entity::Kind::Component(pb::Component {
            id: Some(id(&namespace, "peer")),
            title: "Peer".into(),
            ..Default::default()
        })),
        "",
    )
    .await
    .unwrap()
    .entity
    .unwrap();
    for kind in [pb::EntityKind::Component, pb::EntityKind::ExternalSystem] {
        let mut raw = owner(&namespace, kind, message.clone());
        let declaration = pb::CommunicationDeclaration {
            label: Some(pb::CatalogTitle {
                value: "Receive from peer".into(),
            }),
            direction: pb::CommunicationDirection::Receive as i32,
            peer: Some(pb::ArchitecturalEndpoint {
                target: Some(pb::architectural_endpoint::Target::Component(
                    pb::ComponentRef {
                        id: reference(&peer).id,
                    },
                )),
            }),
            message: Some(message.clone()),
            ..Default::default()
        };
        match raw.kind.as_mut().unwrap() {
            pb::entity::Kind::Component(v) => v.communications.push(declaration),
            pb::entity::Kind::ExternalSystem(v) => v.communications.push(declaration),
            _ => unreachable!(),
        }
        let saved = put(&mut client, raw, "").await.unwrap().entity.unwrap();
        let current = read_owner(&mut client, &saved).await;
        let comm = match saved.kind.as_ref().unwrap() {
            pb::entity::Kind::Component(v) => &v.communications[0],
            pb::entity::Kind::ExternalSystem(v) => &v.communications[0],
            _ => unreachable!(),
        };
        assert_ne!(
            comm.connection_id,
            claims::declarations(&saved).unwrap().claims[0].connection_id
        );
        let refs = client
            .get_outgoing_references(authed(
                pb::GetReferencesRequest {
                    kind: kind as i32,
                    id: reference(&saved).id,
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(refs.references.len(), 3);
        assert_eq!(
            refs.references
                .iter()
                .filter(|r| r.to.as_ref() == Some(&reference(&peer)))
                .count(),
            1
        );
        let mut invalid = saved.clone();
        match invalid.kind.as_mut().unwrap() {
            pb::entity::Kind::Component(v) => v.communications[0].peer = None,
            pb::entity::Kind::ExternalSystem(v) => v.communications[0].peer = None,
            _ => unreachable!(),
        }
        refused_unchanged(
            &mut client,
            &current,
            invalid,
            "declaration still requires peer",
        )
        .await;
        let mut without_claim = saved.clone();
        claims::declarations_mut(&mut without_claim)
            .unwrap()
            .claims
            .clear();
        let removed = put(&mut client, without_claim, &current.etag)
            .await
            .unwrap()
            .entity
            .unwrap();
        let retained = match removed.kind.as_ref().unwrap() {
            pb::entity::Kind::Component(v) => &v.communications[0],
            pb::entity::Kind::ExternalSystem(v) => &v.communications[0],
            _ => unreachable!(),
        };
        assert_eq!(retained, comm);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_receive_cycles_empty_selections_and_selector_boundaries_are_explicit() {
    use trogon_atlas_core::message_claims as claims;
    let (mut client, namespace) = fixture().await;
    let (spec, message, _) = targets(&mut client, &namespace).await;
    let pb::entity::Kind::Specification(s) = spec.kind.as_ref().unwrap() else {
        panic!()
    };
    let mut second = message.clone();
    second.member_id = s.messages[1].member_id.clone();
    let mut raw = owner(&namespace, pb::EntityKind::Domain, message.clone());
    claims::declarations_mut(&mut raw)
        .unwrap()
        .claims
        .push(pb::MessageClaim {
            message: Some(second.clone()),
            direction: pb::CommunicationDirection::Receive as i32,
            ..Default::default()
        });
    let first = put(&mut client, raw, "").await.unwrap();
    let mut raw = first.entity.unwrap();
    let v = claims::declarations_mut(&mut raw).unwrap();
    v.triggers = vec![
        pb::ClaimTrigger {
            claim_id: v.claims[0].connection_id.clone(),
            message: Some(second),
            ..Default::default()
        },
        pb::ClaimTrigger {
            claim_id: v.claims[1].connection_id.clone(),
            message: Some(message.clone()),
            condition: Some(pb::CatalogText {
                value: "If described".into(),
            }),
            ..Default::default()
        },
    ];
    v.field_selections.push(pb::ClaimFieldSelection {
        claim_id: v.claims[0].connection_id.clone(),
        binding: Some(pb::ConnectionRef {
            owner: Some(reference(&spec)),
            owner_uid: spec.system.as_ref().unwrap().uid.clone(),
            id: s.messages[0].schemas[0].connection_id.clone(),
        }),
        ..Default::default()
    });
    let saved = put(&mut client, raw, &first.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    let mut current = read_owner(&mut client, &saved).await;
    let v = claims::declarations(&saved).unwrap();
    assert_eq!(v.triggers.len(), 2);
    assert_eq!(v.triggers[0].condition, None);
    assert!(v.triggers[1].condition.is_some());
    assert_eq!(v.field_selections[0].selectors, []);
    let mut duplicate = saved.clone();
    let v = claims::declarations_mut(&mut duplicate).unwrap();
    let mut selection = v.field_selections[0].clone();
    selection.connection_id = None;
    v.field_selections.push(selection);
    refused_unchanged(
        &mut client,
        &current,
        duplicate,
        "duplicate local binding pair",
    )
    .await;
    let mut self_trigger = saved.clone();
    let t = &mut claims::declarations_mut(&mut self_trigger)
        .unwrap()
        .triggers[0];
    t.message = Some(message);
    t.connection_id = None;
    refused_unchanged(&mut client, &current, self_trigger, "self trigger").await;
    let selector = pb::ClaimFieldSelector {
        selector: Some(pb::claim_field_selector::Selector::Path(pb::FieldPath {
            segments: vec!["name".into()],
        })),
    };
    let mut limit = saved;
    claims::declarations_mut(&mut limit)
        .unwrap()
        .field_selections[0]
        .selectors = vec![selector.clone(); 256];
    let accepted = put(&mut client, limit, &current.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    current = read_owner(&mut client, &accepted).await;
    assert_eq!(
        claims::declarations(&accepted).unwrap().field_selections[0]
            .selectors
            .len(),
        256
    );
    let mut overflow = accepted;
    claims::declarations_mut(&mut overflow)
        .unwrap()
        .field_selections[0]
        .selectors
        .push(selector);
    refused_unchanged(&mut client, &current, overflow, "257 selectors").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_field_path_depth32_is_validated_against_real_inline_schema() {
    use trogon_atlas_core::message_claims as claims;
    let (mut client, namespace) = fixture().await;
    let (spec, message, _) = targets(&mut client, &namespace).await;
    let mut schema = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(&namespace, "payload")),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    let mut field = pb::FieldSpec {
        name: "name".into(),
        r#type: Some(pb::FieldType {
            kind: Some(pb::field_type::Kind::String(pb::field_type::StringType {})),
        }),
        ..Default::default()
    };
    for _ in 1..32 {
        field = pb::FieldSpec {
            name: "name".into(),
            r#type: Some(pb::FieldType {
                kind: Some(pb::field_type::Kind::Object(pb::field_type::ObjectType {
                    fields: vec![field],
                })),
            }),
            ..Default::default()
        };
    }
    let pb::entity::Kind::Schema(s) = schema.entity.as_mut().unwrap().kind.as_mut().unwrap() else {
        panic!()
    };
    s.fields = vec![field];
    put(&mut client, schema.entity.unwrap(), &schema.etag)
        .await
        .unwrap();
    let first = put(
        &mut client,
        owner(&namespace, pb::EntityKind::Domain, message),
        "",
    )
    .await
    .unwrap();
    let mut raw = first.entity.unwrap();
    let pb::entity::Kind::Specification(s) = spec.kind.as_ref().unwrap() else {
        panic!()
    };
    let v = claims::declarations_mut(&mut raw).unwrap();
    v.field_selections.push(pb::ClaimFieldSelection {
        claim_id: v.claims[0].connection_id.clone(),
        binding: Some(pb::ConnectionRef {
            owner: Some(reference(&spec)),
            owner_uid: spec.system.as_ref().unwrap().uid.clone(),
            id: s.messages[0].schemas[0].connection_id.clone(),
        }),
        selectors: vec![pb::ClaimFieldSelector {
            selector: Some(pb::claim_field_selector::Selector::Path(pb::FieldPath {
                segments: vec!["name".into(); 32],
            })),
        }],
        ..Default::default()
    });
    let saved = put(&mut client, raw, &first.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    let current = read_owner(&mut client, &saved).await;
    let mut invalid = saved;
    claims::declarations_mut(&mut invalid)
        .unwrap()
        .field_selections[0]
        .selectors[0]
        .selector = Some(pb::claim_field_selector::Selector::Path(pb::FieldPath {
        segments: vec!["name".into(); 33],
    }));
    refused_unchanged(&mut client, &current, invalid, "33 segments").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn native_owner_public_count_and_prepared_byte_limits_leave_complete_state_unchanged() {
    use prost::Message;
    use trogon_atlas_core::message_claims as claims;
    let (mut client, namespace) = fixture().await;
    let (_, message, channel) = targets(&mut client, &namespace).await;
    let peer = put(
        &mut client,
        entity(pb::entity::Kind::Component(pb::Component {
            id: Some(id(&namespace, "capacity-peer")),
            title: "Capacity peer".into(),
            ..Default::default()
        })),
        "",
    )
    .await
    .unwrap()
    .entity
    .unwrap();
    for kind in [
        pb::EntityKind::Component,
        pb::EntityKind::ExternalSystem,
        pb::EntityKind::Domain,
    ] {
        let mut raw = owner(&namespace, kind, message.clone());
        let v = claims::declarations_mut(&mut raw).unwrap();
        v.claims = vec![v.claims[0].clone(); 1024];
        let saved = put(&mut client, raw, "").await.unwrap().entity.unwrap();
        let mut current = read_owner(&mut client, &saved).await;
        assert_eq!(
            claims::declarations(current.entity.as_ref().unwrap())
                .unwrap()
                .claims
                .len(),
            1024
        );
        assert_eq!(
            trogon_atlas_core::connections::connection_occurrences(
                current.entity.as_ref().unwrap()
            )
            .unwrap()
            .len(),
            1024
        );
        let mut overflow = saved.clone();
        let v = claims::declarations_mut(&mut overflow).unwrap();
        let mut extra = v.claims[0].clone();
        extra.connection_id = None;
        v.claims.push(extra);
        refused_unchanged(&mut client, &current, overflow, "1025 claims").await;
        let mut routes = saved;
        let v = claims::declarations_mut(&mut routes).unwrap();
        v.routes = vec![
            pb::ClaimRoute {
                claim_id: v.claims[0].connection_id.clone(),
                channel: Some(pb::ChannelRef {
                    id: reference(&channel).id
                }),
                parameter_values: [("tenant".into(), String::new())].into_iter().collect(),
                ..Default::default()
            };
            3072
        ];
        let saved = put(&mut client, routes, &current.etag)
            .await
            .unwrap()
            .entity
            .unwrap();
        current = read_owner(&mut client, &saved).await;
        assert_eq!(
            trogon_atlas_core::connections::connection_occurrences(
                current.entity.as_ref().unwrap()
            )
            .unwrap()
            .len(),
            4096
        );
        let mut overflow = saved;
        let v = claims::declarations_mut(&mut overflow).unwrap();
        let mut extra = v.routes[0].clone();
        extra.connection_id = None;
        v.routes.push(extra);
        refused_unchanged(&mut client, &current, overflow, "4097 occurrences").await;
        if kind != pb::EntityKind::Domain {
            let mut mixed = current.entity.clone().unwrap();
            let relationship = pb::ArchitecturalRelationship {
                target: Some(pb::ArchitecturalEndpoint {
                    target: Some(pb::architectural_endpoint::Target::Component(
                        pb::ComponentRef {
                            id: reference(&peer).id,
                        },
                    )),
                }),
                label: Some(pb::CatalogTitle {
                    value: "Existing relationship".into(),
                }),
                ..Default::default()
            };
            match mixed.kind.as_mut().unwrap() {
                pb::entity::Kind::Component(v) => v.relationships.push(relationship),
                pb::entity::Kind::ExternalSystem(v) => v.relationships.push(relationship),
                _ => unreachable!(),
            }
            refused_unchanged(
                &mut client,
                &current,
                mixed.clone(),
                "existing relationship shares occurrence cap",
            )
            .await;
            claims::declarations_mut(&mut mixed).unwrap().routes.pop();
            let saved = put(&mut client, mixed, &current.etag)
                .await
                .unwrap()
                .entity
                .unwrap();
            current = read_owner(&mut client, &saved).await;
            assert_eq!(
                trogon_atlas_core::connections::connection_occurrences(
                    current.entity.as_ref().unwrap()
                )
                .unwrap()
                .len(),
                4096
            );
        }
        let mut padded = current.entity.clone().unwrap();
        let mut last = 0;
        for index in 0..1024 {
            let gap = claims::MAX_ENCODED_BYTES - padded.encoded_len();
            let mut length = gap.min(16_000).saturating_sub(8);
            loop {
                claims::declarations_mut(&mut padded).unwrap().claims[index].doc =
                    Some(pb::CatalogText {
                        value: "x".repeat(length),
                    });
                let size = padded.encoded_len();
                if gap > 16_016 || size == claims::MAX_ENCODED_BYTES {
                    break;
                }
                if size > claims::MAX_ENCODED_BYTES {
                    length -= size - claims::MAX_ENCODED_BYTES;
                } else {
                    length += claims::MAX_ENCODED_BYTES - size;
                }
            }
            last = index;
            if padded.encoded_len() == claims::MAX_ENCODED_BYTES {
                break;
            }
        }
        assert_eq!(padded.encoded_len(), claims::MAX_ENCODED_BYTES);
        let saved = put(&mut client, padded, &current.etag)
            .await
            .unwrap()
            .entity
            .unwrap();
        current = read_owner(&mut client, &saved).await;
        assert_eq!(
            current.entity.as_ref().unwrap().encoded_len(),
            claims::MAX_ENCODED_BYTES
        );
        let mut overflow = saved;
        claims::declarations_mut(&mut overflow).unwrap().claims[last]
            .doc
            .as_mut()
            .unwrap()
            .value
            .push('x');
        refused_unchanged(&mut client, &current, overflow, "prepared owner 1MiB+1").await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn exact_subject_queries_distinguish_every_claim_leaf_from_equal_siblings() {
    use trogon_atlas_core::message_claims as claims;
    let (mut client, namespace) = fixture().await;
    let (spec, message, channel) = targets(&mut client, &namespace).await;
    let pb::entity::Kind::Specification(s) = spec.kind.as_ref().unwrap() else {
        panic!()
    };
    let mut triggered = message.clone();
    triggered.member_id = s.messages[1].member_id.clone();
    let mut raw = owner(&namespace, pb::EntityKind::Domain, message);
    let v = claims::declarations_mut(&mut raw).unwrap();
    v.claims.push(v.claims[0].clone());
    let first = put(&mut client, raw, "").await.unwrap();
    let mut raw = first.entity.unwrap();
    let v = claims::declarations_mut(&mut raw).unwrap();
    for index in 0..2 {
        v.routes.push(pb::ClaimRoute {
            claim_id: v.claims[index].connection_id.clone(),
            channel: Some(pb::ChannelRef {
                id: reference(&channel).id,
            }),
            parameter_values: [("tenant".into(), String::new())].into_iter().collect(),
            ..Default::default()
        });
        let mut target = triggered.clone();
        target.member_id = s.messages[1].member_id.clone();
        v.triggers.push(pb::ClaimTrigger {
            claim_id: v.claims[index].connection_id.clone(),
            message: Some(target),
            ..Default::default()
        });
        v.field_selections.push(pb::ClaimFieldSelection {
            claim_id: v.claims[index].connection_id.clone(),
            binding: Some(pb::ConnectionRef {
                owner: Some(reference(&spec)),
                owner_uid: spec.system.as_ref().unwrap().uid.clone(),
                id: s.messages[0].schemas[0].connection_id.clone(),
            }),
            ..Default::default()
        });
    }
    let saved = put(&mut client, raw, &first.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    let v = claims::declarations(&saved).unwrap();
    for (name, ids) in [
        (
            "claim",
            [
                v.claims[0].connection_id.clone(),
                v.claims[1].connection_id.clone(),
            ],
        ),
        (
            "route",
            [
                v.routes[0].connection_id.clone(),
                v.routes[1].connection_id.clone(),
            ],
        ),
        (
            "trigger",
            [
                v.triggers[0].connection_id.clone(),
                v.triggers[1].connection_id.clone(),
            ],
        ),
        (
            "selection",
            [
                v.field_selections[0].connection_id.clone(),
                v.field_selections[1].connection_id.clone(),
            ],
        ),
    ] {
        let selected = pb::ConnectionRef {
            owner: Some(reference(&saved)),
            owner_uid: saved.system.as_ref().unwrap().uid.clone(),
            id: ids[0].clone(),
        };
        for kind in [
            pb::EntityKind::Team,
            pb::EntityKind::Person,
            pb::EntityKind::Decision,
            pb::EntityKind::Document,
        ] {
            put(
                &mut client,
                standalone_field_association(
                    &namespace,
                    &format!("{name}-{kind:?}").to_lowercase(),
                    kind,
                    selected.clone(),
                ),
                "",
            )
            .await
            .unwrap();
        }
        for (leaf, expected) in [
            (selected.clone(), 1usize),
            (
                pb::ConnectionRef {
                    id: ids[1].clone(),
                    ..selected.clone()
                },
                0,
            ),
        ] {
            let catalog = pb::CatalogSubject {
                selection: Some(pb::catalog_subject::Selection::Connection(leaf.clone())),
            };
            let docs = client
                .list_subject_documents(authed(
                    pb::ListSubjectDocumentsRequest {
                        subject: Some(catalog.clone()),
                    },
                    ROOT_TOKEN,
                ))
                .await
                .unwrap()
                .into_inner();
            let decisions = client
                .list_subject_decisions(authed(
                    pb::ListSubjectDecisionsRequest {
                        subject: Some(catalog),
                        ..Default::default()
                    },
                    ROOT_TOKEN,
                ))
                .await
                .unwrap()
                .into_inner();
            let assignments = client
                .list_subject_assignments(authed(
                    pb::ListSubjectAssignmentsRequest {
                        subject: Some(pb::ResponsibilitySubject {
                            selection: Some(pb::responsibility_subject::Selection::Connection(
                                leaf,
                            )),
                        }),
                        ..Default::default()
                    },
                    ROOT_TOKEN,
                ))
                .await
                .unwrap()
                .into_inner();
            assert_eq!(docs.declarations.len(), expected, "{name} documents");
            assert_eq!(decisions.declarations.len(), expected, "{name} decisions");
            assert_eq!(
                assignments.assignments.len(),
                expected * 2,
                "{name} assignments"
            );
            assert!(!docs.incomplete && !decisions.incomplete && !assignments.incomplete);
            for declaration in docs.declarations.iter().chain(&decisions.declarations) {
                let Some(pb::catalog_declaration::Declaration::Subject(association)) =
                    &declaration.declaration
                else {
                    panic!("unexpected declaration")
                };
                assert_eq!(catalog_leaf(association), &selected);
            }
            for assignment in &assignments.assignments {
                assert_eq!(
                    responsibility_leaf(assignment.assignment.as_ref().unwrap()),
                    &selected
                );
            }
            assert_eq!(
                assignments.subject_state,
                pb::ResponsibilityResolutionState::Resolved as i32
            );
        }
        let context = claims::resolve_connection_context(&saved, &selected).unwrap();
        assert_eq!(
            context,
            if name == "route" {
                reference(&channel)
            } else {
                reference(&spec)
            }
        );
    }
    for target in [reference(&saved), reference(&channel), reference(&spec)] {
        let catalog = pb::CatalogSubject {
            selection: Some(pb::catalog_subject::Selection::Entity(target.clone())),
        };
        let docs = client
            .list_subject_documents(authed(
                pb::ListSubjectDocumentsRequest {
                    subject: Some(catalog.clone()),
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        let decisions = client
            .list_subject_decisions(authed(
                pb::ListSubjectDecisionsRequest {
                    subject: Some(catalog),
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        let assignments = client
            .list_subject_assignments(authed(
                pb::ListSubjectAssignmentsRequest {
                    subject: Some(pb::ResponsibilitySubject {
                        selection: Some(pb::responsibility_subject::Selection::Entity(target)),
                    }),
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        assert!(
            docs.declarations.is_empty()
                && decisions.declarations.is_empty()
                && assignments.assignments.is_empty()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_fingerprints_reorder_retirement_preview_retry_and_owner_incarnations() {
    use trogon_atlas_core::message_claims as claims;
    let (mut client, namespace) = fixture().await;
    let (mut spec, message, mut channel) = targets(&mut client, &namespace).await;
    let etag = read_owner(&mut client, &spec).await.etag;
    let pb::entity::Kind::Specification(s) = spec.kind.as_mut().unwrap() else {
        panic!()
    };
    let mut binding = s.messages[0].schemas[0].clone();
    binding.connection_id = None;
    s.messages[0].schemas.push(binding);
    let mut third = s.messages[1].clone();
    third.member_id = None;
    third.name = Some(pb::MemberName {
        value: "Third".into(),
    });
    for binding in &mut third.schemas {
        binding.connection_id = None;
    }
    s.messages.push(third);
    spec = put(&mut client, spec, &etag).await.unwrap().entity.unwrap();
    let pb::entity::Kind::Specification(s) = spec.kind.as_ref().unwrap() else {
        panic!()
    };
    let pb::entity::Kind::Channel(ch) = channel.kind.as_mut().unwrap() else {
        panic!()
    };
    ch.id.as_mut().unwrap().slug = "second-channel".into();
    for binding in &mut ch.bindings {
        binding.connection_id = None;
    }
    let second_channel = put(&mut client, channel, "").await.unwrap().entity.unwrap();
    let mut raw = owner(&namespace, pb::EntityKind::Domain, message.clone());
    let v = claims::declarations_mut(&mut raw).unwrap();
    v.claims.push(v.claims[0].clone());
    let first = put(&mut client, raw, "").await.unwrap();
    let mut raw = first.entity.unwrap();
    let v = claims::declarations_mut(&mut raw).unwrap();
    v.routes = vec![
        pb::ClaimRoute {
            claim_id: v.claims[0].connection_id.clone(),
            channel: Some(pb::ChannelRef {
                id: Some(id(&namespace, "channel"))
            }),
            parameter_values: [("tenant".into(), String::new())].into_iter().collect(),
            ..Default::default()
        };
        2
    ];
    let mut triggered = message.clone();
    triggered.member_id = s.messages[1].member_id.clone();
    v.triggers = vec![
        pb::ClaimTrigger {
            claim_id: v.claims[0].connection_id.clone(),
            message: Some(triggered),
            ..Default::default()
        };
        2
    ];
    v.field_selections.push(pb::ClaimFieldSelection {
        claim_id: v.claims[0].connection_id.clone(),
        binding: Some(pb::ConnectionRef {
            owner: Some(reference(&spec)),
            owner_uid: spec.system.as_ref().unwrap().uid.clone(),
            id: s.messages[0].schemas[0].connection_id.clone(),
        }),
        ..Default::default()
    });
    let saved = put(&mut client, raw, &first.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    let mut current = read_owner(&mut client, &saved).await;
    let original_ids = trogon_atlas_core::connections::connection_occurrences(&saved).unwrap();
    let mut descriptive = saved.clone();
    let v = claims::declarations_mut(&mut descriptive).unwrap();
    v.claims[0].group = Some(pb::CatalogTitle {
        value: "Display".into(),
    });
    v.claims[0].doc = Some(pb::CatalogText {
        value: "Description".into(),
    });
    v.routes[0]
        .parameter_values
        .insert("tenant".into(), "x".into());
    v.routes[0].delivery_mode = Some(pb::ClaimDeliveryMode::PushPull as i32);
    v.routes[0].doc = Some(pb::CatalogText {
        value: "Route prose".into(),
    });
    v.triggers[0].condition = Some(pb::CatalogText {
        value: "Described condition".into(),
    });
    v.triggers[0].doc = Some(pb::CatalogText {
        value: "Trigger prose".into(),
    });
    v.field_selections[0]
        .selectors
        .push(pb::ClaimFieldSelector {
            selector: Some(pb::claim_field_selector::Selector::Opaque(
                pb::OpaqueTypeSelector {
                    syntax: "json-pointer".into(),
                    value: "/unverified".into(),
                },
            )),
        });
    v.field_selections[0].doc = Some(pb::CatalogText {
        value: "Field prose".into(),
    });
    let saved = put(&mut client, descriptive, &current.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    assert_eq!(
        trogon_atlas_core::connections::connection_occurrences(&saved).unwrap(),
        original_ids
    );
    current = read_owner(&mut client, &saved).await;
    let mut reordered = saved.clone();
    let v = claims::declarations_mut(&mut reordered).unwrap();
    v.claims.reverse();
    v.routes.reverse();
    v.triggers.reverse();
    let reordered = put(&mut client, reordered, &current.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    let before = claims::declarations(&saved).unwrap();
    let after = claims::declarations(&reordered).unwrap();
    assert_eq!(after.claims[1], before.claims[0]);
    assert_eq!(after.routes[1], before.routes[0]);
    assert_eq!(after.triggers[1], before.triggers[0]);
    current = read_owner(&mut client, &reordered).await;
    for case in 0..6 {
        let mut wrong = current.entity.clone().unwrap();
        let v = claims::declarations_mut(&mut wrong).unwrap();
        match case {
            0 => v.routes[0].claim_id = v.claims[0].connection_id.clone(),
            1 => {
                v.routes[0].channel = Some(pb::ChannelRef {
                    id: reference(&second_channel).id,
                });
            }
            2 => v.triggers[0].claim_id = v.claims[0].connection_id.clone(),
            3 => {
                v.triggers[0].message.as_mut().unwrap().member_id = s.messages[2].member_id.clone();
            }
            4 => v.field_selections[0].claim_id = v.claims[0].connection_id.clone(),
            _ => {
                v.field_selections[0].binding.as_mut().unwrap().id =
                    s.messages[0].schemas[1].connection_id.clone();
            }
        }
        refused_unchanged(
            &mut client,
            &current,
            wrong.clone(),
            &format!("fingerprint {case} retained ID"),
        )
        .await;
        let v = claims::declarations_mut(&mut wrong).unwrap();
        let old = match case {
            0 | 1 => v.routes[0].connection_id.take(),
            2 | 3 => v.triggers[0].connection_id.take(),
            _ => v.field_selections[0].connection_id.take(),
        };
        let preview = client
            .put_entity(authed(
                pb::PutEntityRequest {
                    entity: Some(wrong.clone()),
                    if_match: current.etag.clone(),
                    validate_only: true,
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        let pv = claims::declarations(preview.entity.as_ref().unwrap()).unwrap();
        let new = match case {
            0 | 1 => &pv.routes[0].connection_id,
            2 | 3 => &pv.triggers[0].connection_id,
            _ => &pv.field_selections[0].connection_id,
        };
        assert_ne!(*new, old);
        refused_unchanged(
            &mut client,
            &current,
            preview.entity.clone().unwrap(),
            "preview IDs are unreserved",
        )
        .await;
        assert_eq!(
            read_owner(&mut client, &reordered).await.entity,
            current.entity
        );
        assert_eq!(read_owner(&mut client, &reordered).await.etag, current.etag);
        let request = pb::PutEntityRequest {
            entity: Some(wrong),
            if_match: current.etag.clone(),
            operation_id: format!("native-fingerprint-{case}"),
            ..Default::default()
        };
        let applied = client
            .put_entity(authed(request.clone(), ROOT_TOKEN))
            .await
            .unwrap()
            .into_inner();
        let replay = client
            .put_entity(authed(request, ROOT_TOKEN))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(applied.entity, replay.entity);
        assert_eq!(applied.etag, replay.etag);
        // Restore only this changed descriptor with fresh identity before the next independent case.
        let mut restored = current.entity.clone().unwrap();
        let v = claims::declarations_mut(&mut restored).unwrap();
        match case {
            0 | 1 => v.routes[0].connection_id = None,
            2 | 3 => v.triggers[0].connection_id = None,
            _ => v.field_selections[0].connection_id = None,
        }
        let restored = put(&mut client, restored, &applied.etag)
            .await
            .unwrap()
            .entity
            .unwrap();
        current = read_owner(&mut client, &restored).await;
    }
    let source = current.entity.clone().unwrap();
    let source_leaf = pb::ConnectionRef {
        owner: Some(reference(&source)),
        owner_uid: source.system.as_ref().unwrap().uid.clone(),
        id: claims::declarations(&source).unwrap().claims[0]
            .connection_id
            .clone(),
    };
    put(
        &mut client,
        standalone_field_association(
            &namespace,
            "incoming-original",
            pb::EntityKind::Document,
            source_leaf.clone(),
        ),
        "",
    )
    .await
    .unwrap();
    for (slug, version) in [("clone", 1), ("owner", 2)] {
        let mut cloned = source.clone();
        let pb::entity::Kind::Domain(d) = cloned.kind.as_mut().unwrap() else {
            panic!()
        };
        d.id = Some(pb::Id {
            namespace: namespace.clone(),
            slug: slug.into(),
            version,
        });
        let cloned = put(&mut client, cloned, "").await.unwrap().entity.unwrap();
        assert_ne!(
            cloned.system.as_ref().unwrap().uid,
            source.system.as_ref().unwrap().uid
        );
        assert_eq!(claims::declarations(&cloned), claims::declarations(&source));
        let clone_leaf = pb::ConnectionRef {
            owner: Some(reference(&cloned)),
            owner_uid: cloned.system.as_ref().unwrap().uid.clone(),
            ..source_leaf.clone()
        };
        let docs = client
            .list_subject_documents(authed(
                pb::ListSubjectDocumentsRequest {
                    subject: Some(pb::CatalogSubject {
                        selection: Some(pb::catalog_subject::Selection::Connection(clone_leaf)),
                    }),
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        assert!(docs.declarations.is_empty() && !docs.incomplete);
    }
    let mut removed = source.clone();
    let old_route = claims::declarations_mut(&mut removed)
        .unwrap()
        .routes
        .remove(0);
    let removed = put(&mut client, removed, &current.etag).await.unwrap();
    let mut readd = removed.entity.unwrap();
    let mut route = old_route.clone();
    route.connection_id = None;
    claims::declarations_mut(&mut readd)
        .unwrap()
        .routes
        .push(route);
    let readd = put(&mut client, readd, &removed.etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    assert_ne!(
        claims::declarations(&readd)
            .unwrap()
            .routes
            .last()
            .unwrap()
            .connection_id,
        old_route.connection_id
    );
    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Domain as i32,
                id: reference(&readd).id,
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let recreated = put(&mut client, readd.clone(), "")
        .await
        .unwrap()
        .entity
        .unwrap();
    assert_ne!(
        recreated.system.as_ref().unwrap().uid,
        readd.system.as_ref().unwrap().uid
    );
    let old = pb::ConnectionRef {
        owner: Some(reference(&readd)),
        owner_uid: readd.system.as_ref().unwrap().uid.clone(),
        id: claims::declarations(&readd).unwrap().claims[0]
            .connection_id
            .clone(),
    };
    assert!(claims::resolve_connection_context(&recreated, &old).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn native_schema_hydration_is_once_per_request_and_timeouts_remain_errors() {
    use std::sync::atomic::Ordering;
    let store = Arc::new(observed_claim_store::ObservedClaimStore {
        inner: trogon_atlas_testsupport::shared().await.store().await,
        schema_reads: Default::default(),
        timeout: Default::default(),
    });
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
    let mut client = serve_with_registry(service, make_registry()).await;
    let namespace = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "schema-observed".into(),
                parent: "commerce".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let leaf = native_field_leaf(&mut client, &namespace).await;
    store.schema_reads.store(0, Ordering::SeqCst);
    let ops = (0..64)
        .map(|i| pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                entity: Some(standalone_field_association(
                    &namespace,
                    &format!("cached-{i}"),
                    pb::EntityKind::Document,
                    leaf.clone(),
                )),
                ..Default::default()
            })),
        })
        .collect();
    let result = client
        .batch_mutate(authed(
            pb::BatchMutateRequest {
                ops,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        result.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "{result:?}"
    );
    assert_eq!(
        store.schema_reads.load(Ordering::SeqCst),
        1,
        "exact Schema must hydrate once across standalone owners"
    );
    let subject_owner = leaf.owner.as_ref().unwrap();
    let template = client
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: subject_owner.kind,
                id: subject_owner.id.clone(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    store.schema_reads.store(0, Ordering::SeqCst);
    let ops = (0..32)
        .map(|i| {
            let mut raw = template.clone();
            let pb::entity::Kind::Domain(d) = raw.kind.as_mut().unwrap() else {
                panic!()
            };
            d.id.as_mut().unwrap().slug = format!("native-cached-{i}");
            pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                    entity: Some(raw),
                    ..Default::default()
                })),
            }
        })
        .collect();
    let result = client
        .batch_mutate(authed(
            pb::BatchMutateRequest {
                ops,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        result.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "{result:?}"
    );
    assert_eq!(
        store.schema_reads.load(Ordering::SeqCst),
        1,
        "native owners must share exact Schema hydration"
    );
    store.timeout.store(true, Ordering::SeqCst);
    for kind in [
        pb::EntityKind::Document,
        pb::EntityKind::Decision,
        pb::EntityKind::Team,
        pb::EntityKind::Person,
    ] {
        let raw = standalone_field_association(
            &namespace,
            &format!("timeout-{kind:?}").to_lowercase(),
            kind,
            leaf.clone(),
        );
        let error = put(&mut client, raw.clone(), "").await.unwrap_err();
        assert_eq!(error.code(), Code::Internal);
        assert_eq!(error.message(), "storage error");
        let r = reference(&raw);
        assert_eq!(
            client
                .get_entity(authed(
                    pb::GetEntityRequest {
                        kind: r.kind,
                        id: r.id
                    },
                    ROOT_TOKEN
                ))
                .await
                .unwrap_err()
                .code(),
            Code::NotFound
        );
    }
    store.timeout.store(false, Ordering::SeqCst);
    assert!(
        put(
            &mut client,
            standalone_field_association(
                &namespace,
                "after-timeout",
                pb::EntityKind::Document,
                leaf
            ),
            ""
        )
        .await
        .is_ok()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn native_claim_branch_rebase_and_owner_deletion_restore_exact_identity() {
    use support::two_tenants::authed_branch;
    use trogon_atlas_core::message_claims as claims;
    let (mut client, namespace) = fixture().await;
    let (_, message, channel) = targets(&mut client, &namespace).await;
    let first = put(
        &mut client,
        owner(&namespace, pb::EntityKind::Domain, message),
        "",
    )
    .await
    .unwrap();
    let mut raw = first.entity.unwrap();
    let v = claims::declarations_mut(&mut raw).unwrap();
    v.routes.push(pb::ClaimRoute {
        claim_id: v.claims[0].connection_id.clone(),
        channel: Some(pb::ChannelRef {
            id: reference(&channel).id,
        }),
        parameter_values: [("tenant".into(), String::new())].into_iter().collect(),
        ..Default::default()
    });
    let base = put(&mut client, raw, &first.etag).await.unwrap();
    let base_entity = base.entity.unwrap();
    client
        .create_branch(authed(
            pb::CreateBranchRequest {
                name: "native-rebase".into(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let mut branch = base_entity.clone();
    claims::declarations_mut(&mut branch).unwrap().claims[0].group = Some(pb::CatalogTitle {
        value: "Branch group".into(),
    });
    let branch = client
        .put_entity(authed_branch(
            pb::PutEntityRequest {
                entity: Some(branch),
                ..Default::default()
            },
            ROOT_TOKEN,
            "native-rebase",
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let mut baseline = base_entity.clone();
    let pb::entity::Kind::Domain(d) = baseline.kind.as_mut().unwrap() else {
        panic!()
    };
    d.title = "Baseline title".into();
    put(&mut client, baseline, &base.etag).await.unwrap();
    let updated = client
        .update_branch(authed(
            pb::UpdateBranchRequest {
                name: "native-rebase".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(updated.conflicts.len(), 1);
    let conflict = &updated.conflicts[0];
    assert_eq!(conflict.base.as_ref(), Some(&base_entity));
    assert_eq!(conflict.ours.as_ref(), Some(&branch));
    assert!(conflict.auto_merged.is_some());
    client
        .resolve_branch_entry(authed(
            pb::ResolveBranchEntryRequest {
                name: "native-rebase".into(),
                r#ref: Some(reference(&base_entity)),
                resolution: pb::resolve_branch_entry_request::Resolution::TakeMerged as i32,
                expected_state: conflict.state.clone(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let updated = client
        .update_branch(authed(
            pb::UpdateBranchRequest {
                name: "native-rebase".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(updated.conflicts, []);
    let rebased = client
        .get_entity(authed_branch(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Domain as i32,
                id: reference(&base_entity).id,
            },
            ROOT_TOKEN,
            "native-rebase",
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(
        claims::declarations(&rebased),
        claims::declarations(&branch)
    );
    assert_eq!(
        rebased.system.as_ref().unwrap().uid,
        base_entity.system.as_ref().unwrap().uid
    );
    client
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                kind: pb::EntityKind::Domain as i32,
                id: reference(&base_entity).id,
                mode: pb::delete_entity_request::Mode::Force as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let blocked = client
        .merge_branch(authed(
            pb::MergeBranchRequest {
                name: "native-rebase".into(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        blocked.status,
        pb::merge_branch_response::Status::Conflicts as i32
    );
    client
        .resolve_branch_entry(authed(
            pb::ResolveBranchEntryRequest {
                name: "native-rebase".into(),
                r#ref: Some(reference(&base_entity)),
                resolution: pb::resolve_branch_entry_request::Resolution::KeepOurs as i32,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let restored = client
        .get_entity(authed_branch(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Domain as i32,
                id: reference(&base_entity).id,
            },
            ROOT_TOKEN,
            "native-rebase",
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(
        claims::declarations(&restored),
        claims::declarations(&rebased)
    );
    assert_eq!(
        restored.system.as_ref().unwrap().uid,
        rebased.system.as_ref().unwrap().uid
    );
    for dry_run in [true, false] {
        let merged = client
            .merge_branch(authed(
                pb::MergeBranchRequest {
                    name: "native-rebase".into(),
                    dry_run,
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            merged.status,
            if dry_run {
                pb::merge_branch_response::Status::Validated as i32
            } else {
                pb::merge_branch_response::Status::Applied as i32
            },
            "{merged:?}"
        );
    }
    let final_owner = read_owner(&mut client, &base_entity).await.entity.unwrap();
    assert_eq!(
        claims::declarations(&final_owner),
        claims::declarations(&restored)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn independent_native_same_key_creates_require_an_explicit_incarnation_choice() {
    use support::two_tenants::authed_branch;
    let (mut client, namespace) = fixture().await;
    let (_, message, _) = targets(&mut client, &namespace).await;
    client
        .create_branch(authed(
            pb::CreateBranchRequest {
                name: "native-independent".into(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let raw = owner(&namespace, pb::EntityKind::Domain, message);
    let branch = client
        .put_entity(authed_branch(
            pb::PutEntityRequest {
                entity: Some(raw.clone()),
                ..Default::default()
            },
            ROOT_TOKEN,
            "native-independent",
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let baseline = put(&mut client, raw, "").await.unwrap().entity.unwrap();
    assert_ne!(
        branch.system.as_ref().unwrap().uid,
        baseline.system.as_ref().unwrap().uid
    );
    let diff = client
        .diff_branch(authed(
            pb::DiffBranchRequest {
                name: "native-independent".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(diff.entries.len(), 1);
    assert!(diff.entries[0].auto_merged.is_none());
    let blocked = client
        .merge_branch(authed(
            pb::MergeBranchRequest {
                name: "native-independent".into(),
                auto_merge: true,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        blocked.status,
        pb::merge_branch_response::Status::Conflicts as i32
    );
    assert_eq!(
        read_owner(&mut client, &baseline).await.entity,
        Some(baseline.clone())
    );
    let decided_state = diff.entries[0].state.clone();
    let mut advanced = baseline.clone();
    let pb::entity::Kind::Domain(d) = advanced.kind.as_mut().unwrap() else {
        panic!()
    };
    d.title = "Advanced baseline after decision".into();
    let etag = read_owner(&mut client, &baseline).await.etag;
    let advanced = put(&mut client, advanced, &etag)
        .await
        .unwrap()
        .entity
        .unwrap();
    let stale = client
        .resolve_branch_entry(authed(
            pb::ResolveBranchEntryRequest {
                name: "native-independent".into(),
                r#ref: Some(reference(&baseline)),
                resolution: pb::resolve_branch_entry_request::Resolution::TakeTheirs as i32,
                expected_state: decided_state,
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(stale.code(), Code::Aborted);
    assert_eq!(
        read_owner(&mut client, &baseline).await.entity,
        Some(advanced.clone())
    );
    let fresh = client
        .diff_branch(authed(
            pb::DiffBranchRequest {
                name: "native-independent".into(),
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    client
        .resolve_branch_entry(authed(
            pb::ResolveBranchEntryRequest {
                name: "native-independent".into(),
                r#ref: Some(reference(&baseline)),
                resolution: pb::resolve_branch_entry_request::Resolution::TakeTheirs as i32,
                expected_state: fresh.entries[0].state.clone(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap();
    let selected = client
        .get_entity(authed_branch(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Domain as i32,
                id: reference(&baseline).id,
            },
            ROOT_TOKEN,
            "native-independent",
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(selected, advanced);
    let merged = client
        .merge_branch(authed(
            pb::MergeBranchRequest {
                name: "native-independent".into(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        merged.status,
        pb::merge_branch_response::Status::Applied as i32
    );
    assert_eq!(
        read_owner(&mut client, &baseline).await.entity,
        Some(advanced)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn native_graph_work_overflow_is_scoped_to_baseline_or_branch_authorized_lens() {
    use support::two_tenants::{ACME_TOKEN, BETA_TOKEN, authed_branch};
    use trogon_atlas_store::{Store, WriteContext, store::MutationOp};
    let (mut client, _, store) = fixture_with_store().await;
    let visible = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "graph-visible".into(),
                parent: String::new(),
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let hidden = client
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "graph-hidden".into(),
                parent: String::new(),
            },
            BETA_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let (spec, message, _) = targets(&mut client, &visible).await;
    let ops = (0..10)
        .map(|i| {
            let mut raw = owner(&hidden, pb::EntityKind::Domain, message.clone());
            let pb::entity::Kind::Domain(d) = raw.kind.as_mut().unwrap() else {
                panic!()
            };
            d.id.as_mut().unwrap().slug = format!("graph-{i}");
            let v = trogon_atlas_core::message_claims::declarations_mut(&mut raw).unwrap();
            v.claims = vec![v.claims[0].clone(); 1024];
            MutationOp::Create {
                kind: pb::EntityKind::Domain,
                entity: raw,
            }
        })
        .collect::<Vec<_>>();
    store
        .batch_apply(&ops, WriteContext::baseline())
        .await
        .unwrap();
    client
        .create_branch(authed(
            pb::CreateBranchRequest {
                name: "@acme-writer:native-graph".into(),
                ..Default::default()
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap();
    for branch in [None, Some("@acme-writer:native-graph")] {
        for token in [ROOT_TOKEN, ACME_TOKEN] {
            let request = pb::GetReferencesRequest {
                kind: pb::EntityKind::Specification as i32,
                id: reference(&spec).id,
                ..Default::default()
            };
            let request = match branch {
                Some(b) => authed_branch(request, token, b),
                None => authed(request, token),
            };
            let refs = client.get_incoming_references(request).await;
            let impact = pb::GetImpactRequest {
                root: Some(reference(&spec)),
                max_depth: 1,
                ..Default::default()
            };
            let impact = match branch {
                Some(b) => authed_branch(impact, token, b),
                None => authed(impact, token),
            };
            let impact = client.get_impact(impact).await;
            if token == ROOT_TOKEN {
                let error = refs.unwrap_err();
                assert_eq!(error.code(), Code::ResourceExhausted);
                assert!(error.message().contains("occurrence budget"));
                assert_eq!(impact.unwrap_err().code(), Code::ResourceExhausted);
            } else {
                let refs = refs.unwrap().into_inner();
                assert_eq!(refs.references, []);
                assert_eq!(refs.next_page_token, "");
                let nodes = impact.unwrap().into_inner().nodes;
                assert_eq!(
                    nodes
                        .iter()
                        .map(|n| n.entity.clone().unwrap())
                        .collect::<Vec<_>>(),
                    vec![
                        reference(&spec),
                        pb::EntityRef {
                            kind: pb::EntityKind::Schema as i32,
                            id: Some(id(&visible, "payload"))
                        }
                    ]
                );
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_claim_direction_and_message_fingerprints_retire_only_changed_occurrences() {
    use trogon_atlas_core::message_claims as claims;
    let (mut client, namespace) = fixture().await;
    let (spec, message, _) = targets(&mut client, &namespace).await;
    let pb::entity::Kind::Specification(s) = spec.kind.as_ref().unwrap() else {
        panic!()
    };
    for kind in [
        pb::EntityKind::Component,
        pb::EntityKind::ExternalSystem,
        pb::EntityKind::Domain,
    ] {
        let mut raw = owner(&namespace, kind, message.clone());
        let v = claims::declarations_mut(&mut raw).unwrap();
        v.claims.push(v.claims[0].clone());
        let saved = put(&mut client, raw, "").await.unwrap().entity.unwrap();
        let mut current = read_owner(&mut client, &saved).await;
        for direction_change in [true, false] {
            let mut proposed = current.entity.clone().unwrap();
            let c = &mut claims::declarations_mut(&mut proposed).unwrap().claims[0];
            if direction_change {
                c.direction = pb::CommunicationDirection::Send as i32;
            } else {
                c.message.as_mut().unwrap().member_id = s.messages[1].member_id.clone();
            }
            refused_unchanged(
                &mut client,
                &current,
                proposed.clone(),
                "claim fingerprint needs fresh ID",
            )
            .await;
            let previous = claims::declarations(current.entity.as_ref().unwrap()).unwrap();
            let old = previous.claims[0].connection_id.clone();
            let sibling = previous.claims[1].clone();
            claims::declarations_mut(&mut proposed).unwrap().claims[0].connection_id = None;
            let saved = put(&mut client, proposed, &current.etag)
                .await
                .unwrap()
                .entity
                .unwrap();
            let after = claims::declarations(&saved).unwrap();
            assert_ne!(after.claims[0].connection_id, old);
            assert_eq!(after.claims[1], sibling);
            current = read_owner(&mut client, &saved).await;
        }
    }
}
