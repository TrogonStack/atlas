use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::Arc,
};

use tonic::Status;
use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    refs::{entity_id as ref_entity_id, entity_kind as ref_entity_kind, outbound_refs},
    store::ListFilter,
    Store, StoredEntity,
};

use crate::conv::store_err;

#[derive(Debug, Clone)]
pub struct ReverseIndex {
    pub by_target: HashMap<pb::EntityKey, Vec<pb::Reference>>,
}

impl ReverseIndex {
    #[must_use]
    pub fn build(all: &[StoredEntity]) -> Self {
        // Known keys let untyped Storyboard slice refs resolve the same way
        // `closure()` does: probe CommandSlice / ReadModelSlice / AutomationSlice.
        let known: HashSet<pb::EntityKey> = all
            .iter()
            .filter_map(|se| {
                let k = ref_entity_kind(&se.entity)?;
                let i = ref_entity_id(&se.entity)?;
                Some(pb::EntityKey::new(k, i))
            })
            .collect();
        let mut by_target: HashMap<pb::EntityKey, Vec<pb::Reference>> = HashMap::new();
        for se in all {
            let (Some(from_kind), Some(from_id)) =
                (ref_entity_kind(&se.entity), ref_entity_id(&se.entity))
            else {
                continue;
            };
            let from_ref = pb::EntityRef {
                kind: from_kind as i32,
                id: Some(from_id.clone()),
            };
            for o in outbound_refs(&se.entity) {
                for to_kind in resolve_to_kinds(o.to_kind, &o.to_id, &known) {
                    let key = pb::EntityKey::new(to_kind, &o.to_id);
                    by_target.entry(key).or_default().push(pb::Reference {
                        from: Some(from_ref.clone()),
                        from_field: o.from_field.clone(),
                        to: Some(pb::EntityRef {
                            kind: to_kind as i32,
                            id: Some(o.to_id.clone()),
                        }),
                    });
                }
            }
        }
        let owners = TypeOwners::build(all);
        for r in all.iter().filter_map(|se| owners.schema_ref(&se.entity)) {
            if let Some(to) = r.to.as_ref().and_then(|to| to.id.as_ref()) {
                by_target
                    .entry(pb::EntityKey::new(pb::EntityKind::TypeLibrary, to))
                    .or_default()
                    .push(r);
            }
        }
        Self { by_target }
    }

    #[must_use]
    pub fn incoming(&self, kind: pb::EntityKind, id: &pb::Id) -> &[pb::Reference] {
        let key = pb::EntityKey::new(kind, id);
        self.by_target
            .get(&key)
            .map_or(&[], std::vec::Vec::as_slice)
    }
}

/// The live type library that owns each tenant message, so an Event,
/// Command or ReadModel whose `schema` names a tenant type refers to the
/// library declaring it.
#[derive(Debug, Default)]
pub struct TypeOwners {
    live: HashMap<String, BTreeMap<String, pb::Id>>,
}

impl TypeOwners {
    #[must_use]
    pub fn build(all: &[StoredEntity]) -> Self {
        Self::of(all.iter().map(|se| &se.entity))
    }

    fn of<'a>(entities: impl IntoIterator<Item = &'a pb::Entity>) -> Self {
        let mut live: HashMap<String, BTreeMap<String, pb::Id>> = HashMap::new();
        for entity in entities {
            if ref_entity_kind(entity) != Some(pb::EntityKind::TypeLibrary) {
                continue;
            }
            let Some(id) = ref_entity_id(entity) else {
                continue;
            };
            let slugs = live.entry(id.namespace.clone()).or_default();
            match slugs.get(&id.slug) {
                Some(current) if current.version >= id.version => {}
                _ => {
                    slugs.insert(id.slug.clone(), id.clone());
                }
            }
        }
        Self { live }
    }

    fn owner(&self, namespace: &str, full_name: &str) -> Option<&pb::Id> {
        self.live
            .get(namespace)?
            .iter()
            .find(|(slug, _)| {
                full_name
                    .strip_prefix(slug.as_str())
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
            })
            .map(|(_, id)| id)
    }

    /// The reference from `entity`'s `schema` to the library owning its
    /// type, when that type is a tenant one.
    #[must_use]
    pub fn schema_ref(&self, entity: &pb::Entity) -> Option<pb::Reference> {
        let schema = trogon_atlas_core::schema::entity_schema(entity)?;
        if trogon_atlas_core::schema::is_schema(schema) {
            return None;
        }
        let (from_kind, from_id) = (ref_entity_kind(entity)?, ref_entity_id(entity)?);
        let owner = self.owner(
            &from_id.namespace,
            trogon_atlas_core::schema::type_name(schema),
        )?;
        Some(pb::Reference {
            from: Some(pb::EntityRef {
                kind: from_kind as i32,
                id: Some(from_id.clone()),
            }),
            from_field: SCHEMA_FIELD.to_owned(),
            to: Some(pb::EntityRef {
                kind: pb::EntityKind::TypeLibrary as i32,
                id: Some(owner.clone()),
            }),
        })
    }
}

