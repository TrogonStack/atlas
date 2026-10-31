#![allow(clippy::unwrap_used, clippy::expect_used)]
use prost::Message;
use trogon_atlas_core::source_integration::*;
use trogon_atlas_proto::{source_field_ownership::Policy, *};

fn id() -> Id {
    Id {
        namespace: "platform".into(),
        slug: "owner".into(),
        version: 1,
    }
}
fn whole() -> SourceFieldOwnership {
    SourceFieldOwnership {
        policy: Some(Policy::WholeOwner(SourceWholeOwnerOwnership {})),
    }
}
fn title_only() -> SourceFieldOwnership {
    SourceFieldOwnership {
        policy: Some(Policy::Fields(SourceOwnedFields {
            fields: vec![SourceOwnedField::Title as i32],
        })),
    }
}
fn budget() -> RunBudget {
    RunBudget::new(profile_budgets()).unwrap()
}
fn claim() -> StorageAccessClaim {
    StorageAccessClaim {
        store: Some(DataStoreRef {
            id: Some(Id {
                namespace: "platform".into(),
                slug: "receipts".into(),
                version: 1,
            }),
        }),
        direction: StorageDirection::Read as i32,
        ..Default::default()
    }
}
fn link() -> ChannelLink {
    ChannelLink {
        kind: ChannelLinkKind::Composes as i32,
        target: Some(ChannelRef {
            id: Some(Id {
                namespace: "platform".into(),
                slug: "other".into(),
                version: 1,
            }),
        }),
        ..Default::default()
    }
}
fn owner(kind: &str, title: &str, populated: bool) -> Entity {
    Entity {
        kind: Some(match kind {
            "channel" => entity::Kind::Channel(Channel {
                id: Some(id()),
                title: title.into(),
                links: if populated { vec![link()] } else { vec![] },
                ..Default::default()
            }),
            "component" => entity::Kind::Component(Component {
                id: Some(id()),
                title: title.into(),
                storage_claims: if populated { vec![claim()] } else { vec![] },
                ..Default::default()
            }),
            "external_system" => entity::Kind::ExternalSystem(ExternalSystem {
                id: Some(id()),
                title: title.into(),
                storage_claims: if populated { vec![claim()] } else { vec![] },
                ..Default::default()
            }),
            _ => entity::Kind::DataProduct(DataProduct {
                id: Some(id()),
                title: title.into(),
                storage_claims: if populated { vec![claim()] } else { vec![] },
                ..Default::default()
            }),
        }),
        ..Default::default()
    }
}
const OWNERS: [&str; 4] = ["channel", "component", "external_system", "data_product"];

#[test]
fn whole_owner_rejects_proposed_or_current_native_content_without_blanket_rejection() {
    for kind in OWNERS {
        let empty = owner(kind, "old", false);
        let populated = owner(kind, "old", true);
        assert!(validate_ownership(&empty, &whole()).is_ok(), "{kind}");
        assert_eq!(
            validate_ownership(&populated, &whole()),
            Err(SourceError::UnsupportedUnit),
            "{kind}"
        );
        assert!(
            validate_ownership(&populated, &title_only()).is_ok(),
            "{kind}"
        );
        for (incoming, current, label) in [
            (&populated, &empty, "proposed content"),
            (&empty, &populated, "clearing current content"),
            (&populated, &populated, "both sides"),
        ] {
            let result = plan_merge(&empty, &empty, incoming, current, &whole(), &mut budget());
            assert_eq!(
                result.err(),
                Some(SourceError::UnsupportedUnit),
                "{kind} {label}"
            );
        }
        plan_merge(&empty, &empty, &empty, &empty, &whole(), &mut budget()).unwrap();
    }
}

#[test]
fn unrelated_title_updates_preserve_local_collections_byte_for_byte() {
    for kind in OWNERS {
        let base = owner(kind, "old", false);
        let current = owner(kind, "old", true);
        let incoming = owner(kind, "new", false);
        let plan = plan_merge(
            &base,
            &base,
            &incoming,
            &current,
            &title_only(),
            &mut budget(),
        )
        .unwrap();
        let expected = owner(kind, "new", true);
        assert_eq!(
            plan.proposed.encode_to_vec(),
            expected.encode_to_vec(),
            "{kind}"
        );
    }
}
