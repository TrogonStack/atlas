#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_core::{
    connection_persistence::{prepare_entity, prepare_landing, prepare_resolution},
    connections, decision_document as catalog, entity_id, entity_kind, outbound_refs,
    retarget_refs,
    specification::content_digest,
};
use trogon_atlas_proto::{self as pb, entity, Entity, EntityKind};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "catalog-test".into(),
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
fn document() -> Entity {
    Entity {
        kind: Some(entity::Kind::Document(pb::Document {
            id: Some(id("note")),
            title: Some(title("Delivery note")),
            subjects: vec![subject()],
            body: Some(pb::document::Body::Retained(pb::RetainedDocumentText {
                media_type: Some(pb::MediaType {
                    value: "text/markdown".into(),
                }),
                content: "# Delivery\n".into(),
                digest: Some(content_digest("# Delivery\n")),
            })),
            anchors: vec![pb::DocumentAnchor {
                fragment: Some(pb::AnchorFragment {
                    value: "delivery".into(),
                }),
                label: Some(title("Delivery")),
                ..Default::default()
            }],
            ..Default::default()
        })),
        system: None,
    }
}
fn decision() -> Entity {
    Entity {
        kind: Some(entity::Kind::Decision(pb::Decision {
            id: Some(id("choice")),
            title: Some(title("Delivery choice")),
            subjects: vec![subject()],
            status: pb::DecisionStatus::Proposed as i32,
            context: Some(text("Choose delivery")),
            rationale: Some(text("A declared choice")),
            options: vec![pb::DecisionOption {
                title: Some(title("Notify")),
                description: Some(text("Send a notification")),
                ..Default::default()
            }],
            ..Default::default()
        })),
        system: None,
    }
}
fn doc_mut(entity: &mut Entity) -> &mut pb::Document {
    let Some(entity::Kind::Document(value)) = &mut entity.kind else {
        panic!("Document required")
    };
    value
}
fn choice_mut(entity: &mut Entity) -> &mut pb::Decision {
    let Some(entity::Kind::Decision(value)) = &mut entity.kind else {
        panic!("Decision required")
    };
    value
}
fn anchor_selection(owner: &Entity, pinned: bool) -> pb::DocumentSelection {
    let Some(entity::Kind::Document(value)) = &owner.kind else {
        panic!("Document required")
    };
    pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Anchor(
            pb::DocumentAnchorRef {
                document: Some(pb::DocumentRef {
                    id: value.id.clone(),
                }),
                owner_uid: owner.system.as_ref().unwrap().uid.clone(),
                anchor_id: value.anchors[0].anchor_id.clone(),
                expected_digest: if pinned {
                    Some(content_digest("# Delivery\n"))
                } else {
                    None
                },
            },
        )),
    }
}