/// The `schema` references that would point at one of `libraries` once
/// `puts` are written over `before` and every key in `removed` is gone.
#[must_use]
pub fn schema_users<S: std::hash::BuildHasher>(
    before: &[StoredEntity],
    puts: &[&pb::Entity],
    removed: &HashSet<pb::EntityKey, S>,
    libraries: &HashSet<pb::EntityKey, S>,
) -> Vec<pb::Reference> {
    let key_of = |entity: &pb::Entity| {
        Some(pb::EntityKey::new(
            ref_entity_kind(entity)?,
            ref_entity_id(entity)?,
        ))
    };
    let overwritten: HashSet<pb::EntityKey> = puts.iter().filter_map(|e| key_of(e)).collect();
    let world: Vec<&pb::Entity> = before
        .iter()
        .map(|se| &se.entity)
        .filter(|e| key_of(e).is_none_or(|k| !overwritten.contains(&k)))
        .chain(puts.iter().copied())
        .collect();
    let owners = TypeOwners::of(world.iter().copied());
    world
        .into_iter()
        .filter(|e| key_of(e).is_none_or(|k| !removed.contains(&k)))
        .filter_map(|e| owners.schema_ref(e))
        .filter(|r| {
            r.to.as_ref()
                .and_then(|to| to.id.as_ref())
                .is_some_and(|id| {
                    libraries.contains(&pb::EntityKey::new(pb::EntityKind::TypeLibrary, id))
                })
        })
        .collect()
}

/// The `from_field` of a reference from an entity's `schema` to the type
/// library declaring its type.
pub const SCHEMA_FIELD: &str = "schema";

pub struct LoadAllResult {
    pub entities: Vec<StoredEntity>,
    /// True when the store returned exactly the cap, meaning more entities
    /// may exist beyond what was loaded. Callers that need the complete set
    /// (e.g. `delete_by_query`) must treat a truncated snapshot as an error.
    pub truncated: bool,
}

pub async fn load_all(
    store: &Arc<dyn Store>,
    branch: Option<&str>,
) -> Result<LoadAllResult, Status> {
    load_all_with_cap(store, crate::service::MAX_PROJECTION_ENTITIES, branch).await
}

pub async fn load_all_with_cap(
    store: &Arc<dyn Store>,
    cap: usize,
    branch: Option<&str>,
) -> Result<LoadAllResult, Status> {
    let entities = store
        .list(ListFilter::default(), Some(cap), branch)
        .await
        .map_err(store_err)?;
    let truncated = entities.len() >= cap;
    if truncated {
        tracing::warn!(
            cap,
            "snapshot loaded exactly the entity cap; store may have more entities \
             than are visible to read paths"
        );
    }
    Ok(LoadAllResult {
        entities,
        truncated,
    })
}

/// Resolve an outbound target kind. Typed refs pass through; untyped slice
/// refs (Storyboard `slices[]` / trace steps) probe the four concrete slice
/// kinds against `known`, matching `closure()`.
fn resolve_to_kinds<S: std::hash::BuildHasher>(
    to_kind: Option<pb::EntityKind>,
    to_id: &pb::Id,
    known: &HashSet<pb::EntityKey, S>,
) -> Vec<pb::EntityKind> {
    if let Some(k) = to_kind {
        vec![k]
    } else {
        for probe in [
            pb::EntityKind::CommandSlice,
            pb::EntityKind::ReadModelSlice,
            pb::EntityKind::AutomationSlice,
            pb::EntityKind::UiSlice,
        ] {
            if known.contains(&pb::EntityKey::new(probe, to_id)) {
                return vec![probe];
            }
        }
        Vec::new()
    }
}

#[must_use]
pub fn outgoing_of(entity: &pb::Entity) -> Vec<pb::Reference> {
    outgoing_of_resolved(entity, &HashSet::new())
}

