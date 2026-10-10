use std::collections::{HashMap, HashSet};

use trogon_atlas_proto as pb;
use trogon_atlas_store::StoredEntity;

use crate::graph;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlowSourceRole {
    Direct,
    Auxiliary,
}

#[derive(Clone, Debug)]
pub struct FlowEndpoint {
    pub entity_ref: pb::EntityRef,
    pub fields: Vec<pb::FieldSpec>,
    pub source_role: FlowSourceRole,
}

#[derive(Clone, Debug, Default)]
pub struct FlowLink {
    pub upstream: Vec<FlowEndpoint>,
    pub downstream: Vec<FlowEndpoint>,
}

#[must_use]
pub fn infer_data_flow(
    scope: &pb::AnalysisScope,
    all: &[StoredEntity],
) -> Vec<pb::InferredFieldMapping> {
    let links = resolve_scope(scope, all);
    let mut out: Vec<pb::InferredFieldMapping> = Vec::new();
    for link in &links {
        for down in &link.downstream {
            for f in &down.fields {
                out.push(map_field(&down.entity_ref, f, &link.upstream));
            }
        }
    }
    out
}

pub fn check_completeness(
    scope: &pb::AnalysisScope,
    all: &[StoredEntity],
) -> (Vec<pb::CompletenessGap>, f32) {
    let links = resolve_scope(scope, all);
    let mut gaps: Vec<pb::CompletenessGap> = Vec::new();
    let mut total_down_fields: usize = 0;
    let mut traced: usize = 0;

    let mut declared_seen: HashSet<(i32, String, String, u64)> = HashSet::new();

    for link in &links {
        for up in &link.upstream {
            note_undeclared(up, &mut gaps, &mut declared_seen);
        }
        for down in &link.downstream {
            note_undeclared(down, &mut gaps, &mut declared_seen);
        }

        let mut consumed_up: HashSet<(String, String, u64, String)> = HashSet::new();

        for down in &link.downstream {
            for f in &down.fields {
                total_down_fields += 1;
                if is_derived_field(f) {
                    traced += 1;
                    continue;
                }
                let matches = candidate_sources(&f.name, &link.upstream);
                match matches.len() {
                    0 => {
                        gaps.push(pb::CompletenessGap {
                            kind: pb::completeness_gap::Kind::MissingSource as i32,
                            entity: Some(down.entity_ref.clone()),
                            field_path: f.name.clone(),
                            explanation: format!(
                                "no upstream field named '{}' was found in scope",
                                f.name
                            ),
                            confidence: 0.8,
                        });
                    }
                    1 => {
                        let (up_ref, up_field) = &matches[0];
                        if !trogon_atlas_proto::canonical::field_types_compatible(
                            up_field.r#type.as_ref(),
                            f.r#type.as_ref(),
                        ) {
                            gaps.push(pb::CompletenessGap {
                                kind: pb::completeness_gap::Kind::ShapeMismatch as i32,
                                entity: Some(down.entity_ref.clone()),
                                field_path: f.name.clone(),
                                explanation: format!(
                                    "upstream '{}' has kind {}; downstream has kind {}",
                                    up_field.name,
                                    up_field.r#type.as_ref().map_or(
                                        "unspecified",
                                        trogon_atlas_proto::canonical::field_type_label
                                    ),
                                    f.r#type.as_ref().map_or(
                                        "unspecified",
                                        trogon_atlas_proto::canonical::field_type_label
                                    ),
                                ),
                                confidence: 0.8,
                            });
                        }
                        traced += 1;
                        if let Some(id) = up_ref.id.as_ref() {
                            consumed_up.insert((
                                id.namespace.clone(),
                                id.slug.clone(),
                                id.version,
                                up_field.name.clone(),
                            ));
                        }
                    }
                    n => {
                        gaps.push(pb::CompletenessGap {
                            kind: pb::completeness_gap::Kind::AmbiguousSource as i32,
                            entity: Some(down.entity_ref.clone()),
                            field_path: f.name.clone(),
                            explanation: format!(
                                "{} upstream fields named '{}' compete as the source",
                                n, f.name
                            ),
                            confidence: 0.7,
                        });
                        traced += 1;
                        for (up_ref, up_field) in &matches {
                            if let Some(id) = up_ref.id.as_ref() {
                                consumed_up.insert((
                                    id.namespace.clone(),
                                    id.slug.clone(),
                                    id.version,
                                    up_field.name.clone(),
                                ));
                            }
                        }
                    }
                }
            }
        }

        for up in &link.upstream {
            if up.source_role != FlowSourceRole::Direct {
                continue;
            }
            let Some(id) = up.entity_ref.id.as_ref() else {
                continue;
            };
            for f in &up.fields {
                let key = (
                    id.namespace.clone(),
                    id.slug.clone(),
                    id.version,
                    f.name.clone(),
                );
                if !consumed_up.contains(&key) {
                    gaps.push(pb::CompletenessGap {
                        kind: pb::completeness_gap::Kind::DanglingInput as i32,
                        entity: Some(up.entity_ref.clone()),
                        field_path: f.name.clone(),
                        explanation: format!(
                            "upstream field '{}' is not consumed by any downstream entity in scope",
                            f.name
                        ),
                        confidence: 0.6,
                    });
                }
            }
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let score = if total_down_fields == 0 {
        1.0
    } else {
        traced as f32 / total_down_fields as f32
    };
    (gaps, score)
}

