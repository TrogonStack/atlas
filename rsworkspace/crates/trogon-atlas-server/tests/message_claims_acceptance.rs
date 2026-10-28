#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use std::sync::Arc;

use support::two_tenants::{authed, make_registry, serve_with_registry, ROOT_TOKEN};
use tonic::{transport::Channel, Code};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};
type Client = pb::event_model_service_client::EventModelServiceClient<Channel>;
fn id(ns: &str, s: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: s.into(),
        version: 1,
    }
}
fn owner(ns: &str, kind: pb::EntityKind, message: pb::ContractMemberRef) -> pb::Entity {
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
                id: Some(id(ns, "owner")),
                title: "Owner".into(),
                message_claims: claims,
                ..Default::default()
            }),
            pb::EntityKind::ExternalSystem => {
                pb::entity::Kind::ExternalSystem(pb::ExternalSystem {
                    id: Some(id(ns, "owner")),
                    title: "Owner".into(),
                    message_claims: claims,
                    ..Default::default()
                })
            }
            _ => pb::entity::Kind::Domain(pb::Domain {
                id: Some(id(ns, "owner")),
                title: "Owner".into(),
                message_claims: claims,
                ..Default::default()
            }),
        }),
        ..Default::default()
    }
}
async fn fixture() -> (Client, String) {
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
    let mut client = serve_with_registry(service, make_registry()).await;
    let ns = client
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
    (client, ns)
}
async fn put(
    client: &mut Client,
    e: pb::Entity,
    etag: &str,
) -> Result<pb::PutEntityResponse, tonic::Status> {
    client
        .put_entity(authed(
            pb::PutEntityRequest {
                entity: Some(e),
                if_match: etag.into(),
                ..Default::default()
            },
            ROOT_TOKEN,
        ))
        .await
        .map(tonic::Response::into_inner)
}
fn unavailable_message(ns: &str) -> pb::ContractMemberRef {
    pb::ContractMemberRef {
        specification: Some(pb::SpecificationRef {
            id: Some(id(ns, "missing")),
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
    let (mut c, ns) = fixture().await;
    for kind in [
        pb::EntityKind::Component,
        pb::EntityKind::ExternalSystem,
        pb::EntityKind::Domain,
    ] {
        let result = put(&mut c, owner(&ns, kind, unavailable_message(&ns)), "").await;
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
fn reference(e: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(e).unwrap()
}
async fn targets(c: &mut Client, ns: &str) -> (pb::Entity, pb::ContractMemberRef, pb::Entity) {
    let schema = put(
        c,
        entity(pb::entity::Kind::Schema(pb::Schema {
            id: Some(id(ns, "payload")),
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
        c,
        entity(pb::entity::Kind::Specification(pb::Specification {
            id: Some(id(ns, "messages")),
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
    let Some(pb::entity::Kind::Specification(s)) = &spec.kind else {
        panic!()
    };
    let m = pb::ContractMemberRef {
        specification: Some(pb::SpecificationRef { id: s.id.clone() }),
        owner_uid: spec.system.as_ref().unwrap().uid.clone(),
        kind: pb::ContractMemberKind::Message as i32,
        member_id: s.messages[0].member_id.clone(),
        expected_digest: None,
    };
    let model = put(
        c,
        entity(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id(ns, "observed")),
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
        c,
        entity(pb::entity::Kind::Processor(pb::Processor {
            id: Some(id(ns, "processor")),
            title: "Processor".into(),
            ..Default::default()
        })),
        "",
    )
    .await
    .unwrap();
    let slice = put(
        c,
        entity(pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id(ns, "slice")),
            title: "Slice".into(),
            source_read_models: vec![pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: reference(&model).id,
                }),
                ..Default::default()
            }],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(id(ns, "processor")),
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
    let Some(pb::entity::Kind::AutomationSlice(s)) = &slice.kind else {
        panic!()
    };
    let channel = put(
        c,
        entity(pb::entity::Kind::Channel(pb::Channel {
            id: Some(id(ns, "channel")),
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
                parameter_values: [("tenant".into(), "".into())].into_iter().collect(),
                observation: Some(pb::ConnectionRef {
                    owner: Some(reference(&slice)),
                    owner_uid: slice.system.as_ref().unwrap().uid.clone(),
                    id: s.source_read_models[0].connection_id.clone(),
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
    (spec, m, channel)
}
#[tokio::test(flavor = "multi_thread")]
async fn authenticated_native_claim_authoring_keeps_flat_identity_and_exact_field_validation() {
    use trogon_atlas_core::message_claims as claims;
    let (mut c, ns) = fixture().await;
    let (spec, m, channel) = targets(&mut c, &ns).await;
    let Some(pb::entity::Kind::Specification(s)) = &spec.kind else {
        panic!()
    };
    for kind in [
        pb::EntityKind::Component,
        pb::EntityKind::ExternalSystem,
        pb::EntityKind::Domain,
    ] {
        let mut proposed = owner(&ns, kind, m.clone());
        let v = claims::declarations_mut(&mut proposed).unwrap();
        v.claims.push(v.claims[0].clone());
        let first = put(&mut c, proposed, "").await.unwrap();
        let first_entity = first.entity.unwrap();
        let mut next = first_entity.clone();
        let v = claims::declarations_mut(&mut next).unwrap();
        assert_ne!(v.claims[0].connection_id, v.claims[1].connection_id);
        v.routes.push(pb::ClaimRoute {
            claim_id: v.claims[0].connection_id.clone(),
            channel: Some(pb::ChannelRef {
                id: reference(&channel).id,
            }),
            ..Default::default()
        });
        assert_eq!(
            put(&mut c, next.clone(), &first.etag)
                .await
                .unwrap_err()
                .code(),
            Code::InvalidArgument
        );
        let v = claims::declarations_mut(&mut next).unwrap();
        v.routes[0]
            .parameter_values
            .insert("tenant".into(), "".into());
        let mut triggered = m.clone();
        triggered.member_id = s.messages[1].member_id.clone();
        v.triggers.push(pb::ClaimTrigger {
            claim_id: v.claims[0].connection_id.clone(),
            message: Some(triggered),
            ..Default::default()
        });
        v.field_selections.push(pb::ClaimFieldSelection {
            claim_id: v.claims[0].connection_id.clone(),
            binding: Some(pb::ConnectionRef {
                owner: Some(reference(&spec)),
                owner_uid: spec.system.as_ref().unwrap().uid.clone(),
                id: s.messages[0].schemas[0].connection_id.clone(),
            }),
            selectors: vec![pb::ClaimFieldSelector {
                selector: Some(pb::claim_field_selector::Selector::Path(pb::FieldPath {
                    segments: vec!["name".into()],
                })),
            }],
            ..Default::default()
        });
        let valid = put(&mut c, next, &first.etag).await.unwrap();
        let e = valid.entity.unwrap();
        let v = claims::declarations(&e).unwrap();
        assert_eq!(v.routes[0].delivery_mode, None);
        let refs = c
            .get_outgoing_references(authed(
                pb::GetReferencesRequest {
                    kind: kind as i32,
                    id: reference(&e).id,
                    ..Default::default()
                },
                ROOT_TOKEN,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(refs.references.len(), 5);
        assert!(refs
            .references
            .iter()
            .all(|r| r.connection_id.is_some() && r.from_uid == e.system.as_ref().unwrap().uid));
        assert_eq!(
            refs.references
                .iter()
                .find(|r| r.from_field == "message_claims.routes[0].channel")
                .unwrap()
                .connection_id,
            v.routes[0].connection_id
        );
        let mut wrong = e.clone();
        claims::declarations_mut(&mut wrong)
            .unwrap()
            .field_selections[0]
            .selectors[0]
            .selector = Some(pb::claim_field_selector::Selector::Path(pb::FieldPath {
            segments: vec!["missing".into()],
        }));
        assert_eq!(
            put(&mut c, wrong, &valid.etag).await.unwrap_err().code(),
            Code::InvalidArgument
        );
        let mut wrong = e.clone();
        let v = claims::declarations_mut(&mut wrong).unwrap();
        v.field_selections[0].binding.as_mut().unwrap().id =
            s.messages[1].schemas[0].connection_id.clone();
        v.field_selections[0].connection_id = None;
        assert_eq!(
            put(&mut c, wrong, &valid.etag).await.unwrap_err().code(),
            Code::InvalidArgument
        );
        let mut opaque = e.clone();
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
        put(&mut c, opaque, &valid.etag).await.unwrap();
    }
}

#[tokio::test(flavor="multi_thread")]
async fn selected_native_fields_become_unavailable_without_rebinding_or_vetoing_title_edits(){
 use trogon_atlas_core::message_claims as claims;
 let (mut c,ns)=fixture().await;let (spec,m,_)=targets(&mut c,&ns).await;let Some(pb::entity::Kind::Specification(s))=&spec.kind else{panic!()};
 let first=put(&mut c,owner(&ns,pb::EntityKind::Domain,m),"").await.unwrap();let mut e=first.entity.unwrap();let v=claims::declarations_mut(&mut e).unwrap();v.field_selections.push(pb::ClaimFieldSelection{claim_id:v.claims[0].connection_id.clone(),binding:Some(pb::ConnectionRef{owner:Some(reference(&spec)),owner_uid:spec.system.as_ref().unwrap().uid.clone(),id:s.messages[0].schemas[0].connection_id.clone()}),selectors:vec![pb::ClaimFieldSelector{selector:Some(pb::claim_field_selector::Selector::Path(pb::FieldPath{segments:vec!["name".into()]}))}],..Default::default()});
 let stored=put(&mut c,e,&first.etag).await.unwrap();let e=stored.entity.unwrap();let selected=pb::ConnectionRef{owner:Some(reference(&e)),owner_uid:e.system.as_ref().unwrap().uid.clone(),id:claims::declarations(&e).unwrap().field_selections[0].connection_id.clone()};
 let request=||authed(pb::ListSubjectAssignmentsRequest{subject:Some(pb::ResponsibilitySubject{selection:Some(pb::responsibility_subject::Selection::Connection(selected.clone()))}),..Default::default()},ROOT_TOKEN);
 assert_eq!(c.list_subject_assignments(request()).await.unwrap().into_inner().subject_state,pb::ResponsibilityResolutionState::Resolved as i32);
 let schema=c.get_entity(authed(pb::GetEntityRequest{kind:pb::EntityKind::Schema as i32,id:Some(id(&ns,"payload"))},ROOT_TOKEN)).await.unwrap().into_inner();let mut changed=schema.entity.unwrap();if let Some(pb::entity::Kind::Schema(s))=&mut changed.kind{s.fields.clear();}put(&mut c,changed,&schema.etag).await.unwrap();
 assert_eq!(c.list_subject_assignments(request()).await.unwrap().into_inner().subject_state,pb::ResponsibilityResolutionState::Unresolved as i32);
 let mut title=e.clone();if let Some(pb::entity::Kind::Domain(d))=&mut title.kind{d.title="Unrelated title".into();}let title=put(&mut c,title,&stored.etag).await.unwrap();assert_eq!(claims::declarations(title.entity.as_ref().unwrap()),claims::declarations(&e));
}