/// Like [`outgoing_of`], but resolves intentionally-untyped slice refs against
/// `known` (same probe order as [`closure`]).
#[must_use]
pub fn outgoing_of_resolved<S: std::hash::BuildHasher>(
    entity: &pb::Entity,
    known: &HashSet<pb::EntityKey, S>,
) -> Vec<pb::Reference> {
    let (Some(from_kind), Some(from_id)) = (ref_entity_kind(entity), ref_entity_id(entity)) else {
        return Vec::new();
    };
    let from_ref = pb::EntityRef {
        kind: from_kind as i32,
        id: Some(from_id.clone()),
    };
    let mut out = Vec::new();
    for o in outbound_refs(entity) {
        for to_kind in resolve_to_kinds(o.to_kind, &o.to_id, known) {
            out.push(pb::Reference {
                from: Some(from_ref.clone()),
                from_field: o.from_field.clone(),
                to: Some(pb::EntityRef {
                    kind: to_kind as i32,
                    id: Some(o.to_id.clone()),
                }),
            });
        }
    }
    out
}

/// Which end of a `pb::Reference` is the "other entity" relative to the
/// caller. For incoming refs (queried entity is `to`), the other end is
/// `from`: the referrer. For outgoing refs (queried entity is `from`),
/// the other end is `to`: the target. The earlier implementation always
/// preferred `from` with a `to` fallback, which silently filtered the
/// wrong side for outgoing references.
#[derive(Clone, Copy, Debug)]
pub enum RefSide {
    From,
    To,
}

#[must_use]
pub fn filter_refs_by_kinds(
    refs: Vec<pb::Reference>,
    kinds: &[pb::EntityKind],
    other_side: RefSide,
) -> Vec<pb::Reference> {
    if kinds.is_empty() {
        return refs;
    }
    let set: HashSet<i32> = kinds.iter().map(|k| *k as i32).collect();
    refs.into_iter()
        .filter(|r| {
            let endpoint = match other_side {
                RefSide::From => r.from.as_ref(),
                RefSide::To => r.to.as_ref(),
            };
            let other_kind = endpoint.map_or(0, |e| e.kind);
            set.contains(&other_kind)
        })
        .collect()
}

/// Supersession ancestors: walk `supersedes` links backward from (kind, id),
/// returning the chain in order of descending recency (the requested entity
/// first, then its predecessor, etc.).
#[must_use]
pub fn supersession_history(
    all: &[StoredEntity],
    kind: pb::EntityKind,
    id: &pb::Id,
) -> Vec<pb::EntityRef> {
    let by_key: HashMap<pb::EntityKey, &StoredEntity> = all
        .iter()
        .filter_map(|se| {
            let k = ref_entity_kind(&se.entity)?;
            let i = ref_entity_id(&se.entity)?;
            Some((pb::EntityKey::new(k, i), se))
        })
        .collect();

    let mut chain: Vec<pb::EntityRef> = Vec::new();
    let mut seen: HashSet<pb::EntityKey> = HashSet::new();
    let mut cursor: Option<(pb::EntityKind, pb::Id)> = Some((kind, id.clone()));
    while let Some((k, cur_id)) = cursor.take() {
        let key = pb::EntityKey::new(k, &cur_id);
        if !seen.insert(key.clone()) {
            break;
        }
        chain.push(pb::EntityRef {
            kind: k as i32,
            id: Some(cur_id.clone()),
        });
        if let Some(se) = by_key.get(&key) {
            if let Some(s) = supersedes_of(&se.entity) {
                cursor = Some((k, s.clone()));
            }
        }
    }
    chain
}