fn note_undeclared(
    endpoint: &FlowEndpoint,
    gaps: &mut Vec<pb::CompletenessGap>,
    seen: &mut HashSet<(i32, String, String, u64)>,
) {
    if endpoint.source_role != FlowSourceRole::Direct {
        return;
    }
    if !endpoint.fields.is_empty() {
        return;
    }
    let Some(id) = endpoint.entity_ref.id.as_ref() else {
        return;
    };
    let key = (
        endpoint.entity_ref.kind,
        id.namespace.clone(),
        id.slug.clone(),
        id.version,
    );
    if !seen.insert(key) {
        return;
    }
    gaps.push(pb::CompletenessGap {
        kind: pb::completeness_gap::Kind::UndeclaredFields as i32,
        entity: Some(endpoint.entity_ref.clone()),
        field_path: String::new(),
        explanation: "entity participates in a flow but declares no fields".into(),
        confidence: 0.9,
    });
}

pub(crate) fn map_field(
    target: &pb::EntityRef,
    f: &pb::FieldSpec,
    upstream: &[FlowEndpoint],
) -> pb::InferredFieldMapping {
    if is_derived_field(f) {
        return pb::InferredFieldMapping {
            target_entity: Some(target.clone()),
            target_field: f.name.clone(),
            confidence: 0.9,
            rationale: "field carries DerivedFieldAnnotation".into(),
            source: Some(pb::inferred_field_mapping::Source::Origin(
                pb::InferredOrigin::Derived as i32,
            )),
        };
    }
    let exact = candidate_sources(&f.name, upstream);
    if exact.len() == 1 {
        let (up_ref, up_field) = &exact[0];
        let same_kind = trogon_atlas_proto::canonical::field_types_compatible(
            up_field.r#type.as_ref(),
            f.r#type.as_ref(),
        );
        let confidence = if same_kind { 0.85 } else { 0.55 };
        let rationale = if same_kind {
            "exact name match upstream with matching field kind".into()
        } else {
            "exact name match upstream; field kinds differ".into()
        };
        return pb::InferredFieldMapping {
            target_entity: Some(target.clone()),
            target_field: f.name.clone(),
            confidence,
            rationale,
            source: Some(pb::inferred_field_mapping::Source::FromField(
                pb::InferredFieldSource {
                    from_entity: Some(up_ref.clone()),
                    field_path: up_field.name.clone(),
                },
            )),
        };
    }
    if exact.len() > 1 {
        let (up_ref, up_field) = &exact[0];
        return pb::InferredFieldMapping {
            target_entity: Some(target.clone()),
            target_field: f.name.clone(),
            confidence: 0.4,
            rationale: format!(
                "{} upstream fields share this name; first match picked, treat as ambiguous",
                exact.len()
            ),
            source: Some(pb::inferred_field_mapping::Source::FromField(
                pb::InferredFieldSource {
                    from_entity: Some(up_ref.clone()),
                    field_path: up_field.name.clone(),
                },
            )),
        };
    }
    if let Some(origin) = classify_origin(&f.name) {
        return pb::InferredFieldMapping {
            target_entity: Some(target.clone()),
            target_field: f.name.clone(),
            confidence: 0.6,
            rationale: format!(
                "no upstream field named '{}'; name pattern suggests {:?}",
                f.name, origin
            ),
            source: Some(pb::inferred_field_mapping::Source::Origin(origin as i32)),
        };
    }
    pb::InferredFieldMapping {
        target_entity: Some(target.clone()),
        target_field: f.name.clone(),
        confidence: 0.2,
        rationale: format!(
            "no upstream field named '{}' and no origin pattern matched",
            f.name
        ),
        source: Some(pb::inferred_field_mapping::Source::Origin(
            pb::InferredOrigin::Unknown as i32,
        )),
    }
}

