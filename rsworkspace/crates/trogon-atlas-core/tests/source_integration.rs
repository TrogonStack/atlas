#![allow(clippy::unwrap_used, clippy::expect_used)]
use trogon_atlas_core::{source_integration::*, transcode};
use trogon_atlas_proto::{
    source_field_ownership::Policy, source_selection, Entity, SourceFieldOwnership,
    SourceOwnedField, SourceOwnedFields, SourceSelection, SourceWholeOwnerOwnership,
};

fn decode(value: &serde_json::Value) -> Result<Entity, transcode::TranscodeError> {
    transcode::message_from_json_value("trogonatlas.eventmodel.v1alpha1.Entity", value)
}

fn entity(title: &str, doc: &str) -> Entity {
    decode(&serde_json::json!({"readModel":{"id":{"namespace":"test","slug":"one","version":1},"title":title,"doc":doc}})).expect("valid native input")
}
fn ownership(fields: &[SourceOwnedField]) -> SourceFieldOwnership {
    SourceFieldOwnership {
        policy: Some(Policy::Fields(SourceOwnedFields {
            fields: fields.iter().map(|f| *f as i32).collect(),
        })),
    }
}
fn budget() -> RunBudget {
    RunBudget::new(profile_budgets()).expect("profile budgets")
}

