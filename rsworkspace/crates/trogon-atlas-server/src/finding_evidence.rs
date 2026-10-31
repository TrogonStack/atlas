//! Typed causes attached to validation findings. Evidence is part of a
//! finding's identity, so every list leaves through [`canonical`] and the
//! identity key ignores the presentation-only `detail`.

use std::cmp::Ordering;

pub(crate) use pb::validation_evidence::Role;
use trogon_atlas_proto as pb;

/// One placement of a slice in a storyboard: the storyboard and the index
/// into its `slices` list.
#[derive(Clone, Copy)]
pub(crate) struct Occurrence<'a> {
    storyboard: &'a pb::Id,
    position: usize,
}

impl<'a> Occurrence<'a> {
    pub(crate) fn new(storyboard: &'a pb::Id, position: usize) -> Self {
        Self {
            storyboard,
            position,
        }
    }
}

impl From<Occurrence<'_>> for pb::StoryboardOccurrence {
    fn from(o: Occurrence<'_>) -> Self {
        Self {
            storyboard: Some(o.storyboard.clone()),
            position: u32::try_from(o.position).unwrap_or(u32::MAX),
        }
    }
}

/// Builder for one [`pb::ValidationEvidence`].
pub(crate) struct Evidence(pb::ValidationEvidence);

impl Evidence {
    pub(crate) fn new(role: Role, kind: pb::EntityKind, id: &pb::Id) -> Self {
        Self(pb::ValidationEvidence {
            role: role as i32,
            entity: Some(entity_ref(kind, id)),
            ..Default::default()
        })
    }

    pub(crate) fn at(mut self, field: impl Into<String>) -> Self {
        self.0.field = field.into();
        self
    }

    pub(crate) fn pointing_at(mut self, kind: pb::EntityKind, id: &pb::Id) -> Self {
        self.0.target = Some(entity_ref(kind, id));
        self
    }

    pub(crate) fn in_storyboard(mut self, occurrence: Occurrence<'_>) -> Self {
        self.0.occurrence = Some(occurrence.into());
        self
    }
}

fn entity_ref(kind: pb::EntityKind, id: &pb::Id) -> pb::EntityRef {
    pb::EntityRef {
        kind: kind as i32,
        id: Some(id.clone()),
    }
}

type RefKey = (i32, String, String, u64);
type OccurrenceKey = (String, String, u64, u32);

/// The parts of one evidence item that identify a finding. `detail` is
/// presentation only and stays out.
pub type EvidenceKey = (i32, RefKey, String, RefKey, OccurrenceKey);

fn ref_key(r: Option<&pb::EntityRef>) -> RefKey {
    r.map(|r| {
        let id = r.id.clone().unwrap_or_default();
        (r.kind, id.namespace, id.slug, id.version)
    })
    .unwrap_or_default()
}

fn occurrence_key(o: Option<&pb::StoryboardOccurrence>) -> OccurrenceKey {
    o.map(|o| {
        let id = o.storyboard.clone().unwrap_or_default();
        (id.namespace, id.slug, id.version, o.position)
    })
    .unwrap_or_default()
}

#[must_use]
pub fn evidence_key(e: &pb::ValidationEvidence) -> EvidenceKey {
    (
        e.role,
        ref_key(e.entity.as_ref()),
        e.field.clone(),
        ref_key(e.target.as_ref()),
        occurrence_key(e.occurrence.as_ref()),
    )
}

fn compare(a: &pb::ValidationEvidence, b: &pb::ValidationEvidence) -> Ordering {
    evidence_key(a)
        .cmp(&evidence_key(b))
        .then_with(|| a.detail.cmp(&b.detail))
}

/// Evidence in canonical order with exact duplicates removed, so the same
/// causes always produce the same list whatever order the validator walked
/// the store in.
pub(crate) fn canonical(items: impl IntoIterator<Item = Evidence>) -> Vec<pb::ValidationEvidence> {
    let mut out: Vec<pb::ValidationEvidence> = items.into_iter().map(|e| e.0).collect();
    out.sort_by(compare);
    out.dedup();
    out
}

/// One line per cause, for asserting evidence in tests:
/// `<role> <entity>[.<field>][ -> <target>][ @<storyboard slug>#<position>]`.
#[cfg(test)]
pub(crate) fn describe(evidence: &[pb::ValidationEvidence]) -> Vec<String> {
    let r = |r: Option<&pb::EntityRef>| {
        r.and_then(|r| {
            let kind = pb::EntityKind::try_from(r.kind).ok()?;
            Some(pb::canonical::id_string(kind, r.id.as_ref()?))
        })
    };
    evidence
        .iter()
        .map(|e| {
            let role = Role::try_from(e.role).unwrap_or_default().as_str_name();
            let mut out = format!(
                "{} {}",
                role.trim_start_matches("ROLE_").to_lowercase(),
                r(e.entity.as_ref()).unwrap_or_default()
            );
            if !e.field.is_empty() {
                out.push('.');
                out.push_str(&e.field);
            }
            if let Some(target) = r(e.target.as_ref()) {
                out.push_str(" -> ");
                out.push_str(&target);
            }
            if let Some(o) = e.occurrence.as_ref() {
                let sb = o
                    .storyboard
                    .as_ref()
                    .map(|s| s.slug.as_str())
                    .unwrap_or_default();
                out.push_str(" @");
                out.push_str(sb);
                out.push('#');
                out.push_str(&o.position.to_string());
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(slug: &str, version: u64) -> pb::Id {
        pb::Id {
            namespace: "shop".into(),
            slug: slug.into(),
            version,
        }
    }

    #[test]
    fn canonical_order_ignores_input_order_and_drops_duplicates() {
        let sb = id("checkout", 1);
        let build = |position| {
            Evidence::new(Role::Consumer, pb::EntityKind::ReadModelSlice, &id("rm", 1))
                .at("source_events[0]")
                .pointing_at(pb::EntityKind::Event, &id("placed", 1))
                .in_storyboard(Occurrence::new(&sb, position))
        };
        let issuer = || {
            Evidence::new(Role::Issuer, pb::EntityKind::CommandSlice, &id("a", 2))
                .at("ui")
                .pointing_at(pb::EntityKind::Ui, &id("form", 1))
        };
        let forward = canonical([issuer(), build(3), build(1), build(3)]);
        let backward = canonical([build(1), build(3), issuer()]);
        assert_eq!(forward, backward);
        let positions: Vec<Option<u32>> = forward
            .iter()
            .map(|e| e.occurrence.as_ref().map(|o| o.position))
            .collect();
        assert_eq!(positions, [None, Some(1), Some(3)]);
    }

    #[test]
    fn identity_key_ignores_detail() {
        let mut a = canonical([Evidence::new(
            Role::LatestVersion,
            pb::EntityKind::Event,
            &id("placed", 2),
        )]);
        let b = a.clone();
        a[0].detail = "note".into();
        assert_eq!(evidence_key(&a[0]), evidence_key(&b[0]));
    }
}
