//! Rebuild the baseline as it stood right after a past changeset.
//!
//! The entities bucket keeps one revision per key, so the past is not stored
//! anywhere as a whole. It is derived: start from the current entities and
//! undo every newer changeset from the pre-images its revision rows recorded.

use std::collections::{HashMap, HashSet};

use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    refs::{entity_id, entity_kind},
    store::{ChangesetOp, ChangesetRecord},
    StoredEntity,
};

use crate::ownership::Lens;

/// Most baseline changesets an as-of read rewinds before it refuses. Each
/// one costs a store read per op, so an unbounded walk would let one read
/// hold a request open for as long as the log is long.
pub const MAX_REWIND_CHANGESETS: usize = 1000;

/// The ops of `record` an as-of read undoes for a caller limited to `lens`,
/// one per key.
///
/// The view is narrowed to `lens` anyway, so another owner's keys are left
/// alone: reading their revision rows would let their gaps fail this read and
/// put their ids in the error. Two ops on one key share one folded revision
/// row, so only the first is kept.
#[must_use]
pub fn ops_to_rewind<'a>(record: &'a ChangesetRecord, lens: &Lens) -> Vec<&'a ChangesetOp> {
    let mut seen = HashSet::new();
    record
        .ops
        .iter()
        .filter(|op| match op.entity_ref.id.as_ref() {
            Some(id) => {
                lens.admits(&id.namespace)
                    && seen.insert((
                        op.entity_ref.kind,
                        id.namespace.as_str(),
                        id.slug.as_str(),
                        id.version,
                    ))
            }
            // An op with no id names no namespace, so only a caller who sees
            // every namespace gets to trip over it.
            None => lens.is_unrestricted(),
        })
        .collect()
}

/// One entity's content right before a newer changeset wrote it. `None`
/// means that changeset created the key.
pub type PreImage = (pb::EntityKey, Option<pb::Entity>);

/// Undo `newest_first` on top of `current`.
///
/// A key that several changesets touched takes the pre-image of the oldest
/// one, because that is the content it had before any of them landed. Keys
/// no changeset touched pass through unchanged.
#[must_use]
pub fn rewind(current: &[StoredEntity], newest_first: Vec<PreImage>) -> Vec<StoredEntity> {
    let mut restored: HashMap<pb::EntityKey, Option<pb::Entity>> = HashMap::new();
    for (key, before) in newest_first {
        restored.insert(key, before);
    }

    let mut out = Vec::with_capacity(current.len());
    for stored in current {
        let key = entity_kind(&stored.entity)
            .zip(entity_id(&stored.entity))
            .map(|(kind, id)| pb::EntityKey::new(kind, id));
        match key.and_then(|k| restored.remove(&k)) {
            None => out.push(stored.clone()),
            Some(Some(entity)) => out.push(past(entity)),
            Some(None) => {}
        }
    }
    // What is left was deleted after the target changeset.
    out.extend(restored.into_values().flatten().map(past));
    out
}