#[test]
fn native_resources_route_and_assign_distinct_persisted_identities() {
    for proposed in [document(), decision()] {
        catalog::validate_values(&proposed).unwrap();
        let stored = prepare_entity(None, &proposed).unwrap();
        catalog::validate_stored_values(&stored).unwrap();
        catalog::validate_values(&stored).unwrap();
        assert!(matches!(
            entity_kind(&stored),
            Some(EntityKind::Decision | EntityKind::Document)
        ));
        assert!(entity_id(&stored).is_some());
        assert_eq!(prepare_entity(Some(&stored), &proposed).unwrap(), stored);
        assert_eq!(
            connections::connection_occurrences(&stored).unwrap().len(),
            1
        );
        assert_eq!(outbound_refs(&stored)[0].to_id, id("receipt"));
    }
}
#[test]
fn statuses_enforce_full_proposed_state() {
    let base = prepare_entity(None, &decision()).unwrap();
    for status in [
        pb::DecisionStatus::Proposed,
        pb::DecisionStatus::Accepted,
        pb::DecisionStatus::Rejected,
        pb::DecisionStatus::Withdrawn,
        pb::DecisionStatus::Superseded,
    ] {
        let mut candidate = base.clone();
        let value = choice_mut(&mut candidate);
        value.status = status as i32;
        let adopted = matches!(
            status,
            pb::DecisionStatus::Accepted | pb::DecisionStatus::Superseded
        );
        if adopted {
            value.selected_option = value.options[0].option_id.clone();
            value.consequences.push(pb::DecisionConsequence {
                kind: pb::decision_consequence::Kind::Risk as i32,
                text: Some(text("Delivery may fail")),
            });
        }
        if status == pb::DecisionStatus::Superseded {
            value.replaced_by = Some(pb::DecisionReplacement {
                decision: Some(pb::DecisionRef {
                    id: Some(id("replacement")),
                }),
                ..Default::default()
            });
        }
        catalog::validate_values(&candidate).unwrap();
        let value = choice_mut(&mut candidate);
        value.selected_option = if adopted {
            None
        } else {
            value.options[0].option_id.clone()
        };
        assert!(catalog::validate_values(&candidate).is_err());
    }
    let mut unknown = base;
    choice_mut(&mut unknown).status = 999;
    assert!(catalog::validate_values(&unknown).is_err());
}
#[test]
fn exact_document_bytes_and_inert_link_rules() {
    let mut candidate = document();
    let Some(pb::document::Body::Retained(body)) = &mut doc_mut(&mut candidate).body else {
        unreachable!()
    };
    body.content.pop();
    assert!(catalog::validate_values(&candidate).is_err());
    for bad in [
        "http://example.com",
        "https://user@example.com",
        "https://@example.com",
        "file:///tmp/note",
        "https://example.com/\n",
    ] {
        let mut candidate = document();
        let value = doc_mut(&mut candidate);
        value.anchors.clear();
        value.body = Some(pb::document::Body::Link(pb::ExternalDocumentLink {
            value: bad.into(),
        }));
        assert!(
            catalog::validate_values(&candidate).is_err(),
            "accepted {bad:?}"
        );
    }
    let value = doc_mut(&mut candidate);
    value.anchors.clear();
    value.body = Some(pb::document::Body::Link(pb::ExternalDocumentLink {
        value: "https://example.com/note".into(),
    }));
    catalog::validate_values(&candidate).unwrap();
}
#[test]
fn anchors_preserve_incarnation_and_exact_pin_presence() {
    let stored = prepare_entity(None, &document()).unwrap();
    let pinned = anchor_selection(&stored, true);
    let unpinned = anchor_selection(&stored, false);
    catalog::resolve_document_selection(&pinned, &stored).unwrap();
    let mut edited = stored.clone();
    let Some(pb::document::Body::Retained(body)) = &mut doc_mut(&mut edited).body else {
        unreachable!()
    };
    body.content.push(' ');
    body.digest = Some(content_digest(&body.content));
    let edited = prepare_entity(Some(&stored), &edited).unwrap();
    assert!(catalog::resolve_document_selection(&pinned, &edited).is_err());
    catalog::resolve_document_selection(&unpinned, &edited).unwrap();
    let recreated = prepare_entity(None, &document()).unwrap();
    assert!(catalog::resolve_document_selection(&unpinned, &recreated).is_err());
    let whole = pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Document(
            pb::DocumentRef {
                id: Some(id("note")),
            },
        )),
    };
    assert!(catalog::resolve_document_selection(&whole, &recreated)
        .unwrap()
        .is_none());
}
#[test]
fn catalog_inventory_stays_closed_while_responsibility_admits_resources() {
    for kind in [EntityKind::Decision, EntityKind::Document] {
        let subject = pb::CatalogSubject {
            selection: Some(pb::catalog_subject::Selection::Entity(pb::EntityRef {
                kind: kind as i32,
                id: Some(id("resource")),
            })),
        };
        assert!(catalog::validate_subject(&subject).is_err());
        trogon_atlas_core::responsibility::validate_subject(&catalog::as_responsibility_subject(
            &subject,
        ))
        .unwrap();
    }
}
#[test]
fn wording_and_label_edits_preserve_ids_while_fragment_edits_require_fresh_id() {
    let stored = prepare_entity(None, &document()).unwrap();
    let mut edited = stored.clone();
    doc_mut(&mut edited).anchors[0].label = Some(title("New wording"));
    let prepared = prepare_entity(Some(&stored), &edited).unwrap();
    assert_eq!(
        doc_mut(&mut prepared.clone()).anchors[0].anchor_id,
        doc_mut(&mut stored.clone()).anchors[0].anchor_id
    );
    doc_mut(&mut edited).anchors[0]
        .fragment
        .as_mut()
        .unwrap()
        .value = "renamed".into();
    assert!(prepare_entity(Some(&stored), &edited).is_err());
    doc_mut(&mut edited).anchors[0].anchor_id = None;
    assert!(prepare_entity(Some(&stored), &edited).is_ok());
    let mut removed = stored.clone();
    doc_mut(&mut removed).anchors.clear();
    let removed = prepare_entity(Some(&stored), &removed).unwrap();
    doc_mut(&mut edited).anchors[0].anchor_id = None;
    let fresh = prepare_entity(Some(&removed), &edited).unwrap();
    assert_ne!(
        doc_mut(&mut fresh.clone()).anchors[0].anchor_id,
        doc_mut(&mut stored.clone()).anchors[0].anchor_id
    );
}
#[test]
fn branch_resolution_renews_removed_anchors_and_selected_options() {
    for mut stored in [
        prepare_entity(None, &document()).unwrap(),
        prepare_entity(None, &decision()).unwrap(),
    ] {
        if let Some(entity::Kind::Decision(value)) = &mut stored.kind {
            value.selected_option = value.options[0].option_id.clone();
        }
        let mut opposing = stored.clone();
        match opposing.kind.as_mut().unwrap() {
            entity::Kind::Document(v) => v.anchors.clear(),
            entity::Kind::Decision(v) => {
                v.options.clear();
                v.selected_option = None;
            }
            _ => unreachable!(),
        }
        assert!(prepare_landing(Some(&stored), Some(&opposing), &stored).is_err());
        let resolved = prepare_resolution(Some(&stored), Some(&opposing), &stored).unwrap();
        assert_eq!(resolved.system, stored.system);
        match resolved.kind.unwrap() {
            entity::Kind::Document(v) => assert_ne!(
                v.anchors[0].anchor_id,
                doc_mut(&mut stored).anchors[0].anchor_id
            ),
            entity::Kind::Decision(v) => {
                assert_ne!(
                    v.options[0].option_id,
                    choice_mut(&mut stored).options[0].option_id
                );
                assert_eq!(v.selected_option, v.options[0].option_id);
            }
            _ => unreachable!(),
        }
    }
}
#[test]
fn outbound_and_generic_reference_preserve_pinned_anchor_provenance() {
    let target = prepare_entity(None, &document()).unwrap();
    let selected = anchor_selection(&target, true);
    let mut proposed = decision();
    choice_mut(&mut proposed)
        .documents
        .push(pb::SupportingDocument {
            document: Some(selected.clone()),
            ..Default::default()
        });
    let owner = prepare_entity(None, &proposed).unwrap();
    let outbound = outbound_refs(&owner)
        .into_iter()
        .find(|r| r.to_kind == Some(EntityKind::Document))
        .unwrap();
    let mut reference = pb::Reference {
        from_field: outbound.from_field.clone(),
        ..Default::default()
    };
    catalog::decorate_reference(&owner, &mut reference);
    assert_eq!(reference.document_selection, Some(selected));
    assert!(reference.connection_id.is_some());
    let mut rewritten = owner.clone();
    assert_eq!(
        retarget_refs(
            &mut rewritten,
            EntityKind::Document,
            &id("note"),
            &id("other")
        ),
        Vec::<String>::new()
    );
    assert_eq!(rewritten, owner);
}
#[test]
fn duplicate_full_subjects_and_selections_reject() {
    let mut candidate = document();
    doc_mut(&mut candidate).subjects.push(subject());
    assert!(catalog::validate_values(&candidate).is_err());
    let mut candidate = decision();
    let target = prepare_entity(None, &document()).unwrap();
    let selection = anchor_selection(&target, false);
    let pinned = anchor_selection(&target, true);
    choice_mut(&mut candidate).documents = vec![
        pb::SupportingDocument {
            document: Some(selection.clone()),
            ..Default::default()
        },
        pb::SupportingDocument {
            document: Some(pinned),
            ..Default::default()
        },
    ];
    catalog::validate_values(&candidate).unwrap();
    choice_mut(&mut candidate)
        .documents
        .push(pb::SupportingDocument {
            document: Some(selection),
            ..Default::default()
        });
    assert!(catalog::validate_values(&candidate).is_err());
}
#[test]
fn selected_anchor_requires_complete_unambiguous_fragment_inventory() {
    let stored = prepare_entity(None, &document()).unwrap();
    let selected = anchor_selection(&stored, false);
    let mut malformed = stored;
    let mut duplicate = doc_mut(&mut malformed).anchors[0].clone();
    duplicate.anchor_id = Some(catalog::LocalValueId::new().into());
    doc_mut(&mut malformed).anchors.push(duplicate);
    assert!(catalog::resolve_document_selection(&selected, &malformed).is_err());
}

