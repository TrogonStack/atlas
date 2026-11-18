use trogon_atlas_core::{decision_document, outbound_refs};
use trogon_atlas_proto::{self as pb, entity, Entity, EntityKind};

fn id(slug: String) -> pb::Id {
    pb::Id {
        namespace: "catalog-provenance".into(),
        slug,
        version: 1,
    }
}

fn connection_id(value: String) -> pb::ConnectionId {
    pb::ConnectionId { value }
}

fn subjects() -> Vec<pb::SubjectAssociation> {
    (0..12)
        .map(|i| pb::SubjectAssociation {
            connection_id: Some(connection_id(format!("subject-association-{i}"))),
            subject: Some(pb::CatalogSubject {
                selection: Some(pb::catalog_subject::Selection::Connection(
                    pb::ConnectionRef {
                        owner: Some(pb::EntityRef {
                            kind: EntityKind::AutomationSlice as i32,
                            id: Some(id(format!("slice-{i}"))),
                        }),
                        id: Some(connection_id(format!("observed-occurrence-{i}"))),
                        owner_uid: format!("slice-incarnation-{i}"),
                    },
                )),
            }),
            ..Default::default()
        })
        .collect()
}

fn selection(i: usize) -> pb::DocumentSelection {
    let document = pb::DocumentRef {
        id: Some(id(format!("document-{i}"))),
    };
    pb::DocumentSelection {
        selection: Some(if i.is_multiple_of(2) {
            pb::document_selection::Selection::Document(document)
        } else {
            pb::document_selection::Selection::Anchor(pb::DocumentAnchorRef {
                document: Some(document),
                owner_uid: format!("document-incarnation-{i}"),
                anchor_id: Some(pb::LocalValueId {
                    value: format!("anchor-{i}"),
                }),
                expected_digest: Some(trogon_atlas_core::specification::content_digest(&format!(
                    "content-{i}"
                ))),
            })
        }),
    }
}

fn references(owner: &Entity) -> Vec<pb::Reference> {
    outbound_refs(owner)
        .iter()
        .map(|outbound| {
            let mut reference = pb::Reference {
                from_field: outbound.from_field.clone(),
                to: outbound.to_entity_ref(),
                ..Default::default()
            };
            decision_document::decorate_reference(owner, &mut reference);
            reference
        })
        .collect()
}

fn assert_subjects(references: &[pb::Reference], associations: &[pb::SubjectAssociation]) {
    for (i, association) in associations.iter().enumerate() {
        let field = format!("subjects[{i}].subject.connection.owner");
        let matching: Vec<_> = references
            .iter()
            .filter(|r| r.from_field == field)
            .collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].catalog_subject, association.subject);
        assert_eq!(matching[0].connection_id, association.connection_id);
        assert_eq!(matching[0].document_selection, None);
    }
}

#[test]
fn decision_reference_provenance_preserves_multi_digit_association_indexes() {
    let decision = pb::Decision {
        id: Some(id("choice".into())),
        subjects: subjects(),
        documents: (0..12)
            .map(|i| pb::SupportingDocument {
                connection_id: Some(connection_id(format!("supporting-association-{i}"))),
                document: Some(selection(i)),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let owner = Entity {
        kind: Some(entity::Kind::Decision(decision.clone())),
        system: None,
    };
    let references = references(&owner);
    assert_eq!(references.len(), 24);
    assert_subjects(&references, &decision.subjects);
    for (i, declaration) in decision.documents.iter().enumerate() {
        let field = format!(
            "documents[{i}].document.{}",
            if i.is_multiple_of(2) {
                "document"
            } else {
                "anchor.document"
            }
        );
        let matching: Vec<_> = references
            .iter()
            .filter(|r| r.from_field == field)
            .collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].document_selection, declaration.document);
        assert_eq!(matching[0].connection_id, declaration.connection_id);
        assert_eq!(matching[0].catalog_subject, None);
    }
}

#[test]
fn document_reference_provenance_preserves_multi_digit_association_indexes() {
    let document = pb::Document {
        id: Some(id("guide".into())),
        subjects: subjects(),
        navigation: (0..12)
            .map(|i| pb::DocumentNavigation {
                connection_id: Some(connection_id(format!("navigation-association-{i}"))),
                document: Some(selection(i)),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let owner = Entity {
        kind: Some(entity::Kind::Document(document.clone())),
        system: None,
    };
    let references = references(&owner);
    assert_eq!(references.len(), 24);
    assert_subjects(&references, &document.subjects);
    for (i, declaration) in document.navigation.iter().enumerate() {
        let field = format!(
            "navigation[{i}].document.{}",
            if i.is_multiple_of(2) {
                "document"
            } else {
                "anchor.document"
            }
        );
        let matching: Vec<_> = references
            .iter()
            .filter(|r| r.from_field == field)
            .collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].document_selection, declaration.document);
        assert_eq!(matching[0].connection_id, declaration.connection_id);
        assert_eq!(matching[0].catalog_subject, None);
    }
}
