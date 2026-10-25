#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use pb::event_model_service_server::EventModelService;
use serde_json::json;
use sha2::{Digest, Sha256};
use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::{data_plane, mcp_router},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
    telemetry::{InflightLayer, RpcMetricsLayer},
};

struct Fixture {
    service: EventModelServiceImpl,
    namespace: String,
    expected: HashMap<pb::EntityKey, pb::Entity>,
}

fn reference(entity: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(entity).unwrap()
}

fn uid(entity: &pb::Entity) -> String {
    entity.system.as_ref().unwrap().uid.clone()
}

fn connection(entity: &pb::Entity, id: Option<pb::ConnectionId>) -> pb::ConnectionRef {
    assert!(id.is_some());
    pb::ConnectionRef {
        owner: Some(reference(entity)),
        owner_uid: uid(entity),
        id,
    }
}

fn name(value: &str) -> pb::MemberName {
    pb::MemberName {
        value: value.into(),
    }
}

fn title(value: &str) -> pb::CatalogTitle {
    pb::CatalogTitle {
        value: value.into(),
    }
}

fn text(value: &str) -> pb::CatalogText {
    pb::CatalogText {
        value: value.into(),
    }
}

fn associations(subjects: &[pb::CatalogSubject]) -> Vec<pb::SubjectAssociation> {
    subjects
        .iter()
        .map(|subject| pb::SubjectAssociation {
            subject: Some(subject.clone()),
            ..Default::default()
        })
        .collect()
}

impl Fixture {
    fn id(&self, slug: &str) -> pb::Id {
        pb::Id {
            namespace: self.namespace.clone(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn model(&self) -> pb::ReadModelRef {
        pb::ReadModelRef {
            id: Some(self.id("receipt")),
        }
    }

    fn remember(&mut self, entity: &pb::Entity) {
        let owner = reference(entity);
        self.expected.insert(
            pb::EntityKey::new(
                pb::EntityKind::try_from(owner.kind).unwrap(),
                owner.id.as_ref().unwrap(),
            ),
            entity.clone(),
        );
    }

    async fn put(&mut self, kind: pb::entity::Kind) -> pb::Entity {
        let proposed = pb::Entity {
            kind: Some(kind),
            ..Default::default()
        };
        let owner = reference(&proposed);
        let key = pb::EntityKey::new(
            pb::EntityKind::try_from(owner.kind).unwrap(),
            owner.id.as_ref().unwrap(),
        );
        let if_match = if self.expected.contains_key(&key) {
            self.get(&proposed).await.etag
        } else {
            String::new()
        };
        let response = self
            .service
            .put_entity(Request::new(pb::PutEntityRequest {
                entity: Some(proposed),
                if_match,
                ..Default::default()
            }))
            .await
            .unwrap_or_else(|error| panic!("fixture put failed for {owner:?}: {error}"))
            .into_inner();
        let entity = response.entity.unwrap();
        self.remember(&entity);
        entity
    }

    async fn get(&self, entity: &pb::Entity) -> pb::GetEntityResponse {
        let owner = reference(entity);
        self.service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: owner.kind,
                id: owner.id,
            }))
            .await
            .unwrap()
            .into_inner()
    }

    async fn assert_bidirectional_references(&self, required: &[(&pb::Entity, Vec<&pb::Entity>)]) {
        assert_eq!(required.len(), self.expected.len());
        for (entity, targets) in required {
            let owner = reference(entity);
            let outgoing = self
                .service
                .get_outgoing_references(Request::new(pb::GetReferencesRequest {
                    kind: owner.kind,
                    id: owner.id.clone(),
                    ..Default::default()
                }))
                .await
                .unwrap()
                .into_inner();
            assert_eq!(outgoing.next_page_token, "");
            assert_eq!(
                outgoing.references.len(),
                targets.len(),
                "reference count for {owner:?}"
            );
            assert_eq!(
                outgoing
                    .references
                    .iter()
                    .map(|edge| &edge.from_field)
                    .collect::<HashSet<_>>()
                    .len(),
                targets.len()
            );
            for target in targets {
                let target = reference(target);
                assert_eq!(
                    outgoing
                        .references
                        .iter()
                        .filter(|edge| edge.to.as_ref() == Some(&target))
                        .count(),
                    targets
                        .iter()
                        .filter(|expected| reference(expected) == target)
                        .count(),
                    "missing authored endpoint {owner:?} -> {target:?}"
                );
            }
            for edge in &outgoing.references {
                assert_eq!(edge.from.as_ref(), Some(&owner));
                assert_eq!(edge.from_uid, uid(entity));
                let target = edge.to.as_ref().unwrap();
                let incoming = self
                    .service
                    .get_incoming_references(Request::new(pb::GetReferencesRequest {
                        kind: target.kind,
                        id: target.id.clone(),
                        ..Default::default()
                    }))
                    .await
                    .unwrap()
                    .into_inner();
                assert_eq!(incoming.next_page_token, "");
                assert!(
                    incoming.references.contains(edge),
                    "missing reverse reference {edge:?}"
                );
            }
        }
    }