#[test]
fn changed_fragment_can_request_fresh_anchor_identity_in_one_guarded_edit() {
    let stored = prepare_entity(None, &document()).unwrap();
    let mut proposed = stored.clone();
    doc_mut(&mut proposed).anchors[0]
        .fragment
        .as_mut()
        .unwrap()
        .value = "new-fragment".into();
    doc_mut(&mut proposed).anchors[0].anchor_id = None;
    let mut prepared = prepare_entity(Some(&stored), &proposed).unwrap();
    assert_ne!(
        doc_mut(&mut prepared).anchors[0].anchor_id,
        doc_mut(&mut stored.clone()).anchors[0].anchor_id
    );
}

#[test]
fn identical_option_wording_can_be_added_as_distinct_local_choice() {
    let stored = prepare_entity(None, &decision()).unwrap();
    let mut proposed = stored.clone();
    let mut added = choice_mut(&mut proposed).options[0].clone();
    added.option_id = None;
    choice_mut(&mut proposed).options.push(added);
    let mut prepared = prepare_entity(Some(&stored), &proposed).unwrap();
    let options = &choice_mut(&mut prepared).options;
    assert_eq!(options.len(), 2);
    assert_ne!(options[0].option_id, options[1].option_id);
    catalog::validate_values(&prepared).unwrap();
}

