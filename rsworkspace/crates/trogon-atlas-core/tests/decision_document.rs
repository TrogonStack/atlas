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
fn decision_calendar_dates_are_optional_gregorian_and_ordered() {
    let date = |value: &str| pb::CalendarDate {
        value: value.into(),
    };
    let mut proposed = decision();
    catalog::validate_values(&proposed).unwrap();
    for invalid in [
        "",
        "0000-01-01",
        "2025-02-29",
        "2024-04-31",
        "2024-2-01",
        "2024-01-01T00:00:00Z",
    ] {
        choice_mut(&mut proposed).authored_on = Some(date(invalid));
        assert!(
            catalog::validate_values(&proposed).is_err(),
            "accepted {invalid}"
        );
        choice_mut(&mut proposed).authored_on = None;
        choice_mut(&mut proposed).decided_on = Some(date(invalid));
        assert!(
            catalog::validate_values(&proposed).is_err(),
            "accepted {invalid}"
        );
        choice_mut(&mut proposed).decided_on = None;
    }
    choice_mut(&mut proposed).authored_on = Some(date("2024-02-29"));
    choice_mut(&mut proposed).decided_on = Some(date("2024-02-28"));
    assert!(catalog::validate_values(&proposed).is_err());
    for valid in ["2024-02-29", "2099-01-01"] {
        choice_mut(&mut proposed).decided_on = Some(date(valid));
        catalog::validate_values(&proposed).unwrap();
        let stored = prepare_entity(None, &proposed).unwrap();
        let entity::Kind::Decision(stored) = stored.kind.unwrap() else {
            panic!("decision")
        };
        assert_eq!(stored.authored_on, Some(date("2024-02-29")));
        assert_eq!(stored.decided_on, Some(date(valid)));
        assert_eq!(stored.status, pb::DecisionStatus::Proposed as i32);
    }
}