/// Supersession descendants: walk forward to find entities whose `supersedes`
/// equals our target, recursively.
///
/// Same-kind-only by design. The `parents` index is keyed by
/// `(child_kind, parent_id)`, so a supersession that crosses kinds
/// (e.g. an event superseding a command) does NOT propagate. Cross-kind
/// supersession isn't a meaningful operation in the schema (every kind's
/// `supersedes` field references its own kind), but document it here so a
/// future widening (e.g. "deprecate this command in favor of an event") is
/// a deliberate choice rather than a silent gap in the walk.
pub fn supersession_descendants(
    all: &[StoredEntity],
    kind: pb::EntityKind,
    id: &pb::Id,
) -> Vec<pb::EntityRef> {
    let mut parents: HashMap<pb::EntityKey, Vec<pb::EntityRef>> = HashMap::new();
    for se in all {
        let (Some(child_kind), Some(child_id)) =
            (ref_entity_kind(&se.entity), ref_entity_id(&se.entity))
        else {
            continue;
        };
        if let Some(parent) = supersedes_of(&se.entity) {
            let key = pb::EntityKey::new(child_kind, parent);
            parents.entry(key).or_default().push(pb::EntityRef {
                kind: child_kind as i32,
                id: Some(child_id.clone()),
            });
        }
    }
    let mut out: Vec<pb::EntityRef> = Vec::new();
    let mut stack: Vec<(pb::EntityKind, pb::Id)> = vec![(kind, id.clone())];
    let mut seen: HashSet<pb::EntityKey> = HashSet::new();
    while let Some((k, cur_id)) = stack.pop() {
        let key = pb::EntityKey::new(k, &cur_id);
        if let Some(children) = parents.get(&key) {
            for c in children {
                let Some(cid) = c.id.as_ref() else { continue };
                let ck = match pb::EntityKind::try_from(c.kind) {
                    Ok(k) if k != pb::EntityKind::Unspecified => k,
                    _ => {
                        tracing::warn!(
                            kind_raw = c.kind,
                            "supersession_descendants: skipping node with unknown proto kind"
                        );
                        continue;
                    }
                };
                let ckey = pb::EntityKey::new(ck, cid);
                if seen.insert(ckey) {
                    out.push(c.clone());
                    stack.push((ck, cid.clone()));
                }
            }
        }
    }
    out
}

#[must_use]
pub fn entity_lookup(all: &[StoredEntity]) -> HashMap<pb::EntityKey, pb::Entity> {
    all.iter()
        .filter_map(|se| {
            let k = ref_entity_kind(&se.entity)?;
            let i = ref_entity_id(&se.entity)?;
            Some((pb::EntityKey::new(k, i), se.entity.clone()))
        })
        .collect()
}

pub struct Closure {
    pub entities: HashMap<String, pb::Entity>,
    pub truncated: bool,
}

pub fn closure(all: &[StoredEntity], roots: &[pb::EntityRef], max_entities: usize) -> Closure {
    let by_key = entity_lookup(all);
    let mut out: HashMap<String, pb::Entity> = HashMap::new();
    let mut truncated = false;
    let mut queue: VecDeque<(pb::EntityKind, pb::Id)> = VecDeque::new();
    for r in roots {
        let Some(id) = r.id.as_ref() else { continue };
        let Ok(k) = pb::EntityKind::try_from(r.kind) else {
            continue;
        };
        queue.push_back((k, id.clone()));
    }
    while let Some((k, id)) = queue.pop_front() {
        let canon = trogon_atlas_proto::canonical::id_string(k, &id);
        if out.contains_key(&canon) {
            continue;
        }
        if out.len() >= max_entities {
            truncated = true;
            break;
        }
        let key = pb::EntityKey::new(k, &id);
        let Some(entity) = by_key.get(&key) else {
            continue;
        };
        out.insert(canon, entity.clone());
        for o in outbound_refs(entity) {
            match o.to_kind {
                None => {
                    // Intentionally-untyped slice ref: probe each concrete slice kind.
                    for probe in [
                        pb::EntityKind::CommandSlice,
                        pb::EntityKind::ReadModelSlice,
                        pb::EntityKind::AutomationSlice,
                        pb::EntityKind::UiSlice,
                    ] {
                        let pk = pb::EntityKey::new(probe, &o.to_id);
                        if by_key.contains_key(&pk) {
                            queue.push_back((probe, o.to_id.clone()));
                            break;
                        }
                    }
                }
                Some(to_kind) => {
                    queue.push_back((to_kind, o.to_id.clone()));
                }
            }
        }
    }
    if truncated {
        tracing::warn!(cap = max_entities, "graph closure truncated");
        metrics::counter!("trogon_atlas_closure_truncations_total").increment(1);
    }
    Closure {
        entities: out,
        truncated,
    }
}

#[must_use]
pub fn resolve_entities(all: &[StoredEntity], refs: &[pb::EntityRef]) -> Vec<pb::Entity> {
    let by_key: HashMap<pb::EntityKey, pb::Entity> = all
        .iter()
        .filter_map(|se| {
            let k = ref_entity_kind(&se.entity)?;
            let i = ref_entity_id(&se.entity)?;
            Some((pb::EntityKey::new(k, i), se.entity.clone()))
        })
        .collect();
    refs.iter()
        .filter_map(|r| {
            let id = r.id.as_ref()?;
            let k = pb::EntityKind::try_from(r.kind).ok()?;
            by_key.get(&pb::EntityKey::new(k, id)).cloned()
        })
        .collect()
}

