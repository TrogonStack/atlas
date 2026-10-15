//! Rebuild the baseline as it stood right after a past changeset.
//!
//! The entities bucket keeps one revision per key, so the past is not stored
//! anywhere as a whole. It is derived: start from the current entities and
//! undo every newer changeset from the pre-images its revision rows recorded.

use std::collections::HashMap;

use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    refs::{entity_id, entity_kind},
    StoredEntity,
};

/// Most baseline changesets an as-of read rewinds before it refuses. Each
/// one costs a store read per op, so an unbounded walk would let one read
/// hold a request open for as long as the log is long.
pub const MAX_REWIND_CHANGESETS: usize = 1000;

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

    use super::*;

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