    async fn manifests(&self) -> String {
        let exported = self
            .service
            .export_event_model_manifests(Request::new(pb::ExportEventModelManifestsRequest {
                id: Some(self.id("model")),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!exported.truncated);
        assert_eq!(exported.skipped, Vec::<pb::SkippedEntity>::new());
        assert_eq!(exported.external_manifests, "");
        exported.manifests
    }

    async fn assert_roundtrip(&self) {
        let manifests = trogon_atlas_client::manifest::parse_str(
            &self.manifests().await,
            "connected-architecture-export",
        )
        .unwrap();
        assert_eq!(manifests.len(), self.expected.len());
        for manifest in manifests {
            assert_eq!(manifest.id.namespace, self.namespace);
            assert_eq!(manifest.id.version, 1);
            let original = self
                .expected
                .get(&pb::EntityKey::new(manifest.kind, &manifest.id))
                .unwrap();
            assert_eq!(manifest.entity.kind, original.kind);
            let before = self.get(original).await;
            let response = self
                .service
                .put_entity(Request::new(pb::PutEntityRequest {
                    entity: Some(manifest.entity),
                    if_match: before.etag.clone(),
                    ..Default::default()
                }))
                .await
                .unwrap()
                .into_inner();
            assert!(response.no_op);
            assert_eq!(response.etag, before.etag);
            assert_eq!(response.entity.as_ref(), Some(original));
        }
    }
}

#[tokio::test]
async fn composed_architecture_preserves_exact_selectors_through_source_apply_and_roundtrip() {
    Box::pin(exercise()).await;
}

async fn exercise() -> Fixture {
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
    let namespace = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "connected-architecture".into(),
            parent: "fixture-owner".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let mut fixture = Fixture {
        service,
        namespace,
        expected: HashMap::new(),
    };
    let receipt = fixture
        .put(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(fixture.id("receipt")),
            title: "Receipt".into(),
            doc: "Locally maintained receipt semantics".into(),
            ..Default::default()
        }))
        .await;
    let notifier = fixture
        .put(pb::entity::Kind::Processor(pb::Processor {
            id: Some(fixture.id("notifier")),
            title: "Notifier".into(),
            ..Default::default()
        }))
        .await;
    let observed = fixture
        .put(pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(fixture.id("observe")),
            title: "Observe receipts".into(),
            source_read_models: vec![
                pb::ReadModelEdge {
                    read_model: Some(fixture.model()),
                    ..Default::default()
                };
                2
            ],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(fixture.id("notifier")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await;
    let Some(pb::entity::Kind::AutomationSlice(slice)) = &observed.kind else {
        panic!("slice")
    };
    let observations: Vec<_> = slice
        .source_read_models
        .iter()
        .map(|edge| connection(&observed, edge.connection_id.clone()))
        .collect();
    assert_ne!(observations[0], observations[1]);
    let channel = fixture
        .put(pb::entity::Kind::Channel(pb::Channel {
            id: Some(fixture.id("delivery")),
            title: "Delivery".into(),
            bindings: observations
                .iter()
                .map(|observation| pb::ChannelBinding {
                    observation: Some(observation.clone()),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }))
        .await;
    let store = fixture
        .put(pb::entity::Kind::DataStore(pb::DataStore {
            id: Some(fixture.id("warehouse")),
            title: "Warehouse".into(),
            store_class: Some(pb::StoreClass {
                kind: Some(pb::store_class::Kind::Other("warehouse".into())),
            }),
            attachments: vec![pb::ReadModelAttachment {
                read_model: Some(fixture.model()),
                ..Default::default()
            }],
            ..Default::default()
        }))
        .await;
    let specification = fixture
        .put(pb::entity::Kind::Specification(pb::Specification {
            id: Some(fixture.id("api")),
            title: "Receipt API".into(),
            messages: vec![pb::ContractMessage {
                name: Some(name("Receipt")),
                mapping: Some(pb::MessageReadModelMapping {
                    read_model: Some(fixture.model()),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }))
        .await;
    let Some(pb::entity::Kind::Specification(mut spec)) = specification.kind.clone() else {
        panic!("specification")
    };
    spec.operations.push(pb::ContractOperation {
        name: Some(name("ReadReceipt")),
        messages: vec![pb::OperationMessage {
            message_id: spec.messages[0].member_id.clone(),
            role: pb::operation_message::Role::Response as i32,
            ..Default::default()
        }],
        ..Default::default()
    });
    let specification = fixture.put(pb::entity::Kind::Specification(spec)).await;
    let Some(pb::entity::Kind::Specification(spec)) = &specification.kind else {
        panic!("specification")
    };
    let member = pb::ContractMemberRef {
        specification: Some(pb::SpecificationRef {
            id: Some(fixture.id("api")),
        }),
        owner_uid: uid(&specification),
        kind: pb::ContractMemberKind::Message as i32,
        member_id: spec.messages[0].member_id.clone(),
        expected_digest: None,
    };
    let Some(pb::entity::Kind::DataStore(storage)) = &store.kind else {
        panic!("store")
    };
    let Some(pb::entity::Kind::Channel(transport)) = &channel.kind else {
        panic!("channel")
    };
    assert_ne!(
        transport.bindings[0].connection_id,
        transport.bindings[1].connection_id
    );
    let mut access = vec![pb::OutputAccess {
        selection: Some(pb::output_access::Selection::Store(pb::StoreAccess {
            store: Some(pb::DataStoreRef {
                id: Some(fixture.id("warehouse")),
            }),
            attachment: Some(connection(
                &store,
                storage.attachments[0].connection_id.clone(),
            )),
        })),
        ..Default::default()
    }];
    access.extend(
        transport
            .bindings
            .iter()
            .zip(&observations)
            .map(|(binding, observation)| pb::OutputAccess {
                selection: Some(pb::output_access::Selection::Channel(pb::ChannelAccess {
                    channel: Some(pb::ChannelRef {
                        id: Some(fixture.id("delivery")),
                    }),
                    binding: Some(connection(&channel, binding.connection_id.clone())),
                    observation: Some(observation.clone()),
                })),
                ..Default::default()
            }),
    );
    access.push(pb::OutputAccess {
        selection: Some(pb::output_access::Selection::Operation(
            pb::OperationAccess {
                operation: Some(pb::ContractMemberRef {
                    kind: pb::ContractMemberKind::Operation as i32,
                    member_id: spec.operations[0].member_id.clone(),
                    ..member.clone()
                }),
                response: Some(connection(
                    &specification,
                    spec.operations[0].messages[0].connection_id.clone(),
                )),
                provider_exposure: None,
            },
        )),
        ..Default::default()
    });
    let product = fixture
        .put(pb::entity::Kind::DataProduct(pb::DataProduct {
            id: Some(fixture.id("receipts")),
            title: "Receipt data".into(),
            inputs: vec![pb::DataInputPort {
                name: Some(pb::PortName {
                    value: "Observed receipts".into(),
                }),
                data: Some(pb::InputDataSelection {
                    target: Some(pb::input_data_selection::Target::ReadModel(fixture.model())),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            outputs: vec![pb::DataOutputPort {
                name: Some(pb::PortName {
                    value: "Receipts".into(),
                }),
                data: Some(pb::OutputDataSelection {
                    read_model: Some(fixture.model()),
                    ..Default::default()
                }),
                access,
                ..Default::default()
            }],
            ..Default::default()
        }))
        .await;
    let Some(pb::entity::Kind::DataProduct(mut data)) = product.kind.clone() else {
        panic!("product")
    };
    data.outputs[0].derived_from = Some(pb::DeclaredLineage {
        inputs: vec![pb::LineageInput {
            input_port_id: data.inputs[0].port_id.clone(),
            ..Default::default()
        }],
        ..Default::default()
    });
    let product = fixture.put(pb::entity::Kind::DataProduct(data)).await;
    let Some(pb::entity::Kind::DataProduct(data)) = &product.kind else {
        panic!("product")
    };
    let port = pb::DataPortRef {
        product: Some(pb::DataProductRef {
            id: Some(fixture.id("receipts")),
        }),
        owner_uid: uid(&product),
        direction: pb::DataPortDirection::Output as i32,
        port_id: data.outputs[0].port_id.clone(),
    };
    let responsibilities = vec![
        pb::responsibility_subject::Selection::Connection(observations[0].clone()),
        pb::responsibility_subject::Selection::Member(member.clone()),
        pb::responsibility_subject::Selection::Port(port.clone()),
    ]
    .into_iter()
    .map(|selection| pb::Responsibility {
        kind: pb::ResponsibilityKind::Owner as i32,
        subject: Some(pb::ResponsibilitySubject {
            selection: Some(selection),
        }),
        ..Default::default()
    })
    .collect();
    let person = fixture
        .put(pb::entity::Kind::Person(pb::Person {
            id: Some(fixture.id("maintainer")),
            title: Some(pb::DirectoryDisplayName {
                value: "Receipt maintainer".into(),
            }),
            responsibilities,
            ..Default::default()
        }))
        .await;
    let subjects: Vec<_> = [
        pb::catalog_subject::Selection::Connection(observations[0].clone()),
        pb::catalog_subject::Selection::Connection(observations[1].clone()),
        pb::catalog_subject::Selection::Member(member),
        pb::catalog_subject::Selection::Port(port),
    ]
    .into_iter()
    .map(|selection| pb::CatalogSubject {
        selection: Some(selection),
    })
    .collect();
    let content = "# Receipt delivery\nDelivery remains a runtime responsibility.\n";
    let mut sha256 = String::with_capacity(64);
    for byte in Sha256::digest(content.as_bytes()) {
        write!(sha256, "{byte:02x}").unwrap();
    }
    let digest = pb::ContentDigest { sha256 };
    let document = fixture
        .put(pb::entity::Kind::Document(pb::Document {
            id: Some(fixture.id("delivery-note")),
            title: Some(title("Delivery note")),
            subjects: associations(&subjects),
            body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
                media_type: Some(pb::MediaType {
                    value: "text/markdown".into(),
                }),
                content: content.into(),
                digest: Some(digest.clone()),
            })),
            anchors: vec![pb::DocumentAnchor {
                fragment: Some(pb::AnchorFragment {
                    value: "delivery".into(),
                }),
                label: Some(title("Delivery")),
                ..Default::default()
            }],
            ..Default::default()
        }))
        .await;
    let Some(pb::entity::Kind::Document(doc)) = &document.kind else {
        panic!("document")
    };
    let pinned = pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Anchor(
            pb::DocumentAnchorRef {
                document: Some(pb::DocumentRef {
                    id: Some(fixture.id("delivery-note")),
                }),
                owner_uid: uid(&document),
                anchor_id: doc.anchors[0].anchor_id.clone(),
                expected_digest: Some(digest),
            },
        )),
    };
    let decision = fixture
        .put(pb::entity::Kind::Decision(pb::Decision {
            id: Some(fixture.id("delivery-choice")),
            title: Some(title("Receipt delivery")),
            subjects: associations(&subjects),
            status: pb::DecisionStatus::Proposed as i32,
            context: Some(text("Choose an explicit observation.")),
            rationale: Some(text("Keep both repeated observations distinct.")),
            options: vec![pb::DecisionOption {
                title: Some(title("Declared delivery")),
                description: Some(text("Use the explicitly selected observation.")),
                ..Default::default()
            }],
            documents: vec![pb::SupportingDocument {
                document: Some(pinned.clone()),
                ..Default::default()
            }],
            ..Default::default()
        }))
        .await;
    let Some(pb::entity::Kind::Decision(mut choice)) = decision.kind else {
        panic!("decision")
    };
    choice.status = pb::DecisionStatus::Accepted as i32;
    choice.selected_option = choice.options[0].option_id.clone();
    choice.consequences.push(pb::DecisionConsequence {
        kind: pb::decision_consequence::Kind::Risk as i32,
        text: Some(text("Delivery still requires runtime processing.")),
    });
    let decision = fixture.put(pb::entity::Kind::Decision(choice)).await;
    let team = fixture
        .put(pb::entity::Kind::Team(pb::Team {
            id: Some(fixture.id("receipt-team")),
            title: Some(pb::DirectoryDisplayName {
                value: "Receipt team".into(),
            }),
            members: vec![pb::TeamMember {
                person: Some(pb::PersonRef {
                    id: Some(fixture.id("maintainer")),
                }),
                ..Default::default()
            }],
            responsibilities: [&product, &decision, &document]
                .into_iter()
                .map(|entity| pb::Responsibility {
                    kind: pb::ResponsibilityKind::Steward as i32,
                    subject: Some(pb::ResponsibilitySubject {
                        selection: Some(pb::responsibility_subject::Selection::Entity(reference(
                            entity,
                        ))),
                    }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }))
        .await;
    let component = fixture
        .put(pb::entity::Kind::Component(pb::Component {
            id: Some(fixture.id("receipt-service")),
            title: "Receipt service".into(),
            members: [&notifier, &receipt, &specification, &product]
                .into_iter()
                .map(reference)
                .collect(),
            ..Default::default()
        }))
        .await;
    let model = fixture
        .put(pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(fixture.id("model")),
            title: "Connected receipt architecture".into(),
            members: fixture.expected.values().map(reference).collect(),
            ..Default::default()
        }))
        .await;

    let required = [
        (&receipt, vec![]),
        (
            &component,
            vec![&notifier, &receipt, &specification, &product],
        ),
        (&notifier, vec![]),
        (&observed, vec![&receipt, &receipt, &notifier]),
        (&channel, vec![&observed, &observed]),
        (&store, vec![&receipt]),
        (&specification, vec![&receipt]),
        (
            &product,
            vec![
                &receipt,
                &receipt,
                &store,
                &store,
                &channel,
                &channel,
                &channel,
                &channel,
                &observed,
                &observed,
                &specification,
                &specification,
            ],
        ),
        (&person, vec![&observed, &specification, &product]),
        (
            &document,
            vec![&observed, &observed, &specification, &product],
        ),
        (
            &decision,
            vec![&observed, &observed, &specification, &product, &document],
        ),
        (&team, vec![&person, &product, &decision, &document]),
        (
            &model,
            vec![
                &receipt,
                &notifier,
                &observed,
                &channel,
                &store,
                &specification,
                &product,
                &person,
                &document,
                &decision,
                &team,
                &component,
            ],
        ),
    ];
    fixture.assert_bidirectional_references(&required).await;
    fixture.assert_roundtrip().await;
    adopt_receipt_title(&mut fixture, &receipt).await;
    let Some(pb::entity::Kind::AutomationSlice(mut reordered)) = observed.kind.clone() else {
        panic!("slice")
    };
    reordered.source_read_models.reverse();
    let reordered = fixture
        .put(pb::entity::Kind::AutomationSlice(reordered))
        .await;
    assert_eq!(uid(&reordered), uid(&observed));
    let Some(pb::entity::Kind::AutomationSlice(reordered)) = reordered.kind else {
        panic!("slice")
    };
    assert_eq!(reordered.source_read_models[0], slice.source_read_models[1]);
    assert_eq!(reordered.source_read_models[1], slice.source_read_models[0]);
    let inspection = fixture
        .service
        .inspect_data_product(Request::new(pb::InspectDataProductRequest {
            product: Some(pb::DataProductRef {
                id: Some(fixture.id("receipts")),
            }),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!inspection.incomplete);
    assert_eq!(inspection.product.as_ref(), Some(data));
    let output = inspection
        .ports
        .iter()
        .find(|state| state.port.as_ref().unwrap().port_id == data.outputs[0].port_id)
        .unwrap();
    assert_eq!(output.access.len(), data.outputs[0].access.len());
    assert!(output
        .access
        .iter()
        .all(|access| access.state == pb::DataProductSelectionState::Resolved as i32));
    assert_eq!(
        output.access.last().unwrap().provider,
        pb::DataProductSelectionState::Undeclared as i32
    );
    for subject in &subjects {
        let found = fixture
            .service
            .list_subject_decisions(Request::new(pb::ListSubjectDecisionsRequest {
                subject: Some(subject.clone()),
                status: Some(pb::DecisionStatus::Accepted as i32),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!found.incomplete);
        assert_eq!(
            found.subject_state,
            pb::CatalogResolutionState::Resolved as i32
        );
        assert_eq!(found.declarations.len(), 1);
        assert_eq!(
            found.declarations[0].owner.as_ref().unwrap().uid,
            uid(&decision)
        );
        let docs = fixture
            .service
            .list_subject_documents(Request::new(pb::ListSubjectDocumentsRequest {
                subject: Some(subject.clone()),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!docs.incomplete);
        assert_eq!(
            docs.subject_state,
            pb::CatalogResolutionState::Resolved as i32
        );
        assert_eq!(docs.declarations.len(), 1);
        assert_eq!(
            docs.declarations[0].owner.as_ref().unwrap().uid,
            uid(&document)
        );
    }
    let Some(pb::entity::Kind::Person(maintainer)) = &person.kind else {
        panic!("person")
    };
    for assignment in &maintainer.responsibilities {
        let found = fixture
            .service
            .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
                subject: assignment.subject.clone(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!found.incomplete);
        assert_eq!(
            found.subject_state,
            pb::ResponsibilityResolutionState::Resolved as i32
        );
        assert_eq!(found.assignments.len(), 1);
        assert_eq!(found.assignments[0].assignee_uid, uid(&person));
        assert_eq!(found.assignments[0].assignment.as_ref(), Some(assignment));
    }
    let sibling = fixture
        .service
        .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
            subject: Some(pb::ResponsibilitySubject {
                selection: Some(pb::responsibility_subject::Selection::Connection(
                    observations[1].clone(),
                )),
            }),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!sibling.incomplete);
    assert_eq!(
        sibling.assignments,
        Vec::<pb::ResponsibilityAssignment>::new()
    );
    let memberships = fixture
        .service
        .list_person_teams(Request::new(pb::ListPersonTeamsRequest {
            person: Some(pb::PersonRef {
                id: Some(fixture.id("maintainer")),
            }),
        }))
        .await
        .unwrap()
        .into_inner();
    let Some(pb::entity::Kind::Team(team_value)) = &team.kind else {
        panic!("team")
    };
    assert_eq!(memberships.memberships.len(), 1);
    assert_eq!(memberships.memberships[0].team_uid, uid(&team));
    assert_eq!(
        memberships.memberships[0].membership.as_ref(),
        Some(&team_value.members[0])
    );
    let detail = fixture
        .service
        .get_document_detail(Request::new(pb::GetDocumentDetailRequest {
            document: Some(pinned.clone()),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        detail.selection_state,
        pb::CatalogResolutionState::Resolved as i32
    );
    assert_eq!(detail.anchor.as_ref(), doc.anchors.first());
    let incoming = fixture
        .service
        .list_incoming_document_declarations(Request::new(
            pb::ListIncomingDocumentDeclarationsRequest {
                document: Some(pinned),
            },
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!incoming.incomplete);
    assert_eq!(incoming.declarations.len(), 1);
    assert_eq!(
        incoming.declarations[0].owner.as_ref().unwrap().uid,
        uid(&decision)
    );
    fixture.assert_bidirectional_references(&required).await;
    fixture.assert_roundtrip().await;
    fixture
}

async fn adopt_receipt_title(fixture: &mut Fixture, receipt: &pb::Entity) {
    let original = fixture.get(receipt).await;
    let registration = fixture
        .service
        .create_source_registration(Request::new(pb::CreateSourceRegistrationRequest {
            registration: Some(pb::SourceRegistration {
                id: "receipt-source".into(),
                administration_namespace: fixture.namespace.clone(),
                destination_namespaces: vec![fixture.namespace.clone()],
                profile_revision: 1,
                ownership_policy_revision: "title-only".into(),
                deletion_policy: pb::SourceDeletionPolicy::RetainNative as i32,
                ..Default::default()
            }),
        }))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap();
    let inventory = serde_json::to_vec(&json!({
        "profile": "supplied-native-v1", "profile_revision": 1,
        "registration": registration.id, "source_version": "00.RC-A",
        "partition": {"categories": ["model"], "logical_roots": ["native"], "selected_keys": []},
        "coverage": "PARTIAL",
        "resources": [{"key": {"category": "model", "key": "receipt"}, "file": "native/model.yaml", "pointer": {"kind": "native", "raw": "#/receipt"}}],
        "relations": []
    })).unwrap();
    let bytes = fixture.manifests().await.into_bytes();
    let snapshot = fixture
        .service
        .accept_source_snapshot(Request::new(pb::AcceptSourceSnapshotRequest {
            registration_id: registration.id.clone(),
            registration_revision: registration.revision.clone(),
            inventory_bytes: inventory.clone(),
            files: vec![pb::SourceFile {
                logical_name: "native/model.yaml".into(),
                media_type: "application/yaml".into(),
                content: bytes.clone(),
                ..Default::default()
            }],
        }))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap();
    let source = pb::SourceResourceKey {
        registration_id: registration.id.clone(),
        category: "model".into(),
        key: "receipt".into(),
    };
    fixture
        .service
        .create_source_mapping(Request::new(pb::CreateSourceMappingRequest {
            registration_revision: registration.revision.clone(),
            mapping: Some(pb::SourceMapping {
                id: "receipt-title".into(),
                source: Some(source.clone()),
                selection: Some(pb::SourceSelection {
                    selection: Some(pb::source_selection::Selection::Entity(reference(receipt))),
                }),
                ownership: Some(pb::SourceFieldOwnership {
                    policy: Some(pb::source_field_ownership::Policy::Fields(
                        pb::SourceOwnedFields {
                            fields: vec![pb::SourceOwnedField::Title as i32],
                        },
                    )),
                }),
                adoption: Some(pb::SourceAdoption {
                    observed: Some(pb::SourceObservedOwner {
                        owner: Some(reference(receipt)),
                        uid: uid(receipt),
                        etag: original.etag.clone(),
                    }),
                    authorized_adoption: true,
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap();
    let mut payload = receipt.clone();
    let Some(pb::entity::Kind::ReadModel(model)) = &mut payload.kind else {
        panic!("read model")
    };
    model.title = "Source receipt".into();
    model.doc = "Unowned source documentation".into();
    let preview = fixture
        .service
        .preview_source_run(Request::new(pb::PreviewSourceRunRequest {
            registration_id: registration.id.clone(),
            registration_revision: registration.revision.clone(),
            snapshot_id: snapshot.id.clone(),
            run_id: "receipt-title-update".into(),
            native_payloads: vec![pb::SourceNativePayload {
                source: Some(source),
                entity: Some(payload),
            }],
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(preview.status, pb::SourceRunStatus::Previewed as i32);
    assert_eq!(fixture.get(receipt).await.etag, original.etag);
    let request = pb::ApplySourceRunRequest {
        registration_id: registration.id.clone(),
        run_id: preview.id.clone(),
        if_match: preview.revision,
        preview_digest: preview.preview_digest,
    };
    let applied = fixture
        .service
        .apply_source_run(Request::new(request.clone()))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(applied.status, pb::SourceRunStatus::Applied as i32);
    let current = fixture.get(receipt).await;
    let mut expected = receipt.kind.clone().unwrap();
    let pb::entity::Kind::ReadModel(model) = &mut expected else {
        panic!("read model")
    };
    model.title = "Source receipt".into();
    let stored = current.entity.as_ref().unwrap();
    assert_eq!(stored.kind.as_ref(), Some(&expected));
    assert_eq!(uid(stored), uid(receipt));
    assert_ne!(current.etag, original.etag);
    fixture.remember(stored);
    for entity in fixture.expected.values() {
        assert_eq!(fixture.get(entity).await.entity.as_ref(), Some(entity));
    }
    let replay = fixture
        .service
        .apply_source_run(Request::new(request))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(replay.status, pb::SourceRunStatus::Applied as i32);
    assert_eq!(fixture.get(receipt).await.etag, current.etag);
    let retained = fixture
        .service
        .get_source_snapshot(Request::new(pb::GetSourceSnapshotRequest {
            registration_id: registration.id,
            snapshot_id: snapshot.id,
            include_bytes: true,
        }))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap();
    assert_eq!(retained.source_version, "00.RC-A");
    assert_eq!(retained.bundle_digest, snapshot.bundle_digest);
    assert_eq!(retained.inventory_bytes, inventory);
    assert_eq!(retained.files[0].content, bytes);
}

fn port(variable: &str, default: u16) -> u16 {
    std::env::var(variable).map_or(default, |value| {
        value.parse().expect("fixture port must be a u16")
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual same-graph client acceptance; serves loopback gRPC and MCP for at most ten minutes"]
async fn serve_connected_architecture_fixture() {
    let fixture = Box::pin(exercise()).await;
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "fixture-admin": {"role": "admin", "tokens": ["connected-fixture-admin"]}
        }}))
        .unwrap(),
    )
    .unwrap();
    let grpc_listener = tokio::net::TcpListener::bind((
        std::net::Ipv4Addr::LOCALHOST,
        port("ATLAS_CONNECTED_FIXTURE_GRPC_PORT", 19473),
    ))
    .await
    .unwrap();
    let mcp_listener = tokio::net::TcpListener::bind((
        std::net::Ipv4Addr::LOCALHOST,
        port("ATLAS_CONNECTED_FIXTURE_MCP_PORT", 19474),
    ))
    .await
    .unwrap();
    let grpc_address = grpc_listener.local_addr().unwrap();
    let mcp_address = mcp_listener.local_addr().unwrap();
    let namespace = fixture.namespace.clone();
    let receipt_id = fixture.id("receipt");
    let expected = fixture
        .expected
        .get(&pb::EntityKey::new(pb::EntityKind::ReadModel, &receipt_id))
        .unwrap()
        .clone();
    let service = pb::event_model_service_server::EventModelServiceServer::new(fixture.service)
        .max_decoding_message_size(36 * 1024 * 1024)
        .max_encoding_message_size(36 * 1024 * 1024);
    let grpc = data_plane(service.clone(), BearerAuth::new(registry.clone()));
    let (drain, _) = tokio::sync::watch::channel(0i64);
    let app = mcp_router(
        data_plane(service, BearerAuth::new(registry)),
        RpcMetricsLayer::new(ServerMetrics::default()),
        InflightLayer::new(Arc::new(AtomicI64::new(0)), Arc::new(drain)),
        Some(Duration::from_secs(30)),
        36 * 1024 * 1024,
        "127.0.0.1".parse().unwrap(),
    )
    .unwrap();
    let seconds = std::env::var("ATLAS_CONNECTED_FIXTURE_SECONDS")
        .map_or(600, |value| {
            value
                .parse::<u64>()
                .expect("fixture lifetime must be seconds")
        })
        .clamp(1, 600);
    let mut servers = tokio::task::JoinSet::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    servers.spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(
                grpc_listener,
            ))
            .await
            .unwrap();
    });
    servers.spawn(async move {
        axum::serve(mcp_listener, app).await.unwrap();
    });
    let grpc_endpoint = format!("http://{grpc_address}");
    let mut request = Request::new(pb::GetEntityRequest {
        kind: pb::EntityKind::ReadModel as i32,
        id: Some(receipt_id),
    });
    request.metadata_mut().insert(
        "authorization",
        tonic::metadata::MetadataValue::from_static("Bearer connected-fixture-admin"),
    );
    let startup = tokio::time::timeout_at(deadline, async {
        let mut client =
            pb::event_model_service_client::EventModelServiceClient::connect(grpc_endpoint.clone())
                .await
                .map_err(|error| format!("fixture connection failed: {error}"))?;
        let stored = client
            .get_entity(request)
            .await
            .map_err(|error| format!("fixture startup read failed: {error}"))?
            .into_inner()
            .entity;
        if stored.as_ref() != Some(&expected) {
            return Err("fixture startup read did not match the exercised receipt".to_owned());
        }
        Ok(())
    })
    .await;
    let outcome = match startup {
        Ok(Ok(())) => {
            println!(
                "READY grpc={grpc_endpoint} mcp=http://{mcp_address}/mcp namespace={namespace} model=model version=1 component=receipt-service token=connected-fixture-admin seconds={seconds}"
            );
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => Ok(()),
                result = servers.join_next() => Err(format!("fixture server exited before deadline: {result:?}")),
                result = tokio::signal::ctrl_c() => result.map_err(|error| error.to_string()),
            }
        }
        Ok(Err(error)) => Err(error),
        Err(error) => Err(format!("fixture startup deadline elapsed: {error}")),
    };
    servers.shutdown().await;
    outcome.unwrap_or_else(|error| panic!("{error}"));
}
