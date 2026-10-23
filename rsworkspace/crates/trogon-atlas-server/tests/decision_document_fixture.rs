#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{fmt::Write as _, sync::Arc};

use pb::event_model_service_server::EventModelService;
use sha2::{Digest, Sha256};
use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "responsibility-fixture".into(),
        slug: slug.into(),
        version: 1,
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

fn reference(entity: &pb::Entity) -> pb::EntityRef {
    trogon_atlas_server::conv::entity_to_ref(entity).unwrap()
}

fn uid(entity: &pb::Entity) -> String {
    entity.system.as_ref().unwrap().uid.clone()
}

fn subject(selection: pb::catalog_subject::Selection) -> pb::CatalogSubject {
    pb::CatalogSubject {
        selection: Some(selection),
    }
}

fn associations(subjects: &[pb::CatalogSubject]) -> Vec<pb::SubjectAssociation> {
    subjects
        .iter()
        .map(|value| pb::SubjectAssociation {
            subject: Some(value.clone()),
            ..Default::default()
        })
        .collect()
}

fn retained(content: &str) -> pb::RetainedDocumentText {
    let mut digest = String::with_capacity(64);
    for byte in Sha256::digest(content.as_bytes()) {
        write!(digest, "{byte:02x}").unwrap();
    }
    pb::RetainedDocumentText {
        media_type: Some(pb::MediaType {
            value: "text/markdown".into(),
        }),
        content: content.into(),
        digest: Some(pb::ContentDigest { sha256: digest }),
    }
}

async fn put(service: &EventModelServiceImpl, kind: pb::entity::Kind) -> pb::PutEntityResponse {
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
}