fn classify_origin(name: &str) -> Option<pb::InferredOrigin> {
    let lc = name.to_ascii_lowercase();
    if lc.ends_with("_at")
        || lc == "now"
        || lc.contains("timestamp")
        || lc == "created_at"
        || lc == "updated_at"
    {
        return Some(pb::InferredOrigin::Clock);
    }
    if lc.ends_with("_id")
        && (lc == "id" || lc.starts_with("event_") || lc.starts_with("aggregate_"))
    {
        return Some(pb::InferredOrigin::Generated);
    }
    if lc == "id" || lc == "uuid" {
        return Some(pb::InferredOrigin::Generated);
    }
    None
}

fn candidate_sources<'a>(
    name: &str,
    upstream: &'a [FlowEndpoint],
) -> Vec<(pb::EntityRef, &'a pb::FieldSpec)> {
    let mut direct: Vec<(pb::EntityRef, &pb::FieldSpec)> = Vec::new();
    let mut auxiliary: Vec<(pb::EntityRef, &pb::FieldSpec)> = Vec::new();
    let target = name.to_ascii_lowercase();
    for up in upstream {
        for f in &up.fields {
            if f.name.to_ascii_lowercase() == target {
                match up.source_role {
                    FlowSourceRole::Direct => direct.push((up.entity_ref.clone(), f)),
                    FlowSourceRole::Auxiliary => auxiliary.push((up.entity_ref.clone(), f)),
                }
            }
        }
    }
    if direct.is_empty() {
        auxiliary
    } else {
        direct
    }
}

#[must_use]
pub fn resolve_scope(scope: &pb::AnalysisScope, all: &[StoredEntity]) -> Vec<FlowLink> {
    let by_key = graph::entity_lookup(all);
    let slice_refs = match scope.scope.as_ref() {
        Some(pb::analysis_scope::Scope::Slice(r)) => {
            let Some(id) = r.id.as_ref() else {
                return Vec::new();
            };
            let k = pb::EntityKind::try_from(r.kind).unwrap_or(pb::EntityKind::Unspecified);
            vec![resolve_slice_ref(k, id, &by_key)]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
        }
        Some(pb::analysis_scope::Scope::Storyboard(r)) => {
            let Some(id) = r.id.as_ref() else {
                return Vec::new();
            };
            let entity = by_key.get(&pb::EntityKey::new(pb::EntityKind::Storyboard, id));
            let Some(entity) = entity else {
                return Vec::new();
            };
            let Some(pb::entity::Kind::Storyboard(sb)) = entity.kind.as_ref() else {
                return Vec::new();
            };
            sb.slices
                .iter()
                .filter_map(|sr| sr.id.clone())
                .filter_map(|sid| resolve_slice_ref(pb::EntityKind::Unspecified, &sid, &by_key))
                .collect()
        }
        Some(pb::analysis_scope::Scope::EventModel(r)) => {
            let Some(id) = r.id.as_ref() else {
                return Vec::new();
            };
            let entity = by_key.get(&pb::EntityKey::new(pb::EntityKind::EventModel, id));
            let Some(entity) = entity else {
                return Vec::new();
            };
            let Some(pb::entity::Kind::EventModel(em)) = entity.kind.as_ref() else {
                return Vec::new();
            };
            return flow_links_for_members(&em.members, &by_key);
        }
        None => Vec::new(),
    };

    slice_refs
        .into_iter()
        .map(|(kind, entity)| slice_to_flow(kind, &entity, &by_key))
        .collect()
}

