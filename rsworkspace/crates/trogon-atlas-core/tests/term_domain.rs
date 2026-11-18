use trogon_atlas_core::{outbound_refs, refs::retarget_refs};
use trogon_atlas_proto::{self as pb, entity};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "vocabulary".into(),
        slug: slug.into(),
        version: 1,
    }
}

#[test]
fn exact_domain_is_a_plain_reference_and_can_be_retargeted() {
    let mut owner = pb::Entity {
        kind: Some(entity::Kind::Term(pb::Term {
            id: Some(id("word")),
            owning_domain: Some(pb::DomainRef {
                id: Some(id("domain")),
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    let references = outbound_refs(&owner);
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].from_field, "owning_domain");
    assert_eq!(references[0].to_kind, Some(pb::EntityKind::Domain));
    assert_eq!(references[0].to_id, id("domain"));
    let changed = retarget_refs(
        &mut owner,
        pb::EntityKind::Domain,
        &id("domain"),
        &id("replacement"),
    );
    assert_eq!(changed, vec!["owning_domain"]);
    assert_eq!(outbound_refs(&owner)[0].to_id, id("replacement"));
}

#[test]
fn namespace_never_implies_ownership() {
    let owner = pb::Entity {
        kind: Some(entity::Kind::Term(pb::Term {
            id: Some(id("word")),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert!(outbound_refs(&owner).is_empty());
}

#[test]
fn source_title_updates_preserve_native_domain_and_whole_owner_cannot_clear_it(
) -> Result<(), Box<dyn std::error::Error>> {
    use trogon_atlas_core::source_integration::{extract_unit, replace_unit, OwnershipUnit};
    let mut local = pb::Entity {
        kind: Some(entity::Kind::Term(pb::Term {
            id: Some(id("word")),
            title: "local".into(),
            owning_domain: Some(pb::DomainRef {
                id: Some(id("domain")),
            }),
            ..Default::default()
        })),
        ..Default::default()
    };
    let incoming = pb::Entity {
        kind: Some(entity::Kind::Term(pb::Term {
            id: Some(id("word")),
            title: "source".into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert!(extract_unit(&local, OwnershipUnit::WholeOwner).is_err());
    let clear = extract_unit(&incoming, OwnershipUnit::WholeOwner)?;
    assert!(replace_unit(&mut local, OwnershipUnit::WholeOwner, &clear).is_err());
    let title = OwnershipUnit::Field(pb::SourceOwnedField::Title);
    let update = extract_unit(&incoming, title)?;
    replace_unit(&mut local, title, &update)?;
    let Some(entity::Kind::Term(term)) = local.kind else {
        panic!("term expected")
    };
    assert_eq!(term.title, "source");
    assert_eq!(
        term.owning_domain,
        Some(pb::DomainRef {
            id: Some(id("domain"))
        })
    );
    Ok(())
}