#[test]
fn byte_and_collection_limits_accept_boundaries_and_reject_excess() {
    let mut candidate = decision();
    let value = choice_mut(&mut candidate);
    value.title = Some(title(&"é".repeat(128)));
    value.context = Some(text(&"x".repeat(16_384)));
    value.options = vec![value.options[0].clone(); 64];
    value.subjects = (0..1_024)
        .map(|i| {
            let mut association = subject();
            let Some(pb::catalog_subject::Selection::Entity(reference)) =
                association.subject.as_mut().unwrap().selection.as_mut()
            else {
                unreachable!()
            };
            reference.id = Some(id(&format!("subject-{i}")));
            association
        })
        .collect();
    catalog::validate_values(&candidate).unwrap();
    let mut excessive = candidate.clone();
    choice_mut(&mut excessive).title = Some(title(&"é".repeat(129)));
    assert!(catalog::validate_values(&excessive).is_err());
    let mut excessive = candidate.clone();
    choice_mut(&mut excessive)
        .context
        .as_mut()
        .unwrap()
        .value
        .push('x');
    assert!(catalog::validate_values(&excessive).is_err());
    let mut excessive = candidate.clone();
    let extra = choice_mut(&mut excessive).options[0].clone();
    choice_mut(&mut excessive).options.push(extra);
    assert!(catalog::validate_values(&excessive).is_err());
    choice_mut(&mut candidate).subjects.push(subject());
    assert!(catalog::validate_values(&candidate).is_err());

    let mut candidate = document();
    let value = doc_mut(&mut candidate);
    value.anchors = (0..256)
        .map(|i| pb::DocumentAnchor {
            fragment: Some(pb::AnchorFragment {
                value: format!("anchor-{i}"),
            }),
            label: Some(title("Anchor")),
            anchor_id: None,
        })
        .collect();
    let Some(pb::document::Body::Retained(body)) = &mut value.body else {
        unreachable!()
    };
    body.content = "x".repeat(262_144);
    body.digest = Some(content_digest(&body.content));
    catalog::validate_values(&candidate).unwrap();
    let mut excessive = candidate.clone();
    doc_mut(&mut excessive).anchors.push(pb::DocumentAnchor {
        fragment: Some(pb::AnchorFragment {
            value: "extra".into(),
        }),
        label: Some(title("Extra")),
        anchor_id: None,
    });
    assert!(catalog::validate_values(&excessive).is_err());
    let Some(pb::document::Body::Retained(body)) = &mut doc_mut(&mut candidate).body else {
        unreachable!()
    };
    body.content.push('x');
    body.digest = Some(content_digest(&body.content));
    assert!(catalog::validate_values(&candidate).is_err());
}

