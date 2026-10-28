#![allow(clippy::unwrap_used, clippy::expect_used)]

use prost::Message;
use trogon_atlas_core::{
    decision_document::{self, validate_values},
    document_catalog_context::{
        self, bytes_content_digest, validate_logical_filename, validate_purpose,
        validate_retained_content, validate_retained_media_type, COLLECTION,
    },
    source_integration::{self, SourceError},
};
use trogon_atlas_proto::{
    self as pb, entity, source_field_ownership::Policy, Document, DocumentCollectionAnnotation,
    DocumentPurpose, Entity, EntityKind, LogicalFilename, MediaType, RelatedContent,
    RelatedContentRole, RetainedContent, SourceFieldOwnership, SourceOwnedField,
    SourceOwnedFields, SourceWholeOwnerOwnership,
};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "catalog-test".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn subject() -> pb::SubjectAssociation {
    pb::SubjectAssociation {
        subject: Some(pb::CatalogSubject {
            selection: Some(pb::catalog_subject::Selection::Entity(pb::EntityRef {
                kind: EntityKind::ReadModel as i32,
                id: Some(id("receipt")),
            })),
        }),
        ..Default::default()
    }
}

fn text_document(body: pb::document::Body) -> Entity {
    Entity {
        kind: Some(entity::Kind::Document(Document {
            id: Some(id("note")),
            title: Some(pb::CatalogTitle {
                value: "Note".into(),
            }),
            subjects: vec![subject()],
            body: Some(body),
            ..Default::default()
        })),
        ..Default::default()
    }
}

#[test]
fn bytes_digest_matches_exact_binary_payload() {
    let bytes = b"\xffJSON{\"example\":true}";
    let digest = bytes_content_digest(bytes);
    let mut body = RetainedContent {
        content: bytes.to_vec(),
        media_type: Some(MediaType {
            value: "application/json".into(),
        }),
        digest: Some(digest.clone()),
    };
    validate_retained_content(&body).unwrap();
    body.digest.as_mut().unwrap().sha256 = "0".repeat(64);
    assert!(validate_retained_content(&body).is_err());
    assert_ne!(
        bytes_content_digest(b"a\r\n").sha256,
        bytes_content_digest(b"a\n").sha256
    );
}

#[test]
fn retained_media_type_rejects_parameters_and_mixed_case() {
    for value in ["TEXT/PLAIN", "text/plain; charset=utf-8", "text", ""] {
        assert!(validate_retained_media_type(&MediaType {
            value: value.into(),
        })
        .is_err());
    }
    validate_retained_media_type(&MediaType {
        value: "image/svg+xml".into(),
    })
    .unwrap();
}

#[test]
fn logical_filename_rejects_path_like_names() {
    for value in ["../x", "a/b", "a\\b", ".", "..", ""] {
        assert!(validate_logical_filename(&LogicalFilename {
            value: value.into(),
        })
        .is_err());
    }
    validate_logical_filename(&LogicalFilename {
        value: "diagram.svg".into(),
    })
    .unwrap();
}

#[test]
fn purpose_absent_differs_from_explicit_general_in_manifest() {
    let absent = text_document(pb::document::Body::Retained(pb::RetainedDocumentText {
        media_type: Some(MediaType {
            value: "text/markdown".into(),
        }),
        content: "# Title\n".into(),
        digest: Some(trogon_atlas_core::specification::content_digest("# Title\n")),
    }));
    let mut general = absent.clone();
    if let Some(entity::Kind::Document(doc)) = &mut general.kind {
        doc.purpose = Some(DocumentPurpose::General as i32);
    }
    validate_values(&absent).unwrap();
    validate_values(&general).unwrap();
    assert_ne!(absent.encode_to_vec(), general.encode_to_vec());
}

#[test]
fn binary_body_rejects_anchors_and_empty_or_oversized_bytes() {
    let bytes = vec![0u8; 262_145];
    let mut entity = text_document(pb::document::Body::RetainedContent(RetainedContent {
        media_type: Some(MediaType {
            value: "application/octet-stream".into(),
        }),
        content: bytes,
        digest: Some(bytes_content_digest(&[])),
    }));
    assert!(validate_values(&entity).is_err());
    if let Some(entity::Kind::Document(doc)) = &mut entity.kind {
        if let Some(pb::document::Body::RetainedContent(body)) = &mut doc.body {
            body.content = b"ok".to_vec();
            body.digest = Some(bytes_content_digest(&body.content));
        }
        doc.anchors.push(pb::DocumentAnchor {
            fragment: Some(pb::AnchorFragment {
                value: "x".into(),
            }),
            label: Some(pb::CatalogTitle {
                value: "X".into(),
            }),
            ..Default::default()
        });
    }
    assert!(validate_values(&entity).is_err());
}