#[test]
fn source_change_preserves_locally_owned_fields() {
    let base = entity("before", "local rationale");
    let incoming = entity("after", "source attempt");
    let plan = plan_merge(
        &base,
        &base,
        &incoming,
        &base,
        &ownership(&[SourceOwnedField::Title]),
        &mut budget(),
    )
    .expect("plan");
    assert_eq!(plan.proposed, entity("after", "local rationale"));
    assert_eq!(plan.decisions[0].disposition, MergeDisposition::Update);
}
#[test]
fn native_drift_and_divergence_are_not_overwritten() {
    let base = entity("before", "doc");
    let current = entity("local", "doc");
    let drift = plan_merge(
        &base,
        &base,
        &base,
        &current,
        &ownership(&[SourceOwnedField::Title]),
        &mut budget(),
    )
    .expect("drift");
    assert_eq!(drift.proposed, current);
    assert_eq!(drift.decisions[0].disposition, MergeDisposition::LocalDrift);
    let conflict = plan_merge(
        &base,
        &base,
        &entity("source", "doc"),
        &current,
        &ownership(&[SourceOwnedField::Title]),
        &mut budget(),
    )
    .expect("conflict");
    assert!(conflict.has_conflicts());
    assert_eq!(conflict.proposed, current);
}
#[test]
fn convergence_and_unchanged_remain_distinct() {
    let base = entity("before", "doc");
    let next = entity("same", "doc");
    let converged = plan_merge(
        &base,
        &base,
        &next,
        &next,
        &ownership(&[SourceOwnedField::Title]),
        &mut budget(),
    )
    .expect("convergence");
    assert_eq!(
        converged.decisions[0].disposition,
        MergeDisposition::Convergent
    );
    let unchanged = plan_merge(
        &base,
        &base,
        &base,
        &base,
        &ownership(&[SourceOwnedField::Title]),
        &mut budget(),
    )
    .expect("unchanged");
    assert_eq!(
        unchanged.decisions[0].disposition,
        MergeDisposition::Unchanged
    );
}
#[test]
fn invalid_ownership_never_falls_back_to_whole_owner() {
    let value = entity("title", "doc");
    assert_eq!(
        validate_ownership(&value, &SourceFieldOwnership::default()),
        Err(SourceError::MissingOwnership)
    );
    assert_eq!(
        validate_ownership(&value, &ownership(&[SourceOwnedField::Anchors])),
        Err(SourceError::UnsupportedUnit)
    );
    assert_eq!(
        validate_ownership(
            &value,
            &ownership(&[SourceOwnedField::Title, SourceOwnedField::Title])
        ),
        Err(SourceError::OverlappingUnits)
    );
    assert_eq!(
        validate_ownership(&value, &ownership(&[SourceOwnedField::Unspecified])),
        Err(SourceError::UnsupportedUnit)
    );
}
#[test]
fn local_selections_cannot_seize_owner_fields() {
    let selection = SourceSelection {
        selection: Some(source_selection::Selection::Connection(Default::default())),
    };
    assert_eq!(
        validate_selection_ownership(&selection, &ownership(&[SourceOwnedField::Title])),
        Err(SourceError::LocalOwnership)
    );
}
#[test]
fn optional_presence_is_part_of_owned_value() {
    let absent = decode(
        &serde_json::json!({"document":{"id":{"namespace":"test","slug":"doc","version":1}}}),
    )
    .expect("document");
    let present=decode(&serde_json::json!({"document":{"id":{"namespace":"test","slug":"doc","version":1},"doc":""}})).expect("document");
    let unit = OwnershipUnit::Field(SourceOwnedField::Doc);
    assert_ne!(
        extract_unit(&absent, unit).expect("absent"),
        extract_unit(&present, unit).expect("present")
    );
    let mut changed = present;
    replace_unit(
        &mut changed,
        unit,
        &extract_unit(&absent, unit).expect("value"),
    )
    .expect("replace");
    assert_eq!(changed, absent);
}
#[test]
fn ordered_collections_are_indivisible_and_recurrence_is_preserved() {
    let make = |slugs: &[&str]| {
        decode(&serde_json::json!({"storyboard":{"id":{"namespace":"test","slug":"board","version":1},"slices":slugs.iter().map(|slug|serde_json::json!({"id":{"namespace":"test","slug":slug,"version":1}})).collect::<Vec<_>>()}})).expect("storyboard")
    };
    let first = make(&["a", "b", "a"]);
    let next = make(&["a", "a", "b"]);
    let unit = OwnershipUnit::Field(SourceOwnedField::Slices);
    assert_ne!(
        extract_unit(&first, unit).expect("first"),
        extract_unit(&next, unit).expect("next")
    );
    let mut changed = first;
    replace_unit(
        &mut changed,
        unit,
        &extract_unit(&next, unit).expect("value"),
    )
    .expect("replace");
    assert_eq!(changed, next);
}
#[test]
fn whole_owner_excludes_system_and_preserves_identity() {
    let original = entity("old", "old");
    let incoming = entity("new", "new");
    let policy = SourceFieldOwnership {
        policy: Some(Policy::WholeOwner(SourceWholeOwnerOwnership {})),
    };
    let plan = plan_merge(
        &original,
        &original,
        &incoming,
        &original,
        &policy,
        &mut budget(),
    )
    .expect("whole");
    assert_eq!(plan.proposed, incoming);
    let mut bad = incoming.clone();
    if let Some(trogon_atlas_proto::entity::Kind::ReadModel(ref mut model)) = bad.kind {
        model.id.as_mut().expect("id").slug = "other".into();
    }
    assert_eq!(
        plan_merge(
            &original,
            &original,
            &bad,
            &original,
            &policy,
            &mut budget()
        ),
        Err(SourceError::Identity)
    );
}
#[test]
fn budget_limits_are_inclusive_and_never_reset_on_failed_charge() {
    let mut limits = profile_budgets();
    limits.examined_occurrences = 1;
    let mut budget = RunBudget::new(limits).expect("budget");
    budget
        .charge(BudgetWork::Examinations, 1)
        .expect("boundary");
    assert_eq!(
        budget.charge(BudgetWork::Examinations, 1),
        Err(SourceError::Budget)
    );
    assert_eq!(
        budget.charge(BudgetWork::Examinations, u64::MAX),
        Err(SourceError::Budget)
    );
    assert!(budget.check_elapsed(60_000).is_ok());
    assert_eq!(budget.check_elapsed(60_001), Err(SourceError::Budget));
    let base = entity("base", "doc");
    assert_eq!(
        plan_merge(
            &base,
            &base,
            &base,
            &base,
            &ownership(&[SourceOwnedField::Title]),
            &mut budget
        ),
        Err(SourceError::Budget)
    );
}
#[test]
fn registration_cannot_raise_profile_budgets_or_retention() {
    let mut limits = profile_budgets();
    limits.native_operations = 1001;
    assert_eq!(validate_budgets(&limits), Err(SourceError::Budget));
    limits = profile_budgets();
    limits.phase_timeout_ms = 60_001;
    assert_eq!(validate_budgets(&limits), Err(SourceError::Budget));
    let mut retention = profile_retention();
    assert!(validate_retention(&retention).is_ok());
    retention.registration_bytes += 1;
    assert_eq!(validate_retention(&retention), Err(SourceError::Retention));
}