/// Resolves the `FlowLink`s for an explicit member list, independent of
/// whether an `EventModel` carrying those members is itself in the store.
/// Shared by `resolve_scope`'s `EventModel` scope (members come from the
/// stored entity) and by validation's PII_FLOW_UNMARKED pass (members come
/// straight off the `EventModel` being validated, which may be inline and
/// not yet persisted).
pub(crate) fn flow_links_for_members(
    members: &[pb::EntityRef],
    by_key: &HashMap<pb::EntityKey, pb::Entity>,
) -> Vec<FlowLink> {
    let mut links: Vec<FlowLink> = members
        .iter()
        .filter_map(|m| {
            let id = m.id.as_ref()?.clone();
            let k = pb::EntityKind::try_from(m.kind).ok()?;
            if !matches!(
                k,
                pb::EntityKind::CommandSlice
                    | pb::EntityKind::ReadModelSlice
                    | pb::EntityKind::AutomationSlice
                    | pb::EntityKind::UiSlice
            ) {
                return None;
            }
            resolve_slice_ref(k, &id, by_key)
        })
        .map(|(kind, entity)| slice_to_flow(kind, &entity, by_key))
        .collect();
    links.extend(members.iter().filter_map(|m| {
        let id = m.id.as_ref()?;
        let kind = pb::EntityKind::try_from(m.kind).ok()?;
        if kind == pb::EntityKind::ReadModel {
            read_model_to_flow(id, by_key)
        } else {
            None
        }
    }));
    links
}

fn read_model_to_flow(
    read_model_id: &pb::Id,
    by_key: &HashMap<pb::EntityKey, pb::Entity>,
) -> Option<FlowLink> {
    let entity = by_key.get(&pb::EntityKey::new(
        pb::EntityKind::ReadModel,
        read_model_id,
    ))?;
    let pb::entity::Kind::ReadModel(read_model) = entity.kind.as_ref()? else {
        return None;
    };
    let mut upstream: Vec<FlowEndpoint> = read_model
        .source_events
        .iter()
        .filter_map(|event_ref| event_ref.id.as_ref())
        .filter_map(|id| endpoint_for(pb::EntityKind::Event, id, by_key))
        .collect();
    let mut seen_lanes = HashSet::new();
    for event_ref in &read_model.source_events {
        let Some(event_id) = event_ref.id.as_ref() else {
            continue;
        };
        let Some(pb::entity::Kind::Event(event)) = by_key
            .get(&pb::EntityKey::new(pb::EntityKind::Event, event_id))
            .and_then(|entity| entity.kind.as_ref())
        else {
            continue;
        };
        let Some(lane_id) = event.swimlane.as_ref().and_then(|s| s.id.as_ref()) else {
            continue;
        };
        let key = (
            lane_id.namespace.clone(),
            lane_id.slug.clone(),
            lane_id.version,
        );
        if seen_lanes.insert(key) {
            if let Some(state) = swimlane_state_endpoint(lane_id, by_key) {
                upstream.push(state);
            }
        }
    }
    let downstream = endpoint_for(pb::EntityKind::ReadModel, read_model_id, by_key)
        .into_iter()
        .collect();
    Some(FlowLink {
        upstream,
        downstream,
    })
}

fn resolve_slice_ref(
    kind: pb::EntityKind,
    id: &pb::Id,
    by_key: &HashMap<pb::EntityKey, pb::Entity>,
) -> Option<(pb::EntityKind, pb::Entity)> {
    let probes: &[pb::EntityKind] = if matches!(kind, pb::EntityKind::Unspecified) {
        &[
            pb::EntityKind::CommandSlice,
            pb::EntityKind::ReadModelSlice,
            pb::EntityKind::AutomationSlice,
            pb::EntityKind::UiSlice,
        ]
    } else {
        std::slice::from_ref(&kind)
    };
    for k in probes {
        let key = pb::EntityKey::new(*k, id);
        if let Some(entity) = by_key.get(&key) {
            return Some((*k, entity.clone()));
        }
    }
    None
}