#[test]
fn decision_dates_remain_native_under_source_ownership() {
    use pb::source_field_ownership::Policy;
    use trogon_atlas_core::source_integration::*;
    let mut dated = decision();
    choice_mut(&mut dated).authored_on = Some(pb::CalendarDate {
        value: "2024-02-29".into(),
    });
    let title_policy = pb::SourceFieldOwnership {
        policy: Some(Policy::Fields(pb::SourceOwnedFields {
            fields: vec![pb::SourceOwnedField::Title as i32],
        })),
    };
    let whole_policy = pb::SourceFieldOwnership {
        policy: Some(Policy::WholeOwner(pb::SourceWholeOwnerOwnership {})),
    };
    for policy in [&title_policy, &whole_policy] {
        assert_eq!(
            validate_create_ownership(&dated, policy),
            Err(SourceError::UnsupportedUnit)
        );
    }
    assert!(validate_ownership(&dated, &title_policy).is_ok());
    assert_eq!(
        validate_ownership(&dated, &whole_policy),
        Err(SourceError::UnsupportedUnit)
    );
    let plain = decision();
    let mut incoming = plain.clone();
    choice_mut(&mut incoming).title = Some(title("Updated source title"));
    let mut budget = RunBudget::new(profile_budgets()).unwrap();
    let merged = plan_merge(
        &plain,
        &plain,
        &incoming,
        &dated,
        &title_policy,
        &mut budget,
    )
    .unwrap();
    assert_eq!(
        choice_mut(&mut merged.proposed.clone()).authored_on,
        choice_mut(&mut dated).authored_on
    );
    assert!(plan_merge(&plain, &plain, &plain, &dated, &whole_policy, &mut budget).is_err());
    assert!(plan_merge(&plain, &plain, &dated, &plain, &whole_policy, &mut budget).is_err());
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

#[test]
fn decision_makers_and_amendments_admit_only_exact_distinct_targets() {
    let mut value = decision();
    let maker = pb::DecisionMaker {
        assignee: Some(pb::decision_maker::Assignee::Person(pb::PersonRef {
            id: Some(id("maker")),
        })),
        ..Default::default()
    };
    choice_mut(&mut value).decision_makers = vec![maker.clone(), maker.clone()];
    assert!(
        catalog::validate_values(&value).is_err(),
        "duplicate makers accepted"
    );
    choice_mut(&mut value).decision_makers = vec![maker];
    let amendment = pb::DecisionAmendment {
        decision: Some(pb::DecisionRef {
            id: Some(id("prior-choice")),
        }),
        ..Default::default()
    };
    choice_mut(&mut value).amends = vec![amendment.clone(), amendment.clone()];
    assert!(
        catalog::validate_values(&value).is_err(),
        "duplicate amendments accepted"
    );
    choice_mut(&mut value).amends = vec![amendment];
    catalog::validate_values(&value).unwrap();
    let stored = prepare_entity(None, &value).unwrap();
    assert_eq!(
        connections::connection_occurrences(&stored).unwrap().len(),
        3
    );
    assert_eq!(outbound_refs(&stored).len(), 3);
    choice_mut(&mut value).amends[0]
        .decision
        .as_mut()
        .unwrap()
        .id = Some(id("choice"));
    assert!(
        catalog::validate_values(&value).is_err(),
        "self amendment accepted"
    );
    choice_mut(&mut value).amends.clear();
    choice_mut(&mut value).decision_makers[0].assignee = None;
    assert!(
        catalog::validate_values(&value).is_err(),
        "absent maker accepted"
    );
}

#[test]
fn maker_and_amendment_identity_and_source_isolation_are_native() {
    use trogon_atlas_core::source_integration::*;
    let mut proposed = decision();
    choice_mut(&mut proposed).decision_makers = vec![pb::DecisionMaker {
        assignee: Some(pb::decision_maker::Assignee::Team(pb::TeamRef {
            id: Some(id("makers")),
        })),
        ..Default::default()
    }];
    choice_mut(&mut proposed).amends = vec![pb::DecisionAmendment {
        decision: Some(pb::DecisionRef {
            id: Some(id("prior")),
        }),
        ..Default::default()
    }];
    let stored = prepare_entity(None, &proposed).unwrap();
    let mut edited = stored.clone();
    choice_mut(&mut edited).decision_makers[0].doc = Some(text("Historical team participation"));
    choice_mut(&mut edited).amends[0].doc = Some(text("Clarifies the original"));
    let edited = prepare_entity(Some(&stored), &edited).unwrap();
    assert_eq!(
        connections::connection_occurrences(&stored).unwrap(),
        connections::connection_occurrences(&edited).unwrap()
    );
    let mut retargeted = edited.clone();
    choice_mut(&mut retargeted).amends[0]
        .decision
        .as_mut()
        .unwrap()
        .id = Some(id("other"));
    assert!(prepare_entity(Some(&stored), &retargeted).is_err());
    choice_mut(&mut retargeted).amends[0].connection_id = None;
    let retargeted = prepare_entity(Some(&stored), &retargeted).unwrap();
    assert_ne!(
        choice_mut(&mut stored.clone()).amends[0].connection_id,
        choice_mut(&mut retargeted.clone()).amends[0].connection_id
    );
    let units = [
        pb::SourceFieldOwnership {
            policy: Some(pb::source_field_ownership::Policy::Fields(
                pb::SourceOwnedFields {
                    fields: vec![pb::SourceOwnedField::Title as i32],
                },
            )),
        },
        pb::SourceFieldOwnership {
            policy: Some(pb::source_field_ownership::Policy::WholeOwner(
                pb::SourceWholeOwnerOwnership {},
            )),
        },
    ];
    for policy in &units {
        assert!(validate_create_ownership(&stored, policy).is_err());
    }
    assert!(validate_ownership(&stored, &units[1]).is_err());
    let mut plain = decision();
    choice_mut(&mut plain).title = Some(title("Source title"));
    let base = decision();
    let mut budget = RunBudget::new(profile_budgets()).unwrap();
    let merged = plan_merge(&base, &base, &plain, &stored, &units[0], &mut budget).unwrap();
    assert_eq!(
        choice_mut(&mut merged.proposed.clone()).decision_makers,
        choice_mut(&mut stored.clone()).decision_makers
    );
    assert_eq!(
        choice_mut(&mut merged.proposed.clone()).amends,
        choice_mut(&mut stored.clone()).amends
    );
    let mut invalid = proposed;
    if let Some(pb::decision_maker::Assignee::Team(r)) =
        &mut choice_mut(&mut invalid).decision_makers[0].assignee
    {
        r.id.as_mut().unwrap().version = 0;
    }
    assert!(catalog::validate_values(&invalid).is_err());
}

#[test]
fn makers_and_amendments_branch_restore_and_reference_retarget_renew_occurrences() {
    let mut proposed = decision();
    choice_mut(&mut proposed).decision_makers = vec![pb::DecisionMaker {
        assignee: Some(pb::decision_maker::Assignee::Person(pb::PersonRef {
            id: Some(id("maker")),
        })),
        ..Default::default()
    }];
    choice_mut(&mut proposed).amends = vec![pb::DecisionAmendment {
        decision: Some(pb::DecisionRef {
            id: Some(id("original")),
        }),
        ..Default::default()
    }];
    let stored = prepare_entity(None, &proposed).unwrap();
    let mut removed = stored.clone();
    choice_mut(&mut removed).decision_makers.clear();
    choice_mut(&mut removed).amends.clear();
    let removed = prepare_entity(Some(&stored), &removed).unwrap();
    assert!(prepare_landing(Some(&stored), Some(&removed), &stored).is_err());
    let restored = prepare_resolution(Some(&stored), Some(&removed), &stored).unwrap();
    let old = choice_mut(&mut stored.clone()).clone();
    let new = choice_mut(&mut restored.clone()).clone();
    assert_ne!(
        old.decision_makers[0].connection_id,
        new.decision_makers[0].connection_id
    );
    assert_ne!(old.amends[0].connection_id, new.amends[0].connection_id);
    assert_eq!(
        old.decision_makers[0].assignee,
        new.decision_makers[0].assignee
    );
    assert_eq!(old.amends[0].decision, new.amends[0].decision);
    let mut retargeted = stored.clone();
    let paths = retarget_refs(
        &mut retargeted,
        EntityKind::Person,
        &id("maker"),
        &id("other-maker"),
    );
    assert!(paths.iter().any(|path| path.contains("decision_makers")));
    connections::renew_retargeted_connection_ids(&stored, &mut retargeted).unwrap();
    assert_ne!(
        choice_mut(&mut retargeted).decision_makers[0].connection_id,
        old.decision_makers[0].connection_id
    );
    assert_eq!(choice_mut(&mut retargeted).amends, old.amends);
}

#[test]
fn decision_extension_collection_bounds_are_inclusive() {
    let mut value = decision();
    choice_mut(&mut value).decision_makers = (0..128)
        .map(|i| pb::DecisionMaker {
            assignee: Some(pb::decision_maker::Assignee::Person(pb::PersonRef {
                id: Some(id(&format!("maker-{i}"))),
            })),
            ..Default::default()
        })
        .collect();
    choice_mut(&mut value).amends = (0..128)
        .map(|i| pb::DecisionAmendment {
            decision: Some(pb::DecisionRef {
                id: Some(id(&format!("original-{i}"))),
            }),
            ..Default::default()
        })
        .collect();
    catalog::validate_values(&value).unwrap();
    let mut excessive = value.clone();
    choice_mut(&mut excessive)
        .decision_makers
        .push(pb::DecisionMaker {
            assignee: Some(pb::decision_maker::Assignee::Team(pb::TeamRef {
                id: Some(id("extra-maker")),
            })),
            ..Default::default()
        });
    assert!(catalog::validate_values(&excessive).is_err());
    choice_mut(&mut value).amends.push(pb::DecisionAmendment {
        decision: Some(pb::DecisionRef {
            id: Some(id("extra-original")),
        }),
        ..Default::default()
    });
    assert!(catalog::validate_values(&value).is_err());
}

fn subject_for(slug: &str) -> pb::SubjectAssociation {
    let mut association = subject();
    let Some(pb::catalog_subject::Selection::Entity(reference)) =
        association.subject.as_mut().unwrap().selection.as_mut()
    else {
        unreachable!()
    };
    reference.id = Some(id(slug));
    association
}

fn whole_document(slug: &str) -> pb::DocumentSelection {
    pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Document(
            pb::DocumentRef { id: Some(id(slug)) },
        )),
    }
}