fn supersedes_of(entity: &pb::Entity) -> Option<&pb::Id> {
    use pb::entity::Kind as K;
    match entity.kind.as_ref()? {
        K::Event(x) => x.supersedes.as_ref(),
        K::Command(x) => x.supersedes.as_ref(),
        K::ReadModel(x) => x.supersedes.as_ref(),
        K::Processor(x) => x.supersedes.as_ref(),
        K::Ui(x) => x.supersedes.as_ref(),
        K::Persona(x) => x.supersedes.as_ref(),
        K::Swimlane(x) => x.supersedes.as_ref(),
        K::CommandSlice(x) => x.supersedes.as_ref(),
        K::ReadModelSlice(x) => x.supersedes.as_ref(),
        K::AutomationSlice(x) => x.supersedes.as_ref(),
        K::Storyboard(x) => x.supersedes.as_ref(),
        K::EventModel(x) => x.supersedes.as_ref(),
        K::Component(x) => x.supersedes.as_ref(),
        K::ExternalSystem(x) => x.supersedes.as_ref(),
        K::Tracker(x) => x.supersedes.as_ref(),
        K::BoundedContext(x) => x.supersedes.as_ref(),
        K::Domain(x) => x.supersedes.as_ref(),
        K::Subdomain(x) => x.supersedes.as_ref(),
        K::Schema(x) => x.supersedes.as_ref(),
        K::Project(x) => x.supersedes.as_ref(),
        K::Screen(x) => x.supersedes.as_ref(),
        K::Term(x) => x.supersedes.as_ref(),
        K::Ambiguity(x) => x.supersedes.as_ref(),
        K::UiSlice(x) => x.supersedes.as_ref(),
        K::ServiceLevelIndicator(x) => x.supersedes.as_ref(),
        K::ServiceLevelObjective(x) => x.supersedes.as_ref(),
        K::AlertPolicy(x) => x.supersedes.as_ref(),
        K::AlertNotificationTarget(x) => x.supersedes.as_ref(),
        K::TypeLibrary(x) => x.supersedes.as_ref(),
    }
}

/// Breadth-first impact analysis from a root `EntityRef`. Each visited node
/// records hop count and the path of references taken to reach it.
pub fn impact(
    all: &[StoredEntity],
    root_kind: pb::EntityKind,
    root_id: &pb::Id,
    max_depth: u32,
    follow_incoming: bool,
) -> Vec<pb::ImpactNode> {
    let known: HashSet<pb::EntityKey> = all
        .iter()
        .filter_map(|se| {
            let k = ref_entity_kind(&se.entity)?;
            let i = ref_entity_id(&se.entity)?;
            Some(pb::EntityKey::new(k, i))
        })
        .collect();
    let owners = TypeOwners::build(all);
    let outbound: HashMap<pb::EntityKey, Vec<pb::Reference>> = all
        .iter()
        .filter_map(|se| {
            let k = ref_entity_kind(&se.entity)?;
            let i = ref_entity_id(&se.entity)?;
            let mut refs = outgoing_of_resolved(&se.entity, &known);
            refs.extend(owners.schema_ref(&se.entity));
            Some((pb::EntityKey::new(k, i), refs))
        })
        .collect();
    let reverse = ReverseIndex::build(all);

    let root_ref = pb::EntityRef {
        kind: root_kind as i32,
        id: Some(root_id.clone()),
    };
    let mut out: Vec<pb::ImpactNode> = vec![pb::ImpactNode {
        entity: Some(root_ref.clone()),
        depth: 0,
        path: Vec::new(),
    }];

    let mut visited: HashSet<pb::EntityKey> = HashSet::new();
    visited.insert(pb::EntityKey::new(root_kind, root_id));

    let mut queue: VecDeque<(pb::EntityKind, pb::Id, u32, Vec<pb::Reference>)> = VecDeque::new();
    queue.push_back((root_kind, root_id.clone(), 0, Vec::new()));

    while let Some((k, id, depth, path)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        let key = pb::EntityKey::new(k, &id);
        // Tag each ref with the side we should walk to. For an outbound
        // ref (`from = self`), the neighbor is `r.to`. For an incoming
        // ref pulled from the reverse index (`to = self`), the neighbor
        // is `r.from`. Folding both into one list with a single rule
        // (the prior code: "prefer r.from when follow_incoming") sent
        // outbound traversals back to the current node and silently
        // dropped every outbound neighbor.
        let mut next_refs: Vec<(pb::Reference, bool)> = Vec::new();
        if let Some(rs) = outbound.get(&key) {
            next_refs.extend(rs.iter().cloned().map(|r| (r, false)));
        }
        if follow_incoming {
            next_refs.extend(reverse.incoming(k, &id).iter().cloned().map(|r| (r, true)));
        }
        for (r, is_incoming) in next_refs {
            let other = if is_incoming {
                r.from.clone()
            } else {
                r.to.clone()
            };
            let Some(other) = other else { continue };
            let Some(oid) = other.id.as_ref() else {
                continue;
            };
            let ok = match pb::EntityKind::try_from(other.kind) {
                Ok(k) if k != pb::EntityKind::Unspecified => k,
                _ => {
                    tracing::warn!(
                        kind_raw = other.kind,
                        "impact: skipping node with unknown proto kind in BFS traversal"
                    );
                    continue;
                }
            };
            let okey = pb::EntityKey::new(ok, oid);
            if !visited.insert(okey.clone()) {
                continue;
            }
            let mut new_path = path.clone();
            new_path.push(r);
            out.push(pb::ImpactNode {
                entity: Some(pb::EntityRef {
                    kind: ok as i32,
                    id: Some(oid.clone()),
                }),
                depth: depth + 1,
                path: new_path.clone(),
            });
            queue.push_back((ok, oid.clone(), depth + 1, new_path));
        }
    }
    out
}