#[test]
fn related_content_enforces_role_identity_and_self_rejection() {
    let target = id("child");
    let mut entity = text_document(pb::document::Body::Retained(pb::RetainedDocumentText {
        media_type: Some(MediaType {
            value: "text/markdown".into(),
        }),
        content: "# Parent\n".into(),
        digest: Some(trogon_atlas_core::specification::content_digest("# Parent\n")),
    }));
    if let Some(entity::Kind::Document(doc)) = &mut entity.kind {
        doc.related_content = vec![RelatedContent {
            document: Some(pb::DocumentSelection {
                selection: Some(pb::document_selection::Selection::Document(
                    pb::DocumentRef {
                        id: Some(target.clone()),
                    },
                )),
            }),
            role: RelatedContentRole::Example as i32,
            ..Default::default()
        }];
    }
    validate_values(&entity).unwrap();
    if let Some(entity::Kind::Document(doc)) = &mut entity.kind {
        doc.related_content.push(doc.related_content[0].clone());
    }
    assert!(validate_values(&entity).is_err());
    if let Some(entity::Kind::Document(doc)) = &mut entity.kind {
        doc.related_content.pop();
        doc.related_content[0].document = Some(pb::DocumentSelection {
            selection: Some(pb::document_selection::Selection::Document(
                pb::DocumentRef {
                    id: doc.id.clone(),
                },
            )),
        });
    }
    assert!(validate_values(&entity).is_err());
}

#[test]
fn collection_annotation_is_singleton_and_validated() {
    let mut entity = text_document(pb::document::Body::Link(pb::ExternalDocumentLink {
        value: "https://example.com/doc".into(),
    }));
    let annotation = DocumentCollectionAnnotation {
        category: Some(pb::CatalogTitle {
            value: "Guides".into(),
        }),
        description: Some(pb::CatalogText {
            value: "Guide section".into(),
        }),
    };
    if let Some(entity::Kind::Document(doc)) = &mut entity.kind {
        doc.metadata = vec![prost_types::Any {
            type_url: format!("type.googleapis.com/{COLLECTION}"),
            value: annotation.encode_to_vec(),
        }];
    }
    validate_values(&entity).unwrap();
    if let Some(entity::Kind::Document(doc)) = &mut entity.kind {
        doc.metadata.push(doc.metadata[0].clone());
    }
    assert!(validate_values(&entity).is_err());
}

#[test]
fn source_ownership_rejects_native_document_extensions() {
    let mut entity = text_document(pb::document::Body::Retained(pb::RetainedDocumentText {
        media_type: Some(MediaType {
            value: "text/markdown".into(),
        }),
        content: "# Native\n".into(),
        digest: Some(trogon_atlas_core::specification::content_digest("# Native\n")),
    }));
    if let Some(entity::Kind::Document(doc)) = &mut entity.kind {
        doc.purpose = Some(DocumentPurpose::Diagram as i32);
    }
    validate_values(&entity).unwrap();
    let whole = SourceFieldOwnership {
        policy: Some(Policy::WholeOwner(SourceWholeOwnerOwnership {})),
    };
    assert_eq!(
        source_integration::validate_ownership(&entity, &whole),
        Err(SourceError::UnsupportedUnit)
    );
    let title = SourceFieldOwnership {
        policy: Some(Policy::Fields(SourceOwnedFields {
            fields: vec![SourceOwnedField::Title as i32],
        })),
    };
    assert_eq!(
        source_integration::validate_ownership(&entity, &title),
        Err(SourceError::UnsupportedUnit)
    );
    let mut plain = text_document(pb::document::Body::Retained(pb::RetainedDocumentText {
        media_type: Some(MediaType {
            value: "text/markdown".into(),
        }),
        content: "# Native\n".into(),
        digest: Some(trogon_atlas_core::specification::content_digest("# Native\n")),
    }));
    validate_values(&plain).unwrap();
    assert!(source_integration::validate_ownership(&plain, &title).is_ok());
}
