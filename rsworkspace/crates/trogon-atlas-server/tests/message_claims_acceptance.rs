#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
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
    let (spec, message, channel) = targets(&mut client, &namespace).await;
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
    let boundary = client
        .list_subject_documents(request(selected.clone()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(boundary.declarations.len(), 4096);
    assert!(!boundary.incomplete);
    let sibling = pb::ConnectionRef {
        id: declarations.claims[1].connection_id.clone(),
        ..selected.clone()
    };
    let sibling = client
        .list_subject_documents(request(sibling))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(sibling.declarations, []);
    assert!(!sibling.incomplete);
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
    let boundary = client
        .list_subject_documents(request())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(boundary.declarations, []);
    assert!(!boundary.incomplete);
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