fn slice_to_flow(
    kind: pb::EntityKind,
    entity: &pb::Entity,
    by_key: &HashMap<pb::EntityKey, pb::Entity>,
) -> FlowLink {
    match (kind, entity.kind.as_ref()) {
        (pb::EntityKind::CommandSlice, Some(pb::entity::Kind::CommandSlice(cs))) => {
            let command_id = cs
                .command
                .as_ref()
                .and_then(|ce| ce.command.as_ref()?.id.clone());
            let mut upstream: Vec<FlowEndpoint> = command_id
                .as_ref()
                .and_then(|id| endpoint_for(pb::EntityKind::Command, id, by_key))
                .into_iter()
                .collect();
            if let Some(id) = command_id.as_ref() {
                if let Some(pb::entity::Kind::Command(command)) = by_key
                    .get(&pb::EntityKey::new(pb::EntityKind::Command, id))
                    .and_then(|entity| entity.kind.as_ref())
                {
                    if let Some(lane_id) = command.swimlane.as_ref().and_then(|s| s.id.as_ref()) {
                        upstream.extend(auxiliary_command_sources_on_lane(lane_id, id, by_key));
                        if let Some(state) = swimlane_state_endpoint(lane_id, by_key) {
                            upstream.push(state);
                        }
                    }
                }
            }
            let downstream: Vec<FlowEndpoint> = cs
                .emitted_events
                .iter()
                .filter_map(|ee| ee.event.as_ref()?.id.clone())
                .filter_map(|id| endpoint_for(pb::EntityKind::Event, &id, by_key))
                .collect();
            FlowLink {
                upstream,
                downstream,
            }
        }
        (pb::EntityKind::ReadModelSlice, Some(pb::entity::Kind::ReadModelSlice(rs))) => {
            let mut upstream: Vec<FlowEndpoint> = rs
                .source_events
                .iter()
                .filter_map(|ee| ee.event.as_ref()?.id.clone())
                .filter_map(|id| endpoint_for(pb::EntityKind::Event, &id, by_key))
                .collect();
            let mut seen_lanes = HashSet::new();
            for source_event in &rs.source_events {
                let Some(event_id) = source_event.event.as_ref().and_then(|r| r.id.as_ref()) else {
                    continue;
                };
                let Some(pb::entity::Kind::Event(event)) = by_key
                    .get(&pb::EntityKey::new(pb::EntityKind::Event, event_id))
                    .and_then(|entity| entity.kind.as_ref())
                else {
                    continue;
                };
                let Some(lane_id) = event.swimlane.as_ref().and_then(|s| s.id.as_ref()) else {
                    continue;
                };
                let key = (
                    lane_id.namespace.clone(),
                    lane_id.slug.clone(),
                    lane_id.version,
                );
                if seen_lanes.insert(key) {
                    if let Some(state) = swimlane_state_endpoint(lane_id, by_key) {
                        upstream.push(state);
                    }
                }
            }
            let downstream: Vec<FlowEndpoint> = rs
                .read_model
                .as_ref()
                .and_then(|re| re.read_model.as_ref()?.id.clone())
                .and_then(|id| endpoint_for(pb::EntityKind::ReadModel, &id, by_key))
                .into_iter()
                .collect();
            FlowLink {
                upstream,
                downstream,
            }
        }
        (pb::EntityKind::AutomationSlice, Some(pb::entity::Kind::AutomationSlice(as_))) => {
            let upstream: Vec<FlowEndpoint> = as_
                .source_read_models
                .iter()
                .filter_map(|re| re.read_model.as_ref()?.id.clone())
                .filter_map(|id| endpoint_for(pb::EntityKind::ReadModel, &id, by_key))
                .collect();
            let downstream: Vec<FlowEndpoint> = as_
                .emitted_command
                .as_ref()
                .and_then(|ce| ce.command.as_ref()?.id.clone())
                .and_then(|id| endpoint_for(pb::EntityKind::Command, &id, by_key))
                .into_iter()
                .collect();
            FlowLink {
                upstream,
                downstream,
            }
        }
        (pb::EntityKind::UiSlice, Some(pb::entity::Kind::UiSlice(us))) => {
            let upstream: Vec<FlowEndpoint> = us
                .source_read_models
                .iter()
                .filter_map(|re| re.read_model.as_ref()?.id.clone())
                .filter_map(|id| endpoint_for(pb::EntityKind::ReadModel, &id, by_key))
                .collect();
            let downstream: Vec<FlowEndpoint> = us
                .ui
                .as_ref()
                .and_then(|ue| ue.ui.as_ref()?.id.clone())
                .and_then(|id| endpoint_for(pb::EntityKind::Ui, &id, by_key))
                .into_iter()
                .collect();
            FlowLink {
                upstream,
                downstream,
            }
        }
        _ => FlowLink::default(),
    }
}

fn endpoint_for(
    kind: pb::EntityKind,
    id: &pb::Id,
    by_key: &HashMap<pb::EntityKey, pb::Entity>,
) -> Option<FlowEndpoint> {
    let entity = by_key.get(&pb::EntityKey::new(kind, id))?;
    let fields = fields_of(entity);
    Some(FlowEndpoint {
        entity_ref: pb::EntityRef {
            kind: kind as i32,
            id: Some(id.clone()),
        },
        fields,
        source_role: FlowSourceRole::Direct,
    })
}