#[test]
fn association_labels_are_mutable_while_full_selection_pins_require_fresh_identity() {
    let target = prepare_entity(None, &document()).unwrap();
    let mut proposed = document();
    doc_mut(&mut proposed).id = Some(id("index"));
    doc_mut(&mut proposed)
        .navigation
        .push(pb::DocumentNavigation {
            document: Some(anchor_selection(&target, false)),
            label: Some(title("Initial")),
            ..Default::default()
        });
    let stored = prepare_entity(None, &proposed).unwrap();
    let mut edited = stored.clone();
    doc_mut(&mut edited).navigation[0].label = Some(title("Reworded"));
    let prepared = prepare_entity(Some(&stored), &edited).unwrap();
    assert_eq!(
        connections::connection_occurrences(&stored).unwrap(),
        connections::connection_occurrences(&prepared).unwrap()
    );
    doc_mut(&mut edited).navigation[0].document = Some(anchor_selection(&target, true));
    assert!(prepare_entity(Some(&stored), &edited).is_err());
}

#[test]
fn distinguishable_complete_option_replacement_mints_fresh_ids_in_one_guarded_edit() {
    let stored = prepare_entity(None, &decision()).unwrap();
    let mut proposed = stored.clone();
    choice_mut(&mut proposed).options = vec![pb::DecisionOption {
        option_id: None,
        title: Some(title("Poll")),
        description: Some(text("Poll the receipt")),
    }];
    catalog::validate_values(&proposed).unwrap();
    assert!(!catalog::equivalent_manifest(&stored, &proposed).unwrap());
    let mut prepared = prepare_entity(Some(&stored), &proposed).unwrap();
    catalog::validate_values(&prepared).unwrap();
    assert_eq!(
        prepared.system.as_ref().unwrap().uid,
        stored.system.as_ref().unwrap().uid
    );
    assert_eq!(catalog::subjects(&prepared), catalog::subjects(&stored));
    let fresh = choice_mut(&mut prepared).options[0].option_id.clone();
    assert!(fresh.is_some());
    assert_ne!(fresh, choice_mut(&mut stored.clone()).options[0].option_id);
}

#[test]
fn edited_manifest_cannot_strip_retained_option_identity_or_supply_unknown_id() {
    let stored = prepare_entity(None, &decision()).unwrap();
    let mut stripped = stored.clone();
    let value = choice_mut(&mut stripped);
    value.doc = Some("Changed owner prose".into());
    value.options[0].option_id = None;
    assert!(prepare_entity(Some(&stored), &stripped).is_err());
    let mut unknown = stored.clone();
    choice_mut(&mut unknown).options[0].option_id = Some(catalog::LocalValueId::new().into());
    assert!(prepare_entity(Some(&stored), &unknown).is_err());
}

fn assert_anchor_rejects_corrupt_associations(corrupt: impl FnOnce(&mut pb::Document)) {
    let owner = prepare_entity(None, &document()).unwrap();
    let selection = anchor_selection(&owner, false);
    let mut malformed = owner;
    corrupt(doc_mut(&mut malformed));
    assert!(connections::validate_stored_connection_ids(&malformed).is_err());
    assert!(catalog::resolve_document_selection(&selection, &malformed).is_err());
}

#[test]
fn anchor_selection_rejects_missing_persisted_association_identity() {
    assert_anchor_rejects_corrupt_associations(|owner| owner.subjects[0].connection_id = None);
}

#[test]
fn anchor_selection_rejects_malformed_persisted_association_identity() {
    assert_anchor_rejects_corrupt_associations(|owner| {
        owner.subjects[0].connection_id = Some(pb::ConnectionId {
            value: "malformed".into(),
        });
    });
}

#[test]
fn anchor_selection_rejects_duplicate_persisted_association_identities() {
    assert_anchor_rejects_corrupt_associations(|owner| {
        let mut duplicate = owner.subjects[0].clone();
        let Some(pb::catalog_subject::Selection::Entity(reference)) =
            duplicate.subject.as_mut().unwrap().selection.as_mut()
        else {
            unreachable!()
        };
        reference.id = Some(id("other-receipt"));
        owner.subjects.push(duplicate);
    });
}

#[test]
fn anchor_resolution_checks_identity_without_hydrating_unrelated_outgoing_targets() {
    let mut proposed = document();
    doc_mut(&mut proposed)
        .navigation
        .push(pb::DocumentNavigation {
            document: Some(pb::DocumentSelection {
                selection: Some(pb::document_selection::Selection::Document(
                    pb::DocumentRef {
                        id: Some(id("unavailable-navigation")),
                    },
                )),
            }),
            ..Default::default()
        });
    let owner = prepare_entity(None, &proposed).unwrap();
    catalog::resolve_document_selection(&anchor_selection(&owner, false), &owner).unwrap();
}