#[cfg(test)]
mod impact_tests {
    use trogon_atlas_store::store::StoredEntity;

    use super::*;

    fn id(ns: &str, slug: &str) -> pb::Id {
        pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored_command_slice(
        ns: &str,
        slug: &str,
        emits_event: Option<(&str, &str)>,
    ) -> StoredEntity {
        let mut slice = pb::CommandSlice {
            id: Some(id(ns, slug)),
            title: slug.into(),
            ..Default::default()
        };
        if let Some((ens, eslug)) = emits_event {
            slice.emitted_events = vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id(ens, eslug)),
                }),
                ..Default::default()
            }];
        }
        StoredEntity {
            entity: pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::CommandSlice(slice)),
            },
            etag: String::new(),
        }
    }

    fn stored_event(ns: &str, slug: &str) -> StoredEntity {
        StoredEntity {
            entity: pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::Event(pb::Event {
                    id: Some(id(ns, slug)),
                    title: slug.into(),
                    ..Default::default()
                })),
            },
            etag: String::new(),
        }
    }

    /// Regression test for the `follow_incoming=true` ref-side bug: an
    /// outbound ref from the root must visit `r.to`, not `r.from`. Before
    /// the fix, every impact traversal with `follow_incoming=true`
    /// silently dropped outbound neighbors and reported only the root.
    #[test]
    fn impact_visits_outbound_neighbor_when_follow_incoming() {
        let all = vec![
            stored_command_slice("orders", "place-slice", Some(("orders", "placed"))),
            stored_event("orders", "placed"),
        ];
        let nodes = impact(
            &all,
            pb::EntityKind::CommandSlice,
            &id("orders", "place-slice"),
            3,
            /*follow_incoming=*/ true,
        );
        let entity_ids: Vec<String> = nodes
            .iter()
            .filter_map(|n| {
                let e = n.entity.as_ref()?;
                let i = e.id.as_ref()?;
                Some(format!("{:?}/{}/{}", e.kind, i.namespace, i.slug))
            })
            .collect();
        assert!(
            entity_ids.iter().any(|s| s.contains("placed")),
            "impact dropped the outbound event: {entity_ids:?}"
        );
        assert!(
            entity_ids.iter().any(|s| s.contains("place")),
            "root must always be present: {entity_ids:?}"
        );
    }

    /// `filter_refs_by_kinds` must filter the *other* end of the ref,
    /// distinguished by `RefSide`. An incoming ref's referrer kind is in
    /// `from`; an outgoing ref's target kind is in `to`. The earlier
    /// implementation always preferred `from`, which silently filtered
    /// the wrong side for outgoing references.
    #[test]
    fn filter_refs_by_kinds_uses_correct_side() {
        let r = pb::Reference {
            from: Some(pb::EntityRef {
                kind: pb::EntityKind::Command as i32,
                id: Some(id("a", "b")),
            }),
            to: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("a", "c")),
            }),
            ..Default::default()
        };
        let kept_to = filter_refs_by_kinds(vec![r.clone()], &[pb::EntityKind::Event], RefSide::To);
        assert_eq!(kept_to.len(), 1, "RefSide::To should match `to.kind=Event`");
        let kept_from =
            filter_refs_by_kinds(vec![r.clone()], &[pb::EntityKind::Event], RefSide::From);
        assert_eq!(
            kept_from.len(),
            0,
            "RefSide::From should NOT match `from.kind=Command`"
        );
    }

    #[test]
    fn bug_reverse_index_includes_storyboard_slice_refs() {
        // Storyboard.slices[] emits outbound_refs with to_kind=None.
        // ReverseIndex::build currently `continue`s on None, so a CommandSlice
        // never learns which Storyboards point at it, unlike closure(), which
        // probes the four concrete slice kinds.
        let slice_id = id("orders", "place-slice");
        let board = StoredEntity {
            entity: pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::Storyboard(pb::Storyboard {
                    id: Some(id("orders", "happy-path")),
                    title: "happy".into(),
                    slices: vec![pb::SliceRef {
                        id: Some(slice_id.clone()),
                    }],
                    ..Default::default()
                })),
            },
            etag: String::new(),
        };
        let slice = stored_command_slice("orders", "place-slice", None);
        let index = ReverseIndex::build(&[board, slice]);
        let incoming = index.incoming(pb::EntityKind::CommandSlice, &slice_id);
        assert!(
            !incoming.is_empty(),
            "ReverseIndex must surface Storyboard→CommandSlice via untyped slice ref; got {incoming:?}"
        );
    }

    fn stored_ui(ns: &str, slug: &str, screen: Option<(&str, &str)>) -> StoredEntity {
        let mut ui = pb::Ui {
            id: Some(id(ns, slug)),
            title: slug.into(),
            ..Default::default()
        };
        if let Some((sns, sslug)) = screen {
            ui.slot = Some(pb::ScreenSlotRef {
                screen: Some(pb::ScreenRef {
                    id: Some(id(sns, sslug)),
                }),
                slot: "main".into(),
            });
        }
        StoredEntity {
            entity: pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::Ui(ui)),
            },
            etag: String::new(),
        }
    }

    fn stored_screen(ns: &str, slug: &str) -> StoredEntity {
        StoredEntity {
            entity: pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::Screen(pb::Screen {
                    id: Some(id(ns, slug)),
                    title: slug.into(),
                    ..Default::default()
                })),
            },
            etag: String::new(),
        }
    }

    fn stored_term(
        ns: &str,
        slug: &str,
        embodied: Option<(pb::EntityKind, &str, &str)>,
    ) -> StoredEntity {
        let mut term = pb::Term {
            id: Some(id(ns, slug)),
            title: slug.into(),
            ..Default::default()
        };
        if let Some((k, ens, eslug)) = embodied {
            term.embodied_by = vec![pb::EntityRef {
                kind: k as i32,
                id: Some(id(ens, eslug)),
            }];
        }
        StoredEntity {
            entity: pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::Term(term)),
            },
            etag: String::new(),
        }
    }

    fn stored_ambiguity(ns: &str, slug: &str, terms: &[(&str, &str)]) -> StoredEntity {
        StoredEntity {
            entity: pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::Ambiguity(pb::Ambiguity {
                    id: Some(id(ns, slug)),
                    terms: terms
                        .iter()
                        .map(|(tns, tslug)| pb::TermRef {
                            id: Some(id(tns, tslug)),
                        })
                        .collect(),
                    ..Default::default()
                })),
            },
            etag: String::new(),
        }
    }

    #[test]
    fn reverse_index_includes_ui_slot_screen() {
        let screen_id = id("app", "checkout");
        let ui = stored_ui("checkout", "order-panel", Some(("app", "checkout")));
        let screen = stored_screen("app", "checkout");
        let index = ReverseIndex::build(&[ui, screen]);
        let incoming = index.incoming(pb::EntityKind::Screen, &screen_id);
        assert!(
            !incoming.is_empty(),
            "ReverseIndex must surface Ui→Screen via slot.screen; got {incoming:?}"
        );
    }

    #[test]
    fn impact_from_screen_reaches_ui_contributor() {
        let all = vec![
            stored_ui("checkout", "order-panel", Some(("app", "checkout"))),
            stored_screen("app", "checkout"),
        ];
        let nodes = impact(
            &all,
            pb::EntityKind::Screen,
            &id("app", "checkout"),
            2,
            true,
        );
        let has_ui = nodes.iter().any(|n| {
            n.entity
                .as_ref()
                .is_some_and(|e| e.kind == pb::EntityKind::Ui as i32)
        });
        assert!(
            has_ui,
            "impact from Screen must reach contributing Ui; got {nodes:?}"
        );
    }

    #[test]
    fn closure_from_ui_includes_screen() {
        let all = vec![
            stored_ui("checkout", "order-panel", Some(("app", "checkout"))),
            stored_screen("app", "checkout"),
        ];
        let root = pb::EntityRef {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id("checkout", "order-panel")),
        };
        let c = closure(&all, &[root], 100);
        let has_screen = c
            .entities
            .values()
            .any(|e| matches!(e.kind.as_ref(), Some(pb::entity::Kind::Screen(_))));
        assert!(
            has_screen,
            "closure from Ui must include slot Screen; keys={:?}",
            c.entities.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn reverse_index_includes_ambiguity_terms() {
        let term_id = id("checkout", "review");
        let amb = stored_ambiguity(
            "marketplace",
            "review-collision",
            &[("checkout", "review"), ("payments", "review")],
        );
        let t1 = stored_term("checkout", "review", None);
        let t2 = stored_term("payments", "review", None);
        let index = ReverseIndex::build(&[amb, t1, t2]);
        let incoming = index.incoming(pb::EntityKind::Term, &term_id);
        assert!(
            !incoming.is_empty(),
            "ReverseIndex must surface Ambiguity→Term; got {incoming:?}"
        );
    }

    #[test]
    fn impact_from_term_reaches_ambiguity() {
        let all = vec![
            stored_ambiguity(
                "marketplace",
                "review-collision",
                &[("checkout", "review"), ("payments", "review")],
            ),
            stored_term("checkout", "review", None),
            stored_term("payments", "review", None),
        ];
        let nodes = impact(
            &all,
            pb::EntityKind::Term,
            &id("checkout", "review"),
            2,
            true,
        );
        let has_amb = nodes.iter().any(|n| {
            n.entity
                .as_ref()
                .is_some_and(|e| e.kind == pb::EntityKind::Ambiguity as i32)
        });
        assert!(
            has_amb,
            "impact from Term must reach Ambiguity; got {nodes:?}"
        );
    }

    #[test]
    fn closure_from_ambiguity_includes_terms() {
        let all = vec![
            stored_ambiguity(
                "marketplace",
                "review-collision",
                &[("checkout", "review"), ("payments", "review")],
            ),
            stored_term("checkout", "review", None),
            stored_term("payments", "review", None),
        ];
        let root = pb::EntityRef {
            kind: pb::EntityKind::Ambiguity as i32,
            id: Some(id("marketplace", "review-collision")),
        };
        let c = closure(&all, &[root], 100);
        assert_eq!(
            c.entities.len(),
            3,
            "Ambiguity closure must include both Terms; keys={:?}",
            c.entities.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn reverse_index_includes_term_embodied_by() {
        let event_id = id("checkout", "placed");
        let term = stored_term(
            "checkout",
            "order",
            Some((pb::EntityKind::Event, "checkout", "placed")),
        );
        let event = stored_event("checkout", "placed");
        let index = ReverseIndex::build(&[term, event]);
        let incoming = index.incoming(pb::EntityKind::Event, &event_id);
        assert!(
            incoming.iter().any(|r| r
                .from
                .as_ref()
                .is_some_and(|f| f.kind == pb::EntityKind::Term as i32)),
            "ReverseIndex must surface Term→Event via embodied_by; got {incoming:?}"
        );
    }

    #[test]
    fn impact_reaches_shared_emitter_sibling() {
        // Two command slices emit the same event. Impact from one slice with
        // follow_incoming must reach the sibling emitter through the shared event.
        let all = vec![
            stored_command_slice("orders", "place-slice", Some(("orders", "placed"))),
            stored_command_slice("orders", "admin-place-slice", Some(("orders", "placed"))),
            stored_event("orders", "placed"),
        ];
        let nodes = impact(
            &all,
            pb::EntityKind::CommandSlice,
            &id("orders", "place-slice"),
            3,
            true,
        );
        let has_sibling = nodes.iter().any(|n| {
            n.entity
                .as_ref()
                .and_then(|e| e.id.as_ref())
                .is_some_and(|i| i.slug == "admin-place-slice")
        });
        assert!(
            has_sibling,
            "impact must reach shared-emitter sibling via Event; got {:?}",
            nodes
                .iter()
                .filter_map(|n| n
                    .entity
                    .as_ref()
                    .and_then(|e| e.id.as_ref())
                    .map(|i| i.slug.as_str()))
                .collect::<Vec<_>>()
        );
    }
}