#[test]
fn oneof_body_is_an_indivisible_ownership_unit() {
    let make = |body: serde_json::Value| {
        let mut document = serde_json::json!({"id":{"namespace":"test","slug":"doc","version":1}});
        document
            .as_object_mut()
            .expect("object")
            .extend(body.as_object().expect("body").clone());
        decode(&serde_json::json!({"document":document})).expect("document")
    };
    let retained = make(serde_json::json!({"retained":{}}));
    let link = make(serde_json::json!({"link":{}}));
    let unit = OwnershipUnit::Field(SourceOwnedField::Body);
    assert_ne!(
        extract_unit(&retained, unit).expect("retained"),
        extract_unit(&link, unit).expect("link")
    );
    let mut changed = retained;
    replace_unit(
        &mut changed,
        unit,
        &extract_unit(&link, unit).expect("value"),
    )
    .expect("replace body");
    assert_eq!(changed, link);
}

#[test]
fn closed_ownership_enum_covers_every_native_writable_unit() {
    use prost_reflect::Kind;
    let wrapper = transcode::entity_descriptor().expect("entity descriptor");
    let fields =
        transcode::message_descriptor("trogonatlas.api.eventmodel.v1alpha1.SourceFieldOwnership")
            .expect("ownership descriptor");
    assert!(fields.get_field_by_name("policy").is_none());
    for variant in wrapper
        .oneofs()
        .find(|o| o.name() == "kind")
        .expect("kind")
        .fields()
    {
        let Kind::Message(owner) = variant.kind() else {
            panic!("owner message")
        };
        let names = owner
            .fields()
            .filter(|f| f.name() != "id" && f.containing_oneof().is_none_or(|o| o.is_synthetic()))
            .map(|f| f.name().to_owned())
            .chain(
                owner
                    .oneofs()
                    .filter(|o| !o.is_synthetic())
                    .map(|o| o.name().to_owned()),
            );
        for name in names {
            let enum_name = format!("SOURCE_OWNED_FIELD_{}", name.to_ascii_uppercase());
            if (name == "communications"
                && matches!(
                    owner.full_name(),
                    "trogonatlas.eventmodel.v1alpha1.Component"
                        | "trogonatlas.eventmodel.v1alpha1.ExternalSystem"
                ))
                || (name == "links"
                    && owner.full_name() == "trogonatlas.eventmodel.v1alpha1.Channel")
                || (name == "storage_claims"
                    && matches!(
                        owner.full_name(),
                        "trogonatlas.eventmodel.v1alpha1.Component"
                            | "trogonatlas.eventmodel.v1alpha1.ExternalSystem"
                            | "trogonatlas.eventmodel.v1alpha1.DataProduct"
                    ))
            {
                assert!(SourceOwnedField::from_str_name(&enum_name).is_none());
                continue;
            }
            let field = SourceOwnedField::from_str_name(&enum_name)
                .unwrap_or_else(|| panic!("missing unit {enum_name} on {}", owner.full_name()));
            let native = prost_reflect::DynamicMessage::new(owner.clone());
            let mut dynamic = prost_reflect::DynamicMessage::new(wrapper.clone());
            dynamic.set_field(&variant, prost_reflect::Value::Message(native));
            use prost::Message;
            let entity = Entity::decode(dynamic.encode_to_vec().as_slice()).expect("entity");
            let result = validate_ownership(&entity, &ownership(&[field]));
            if name == "relationships"
                && matches!(
                    owner.full_name(),
                    "trogonatlas.eventmodel.v1alpha1.Component"
                        | "trogonatlas.eventmodel.v1alpha1.ExternalSystem"
                )
            {
                assert_eq!(result, Err(SourceError::UnsupportedUnit));
            } else {
                assert!(result.is_ok(), "unit {enum_name} on {}", owner.full_name());
            }
        }
    }
}