/// A rewound entity has no etag: the KV revision it had then is gone, and an
/// as-of view is read-only, so nothing can CAS against it.
fn past(entity: pb::Entity) -> StoredEntity {
    StoredEntity {
        entity,
        etag: String::new(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::{collections::BTreeSet, sync::Arc};

    use trogon_atlas_core::NamespaceId;
    use trogon_atlas_store::store::ChangeKind;

    use super::*;

    fn op(namespace: &str, slug: &str) -> ChangesetOp {
        ChangesetOp {
            kind: ChangeKind::Put,
            entity_ref: pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(pb::Id {
                    namespace: namespace.into(),
                    slug: slug.into(),
                    version: 1,
                }),
            },
        }
    }

    fn changeset(ops: Vec<ChangesetOp>) -> ChangesetRecord {
        ChangesetRecord {
            id: "0190f5a0-0000-7000-8000-000000000000".into(),
            author: "acme-bot".into(),
            at: "2026-10-15T00:00:00Z".into(),
            message: String::new(),
            rpc: "PutEntity".into(),
            branch: None,
            ops,
            operation_id: None,
        }
    }

    fn only(namespaces: &[&str]) -> Lens {
        Lens::Only(Arc::new(
            namespaces
                .iter()
                .map(|ns| NamespaceId::parse(ns).unwrap())
                .collect::<BTreeSet<_>>(),
        ))
    }

    fn slugs(ops: &[&ChangesetOp]) -> Vec<String> {
        ops.iter()
            .map(|op| op.entity_ref.id.as_ref().unwrap().slug.clone())
            .collect()
    }

    #[test]
    fn ops_to_rewind_keeps_every_namespace_for_an_unrestricted_caller() {
        let record = changeset(vec![op("acme", "paid"), op("globex", "shipped")]);
        assert_eq!(
            slugs(&ops_to_rewind(&record, &Lens::Everything)),
            ["paid", "shipped"]
        );
    }

    #[test]
    fn ops_to_rewind_skips_namespaces_the_caller_cannot_read() {
        let record = changeset(vec![op("acme", "paid"), op("globex", "shipped")]);
        assert_eq!(slugs(&ops_to_rewind(&record, &only(&["acme"]))), ["paid"]);
    }

    #[test]
    fn ops_to_rewind_is_empty_for_another_owners_changeset() {
        let record = changeset(vec![op("globex", "shipped")]);
        assert!(ops_to_rewind(&record, &only(&["acme"])).is_empty());
    }

    #[test]
    fn ops_to_rewind_keeps_one_op_per_key() {
        let record = changeset(vec![op("acme", "paid"), op("acme", "paid")]);
        assert_eq!(slugs(&ops_to_rewind(&record, &Lens::Everything)), ["paid"]);
    }

    #[test]
    fn ops_to_rewind_drops_an_op_without_an_id_for_a_bound_caller() {
        let mut no_id = op("acme", "paid");
        no_id.entity_ref.id = None;
        let record = changeset(vec![no_id]);
        assert!(ops_to_rewind(&record, &only(&["acme"])).is_empty());
        assert_eq!(ops_to_rewind(&record, &Lens::Everything).len(), 1);
    }

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "acme".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn event(slug: &str, title: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(slug)),
                title: title.into(),
                ..Default::default()
            })),
        }
    }

    fn key(slug: &str) -> pb::EntityKey {
        pb::EntityKey::new(pb::EntityKind::Event, &id(slug))
    }

    fn stored(slug: &str, title: &str) -> StoredEntity {
        StoredEntity {
            entity: event(slug, title),
            etag: "7".into(),
        }
    }

    /// Slug to title, sorted, so the assertions ignore output order.
    fn titles(entities: &[StoredEntity]) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = entities
            .iter()
            .map(|se| match se.entity.kind.as_ref() {
                Some(pb::entity::Kind::Event(e)) => {
                    (e.id.as_ref().unwrap().slug.clone(), e.title.clone())
                }
                _ => panic!("expected an Event"),
            })
            .collect();
        out.sort();
        out
    }

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(s, t)| ((*s).to_string(), (*t).to_string()))
            .collect()
    }

    #[test]
    fn an_edit_restores_the_pre_image() {
        let current = vec![stored("order-placed", "Order was placed")];
        let out = rewind(
            &current,
            vec![(
                key("order-placed"),
                Some(event("order-placed", "Order placed")),
            )],
        );
        assert_eq!(titles(&out), pairs(&[("order-placed", "Order placed")]));
        assert_eq!(out[0].etag, "", "a rewound entity has no live etag");
    }

    #[test]
    fn a_delete_brings_the_entity_back() {
        let out = rewind(
            &[],
            vec![(
                key("cart-emptied"),
                Some(event("cart-emptied", "Cart emptied")),
            )],
        );
        assert_eq!(titles(&out), pairs(&[("cart-emptied", "Cart emptied")]));
    }

    #[test]
    fn a_create_removes_the_entity() {
        let current = vec![stored("refund-issued", "Refund issued")];
        let out = rewind(&current, vec![(key("refund-issued"), None)]);
        assert_eq!(titles(&out), pairs(&[]));
    }

    /// Created and then deleted after the target: absent now and absent then.
    #[test]
    fn add_then_delete_stays_absent() {
        let out = rewind(
            &[],
            vec![
                (
                    key("coupon-applied"),
                    Some(event("coupon-applied", "Coupon")),
                ),
                (key("coupon-applied"), None),
            ],
        );
        assert_eq!(titles(&out), pairs(&[]));
    }

    #[test]
    fn the_oldest_pre_image_wins_across_changesets() {
        let current = vec![stored("paid", "Payment settled")];
        let out = rewind(
            &current,
            vec![
                (key("paid"), Some(event("paid", "Payment captured"))),
                (key("paid"), Some(event("paid", "Paid"))),
            ],
        );
        assert_eq!(titles(&out), pairs(&[("paid", "Paid")]));
    }

    #[test]
    fn untouched_entities_pass_through_with_their_etag() {
        let current = vec![stored("shipped", "Shipped"), stored("paid", "Paid now")];
        let out = rewind(&current, vec![(key("paid"), Some(event("paid", "Paid")))]);
        assert_eq!(
            titles(&out),
            pairs(&[("paid", "Paid"), ("shipped", "Shipped")])
        );
        let shipped = out
            .iter()
            .find(|se| entity_id(&se.entity).is_some_and(|i| i.slug == "shipped"))
            .unwrap();
        assert_eq!(shipped.etag, "7");
    }
}