fn auxiliary_endpoint_for(
    kind: pb::EntityKind,
    id: &pb::Id,
    fields: Vec<pb::FieldSpec>,
) -> Option<FlowEndpoint> {
    if fields.is_empty() {
        return None;
    }
    Some(FlowEndpoint {
        entity_ref: pb::EntityRef {
            kind: kind as i32,
            id: Some(id.clone()),
        },
        fields,
        source_role: FlowSourceRole::Auxiliary,
    })
}

fn auxiliary_command_sources_on_lane(
    lane_id: &pb::Id,
    current_command_id: &pb::Id,
    by_key: &HashMap<pb::EntityKey, pb::Entity>,
) -> Vec<FlowEndpoint> {
    by_key
        .values()
        .filter_map(|entity| match entity.kind.as_ref()? {
            pb::entity::Kind::Command(command) => Some(command),
            _ => None,
        })
        .filter(|command| command.id.as_ref() != Some(current_command_id))
        .filter(|command| command.swimlane.as_ref().and_then(|s| s.id.as_ref()) == Some(lane_id))
        .filter_map(|command| {
            auxiliary_endpoint_for(
                pb::EntityKind::Command,
                command.id.as_ref()?,
                schema_fields(command.schema.as_ref()),
            )
        })
        .collect()
}

fn swimlane_state_endpoint(
    lane_id: &pb::Id,
    by_key: &HashMap<pb::EntityKey, pb::Entity>,
) -> Option<FlowEndpoint> {
    let entity = by_key.get(&pb::EntityKey::new(pb::EntityKind::Swimlane, lane_id))?;
    let pb::entity::Kind::Swimlane(lane) = entity.kind.as_ref()? else {
        return None;
    };
    auxiliary_endpoint_for(pb::EntityKind::Swimlane, lane_id, lane.state.clone())
}

pub(crate) fn is_derived_field(field: &pb::FieldSpec) -> bool {
    field.metadata.iter().any(|m| {
        m.type_url
            .ends_with("trogonatlas.eventmodel.v1alpha1.DerivedFieldAnnotation")
    })
}

fn fields_of(entity: &pb::Entity) -> Vec<pb::FieldSpec> {
    use pb::entity::Kind as K;
    match entity.kind.as_ref() {
        Some(K::Event(x)) => schema_fields(x.schema.as_ref()),
        Some(K::Command(x)) => schema_fields(x.schema.as_ref()),
        Some(K::ReadModel(x)) => schema_fields(x.schema.as_ref()),
        _ => Vec::new(),
    }
}