fn rejection(candidate: &Entity) -> String {
    catalog::validate_values(candidate).unwrap_err().to_string()
}

#[test]
fn document_owner_inventory_is_bound_by_collection_limits_before_the_combined_ceiling() {
    let mut candidate = document();
    let value = doc_mut(&mut candidate);
    value.subjects = (0..catalog::MAX_SUBJECTS)
        .map(|i| subject_for(&format!("subject-{i}")))
        .collect();
    value.navigation = (0..128)
        .map(|i| pb::DocumentNavigation {
            document: Some(whole_document(&format!("navigation-{i}"))),
            ..Default::default()
        })
        .collect();
    value.related_content = (0..128)
        .map(|i| pb::RelatedContent {
            document: Some(whole_document(&format!("related-{i}"))),
            role: pb::RelatedContentRole::Attachment as i32,
            ..Default::default()
        })
        .collect();
    let reachable = value.subjects.len() + value.navigation.len() + value.related_content.len();
    assert_eq!(reachable, 1_280);
    assert!(reachable < catalog::MAX_ASSOCIATIONS);
    catalog::validate_values(&candidate).unwrap();

    let mut excessive = candidate.clone();
    doc_mut(&mut excessive)
        .subjects
        .push(subject_for("one-more"));
    assert_eq!(rejection(&excessive), "subject collection outside bounds");
    let mut excessive = candidate.clone();
    doc_mut(&mut excessive)
        .navigation
        .push(pb::DocumentNavigation {
            document: Some(whole_document("navigation-extra")),
            ..Default::default()
        });
    assert_eq!(rejection(&excessive), "document collection limit exceeded");
    let mut excessive = candidate;
    doc_mut(&mut excessive)
        .related_content
        .push(pb::RelatedContent {
            document: Some(whole_document("related-extra")),
            role: pb::RelatedContentRole::Attachment as i32,
            ..Default::default()
        });
    assert_eq!(rejection(&excessive), "related content limit exceeded");
}