async fn native_subjects(service: &EventModelServiceImpl) -> Vec<pb::CatalogSubject> {
    let receipt = put(
        service,
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id("receipt")),
            title: "Receipt".into(),
            ..Default::default()
        }),
    )
    .await
    .entity
    .unwrap();
    put(
        service,
        pb::entity::Kind::Processor(pb::Processor {
            id: Some(id("notifier")),
            title: "Notifier".into(),
            ..Default::default()
        }),
    )
    .await;
    let model_ref = || pb::ReadModelRef {
        id: Some(id("receipt")),
    };
    let observed = put(
        service,
        pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id("observe")),
            title: "Observe receipt".into(),
            source_read_models: (0..2)
                .map(|_| pb::ReadModelEdge {
                    read_model: Some(model_ref()),
                    ..Default::default()
                })
                .collect(),
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(id("notifier")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await
    .entity
    .unwrap();
    let specification = put(
        service,
        pb::entity::Kind::Specification(pb::Specification {
            id: Some(id("receipt-contract")),
            title: "Receipt contract".into(),
            messages: vec![pb::ContractMessage {
                name: Some(pb::MemberName {
                    value: "Receipt".into(),
                }),
                mapping: Some(pb::MessageReadModelMapping {
                    read_model: Some(model_ref()),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await
    .entity
    .unwrap();
    let product = put(
        service,
        pb::entity::Kind::DataProduct(pb::DataProduct {
            id: Some(id("receipts")),
            title: "Receipts".into(),
            outputs: vec![pb::DataOutputPort {
                name: Some(pb::PortName {
                    value: "Receipts".into(),
                }),
                data: Some(pb::OutputDataSelection {
                    read_model: Some(model_ref()),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await
    .entity
    .unwrap();
    let Some(pb::entity::Kind::AutomationSlice(slice)) = &observed.kind else {
        panic!("slice")
    };
    let Some(pb::entity::Kind::Specification(spec)) = &specification.kind else {
        panic!("specification")
    };
    let Some(pb::entity::Kind::DataProduct(data)) = &product.kind else {
        panic!("product")
    };
    let mut subjects = vec![subject(pb::catalog_subject::Selection::Entity(reference(
        &receipt,
    )))];
    for edge in &slice.source_read_models {
        subjects.push(subject(pb::catalog_subject::Selection::Connection(
            pb::ConnectionRef {
                owner: Some(reference(&observed)),
                owner_uid: uid(&observed),
                id: edge.connection_id.clone(),
            },
        )));
    }
    subjects.push(subject(pb::catalog_subject::Selection::Member(
        pb::ContractMemberRef {
            specification: Some(pb::SpecificationRef {
                id: Some(id("receipt-contract")),
            }),
            owner_uid: uid(&specification),
            kind: pb::ContractMemberKind::Message as i32,
            member_id: spec.messages[0].member_id.clone(),
            expected_digest: None,
        },
    )));
    subjects.push(subject(pb::catalog_subject::Selection::Port(
        pb::DataPortRef {
            product: Some(pb::DataProductRef {
                id: Some(id("receipts")),
            }),
            owner_uid: uid(&product),
            direction: pb::DataPortDirection::Output as i32,
            port_id: data.outputs[0].port_id.clone(),
        },
    )));
    assert_ne!(subjects[1], subjects[2]);
    subjects
}

async fn adopt(
    service: &EventModelServiceImpl,
    subjects: &[pb::CatalogSubject],
    documents: Vec<pb::SupportingDocument>,
) -> pb::Entity {
    let proposed = put(
        service,
        pb::entity::Kind::Decision(pb::Decision {
            id: Some(id("receipt-delivery-choice")),
            title: Some(title("Receipt delivery")),
            subjects: associations(subjects),
            status: pb::DecisionStatus::Proposed as i32,
            context: Some(text("Choose how the receipt is delivered.")),
            rationale: Some(text(
                "A declared observation preserves the integration boundary.",
            )),
            options: vec![pb::DecisionOption {
                title: Some(title("Observe receipt")),
                description: Some(text("Use the existing processor observation.")),
                ..Default::default()
            }],
            documents,
            ..Default::default()
        }),
    )
    .await;
    let mut entity = proposed.entity.unwrap();
    let Some(pb::entity::Kind::Decision(decision)) = &mut entity.kind else {
        panic!("decision")
    };
    decision.status = pb::DecisionStatus::Accepted as i32;
    decision.selected_option = decision.options[0].option_id.clone();
    decision.consequences.push(pb::DecisionConsequence {
        kind: pb::decision_consequence::Kind::Risk as i32,
        text: Some(text("Delivery still depends on runtime processing.")),
    });
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity),
            if_match: proposed.etag,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

#[tokio::test]
async fn decisions_and_documents_preserve_connected_subjects_and_byte_pins() {
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap()
    .with_snapshot_cap(0);
    let subjects = native_subjects(&service).await;
    let document = put(
        &service,
        pb::entity::Kind::Document(pb::Document {
            id: Some(id("receipt-delivery-note")),
            title: Some(title("Receipt delivery note")),
            subjects: associations(&subjects),
            body: Some(pb::document::Body::Retained(retained(
                "# Receipt delivery\n",
            ))),
            anchors: vec![pb::DocumentAnchor {
                fragment: Some(pb::AnchorFragment {
                    value: "delivery".into(),
                }),
                label: Some(title("Delivery")),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await
    .entity
    .unwrap();
    let Some(pb::entity::Kind::Document(value)) = &document.kind else {
        panic!("document")
    };
    let Some(pb::document::Body::Retained(body)) = &value.body else {
        panic!("retained")
    };
    let whole = pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Document(
            pb::DocumentRef {
                id: Some(id("receipt-delivery-note")),
            },
        )),
    };
    let pinned = pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Anchor(
            pb::DocumentAnchorRef {
                document: Some(pb::DocumentRef {
                    id: Some(id("receipt-delivery-note")),
                }),
                owner_uid: uid(&document),
                anchor_id: value.anchors[0].anchor_id.clone(),
                expected_digest: body.digest.clone(),
            },
        )),
    };
    let decision = adopt(
        &service,
        &subjects,
        [whole.clone(), pinned.clone()]
            .into_iter()
            .map(|selection| pb::SupportingDocument {
                document: Some(selection),
                ..Default::default()
            })
            .collect(),
    )
    .await;
    let team = put(
        &service,
        pb::entity::Kind::Team(pb::Team {
            id: Some(id("receipts-team")),
            title: Some(pb::DirectoryDisplayName {
                value: "Receipts team".into(),
            }),
            responsibilities: [
                (&decision, pb::ResponsibilityKind::Owner),
                (&document, pb::ResponsibilityKind::Steward),
            ]
            .into_iter()
            .map(|(entity, kind)| pb::Responsibility {
                kind: kind as i32,
                subject: Some(pb::ResponsibilitySubject {
                    selection: Some(pb::responsibility_subject::Selection::Entity(reference(
                        entity,
                    ))),
                }),
                ..Default::default()
            })
            .collect(),
            ..Default::default()
        }),
    )
    .await
    .entity
    .unwrap();
    for chosen in &subjects {
        let decisions = service
            .list_subject_decisions(Request::new(pb::ListSubjectDecisionsRequest {
                subject: Some(chosen.clone()),
                status: Some(pb::DecisionStatus::Accepted as i32),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!decisions.incomplete);
        assert_eq!(
            decisions.subject_state,
            pb::CatalogResolutionState::Resolved as i32
        );
        assert_eq!(decisions.declarations.len(), 1);
        assert_eq!(
            decisions.declarations[0].owner.as_ref().unwrap().uid,
            uid(&decision)
        );
        let Some(pb::catalog_declaration::Declaration::Subject(association)) =
            &decisions.declarations[0].declaration
        else {
            panic!("subject association")
        };
        assert_eq!(association.subject.as_ref(), Some(chosen));
        assert!(association.connection_id.is_some());
        let scope = decisions.scope.unwrap();
        assert_ne!(scope.revision, "");
        assert!(scope.visited_subjects.contains(chosen));
        let documents = service
            .list_subject_documents(Request::new(pb::ListSubjectDocumentsRequest {
                subject: Some(chosen.clone()),
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!documents.incomplete);
        assert_eq!(documents.declarations.len(), 1);
        assert_eq!(
            documents.declarations[0].owner.as_ref().unwrap().uid,
            uid(&document)
        );
    }
    for selection in [&whole, &pinned] {
        let incoming = service
            .list_incoming_document_declarations(Request::new(
                pb::ListIncomingDocumentDeclarationsRequest {
                    document: Some(selection.clone()),
                },
            ))
            .await
            .unwrap()
            .into_inner();
        assert!(!incoming.incomplete);
        assert_eq!(incoming.declarations.len(), 1);
        let Some(pb::catalog_declaration::Declaration::Supporting(supporting)) =
            &incoming.declarations[0].declaration
        else {
            panic!("supporting document")
        };
        assert_eq!(supporting.document.as_ref(), Some(selection));
    }
    let detail = service
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
    assert_eq!(detail.anchor.as_ref(), value.anchors.first());
    for entity in [&decision, &document] {
        let assignments = service
            .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
                subject: Some(pb::ResponsibilitySubject {
                    selection: Some(pb::responsibility_subject::Selection::Entity(reference(
                        entity,
                    ))),
                }),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(assignments.assignments.len(), 1);
        assert_eq!(assignments.assignments[0].assignee_uid, uid(&team));
    }
    let incoming = service
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Document as i32,
            id: Some(id("receipt-delivery-note")),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(incoming
        .references
        .iter()
        .any(|hit| hit.document_selection.as_ref() == Some(&pinned)
            && hit.from_uid == uid(&decision)
            && hit.connection_id.is_some()));
    let impact = service
        .get_impact(Request::new(pb::GetImpactRequest {
            root: Some(reference(&document)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(impact
        .nodes
        .iter()
        .any(|node| node.entity.as_ref() == Some(&reference(&decision))
            && node.path.iter().any(|hit| hit.document_selection.is_some())));
    let current = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::AutomationSlice as i32,
            id: Some(id("observe")),
        }))
        .await
        .unwrap()
        .into_inner();
    let mut reordered = current.entity.unwrap();
    let Some(pb::entity::Kind::AutomationSlice(slice)) = &mut reordered.kind else {
        panic!("slice")
    };
    slice.source_read_models.reverse();
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(reordered),
            if_match: current.etag,
            ..Default::default()
        }))
        .await
        .unwrap();
    for occurrence in &subjects[1..3] {
        let result = service
            .list_subject_decisions(Request::new(pb::ListSubjectDecisionsRequest {
                subject: Some(occurrence.clone()),
                status: None,
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(result.declarations.len(), 1);
        let Some(pb::catalog_declaration::Declaration::Subject(association)) =
            &result.declarations[0].declaration
        else {
            panic!("subject")
        };
        assert_eq!(association.subject.as_ref(), Some(occurrence));
    }
    let current = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Document as i32,
            id: Some(id("receipt-delivery-note")),
        }))
        .await
        .unwrap()
        .into_inner();
    let mut edited = current.entity.unwrap();
    let Some(pb::entity::Kind::Document(doc)) = &mut edited.kind else {
        panic!("document")
    };
    doc.body = Some(pb::document::Body::Retained(retained(
        "# Receipt delivery\nUpdated rationale.\n",
    )));
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(edited),
            if_match: current.etag,
            ..Default::default()
        }))
        .await
        .unwrap();
    let detail = service
        .get_decision_detail(Request::new(pb::GetDecisionDetailRequest {
            decision: Some(pb::DecisionRef {
                id: Some(id("receipt-delivery-choice")),
            }),
            follow_replacement: false,
        }))
        .await
        .unwrap()
        .into_inner();
    for (selection, expected) in [
        (&pinned, pb::CatalogResolutionState::Unresolved),
        (&whole, pb::CatalogResolutionState::Resolved),
    ] {
        assert!(detail.declarations.iter().any(|entry| {
            matches!(&entry.declaration, Some(pb::catalog_declaration::Declaration::Supporting(value)) if value.document.as_ref() == Some(selection))
                && entry.state == expected as i32
        }));
    }
    let refusal = service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Document as i32,
            id: Some(id("receipt-delivery-note")),
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(refusal.code(), tonic::Code::FailedPrecondition);
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Document as i32,
            id: Some(id("receipt-delivery-note")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let retained = service
        .get_decision_detail(Request::new(pb::GetDecisionDetailRequest {
            decision: Some(pb::DecisionRef {
                id: Some(id("receipt-delivery-choice")),
            }),
            follow_replacement: false,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        retained
            .declarations
            .iter()
            .filter(|entry| matches!(
                entry.declaration,
                Some(pb::catalog_declaration::Declaration::Supporting(_))
            ))
            .count(),
        2
    );
    assert!(retained
        .declarations
        .iter()
        .filter(|entry| matches!(
            entry.declaration,
            Some(pb::catalog_declaration::Declaration::Supporting(_))
        ))
        .all(|entry| entry.state == pb::CatalogResolutionState::Unavailable as i32));
    for selection in [&whole, &pinned] {
        assert!(retained.declarations.iter().any(|entry| {
            matches!(&entry.declaration, Some(pb::catalog_declaration::Declaration::Supporting(value)) if value.document.as_ref() == Some(selection))
        }));
    }
}
