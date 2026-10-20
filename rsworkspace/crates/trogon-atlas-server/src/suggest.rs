//! Near-match slugs for a direct read whose key does not resolve.

use trogon_atlas_proto as pb;
use trogon_atlas_store::StoredEntity;

use crate::conv::{entity_id, entity_kind};

/// Enough to catch a typo or a missing suffix without burying the answer.
pub const MAX_SUGGESTIONS: usize = 3;

/// Slugs of the same kind in the same namespace as `missing`, closest first.
///
/// `entities` must already be the caller's lensed snapshot: anything in it
/// can be named back to the caller, so it must hold nothing they cannot see.
pub fn did_you_mean(
    kind: pb::EntityKind,
    missing: &pb::Id,
    entities: &[StoredEntity],
) -> Vec<String> {
    let mut ranked: Vec<(u8, usize, &str)> = entities
        .iter()
        .filter(|se| entity_kind(&se.entity).ok() == Some(kind))
        .filter_map(|se| entity_id(&se.entity).ok())
        .filter(|id| id.namespace == missing.namespace)
        .filter_map(|id| {
            let (tier, distance) = rank(&missing.slug, &id.slug)?;
            Some((tier, distance, id.slug.as_str()))
        })
        .collect();
    ranked.sort_unstable();
    ranked.dedup_by(|a, b| a.2 == b.2);
    ranked
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, _, slug)| slug.to_string())
        .collect()
}

/// A prefix or substring match beats a raw edit distance, because a slug
/// that is missing its suffix is the common mistake and can be far apart in
/// edits (`place-order` vs `place-order-slice`).
fn rank(missing: &str, candidate: &str) -> Option<(u8, usize)> {
    if candidate == missing {
        return None;
    }
    let distance = edit_distance(missing, candidate);
    let tier = if candidate.starts_with(missing) || missing.starts_with(candidate) {
        0
    } else if candidate.contains(missing) || missing.contains(candidate) {
        1
    } else if distance <= (missing.len() / 3).max(1) {
        2
    } else {
        return None;
    };
    Some((tier, distance))
}

/// Levenshtein distance over bytes; slugs are ASCII by validation.
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitute = prev[j] + usize::from(ca != cb);
            cur[j + 1] = substitute.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(namespace: &str, slug: &str) -> pb::Id {
        pb::Id {
            namespace: namespace.into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(kind: pb::entity::Kind) -> StoredEntity {
        StoredEntity {
            entity: pb::Entity {
                system: None,
                kind: Some(kind),
            },
            etag: String::new(),
        }
    }

    fn command_slice(namespace: &str, slug: &str) -> StoredEntity {
        stored(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(namespace, slug)),
            ..Default::default()
        }))
    }

    fn event(namespace: &str, slug: &str) -> StoredEntity {
        stored(pb::entity::Kind::Event(pb::Event {
            id: Some(id(namespace, slug)),
            ..Default::default()
        }))
    }

    #[test]
    fn a_missing_suffix_ranks_above_a_closer_typo() {
        let entities = [
            command_slice("acme-storefront", "plac-order"),
            command_slice("acme-storefront", "place-order-slice"),
        ];
        let got = did_you_mean(
            pb::EntityKind::CommandSlice,
            &id("acme-storefront", "place-order"),
            &entities,
        );
        assert_eq!(got, ["place-order-slice", "plac-order"]);
    }

    #[test]
    fn only_the_same_kind_and_namespace_are_suggested() {
        let entities = [
            event("acme-storefront", "place-order-slice"),
            command_slice("acme-billing", "place-order-slice"),
            command_slice("acme-storefront", "place-order-v2"),
        ];
        let got = did_you_mean(
            pb::EntityKind::CommandSlice,
            &id("acme-storefront", "place-order"),
            &entities,
        );
        assert_eq!(got, ["place-order-v2"]);
    }

    #[test]
    fn at_most_three_distinct_slugs_are_suggested() {
        let entities: Vec<_> = ["order-a", "order-b", "order-c", "order-d", "order-a"]
            .into_iter()
            .map(|slug| command_slice("acme-storefront", slug))
            .collect();
        let got = did_you_mean(
            pb::EntityKind::CommandSlice,
            &id("acme-storefront", "order"),
            &entities,
        );
        assert_eq!(got, ["order-a", "order-b", "order-c"]);
    }

    #[test]
    fn a_typo_is_suggested_but_an_unrelated_slug_is_not() {
        let entities = [
            command_slice("acme-storefront", "place-ordr"),
            command_slice("acme-storefront", "refund-payment"),
        ];
        let got = did_you_mean(
            pb::EntityKind::CommandSlice,
            &id("acme-storefront", "place-order"),
            &entities,
        );
        assert_eq!(got, ["place-ordr"]);
    }
}
