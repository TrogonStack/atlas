//! Structural three-way merge over canonical JSON.
//!
//! Stage 2 of the resolution ladder in `docs/explanation/branching.md`.
//! Stage 1 resolves an edit/edit conflict by picking a whole entity: ours,
//! theirs, or one written by hand. That is coarse in the case that actually
//! dominates once agents write concurrently, where baseline edited one field
//! and the branch edited a different one and nothing really collided.
//!
//! This module answers the narrower question: given the base an entity was
//! forked from, is every difference attributable to exactly one side? If so
//! the merge is mechanical. If both sides moved the same path to different
//! values, that is a real conflict and stays one.
//!
//! # Why JSON and not the typed entity
//!
//! Entities are 23 oneof variants. A typed merge would be 23 hand-written
//! mergers that drift the moment a field is added, and `crate::diff` already
//! demonstrates the maintenance cost of that shape. Canonical proto3 JSON
//! ([`trogon_atlas_core::transcode`]) is the same representation content
//! hashing, semantic equality, and conflict field paths already use, so a
//! merge expressed over it agrees with all three by construction and covers
//! every kind, including kinds that do not exist yet.
//!
//! # What is deliberately not merged
//!
//! - **Arrays are atomic.** `conflict_field_paths` reports *into* arrays by
//!   index because naming a position helps a human read a conflict. Merging
//!   by index would be a different claim: that position `i` means the same
//!   member on both sides. One insertion makes that false and silently
//!   shuffles fields between members. Merging lists by identity (scenarios
//!   by id, members as sets) is stage 3 and needs per-field strategy, so
//!   until then an array both sides changed differently is a conflict.
//! - **The `system` block.** Server-owned provenance, not content, and
//!   cleared here for the same reason [`trogon_atlas_core::entity_content_hash`]
//!   clears it. The merged entity carries `ours`'s block forward, which is
//!   what the ordinary merge write would have done, and the store re-stamps
//!   it on write regardless.
//! - **A changed entity kind.** Merging an Event with a Command would emit
//!   an object with two oneof arms set, which is not a representable entity.
//!   That is a conflict at the root.

use serde_json::Value;
use trogon_atlas_proto as pb;

/// A canonical-JSON path into an entity, rendered the way branch conflict
/// reports render it: `doc`, `fields[2].name`, `<root>` for the whole
/// entity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FieldPath(String);

impl FieldPath {
    #[must_use]
    pub fn root() -> Self {
        Self(String::new())
    }

    #[must_use]
    fn key(&self, key: &str) -> Self {
        if self.0.is_empty() {
            Self(key.to_owned())
        } else {
            Self(format!("{}.{key}", self.0))
        }
    }

    /// The rendered form. The root has no name of its own, so it borrows the
    /// same `<root>` placeholder `conflict_field_paths` uses.
    #[must_use]
    pub fn as_str(&self) -> &str {
        if self.0.is_empty() {
            "<root>"
        } else {
            &self.0
        }
    }
}

impl std::fmt::Display for FieldPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The paths where both sides moved away from the base, and disagreed.
///
/// Never empty: an empty set is a merge, and the two are different answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictPaths(Vec<FieldPath>);