#[test]
fn decision_owner_inventory_is_bound_by_collection_limits_before_the_combined_ceiling() {
    let mut candidate = prepare_entity(None, &decision()).unwrap();
    let value = choice_mut(&mut candidate);
    value.status = pb::DecisionStatus::Superseded as i32;
    value.selected_option = value.options[0].option_id.clone();
    value.consequences.push(pb::DecisionConsequence {
        kind: pb::decision_consequence::Kind::Risk as i32,
        text: Some(text("Delivery may fail")),
    });
    value.replaced_by = Some(pb::DecisionReplacement {
        decision: Some(pb::DecisionRef {
            id: Some(id("replacement")),
        }),
        ..Default::default()
    });
    value.subjects = (0..catalog::MAX_SUBJECTS)
        .map(|i| subject_for(&format!("subject-{i}")))
        .collect();
    value.decision_makers = (0..128)
        .map(|i| pb::DecisionMaker {
            assignee: Some(pb::decision_maker::Assignee::Person(pb::PersonRef {
                id: Some(id(&format!("maker-{i}"))),
            })),
            ..Default::default()
        })
        .collect();
    value.amends = (0..128)
        .map(|i| pb::DecisionAmendment {
            decision: Some(pb::DecisionRef {
                id: Some(id(&format!("amended-{i}"))),
            }),
            ..Default::default()
        })
        .collect();
    value.related_decisions = (0..128)
        .map(|i| pb::RelatedDecision {
            decision: Some(pb::DecisionRef {
                id: Some(id(&format!("related-{i}"))),
            }),
            ..Default::default()
        })
        .collect();
    value.documents = (0..128)
        .map(|i| pb::SupportingDocument {
            document: Some(whole_document(&format!("supporting-{i}"))),
            ..Default::default()
        })
        .collect();
    let reachable = value.subjects.len()
        + value.decision_makers.len()
        + value.amends.len()
        + value.related_decisions.len()
        + value.documents.len()
        + 1;
    assert_eq!(reachable, 1_537);
    assert!(reachable < catalog::MAX_ASSOCIATIONS);
    catalog::validate_values(&candidate).unwrap();

    let mut excessive = candidate.clone();
    choice_mut(&mut excessive)
        .subjects
        .push(subject_for("one-more"));
    assert_eq!(rejection(&excessive), "subject collection outside bounds");
    let mut excessive = candidate.clone();
    choice_mut(&mut excessive)
        .decision_makers
        .push(pb::DecisionMaker {
            assignee: Some(pb::decision_maker::Assignee::Team(pb::TeamRef {
                id: Some(id("maker-extra")),
            })),
            ..Default::default()
        });
    assert_eq!(
        rejection(&excessive),
        "decision relationship limit exceeded"
    );
    let mut excessive = candidate.clone();
    choice_mut(&mut excessive)
        .amends
        .push(pb::DecisionAmendment {
            decision: Some(pb::DecisionRef {
                id: Some(id("amended-extra")),
            }),
            ..Default::default()
        });
    assert_eq!(
        rejection(&excessive),
        "decision relationship limit exceeded"
    );
    let mut excessive = candidate.clone();
    choice_mut(&mut excessive)
        .related_decisions
        .push(pb::RelatedDecision {
            decision: Some(pb::DecisionRef {
                id: Some(id("related-extra")),
            }),
            ..Default::default()
        });
    assert_eq!(
        rejection(&excessive),
        "decision relationship limit exceeded"
    );
    let mut excessive = candidate;
    choice_mut(&mut excessive)
        .documents
        .push(pb::SupportingDocument {
            document: Some(whole_document("supporting-extra")),
            ..Default::default()
        });
    assert_eq!(
        rejection(&excessive),
        "decision relationship limit exceeded"
    );
}