fn contextual_equal(a: &OwnedValue, b: &OwnedValue) -> Result<bool, SourceError> {
    use prost_reflect::ReflectMessage;
    let types = transcode::TranscodePool::builtin().unwrap();
    let render = |value: &OwnedValue| {
        types
            .message_json_value(value.message().descriptor().full_name(), value.message())
            .map_err(|_| SourceError::NativeValue)
    };
    Ok(render(a)? == render(b)?)
}
fn contextual_plan(base: &Entity, incoming: &Entity) -> MergePlan {
    plan_merge_with_comparator(
        base,
        base,
        incoming,
        base,
        &SourceFieldOwnership {
            policy: Some(Policy::WholeOwner(SourceWholeOwnerOwnership {})),
        },
        &mut budget(),
        contextual_equal,
    )
    .unwrap()
}
#[test]
fn contextual_merge_preserves_optional_presence_repeated_order_and_oneof_selection() {
    let absent = decode(
        &serde_json::json!({"document":{"id":{"namespace":"test","slug":"doc","version":1}}}),
    )
    .unwrap();
    let present=decode(&serde_json::json!({"document":{"id":{"namespace":"test","slug":"doc","version":1},"doc":""}})).unwrap();
    assert_eq!(
        contextual_plan(&absent, &present).decisions[0].disposition,
        MergeDisposition::Update
    );
    let retained=decode(&serde_json::json!({"document":{"id":{"namespace":"test","slug":"doc","version":1},"retained":{}}})).unwrap();
    let link=decode(&serde_json::json!({"document":{"id":{"namespace":"test","slug":"doc","version":1},"link":{}}})).unwrap();
    assert_eq!(
        contextual_plan(&retained, &link).decisions[0].disposition,
        MergeDisposition::Update
    );
    let board = |slugs: &[&str]| {
        decode(&serde_json::json!({"storyboard":{"id":{"namespace":"test","slug":"board","version":1},"slices":slugs.iter().map(|slug|serde_json::json!({"id":{"namespace":"test","slug":slug,"version":1}})).collect::<Vec<_>>()}})).unwrap()
    };
    assert_eq!(
        contextual_plan(&board(&["a", "b", "a"]), &board(&["b", "a", "a"])).decisions[0]
            .disposition,
        MergeDisposition::Update
    );
}
#[test]
fn contextual_merge_compares_any_map_values_without_rewriting_input_or_default() {
    use trogon_atlas_proto as pb;
    let entry =
        |key: u8, value: u8| vec![0x0a, 0x08, 0x0a, 0x01, key, 0x12, 0x03, 0x1a, 0x01, value];
    let make = |reverse: bool, changed: bool| {
        let first = entry(b'a', if changed { b'3' } else { b'1' });
        let second = entry(b'b', b'2');
        let bytes = if reverse {
            [second, first].concat()
        } else {
            [first, second].concat()
        };
        Entity {
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: "test".into(),
                    slug: "event".into(),
                    version: 1,
                }),
                schema: Some(prost_types::Any {
                    type_url: "type.googleapis.com/google.protobuf.Struct".into(),
                    value: bytes,
                }),
                ..Default::default()
            })),
            ..Default::default()
        }
    };
    let base = make(false, false);
    let reordered = make(true, false);
    let original = reordered.clone();
    let own = SourceFieldOwnership {
        policy: Some(Policy::WholeOwner(SourceWholeOwnerOwnership {})),
    };
    assert_eq!(
        plan_merge(&base, &base, &reordered, &base, &own, &mut budget())
            .unwrap()
            .decisions[0]
            .disposition,
        MergeDisposition::Update
    );
    assert_eq!(
        contextual_plan(&base, &reordered).decisions[0].disposition,
        MergeDisposition::Unchanged
    );
    assert_eq!(
        contextual_plan(&base, &make(true, true)).decisions[0].disposition,
        MergeDisposition::Update
    );
    assert_eq!(reordered, original);
}
#[test]
fn contextual_comparison_failure_propagates_after_eager_historical_pair() {
    let base = entity("before", "doc");
    let incoming = entity("after", "doc");
    let mut calls = 0;
    let outcome = plan_merge_with_comparator(
        &base,
        &base,
        &incoming,
        &base,
        &ownership(&[SourceOwnedField::Title]),
        &mut budget(),
        |_, _| {
            calls += 1;
            if calls == 2 {
                Err(SourceError::NativeValue)
            } else {
                Ok(false)
            }
        },
    );
    assert_eq!(outcome.unwrap_err(), SourceError::NativeValue);
    assert_eq!(calls, 2);
}