impl ConflictPaths {
    #[must_use]
    pub fn rendered(&self) -> Vec<String> {
        self.0.iter().map(|p| p.as_str().to_owned()).collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The outcome of a three-way merge.
#[derive(Debug, Clone)]
pub enum Merge3 {
    /// Every difference came from exactly one side. The entity is the
    /// combination.
    Merged(Box<pb::Entity>),
    /// At least one path moved on both sides, to different values.
    Conflict(ConflictPaths),
}

/// Errors that make a merge unanswerable rather than conflicted.
///
/// Kept distinct from [`Merge3::Conflict`] on purpose: a conflict is a
/// finding about the model, while these are failures to read or rebuild it.
/// A caller that cannot merge should fall back to reporting the conflict it
/// already had, not invent one.
#[derive(Debug, thiserror::Error)]
pub enum Merge3Error {
    #[error("rendering an entity as canonical JSON: {0}")]
    Transcode(#[from] trogon_atlas_core::transcode::TranscodeError),
    #[error("the merged JSON is not a representable entity: {0}")]
    NotAnEntity(#[source] trogon_atlas_core::transcode::TranscodeError),
}

/// Merge `ours` and `theirs` over their common `base`.
///
/// `base` is the entity as it stood when the branch first touched the key
/// (the copy-on-write base), `ours` is the branch's value, `theirs` is the
/// current baseline.
///
/// # Errors
///
/// [`Merge3Error`] when an input cannot be rendered as canonical JSON, or
/// when the merged tree does not read back as an entity.
pub fn three_way(
    base: &pb::Entity,
    ours: &pb::Entity,
    theirs: &pb::Entity,
) -> Result<Merge3, Merge3Error> {
    let base_json = content_json(base)?;
    let ours_json = content_json(ours)?;
    let theirs_json = content_json(theirs)?;

    let mut conflicts = Vec::new();
    let merged_json = merge_value(
        &base_json,
        &ours_json,
        &theirs_json,
        &FieldPath::root(),
        &mut conflicts,
    );

    if !conflicts.is_empty() {
        conflicts.sort();
        conflicts.dedup();
        return Ok(Merge3::Conflict(ConflictPaths(conflicts)));
    }

    let mut merged: pb::Entity = trogon_atlas_core::transcode::message_from_json_value(
        "trogonatlas.eventmodel.v1alpha1.Entity",
        &merged_json,
    )
    .map_err(Merge3Error::NotAnEntity)?;
    merged.system.clone_from(&ours.system);
    Ok(Merge3::Merged(Box::new(merged)))
}

/// Canonical JSON with the server-owned `system` block removed, so
/// provenance cannot manufacture a conflict.
fn content_json(
    entity: &pb::Entity,
) -> Result<Value, trogon_atlas_core::transcode::TranscodeError> {
    let mut stripped = entity.clone();
    stripped.system = None;
    trogon_atlas_core::transcode::entity_json_value(&stripped)
}

/// The merge rule, applied at every node.
///
/// The three equality checks come first and in this order because they are
/// what makes the merge sound: a node where the two sides agree needs no
/// decision at all (even if both moved), and a node where one side still
/// matches the base has exactly one author. Only when all three differ is
/// there anything to recurse into or conflict over.
///
/// Missing keys read as `Value::Null`. proto3 JSON omits default-valued
/// fields, so "absent" and "set to the default" are already the same state
/// upstream of here and treating them alike keeps this consistent with
/// content hashing and semantic equality.
fn merge_value(
    base: &Value,
    ours: &Value,
    theirs: &Value,
    path: &FieldPath,
    conflicts: &mut Vec<FieldPath>,
) -> Value {
    if ours == theirs {
        return ours.clone();
    }
    if ours == base {
        return theirs.clone();
    }
    if theirs == base {
        return ours.clone();
    }

    if let (Value::Object(b), Value::Object(o), Value::Object(t)) = (base, ours, theirs) {
        let mut keys: std::collections::BTreeSet<&String> = std::collections::BTreeSet::new();
        keys.extend(b.keys());
        keys.extend(o.keys());
        keys.extend(t.keys());
        let mut merged = serde_json::Map::with_capacity(keys.len());
        for key in keys {
            let child = merge_value(
                b.get(key).unwrap_or(&Value::Null),
                o.get(key).unwrap_or(&Value::Null),
                t.get(key).unwrap_or(&Value::Null),
                &path.key(key),
                conflicts,
            );
            // Re-omit what proto3 JSON omits, so the merged tree reads
            // back the way it was rendered.
            if !child.is_null() {
                merged.insert(key.clone(), child);
            }
        }
        Value::Object(merged)
    } else {
        conflicts.push(path.clone());
        // The value is unused: a non-empty conflict list discards the
        // whole tree. Returning `ours` keeps the recursion total.
        ours.clone()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn event(title: &str, doc: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: "shop".into(),
                    slug: "order-placed".into(),
                    version: 1,
                }),
                title: title.into(),
                doc: doc.into(),
                ..Default::default()
            })),
        }
    }

    fn with_fields(mut entity: pb::Entity, names: &[&str]) -> pb::Entity {
        if let Some(pb::entity::Kind::Event(ref mut e)) = entity.kind {
            e.schema = Some(trogon_atlas_core::schema::pack_fields(
                names
                    .iter()
                    .map(|n| pb::FieldSpec {
                        name: (*n).into(),
                        ..Default::default()
                    })
                    .collect(),
            ));
        }
        entity
    }

    fn merged(result: Merge3) -> pb::Entity {
        match result {
            Merge3::Merged(entity) => *entity,
            Merge3::Conflict(paths) => {
                panic!("expected a merge, got conflicts at {:?}", paths.rendered())
            }
        }
    }

    fn conflicts(result: Merge3) -> Vec<String> {
        match result {
            Merge3::Conflict(paths) => paths.rendered(),
            Merge3::Merged(_) => panic!("expected a conflict, got a merge"),
        }
    }

    fn event_parts(entity: &pb::Entity) -> (String, String) {
        match &entity.kind {
            Some(pb::entity::Kind::Event(e)) => (e.title.clone(), e.doc.clone()),
            other => panic!("expected an Event, got {other:?}"),
        }
    }

    /// The case the whole stage exists for: baseline edited one field while
    /// the branch edited another. Nothing collided, so nothing should be
    /// asked of a human.
    #[test]
    fn disjoint_field_edits_merge() {
        let base = event("Order Placed", "the original doc");
        let ours = event("Order Was Placed", "the original doc");
        let theirs = event("Order Placed", "a rewritten doc");

        let out = merged(three_way(&base, &ours, &theirs).unwrap());
        assert_eq!(
            event_parts(&out),
            ("Order Was Placed".to_owned(), "a rewritten doc".to_owned())
        );
    }

    #[test]
    fn the_same_path_changed_differently_is_a_conflict() {
        let base = event("Order Placed", "d");
        let ours = event("Ours", "d");
        let theirs = event("Theirs", "d");

        assert_eq!(
            conflicts(three_way(&base, &ours, &theirs).unwrap()),
            ["event.title"]
        );
    }

    /// Both sides making the *same* edit is agreement, not collision.
    #[test]
    fn the_same_path_changed_identically_is_not_a_conflict() {
        let base = event("Order Placed", "d");
        let ours = event("Agreed", "d");
        let theirs = event("Agreed", "d");

        let out = merged(three_way(&base, &ours, &theirs).unwrap());
        assert_eq!(event_parts(&out).0, "Agreed");
    }

    #[test]
    fn an_untouched_side_contributes_nothing() {
        let base = event("Order Placed", "d");
        let ours = event("Ours", "d");

        let out = merged(three_way(&base, &ours, &base).unwrap());
        assert_eq!(event_parts(&out).0, "Ours");

        let out = merged(three_way(&base, &base, &ours).unwrap());
        assert_eq!(event_parts(&out).0, "Ours");
    }

    /// Adding a field on one side and editing a scalar on the other is the
    /// motivating example from `branching.md`.
    #[test]
    fn a_list_added_on_one_side_merges_with_a_scalar_edit_on_the_other() {
        let base = event("Order Placed", "d");
        let ours = with_fields(base.clone(), &["order_id"]);
        let theirs = event("Order Placed", "a rewritten doc");

        let out = merged(three_way(&base, &ours, &theirs).unwrap());
        let (_, doc) = event_parts(&out);
        assert_eq!(doc, "a rewritten doc");
        match &out.kind {
            Some(pb::entity::Kind::Event(e)) => {
                let fields = trogon_atlas_core::schema::schema_fields(e.schema.as_ref());
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].name, "order_id");
            }
            other => panic!("expected an Event, got {other:?}"),
        }
    }

    /// Index-wise array merging would silently interleave members. Both
    /// sides changing one list is a conflict at the list, not a guess.
    #[test]
    fn both_sides_changing_one_list_conflicts_at_the_list() {
        let base = with_fields(event("t", "d"), &["a"]);
        let ours = with_fields(event("t", "d"), &["a", "ours"]);
        let theirs = with_fields(event("t", "d"), &["a", "theirs"]);

        assert_eq!(
            conflicts(three_way(&base, &ours, &theirs).unwrap()),
            ["event.schema.fields"]
        );
    }

    /// A field only one side touched still merges when the other side left
    /// the whole list alone.
    #[test]
    fn one_side_changing_a_list_merges() {
        let base = with_fields(event("t", "d"), &["a"]);
        let ours = with_fields(event("t", "d"), &["a", "b"]);
        let theirs = with_fields(event("Retitled", "d"), &["a"]);

        let out = merged(three_way(&base, &ours, &theirs).unwrap());
        assert_eq!(event_parts(&out).0, "Retitled");
        match &out.kind {
            Some(pb::entity::Kind::Event(e)) => assert_eq!(
                trogon_atlas_core::schema::schema_fields(e.schema.as_ref()).len(),
                2
            ),
            other => panic!("expected an Event, got {other:?}"),
        }
    }

    /// Two arms of the entity oneof cannot both be set, so there is no
    /// merged entity to produce.
    #[test]
    fn a_changed_kind_conflicts_instead_of_producing_a_two_headed_entity() {
        let base = event("t", "d");
        let ours = event("Ours", "d");
        let theirs = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Command(pb::Command {
                id: Some(pb::Id {
                    namespace: "shop".into(),
                    slug: "order-placed".into(),
                    version: 1,
                }),
                title: "Theirs".into(),
                ..Default::default()
            })),
        };

        let paths = conflicts(three_way(&base, &ours, &theirs).unwrap());
        assert!(
            !paths.is_empty(),
            "a kind change must not merge into a two-armed entity"
        );
    }

    /// `system` is provenance. Two different stamps must not read as two
    /// authors disagreeing.
    #[test]
    fn a_differing_system_block_does_not_manufacture_a_conflict() {
        let base = event("Order Placed", "d");
        let mut ours = event("Ours", "d");
        ours.system = Some(pb::SystemMeta {
            uid: "branch-uid".into(),
            ..Default::default()
        });
        let mut theirs = event("Order Placed", "a rewritten doc");
        theirs.system = Some(pb::SystemMeta {
            uid: "baseline-uid".into(),
            ..Default::default()
        });

        let out = merged(three_way(&base, &ours, &theirs).unwrap());
        assert_eq!(
            event_parts(&out),
            ("Ours".to_owned(), "a rewritten doc".to_owned())
        );
        assert_eq!(
            out.system.as_ref().map(|s| s.uid.as_str()),
            Some("branch-uid"),
            "the merged entity carries the branch's stamp, as an ordinary merge write would"
        );
    }

    /// Clearing a field on one side is an edit like any other: proto3 JSON
    /// omits it, so it must not read as "unchanged".
    #[test]
    fn clearing_a_field_on_one_side_merges() {
        let base = event("Order Placed", "the original doc");
        let ours = event("Order Placed", "");
        let theirs = event("Retitled", "the original doc");

        let out = merged(three_way(&base, &ours, &theirs).unwrap());
        assert_eq!(event_parts(&out), ("Retitled".to_owned(), String::new()));
    }

    #[test]
    fn clearing_the_same_field_to_different_values_conflicts() {
        let base = event("Order Placed", "the original doc");
        let ours = event("Order Placed", "");
        let theirs = event("Order Placed", "a rewritten doc");

        assert_eq!(
            conflicts(three_way(&base, &ours, &theirs).unwrap()),
            ["event.doc"]
        );
    }

    /// Identical inputs are the degenerate case and must round-trip rather
    /// than being rebuilt into something subtly different.
    #[test]
    fn three_identical_entities_merge_to_the_same_entity() {
        let base = with_fields(event("t", "d"), &["a", "b"]);
        let out = merged(three_way(&base, &base, &base).unwrap());
        assert!(trogon_atlas_core::semantically_equal(&out, &base));
    }

    /// Order must not depend on which side is called ours: a conflict is
    /// symmetric, and a merge of disjoint edits is the same entity either
    /// way.
    #[test]
    fn the_merge_is_symmetric_in_its_two_sides() {
        let base = event("Order Placed", "the original doc");
        let ours = event("Order Was Placed", "the original doc");
        let theirs = event("Order Placed", "a rewritten doc");

        let forward = merged(three_way(&base, &ours, &theirs).unwrap());
        let backward = merged(three_way(&base, &theirs, &ours).unwrap());
        assert!(trogon_atlas_core::semantically_equal(&forward, &backward));
    }

    #[test]
    fn conflict_paths_are_sorted_and_deduplicated() {
        let base = event("t", "d");
        let ours = event("ours-title", "ours-doc");
        let theirs = event("theirs-title", "theirs-doc");

        assert_eq!(
            conflicts(three_way(&base, &ours, &theirs).unwrap()),
            ["event.doc", "event.title"]
        );
    }
}