pub(crate) use trogon_atlas_core::schema::schema_fields;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use prost::Message;

    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn field(name: &str, kind: pb::field_type::Kind) -> pb::FieldSpec {
        pb::FieldSpec {
            name: name.into(),
            r#type: Some(pb::FieldType { kind: Some(kind) }),
            ..Default::default()
        }
    }

    fn derived_field(name: &str, kind: pb::field_type::Kind) -> pb::FieldSpec {
        let annotation = pb::DerivedFieldAnnotation {
            doc: "runtime supplies this value".into(),
            kind: 0,
        };
        pb::FieldSpec {
            name: name.into(),
            r#type: Some(pb::FieldType { kind: Some(kind) }),
            metadata: vec![prost_types::Any {
                type_url:
                    "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.DerivedFieldAnnotation"
                        .into(),
                value: annotation.encode_to_vec(),
            }],
            ..Default::default()
        }
    }

    fn string_kind() -> pb::field_type::Kind {
        pb::field_type::Kind::String(pb::field_type::StringType {})
    }

    fn int_kind() -> pb::field_type::Kind {
        pb::field_type::Kind::Int(pb::field_type::IntType {})
    }

    fn timestamp_kind() -> pb::field_type::Kind {
        pb::field_type::Kind::Timestamp(pb::field_type::TimestampType {})
    }

    fn swimlane(slug: &str, state: Vec<pb::FieldSpec>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Swimlane(pb::Swimlane {
                id: Some(id(slug)),
                state,
                ..Default::default()
            })),
        }
    }

    fn command(slug: &str, lane: &str, fields: Vec<pb::FieldSpec>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Command(pb::Command {
                id: Some(id(slug)),
                swimlane: Some(pb::SwimlaneRef { id: Some(id(lane)) }),
                schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
                ..Default::default()
            })),
        }
    }

    fn event(slug: &str, lane: &str, fields: Vec<pb::FieldSpec>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(slug)),
                swimlane: Some(pb::SwimlaneRef { id: Some(id(lane)) }),
                schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
                ..Default::default()
            })),
        }
    }

    fn read_model(slug: &str, fields: Vec<pb::FieldSpec>, source_events: Vec<&str>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(slug)),
                schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
                source_events: source_events
                    .into_iter()
                    .map(|slug| pb::EventRef { id: Some(id(slug)) })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn command_slice(slug: &str, command: &str, emitted_events: Vec<&str>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id(slug)),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef {
                        id: Some(id(command)),
                    }),
                    ..Default::default()
                }),
                emitted_events: emitted_events
                    .into_iter()
                    .map(|slug| pb::EventEdge {
                        event: Some(pb::EventRef { id: Some(id(slug)) }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn read_model_slice(slug: &str, source_events: Vec<&str>, read_model: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
                id: Some(id(slug)),
                source_events: source_events
                    .into_iter()
                    .map(|slug| pb::EventEdge {
                        event: Some(pb::EventRef { id: Some(id(slug)) }),
                        ..Default::default()
                    })
                    .collect(),
                read_model: Some(pb::ReadModelEdge {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(id(read_model)),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        }
    }

    fn event_model(slug: &str, members: Vec<(pb::EntityKind, &str)>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
                id: Some(id(slug)),
                members: members
                    .into_iter()
                    .map(|(kind, slug)| pb::EntityRef {
                        kind: kind as i32,
                        id: Some(id(slug)),
                    })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn scope(kind: pb::EntityKind, slug: &str) -> pb::AnalysisScope {
        pb::AnalysisScope {
            scope: Some(pb::analysis_scope::Scope::Slice(pb::EntityRef {
                kind: kind as i32,
                id: Some(id(slug)),
            })),
        }
    }

    fn event_model_scope(slug: &str) -> pb::AnalysisScope {
        pb::AnalysisScope {
            scope: Some(pb::analysis_scope::Scope::EventModel(pb::EntityRef {
                kind: pb::EntityKind::EventModel as i32,
                id: Some(id(slug)),
            })),
        }
    }

    fn count(gaps: &[pb::CompletenessGap], kind: pb::completeness_gap::Kind) -> usize {
        gaps.iter().filter(|gap| gap.kind == kind as i32).count()
    }

    #[test]
    fn derived_event_field_is_complete() {
        let all = vec![
            stored(swimlane("order", Vec::new())),
            stored(command(
                "place-order",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(event(
                "order-placed",
                "order",
                vec![
                    field("order_id", string_kind()),
                    derived_field("placed_at", timestamp_kind()),
                ],
            )),
            stored(command_slice(
                "place-order-slice",
                "place-order",
                vec!["order-placed"],
            )),
        ];
        let (gaps, score) = check_completeness(
            &scope(pb::EntityKind::CommandSlice, "place-order-slice"),
            &all,
        );
        assert!(gaps.is_empty(), "{gaps:?}");
        assert!((score - 1.0).abs() < f32::EPSILON, "score = {score}");
    }

    #[test]
    fn read_model_can_source_field_from_event_lane_state() {
        let all = vec![
            stored(swimlane("order", vec![field("status", string_kind())])),
            stored(event(
                "order-placed",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(read_model(
                "orders",
                vec![
                    field("order_id", string_kind()),
                    field("status", string_kind()),
                ],
                vec!["order-placed"],
            )),
            stored(read_model_slice(
                "orders-slice",
                vec!["order-placed"],
                "orders",
            )),
        ];
        let (gaps, score) =
            check_completeness(&scope(pb::EntityKind::ReadModelSlice, "orders-slice"), &all);
        assert!(gaps.is_empty(), "{gaps:?}");
        assert!((score - 1.0).abs() < f32::EPSILON, "score = {score}");
    }

    #[test]
    fn event_model_scope_checks_read_model_members() {
        let all = vec![
            stored(swimlane("order", Vec::new())),
            stored(event(
                "order-placed",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(read_model(
                "orders",
                vec![field("customer_id", string_kind())],
                vec!["order-placed"],
            )),
            stored(event_model(
                "ordering",
                vec![(pb::EntityKind::ReadModel, "orders")],
            )),
        ];
        let (gaps, score) = check_completeness(&event_model_scope("ordering"), &all);
        assert_eq!(count(&gaps, pb::completeness_gap::Kind::MissingSource), 1);
        assert!(score.abs() < f32::EPSILON, "score = {score}");
    }

    #[test]
    fn missing_source_and_dangling_input_are_reported() {
        let all = vec![
            stored(swimlane("order", Vec::new())),
            stored(command(
                "place-order",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(event(
                "order-placed",
                "order",
                vec![field("customer_id", string_kind())],
            )),
            stored(command_slice(
                "place-order-slice",
                "place-order",
                vec!["order-placed"],
            )),
        ];
        let (gaps, score) = check_completeness(
            &scope(pb::EntityKind::CommandSlice, "place-order-slice"),
            &all,
        );
        assert_eq!(count(&gaps, pb::completeness_gap::Kind::MissingSource), 1);
        assert_eq!(count(&gaps, pb::completeness_gap::Kind::DanglingInput), 1);
        assert!(score.abs() < f32::EPSILON, "score = {score}");
    }

    #[test]
    fn shape_mismatch_is_reported() {
        let all = vec![
            stored(swimlane("order", Vec::new())),
            stored(command(
                "place-order",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(event(
                "order-placed",
                "order",
                vec![field("order_id", int_kind())],
            )),
            stored(command_slice(
                "place-order-slice",
                "place-order",
                vec!["order-placed"],
            )),
        ];
        let (gaps, score) = check_completeness(
            &scope(pb::EntityKind::CommandSlice, "place-order-slice"),
            &all,
        );
        assert_eq!(count(&gaps, pb::completeness_gap::Kind::ShapeMismatch), 1);
        assert!((score - 1.0).abs() < f32::EPSILON, "score = {score}");
    }

    #[test]
    fn ambiguous_sources_are_reported() {
        let all = vec![
            stored(swimlane("order", Vec::new())),
            stored(event(
                "order-placed",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(event(
                "order-adjusted",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(read_model(
                "orders",
                vec![field("order_id", string_kind())],
                vec!["order-placed", "order-adjusted"],
            )),
            stored(read_model_slice(
                "orders-slice",
                vec!["order-placed", "order-adjusted"],
                "orders",
            )),
        ];
        let (gaps, score) =
            check_completeness(&scope(pb::EntityKind::ReadModelSlice, "orders-slice"), &all);
        assert_eq!(count(&gaps, pb::completeness_gap::Kind::AmbiguousSource), 1);
        assert!((score - 1.0).abs() < f32::EPSILON, "score = {score}");
    }

    #[test]
    fn undeclared_fields_are_reported() {
        let all = vec![
            stored(swimlane("order", Vec::new())),
            stored(command(
                "place-order",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(event("order-placed", "order", Vec::new())),
            stored(command_slice(
                "place-order-slice",
                "place-order",
                vec!["order-placed"],
            )),
        ];
        let (gaps, _score) = check_completeness(
            &scope(pb::EntityKind::CommandSlice, "place-order-slice"),
            &all,
        );
        assert_eq!(
            count(&gaps, pb::completeness_gap::Kind::UndeclaredFields),
            1
        );
    }

    #[test]
    fn infer_data_flow_happy_path_command_slice() {
        let all = vec![
            stored(swimlane("order", Vec::new())),
            stored(command(
                "place-order",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(event(
                "order-placed",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(command_slice(
                "place-order-slice",
                "place-order",
                vec!["order-placed"],
            )),
        ];
        let mappings = infer_data_flow(
            &scope(pb::EntityKind::CommandSlice, "place-order-slice"),
            &all,
        );
        assert!(
            !mappings.is_empty(),
            "expected at least one inferred field mapping"
        );
        let matched = mappings.iter().find(|m| m.target_field == "order_id");
        assert!(matched.is_some(), "expected a mapping for 'order_id'");
        let m = matched.unwrap();
        assert!(
            m.confidence > 0.5,
            "expected reasonable confidence for an exact name match"
        );
    }

    #[test]
    fn infer_data_flow_empty_model_returns_no_mappings() {
        let mappings = infer_data_flow(
            &scope(pb::EntityKind::CommandSlice, "nonexistent-slice"),
            &[],
        );
        assert!(
            mappings.is_empty(),
            "expected no mappings for an empty entity store"
        );
    }
}
