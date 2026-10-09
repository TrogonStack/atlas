//! Reference extraction: given an Entity, enumerate every (target, path)
//! pair where the entity points at another addressable thing. Used by
//! the reverse-reference index, `GetIncomingReferences`, and `GetImpact`.

use trogon_atlas_proto::{
    entity::Kind as EntityOneof, type_ref, Entity, EntityKind, EntityRef, Id,
};

/// One outbound reference: who we point at, and where in our own
/// structure the pointer lives.
///
/// `to_kind` is `None` for intentionally-untyped slice references (Storyboard
/// `slices[]` and `traces[].steps[].slice`, and a `ServiceLevelIndicator`
/// connection's `command_slice` / `read_model_slice` / `automation_slice` /
/// `ui_slice`). The resolver must probe each of the four concrete slice kinds
/// (`CommandSlice`, `ReadModelSlice`, `AutomationSlice`, `UiSlice`) against
/// the store to locate the actual entity. A `None` here is a design choice,
/// not an error; treat `Some(EntityKind::Unspecified)` as an error from
/// upstream data.
#[derive(Debug, Clone)]
pub struct OutboundRef {
    pub to_kind: Option<EntityKind>,
    pub to_id: Id,
    pub from_field: String,
}

impl OutboundRef {
    /// Build an `EntityRef` from this outbound reference.
    ///
    /// Returns `None` when `to_kind` is `None` (intentionally-untyped slice
    /// ref) because `EntityRef` requires a concrete kind. Callers that need to
    /// resolve untyped refs must probe the store for each candidate slice kind.
    #[must_use]
    pub fn to_entity_ref(&self) -> Option<EntityRef> {
        let kind = self.to_kind?;
        Some(EntityRef {
            kind: kind as i32,
            id: Some(self.to_id.clone()),
        })
    }
}

/// Enumerate every outbound reference from `entity`.
///
/// Returns a `Vec<OutboundRef>` where each entry describes one pointer this
/// entity holds to another addressable entity. `OutboundRef::to_kind` is
/// `None` for intentionally-untyped slice references (Storyboard slices and
/// trace steps); all other references carry a concrete `EntityKind`. Unknown
/// member kind integers in EventModel/Component members are skipped with a
/// warning rather than emitting an `Unspecified` ref.
pub fn outbound_refs(entity: &Entity) -> Vec<OutboundRef> {
    let mut out = Vec::new();
    let Some(kind) = entity.kind.as_ref() else {
        return out;
    };
    let supersedes_of = match kind {
        EntityOneof::Event(x) => x.supersedes.as_ref(),
        EntityOneof::Command(x) => x.supersedes.as_ref(),
        EntityOneof::ReadModel(x) => x.supersedes.as_ref(),
        EntityOneof::Processor(x) => x.supersedes.as_ref(),
        EntityOneof::Ui(x) => x.supersedes.as_ref(),
        EntityOneof::Persona(x) => x.supersedes.as_ref(),
        EntityOneof::Swimlane(x) => x.supersedes.as_ref(),
        EntityOneof::CommandSlice(x) => x.supersedes.as_ref(),
        EntityOneof::ReadModelSlice(x) => x.supersedes.as_ref(),
        EntityOneof::AutomationSlice(x) => x.supersedes.as_ref(),
        EntityOneof::Storyboard(x) => x.supersedes.as_ref(),
        EntityOneof::EventModel(x) => x.supersedes.as_ref(),
        EntityOneof::Component(x) => x.supersedes.as_ref(),
        EntityOneof::ExternalSystem(x) => x.supersedes.as_ref(),
        EntityOneof::Tracker(x) => x.supersedes.as_ref(),
        EntityOneof::BoundedContext(x) => x.supersedes.as_ref(),
        EntityOneof::Domain(x) => x.supersedes.as_ref(),
        EntityOneof::Subdomain(x) => x.supersedes.as_ref(),
        EntityOneof::Schema(x) => x.supersedes.as_ref(),
        EntityOneof::Project(x) => x.supersedes.as_ref(),
        EntityOneof::Screen(x) => x.supersedes.as_ref(),
        EntityOneof::Term(x) => x.supersedes.as_ref(),
        EntityOneof::Ambiguity(x) => x.supersedes.as_ref(),
        EntityOneof::UiSlice(x) => x.supersedes.as_ref(),
        EntityOneof::ServiceLevelIndicator(x) => x.supersedes.as_ref(),
        EntityOneof::ServiceLevelObjective(x) => x.supersedes.as_ref(),
        EntityOneof::AlertPolicy(x) => x.supersedes.as_ref(),
        EntityOneof::AlertNotificationTarget(x) => x.supersedes.as_ref(),
        EntityOneof::TypeLibrary(x) => x.supersedes.as_ref(),
    };
    if let Some(s) = supersedes_of {
        if let Some(self_kind) = entity_kind(entity) {
            out.push(OutboundRef {
                to_kind: Some(self_kind),
                to_id: s.clone(),
                from_field: "supersedes".into(),
            });
        }
    }
    match kind {
        EntityOneof::Event(x) => {
            if let Some(sl) = &x.swimlane {
                if let Some(id) = &sl.id {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::Swimlane),
                        to_id: id.clone(),
                        from_field: "swimlane".into(),
                    });
                }
            }
            collect_schema_any_field_refs(x.schema.as_ref(), &mut out);
        }
        EntityOneof::Command(x) => {
            if let Some(sl) = &x.swimlane {
                if let Some(id) = &sl.id {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::Swimlane),
                        to_id: id.clone(),
                        from_field: "swimlane".into(),
                    });
                }
            }
            collect_schema_any_field_refs(x.schema.as_ref(), &mut out);
        }
        EntityOneof::ReadModel(x) => {
            for (i, ev) in x.source_events.iter().enumerate() {
                if let Some(id) = &ev.id {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::Event),
                        to_id: id.clone(),
                        from_field: format!("source_events[{i}]"),
                    });
                }
            }
            collect_schema_any_field_refs(x.schema.as_ref(), &mut out);
            if let Some(src) = &x.external_source {
                if let Some(id) = src.system.as_ref().and_then(|s| s.id.as_ref()) {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::ExternalSystem),
                        to_id: id.clone(),
                        from_field: "external_source.system".into(),
                    });
                }
            }
        }
        EntityOneof::Processor(x) => {
            for (i, c) in x.calls.iter().enumerate() {
                if let Some(id) = &c.id {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::ExternalSystem),
                        to_id: id.clone(),
                        from_field: format!("calls[{i}]"),
                    });
                }
            }
        }
        EntityOneof::Ui(x) => {
            for (ti, t) in x.transitions.iter().enumerate() {
                if let Some(id) = t.to.as_ref().and_then(|r| r.id.as_ref()) {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::Ui),
                        to_id: id.clone(),
                        from_field: format!("transitions[{ti}].to"),
                    });
                }
            }
            if let Some(id) = x
                .slot
                .as_ref()
                .and_then(|slot| slot.screen.as_ref())
                .and_then(|screen| screen.id.as_ref())
            {
                out.push(OutboundRef {
                    to_kind: Some(EntityKind::Screen),
                    to_id: id.clone(),
                    from_field: "slot.screen".into(),
                });
            }
        }
        EntityOneof::Persona(_) | EntityOneof::ExternalSystem(_) | EntityOneof::Screen(_) => {}
        EntityOneof::Domain(x) => {
            if let Some(id) = x.project.as_ref().and_then(|p| p.id.as_ref()) {
                out.push(OutboundRef {
                    to_kind: Some(EntityKind::Project),
                    to_id: id.clone(),
                    from_field: "project".into(),
                });
            }
        }
        EntityOneof::Schema(x) => {
            for (i, f) in x.fields.iter().enumerate() {
                collect_field_refs(f, &format!("fields[{i}]"), &mut out, 0);
            }
        }
        EntityOneof::Swimlane(x) => {
            for (i, f) in x.state.iter().enumerate() {
                collect_field_refs(f, &format!("state[{i}]"), &mut out, 0);
            }
            for (ti, t) in x.transitions.iter().enumerate() {
                if let Some(id) = t.after.as_ref().and_then(|r| r.id.as_ref()) {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::Event),
                        to_id: id.clone(),
                        from_field: format!("transitions[{ti}].after"),
                    });
                }
                for (ni, n) in t.next.iter().enumerate() {
                    if let Some(id) = n.event.as_ref().and_then(|r| r.id.as_ref()) {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::Event),
                            to_id: id.clone(),
                            from_field: format!("transitions[{ti}].next[{ni}]"),
                        });
                    }
                }
            }
        }
        EntityOneof::CommandSlice(x) => {
            if let Some(p) = &x.persona {
                if let Some(r) = &p.persona {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::Persona),
                            to_id: id.clone(),
                            from_field: "persona".into(),
                        });
                    }
                }
            }
            if let Some(u) = &x.ui {
                if let Some(r) = &u.ui {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::Ui),
                            to_id: id.clone(),
                            from_field: "ui".into(),
                        });
                    }
                }
            }
            if let Some(c) = &x.command {
                if let Some(r) = &c.command {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::Command),
                            to_id: id.clone(),
                            from_field: "command".into(),
                        });
                    }
                }
            }
            for (i, e) in x.emitted_events.iter().enumerate() {
                if let Some(r) = &e.event {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::Event),
                            to_id: id.clone(),
                            from_field: format!("emitted_events[{i}]"),
                        });
                    }
                }
            }
            for (si, sc) in x.scenarios.iter().enumerate() {
                for (gi, g) in sc.given.iter().enumerate() {
                    if let Some(id) = g.event.as_ref().and_then(|r| r.id.as_ref()) {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::Event),
                            to_id: id.clone(),
                            from_field: format!("scenarios[{si}].given[{gi}].event"),
                        });
                    }
                }
                if let Some(id) = sc
                    .when
                    .as_ref()
                    .and_then(|w| w.command.as_ref())
                    .and_then(|r| r.id.as_ref())
                {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::Command),
                        to_id: id.clone(),
                        from_field: format!("scenarios[{si}].when.command"),
                    });
                }
                if let Some(trogon_atlas_proto::command_scenario::Then::Emit(em)) = &sc.then {
                    for (ei, e) in em.events.iter().enumerate() {
                        if let Some(id) = e.event.as_ref().and_then(|r| r.id.as_ref()) {
                            out.push(OutboundRef {
                                to_kind: Some(EntityKind::Event),
                                to_id: id.clone(),
                                from_field: format!("scenarios[{si}].then.emit.events[{ei}].event"),
                            });
                        }
                    }
                }
            }
        }
        EntityOneof::ReadModelSlice(x) => {
            for (i, e) in x.source_events.iter().enumerate() {
                if let Some(r) = &e.event {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::Event),
                            to_id: id.clone(),
                            from_field: format!("source_events[{i}]"),
                        });
                    }
                }
            }
            if let Some(rm) = &x.read_model {
                if let Some(r) = &rm.read_model {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::ReadModel),
                            to_id: id.clone(),
                            from_field: "read_model".into(),
                        });
                    }
                }
            }
            for (si, sc) in x.scenarios.iter().enumerate() {
                for (wi, w) in sc.when.iter().enumerate() {
                    if let Some(id) = w.event.as_ref().and_then(|r| r.id.as_ref()) {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::Event),
                            to_id: id.clone(),
                            from_field: format!("scenarios[{si}].when[{wi}].event"),
                        });
                    }
                }
                if let Some(id) = sc
                    .then
                    .as_ref()
                    .and_then(|t| t.read_model.as_ref())
                    .and_then(|r| r.id.as_ref())
                {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::ReadModel),
                        to_id: id.clone(),
                        from_field: format!("scenarios[{si}].then.read_model"),
                    });
                }
            }
        }
        EntityOneof::AutomationSlice(x) => {
            for (i, e) in x.source_read_models.iter().enumerate() {
                if let Some(r) = &e.read_model {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::ReadModel),
                            to_id: id.clone(),
                            from_field: format!("source_read_models[{i}]"),
                        });
                    }
                }
            }
            if let Some(p) = &x.processor {
                if let Some(r) = &p.processor {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::Processor),
                            to_id: id.clone(),
                            from_field: "processor".into(),
                        });
                    }
                }
            }
            if let Some(c) = &x.emitted_command {
                if let Some(r) = &c.command {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::Command),
                            to_id: id.clone(),
                            from_field: "emitted_command".into(),
                        });
                    }
                }
            }
            for (si, sc) in x.scenarios.iter().enumerate() {
                for (gi, g) in sc.given.iter().enumerate() {
                    if let Some(id) = g.read_model.as_ref().and_then(|r| r.id.as_ref()) {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::ReadModel),
                            to_id: id.clone(),
                            from_field: format!("scenarios[{si}].given[{gi}].read_model"),
                        });
                    }
                }
                if let Some(id) = sc
                    .then
                    .as_ref()
                    .and_then(|t| t.command.as_ref())
                    .and_then(|r| r.id.as_ref())
                {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::Command),
                        to_id: id.clone(),
                        from_field: format!("scenarios[{si}].then.command"),
                    });
                }
            }
        }
        EntityOneof::UiSlice(x) => {
            if let Some(p) = &x.persona {
                if let Some(r) = &p.persona {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::Persona),
                            to_id: id.clone(),
                            from_field: "persona".into(),
                        });
                    }
                }
            }
            for (i, rm) in x.source_read_models.iter().enumerate() {
                if let Some(r) = &rm.read_model {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::ReadModel),
                            to_id: id.clone(),
                            from_field: format!("source_read_models[{i}]"),
                        });
                    }
                }
            }
            if let Some(u) = &x.ui {
                if let Some(r) = &u.ui {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::Ui),
                            to_id: id.clone(),
                            from_field: "ui".into(),
                        });
                    }
                }
            }
        }
        EntityOneof::Storyboard(x) => {
            if let Some(entry) = &x.entry {
                if let Some(rm) = &entry.read_model {
                    if let Some(r) = &rm.read_model {
                        if let Some(id) = &r.id {
                            out.push(OutboundRef {
                                to_kind: Some(EntityKind::ReadModel),
                                to_id: id.clone(),
                                from_field: "entry.read_model".into(),
                            });
                        }
                    }
                }
                if let Some(obs) = &entry.observer {
                    match obs {
                        trogon_atlas_proto::storyboard_entry::Observer::Human(h) => {
                            if let Some(p) = &h.persona {
                                if let Some(r) = &p.persona {
                                    if let Some(id) = &r.id {
                                        out.push(OutboundRef {
                                            to_kind: Some(EntityKind::Persona),
                                            to_id: id.clone(),
                                            from_field: "entry.observer.human.persona".into(),
                                        });
                                    }
                                }
                            }
                            if let Some(u) = &h.ui {
                                if let Some(r) = &u.ui {
                                    if let Some(id) = &r.id {
                                        out.push(OutboundRef {
                                            to_kind: Some(EntityKind::Ui),
                                            to_id: id.clone(),
                                            from_field: "entry.observer.human.ui".into(),
                                        });
                                    }
                                }
                            }
                        }
                        trogon_atlas_proto::storyboard_entry::Observer::Automation(a) => {
                            if let Some(p) = &a.processor {
                                if let Some(r) = &p.processor {
                                    if let Some(id) = &r.id {
                                        out.push(OutboundRef {
                                            to_kind: Some(EntityKind::Processor),
                                            to_id: id.clone(),
                                            from_field: "entry.observer.automation.processor"
                                                .into(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let Some(outcome) = &x.outcome {
                for (i, e) in outcome.events.iter().enumerate() {
                    if let Some(r) = &e.event {
                        if let Some(id) = &r.id {
                            out.push(OutboundRef {
                                to_kind: Some(EntityKind::Event),
                                to_id: id.clone(),
                                from_field: format!("outcome.events[{i}]"),
                            });
                        }
                    }
                }
                if let Some(u) = &outcome.ui {
                    if let Some(r) = &u.ui {
                        if let Some(id) = &r.id {
                            out.push(OutboundRef {
                                to_kind: Some(EntityKind::Ui),
                                to_id: id.clone(),
                                from_field: "outcome.ui".into(),
                            });
                        }
                    }
                }
            }
            for (i, sr) in x.slices.iter().enumerate() {
                if let Some(id) = &sr.id {
                    // SliceRef intentionally carries no kind; None signals that
                    // the resolver must probe all three concrete slice kinds.
                    out.push(OutboundRef {
                        to_kind: None,
                        to_id: id.clone(),
                        from_field: format!("slices[{i}]"),
                    });
                }
            }
            for (ti, t) in x.traces.iter().enumerate() {
                for (si, step) in t.steps.iter().enumerate() {
                    if let Some(sr) = &step.slice {
                        if let Some(id) = &sr.id {
                            out.push(OutboundRef {
                                to_kind: None,
                                to_id: id.clone(),
                                from_field: format!("traces[{ti}].steps[{si}].slice"),
                            });
                        }
                    }
                }
            }
        }
        EntityOneof::EventModel(x) => {
            for (i, m) in x.members.iter().enumerate() {
                if let Some(id) = &m.id {
                    match EntityKind::try_from(m.kind) {
                        Ok(k) if k != EntityKind::Unspecified => {
                            out.push(OutboundRef {
                                to_kind: Some(k),
                                to_id: id.clone(),
                                from_field: format!("members[{i}]"),
                            });
                        }
                        _ => {
                            tracing::warn!(
                                member_index = i,
                                raw_kind = m.kind,
                                "EventModel member has unknown kind; skipping ref"
                            );
                        }
                    }
                }
            }
        }
        EntityOneof::Component(x) => {
            for (i, m) in x.members.iter().enumerate() {
                if let Some(id) = &m.id {
                    match EntityKind::try_from(m.kind) {
                        Ok(k) if k != EntityKind::Unspecified => {
                            out.push(OutboundRef {
                                to_kind: Some(k),
                                to_id: id.clone(),
                                from_field: format!("members[{i}]"),
                            });
                        }
                        _ => {
                            tracing::warn!(
                                member_index = i,
                                raw_kind = m.kind,
                                "Component member has unknown kind; skipping ref"
                            );
                        }
                    }
                }
            }
        }
        EntityOneof::BoundedContext(x) => {
            for (i, r) in x.realizes.iter().enumerate() {
                if let Some(id) = &r.id {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::Subdomain),
                        to_id: id.clone(),
                        from_field: format!("realizes[{i}]"),
                    });
                }
            }
            for (i, r) in x.relationships.iter().enumerate() {
                if let Some(id) = r.upstream.as_ref().and_then(|u| u.id.as_ref()) {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::BoundedContext),
                        to_id: id.clone(),
                        from_field: format!("relationships[{i}].upstream"),
                    });
                }
            }
        }
        EntityOneof::Subdomain(x) => {
            if let Some(id) = x.domain.as_ref().and_then(|r| r.id.as_ref()) {
                out.push(OutboundRef {
                    to_kind: Some(EntityKind::Domain),
                    to_id: id.clone(),
                    from_field: "domain".into(),
                });
            }
        }
        EntityOneof::Tracker(x) => {
            for (i, item) in x.items.iter().enumerate() {
                if let Some(subject) = &item.subject {
                    if let (Ok(kind), Some(id)) =
                        (EntityKind::try_from(subject.kind), subject.id.as_ref())
                    {
                        if kind == EntityKind::Unspecified {
                            tracing::warn!(
                                item_index = i,
                                raw_kind = subject.kind,
                                "Tracker item subject has Unspecified kind; skipping ref"
                            );
                        } else {
                            out.push(OutboundRef {
                                to_kind: Some(kind),
                                to_id: id.clone(),
                                from_field: format!("items[{i}].subject"),
                            });
                        }
                    }
                }
            }
        }
        EntityOneof::Project(x) => {
            let _ = x;
        }
        EntityOneof::Term(x) => {
            for (i, r) in x.embodied_by.iter().enumerate() {
                if let (Ok(kind), Some(id)) = (EntityKind::try_from(r.kind), r.id.as_ref()) {
                    if kind == EntityKind::Unspecified {
                        tracing::warn!(
                            item_index = i,
                            raw_kind = r.kind,
                            "Term embodied_by entry has Unspecified kind; skipping ref"
                        );
                    } else {
                        out.push(OutboundRef {
                            to_kind: Some(kind),
                            to_id: id.clone(),
                            from_field: format!("embodied_by[{i}]"),
                        });
                    }
                }
            }
        }
        EntityOneof::Ambiguity(x) => {
            for (i, r) in x.terms.iter().enumerate() {
                if let Some(id) = &r.id {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::Term),
                        to_id: id.clone(),
                        from_field: format!("terms[{i}]"),
                    });
                }
            }
        }
        EntityOneof::ServiceLevelIndicator(x) => {
            if let Some(conn) = &x.connection {
                connection_refs(conn, "connection", &mut out);
            }
            if let Some(trogon_atlas_proto::service_level_indicator::Measure::OutcomeRatio(or)) =
                &x.measure
            {
                for (i, r) in or.good.iter().enumerate() {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::Event),
                            to_id: id.clone(),
                            from_field: format!("measure.outcome_ratio.good[{i}]"),
                        });
                    }
                }
                for (i, r) in or.bad.iter().enumerate() {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::Event),
                            to_id: id.clone(),
                            from_field: format!("measure.outcome_ratio.bad[{i}]"),
                        });
                    }
                }
            }
        }
        EntityOneof::ServiceLevelObjective(x) => {
            if let Some(id) = x.service.as_ref().and_then(|r| r.id.as_ref()) {
                out.push(OutboundRef {
                    to_kind: Some(EntityKind::Component),
                    to_id: id.clone(),
                    from_field: "service".into(),
                });
            }
            if let Some(id) = x.indicator.as_ref().and_then(|r| r.id.as_ref()) {
                out.push(OutboundRef {
                    to_kind: Some(EntityKind::ServiceLevelIndicator),
                    to_id: id.clone(),
                    from_field: "indicator".into(),
                });
            }
            for (i, r) in x.alert_policies.iter().enumerate() {
                if let Some(id) = &r.id {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::AlertPolicy),
                        to_id: id.clone(),
                        from_field: format!("alert_policies[{i}]"),
                    });
                }
            }
            if let Some(trogon_atlas_proto::commitment::Kind::External(ext)) =
                x.commitment.as_ref().and_then(|c| c.kind.as_ref())
            {
                match &ext.counterparty {
                    Some(trogon_atlas_proto::commitment::external::Counterparty::System(r)) => {
                        if let Some(id) = &r.id {
                            out.push(OutboundRef {
                                to_kind: Some(EntityKind::ExternalSystem),
                                to_id: id.clone(),
                                from_field: "commitment.external.system".into(),
                            });
                        }
                    }
                    Some(trogon_atlas_proto::commitment::external::Counterparty::Persona(r)) => {
                        if let Some(id) = &r.id {
                            out.push(OutboundRef {
                                to_kind: Some(EntityKind::Persona),
                                to_id: id.clone(),
                                from_field: "commitment.external.persona".into(),
                            });
                        }
                    }
                    None => {}
                }
            }
        }
        EntityOneof::AlertPolicy(x) => {
            for (i, r) in x.notification_targets.iter().enumerate() {
                if let Some(id) = &r.id {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::AlertNotificationTarget),
                        to_id: id.clone(),
                        from_field: format!("notification_targets[{i}]"),
                    });
                }
            }
        }
        EntityOneof::AlertNotificationTarget(x) => {
            let _ = x;
        }
        EntityOneof::TypeLibrary(x) => {
            for (i, r) in x.dependencies.iter().enumerate() {
                if let Some(id) = &r.id {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::TypeLibrary),
                        to_id: id.clone(),
                        from_field: format!("dependencies[{i}]"),
                    });
                }
            }
        }
    }
    out
}

/// Extract outbound references from a `Connection`, the span-of-the-graph a
/// `ServiceLevelIndicator` measures. The `SliceRef` endpoints
/// (`command_slice`, `read_model_slice`, `automation_slice`, `ui_slice`) are
/// intentionally untyped (see `OutboundRef::to_kind` docs); the resolver
/// probes the specific slice kind the field name implies.
fn connection_refs(
    conn: &trogon_atlas_proto::Connection,
    prefix: &str,
    out: &mut Vec<OutboundRef>,
) {
    use trogon_atlas_proto::connection::{journey::Scope, Kind as ConnKind};
    match &conn.kind {
        Some(ConnKind::CommandHandling(ch)) => {
            if let Some(id) = ch.command_slice.as_ref().and_then(|s| s.id.as_ref()) {
                out.push(OutboundRef {
                    to_kind: None,
                    to_id: id.clone(),
                    from_field: format!("{prefix}.command_handling.command_slice"),
                });
            }
            for (i, r) in ch.events.iter().enumerate() {
                if let Some(id) = &r.id {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::Event),
                        to_id: id.clone(),
                        from_field: format!("{prefix}.command_handling.events[{i}]"),
                    });
                }
            }
        }
        Some(ConnKind::Projection(p)) => {
            if let Some(id) = p.read_model_slice.as_ref().and_then(|s| s.id.as_ref()) {
                out.push(OutboundRef {
                    to_kind: None,
                    to_id: id.clone(),
                    from_field: format!("{prefix}.projection.read_model_slice"),
                });
            }
            for (i, r) in p.source_events.iter().enumerate() {
                if let Some(id) = &r.id {
                    out.push(OutboundRef {
                        to_kind: Some(EntityKind::Event),
                        to_id: id.clone(),
                        from_field: format!("{prefix}.projection.source_events[{i}]"),
                    });
                }
            }
        }
        Some(ConnKind::Reaction(r)) => {
            if let Some(id) = r.automation_slice.as_ref().and_then(|s| s.id.as_ref()) {
                out.push(OutboundRef {
                    to_kind: None,
                    to_id: id.clone(),
                    from_field: format!("{prefix}.reaction.automation_slice"),
                });
            }
        }
        Some(ConnKind::Display(d)) => {
            if let Some(id) = d.ui_slice.as_ref().and_then(|s| s.id.as_ref()) {
                out.push(OutboundRef {
                    to_kind: None,
                    to_id: id.clone(),
                    from_field: format!("{prefix}.display.ui_slice"),
                });
            }
        }
        Some(ConnKind::Integration(i)) => {
            if let Some(id) = i.processor.as_ref().and_then(|p| p.id.as_ref()) {
                out.push(OutboundRef {
                    to_kind: Some(EntityKind::Processor),
                    to_id: id.clone(),
                    from_field: format!("{prefix}.integration.processor"),
                });
            }
            if let Some(id) = i.system.as_ref().and_then(|s| s.id.as_ref()) {
                out.push(OutboundRef {
                    to_kind: Some(EntityKind::ExternalSystem),
                    to_id: id.clone(),
                    from_field: format!("{prefix}.integration.system"),
                });
            }
        }
        Some(ConnKind::Journey(j)) => {
            match &j.scope {
                Some(Scope::Storyboard(r)) => {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::Storyboard),
                            to_id: id.clone(),
                            from_field: format!("{prefix}.journey.storyboard"),
                        });
                    }
                }
                Some(Scope::EventModel(r)) => {
                    if let Some(id) = &r.id {
                        out.push(OutboundRef {
                            to_kind: Some(EntityKind::EventModel),
                            to_id: id.clone(),
                            from_field: format!("{prefix}.journey.event_model"),
                        });
                    }
                }
                None => {}
            }
            if let Some(id) = j.start.as_ref().and_then(|e| e.id.as_ref()) {
                out.push(OutboundRef {
                    to_kind: Some(EntityKind::Event),
                    to_id: id.clone(),
                    from_field: format!("{prefix}.journey.start"),
                });
            }
            if let Some(id) = j.end.as_ref().and_then(|e| e.id.as_ref()) {
                out.push(OutboundRef {
                    to_kind: Some(EntityKind::Event),
                    to_id: id.clone(),
                    from_field: format!("{prefix}.journey.end"),
                });
            }
        }
        None => {}
    }
}

const MAX_FIELD_DEPTH: usize = 32;

fn collect_field_refs(
    field: &trogon_atlas_proto::FieldSpec,
    prefix: &str,
    out: &mut Vec<OutboundRef>,
    depth: usize,
) {
    if depth >= MAX_FIELD_DEPTH {
        tracing::warn!(
            prefix = prefix,
            "collect_field_refs exceeded max depth ({MAX_FIELD_DEPTH}); stopping recursion"
        );
        return;
    }
    use trogon_atlas_proto::field_type::Kind as FieldTypeKind;
    let kind = field.r#type.as_ref().and_then(|t| t.kind.as_ref());
    if let Some(FieldTypeKind::Ref(rt)) = kind {
        if let Some(inner) = rt.r#ref.as_ref().and_then(|r| r.r#ref.as_ref()) {
            let (ek, id) = match inner {
                type_ref::Ref::Event(x) => (EntityKind::Event, x.id.clone()),
                type_ref::Ref::Command(x) => (EntityKind::Command, x.id.clone()),
                type_ref::Ref::ReadModel(x) => (EntityKind::ReadModel, x.id.clone()),
                type_ref::Ref::Processor(x) => (EntityKind::Processor, x.id.clone()),
                type_ref::Ref::Ui(x) => (EntityKind::Ui, x.id.clone()),
                type_ref::Ref::Persona(x) => (EntityKind::Persona, x.id.clone()),
                type_ref::Ref::Swimlane(x) => (EntityKind::Swimlane, x.id.clone()),
            };
            if let Some(id) = id {
                out.push(OutboundRef {
                    to_kind: Some(ek),
                    to_id: id,
                    from_field: format!("{prefix}.ref"),
                });
            }
        }
    }
    if let Some(FieldTypeKind::Object(obj)) = kind {
        for (i, sub) in obj.fields.iter().enumerate() {
            collect_field_refs(sub, &format!("{prefix}.fields[{i}]"), out, depth + 1);
        }
    }
}

/// Unpack Decision #33 `schema` Any when it carries `trogonatlas.eventmodel.v1alpha1.Schema`
/// and emit TypeRefs from its fields (paths prefixed `schema.fields[...]`).
fn collect_schema_any_field_refs(schema: Option<&prost_types::Any>, out: &mut Vec<OutboundRef>) {
    use prost::Message as _;
    let Some(any) = schema else {
        return;
    };
    if !crate::schema::is_schema(any) {
        return;
    }
    let Ok(s) = trogon_atlas_proto::Schema::decode(any.value.as_slice()) else {
        return;
    };
    for (i, f) in s.fields.iter().enumerate() {
        collect_field_refs(f, &format!("schema.fields[{i}]"), out, 0);
    }
}

/// Retarget TypeRefs inside a Decision #33 `schema` Any when it carries
/// `trogonatlas.eventmodel.v1alpha1.Schema`. Paths are recorded as `schema.fields[...]`.
fn retarget_schema_any_field_refs(
    schema: Option<&mut prost_types::Any>,
    from_kind: EntityKind,
    from_id: &Id,
    to_id: &Id,
    out: &mut Vec<String>,
) {
    use prost::Message as _;
    let Some(any) = schema else {
        return;
    };
    if !crate::schema::is_schema(any) {
        return;
    }
    let Ok(mut s) = trogon_atlas_proto::Schema::decode(any.value.as_slice()) else {
        return;
    };
    let before = out.len();
    for (i, f) in s.fields.iter_mut().enumerate() {
        retarget_field_refs(
            f,
            &format!("schema.fields[{i}]"),
            from_kind,
            from_id,
            to_id,
            out,
            0,
        );
    }
    if out.len() > before {
        any.value = s.encode_to_vec();
    }
}

/// Return the `EntityKind` for `entity` derived from its oneof variant.
///
/// Returns `None` when the entity has no kind set.
#[must_use]
pub fn entity_kind(entity: &Entity) -> Option<EntityKind> {
    Some(match entity.kind.as_ref()? {
        EntityOneof::Event(_) => EntityKind::Event,
        EntityOneof::Command(_) => EntityKind::Command,
        EntityOneof::ReadModel(_) => EntityKind::ReadModel,
        EntityOneof::Processor(_) => EntityKind::Processor,
        EntityOneof::Ui(_) => EntityKind::Ui,
        EntityOneof::Persona(_) => EntityKind::Persona,
        EntityOneof::Swimlane(_) => EntityKind::Swimlane,
        EntityOneof::CommandSlice(_) => EntityKind::CommandSlice,
        EntityOneof::ReadModelSlice(_) => EntityKind::ReadModelSlice,
        EntityOneof::AutomationSlice(_) => EntityKind::AutomationSlice,
        EntityOneof::Storyboard(_) => EntityKind::Storyboard,
        EntityOneof::EventModel(_) => EntityKind::EventModel,
        EntityOneof::Component(_) => EntityKind::Component,
        EntityOneof::ExternalSystem(_) => EntityKind::ExternalSystem,
        EntityOneof::Tracker(_) => EntityKind::Tracker,
        EntityOneof::BoundedContext(_) => EntityKind::BoundedContext,
        EntityOneof::Domain(_) => EntityKind::Domain,
        EntityOneof::Subdomain(_) => EntityKind::Subdomain,
        EntityOneof::Schema(_) => EntityKind::Schema,
        EntityOneof::Project(_) => EntityKind::Project,
        EntityOneof::Screen(_) => EntityKind::Screen,
        EntityOneof::Term(_) => EntityKind::Term,
        EntityOneof::Ambiguity(_) => EntityKind::Ambiguity,
        EntityOneof::UiSlice(_) => EntityKind::UiSlice,
        EntityOneof::ServiceLevelIndicator(_) => EntityKind::ServiceLevelIndicator,
        EntityOneof::ServiceLevelObjective(_) => EntityKind::ServiceLevelObjective,
        EntityOneof::AlertPolicy(_) => EntityKind::AlertPolicy,
        EntityOneof::AlertNotificationTarget(_) => EntityKind::AlertNotificationTarget,
        EntityOneof::TypeLibrary(_) => EntityKind::TypeLibrary,
    })
}

/// Return the `Id` embedded in `entity`'s oneof variant.
///
/// Returns `None` when the entity has no kind set.
#[must_use]
pub fn entity_id(entity: &Entity) -> Option<&Id> {
    match entity.kind.as_ref()? {
        EntityOneof::Event(x) => x.id.as_ref(),
        EntityOneof::Command(x) => x.id.as_ref(),
        EntityOneof::ReadModel(x) => x.id.as_ref(),
        EntityOneof::Processor(x) => x.id.as_ref(),
        EntityOneof::Ui(x) => x.id.as_ref(),
        EntityOneof::Persona(x) => x.id.as_ref(),
        EntityOneof::Swimlane(x) => x.id.as_ref(),
        EntityOneof::CommandSlice(x) => x.id.as_ref(),
        EntityOneof::ReadModelSlice(x) => x.id.as_ref(),
        EntityOneof::AutomationSlice(x) => x.id.as_ref(),
        EntityOneof::Storyboard(x) => x.id.as_ref(),
        EntityOneof::EventModel(x) => x.id.as_ref(),
        EntityOneof::Component(x) => x.id.as_ref(),
        EntityOneof::ExternalSystem(x) => x.id.as_ref(),
        EntityOneof::Tracker(x) => x.id.as_ref(),
        EntityOneof::BoundedContext(x) => x.id.as_ref(),
        EntityOneof::Domain(x) => x.id.as_ref(),
        EntityOneof::Subdomain(x) => x.id.as_ref(),
        EntityOneof::Schema(x) => x.id.as_ref(),
        EntityOneof::Project(x) => x.id.as_ref(),
        EntityOneof::Screen(x) => x.id.as_ref(),
        EntityOneof::Term(x) => x.id.as_ref(),
        EntityOneof::Ambiguity(x) => x.id.as_ref(),
        EntityOneof::UiSlice(x) => x.id.as_ref(),
        EntityOneof::ServiceLevelIndicator(x) => x.id.as_ref(),
        EntityOneof::ServiceLevelObjective(x) => x.id.as_ref(),
        EntityOneof::AlertPolicy(x) => x.id.as_ref(),
        EntityOneof::AlertNotificationTarget(x) => x.id.as_ref(),
        EntityOneof::TypeLibrary(x) => x.id.as_ref(),
    }
}

/// Rewrite every pointer in `entity` that targets `from_id` with kind matching
/// `from_kind` to point at `to_id` instead.
///
/// Returns the list of `from_field` paths that were rewritten. An empty return
/// means no pointers matched. The entity is mutated in place.
///
/// This function is the mutable mirror of `outbound_refs`: it walks the same
/// structural sites in the same order, so the two cannot drift.
pub fn retarget_refs(
    entity: &mut Entity,
    from_kind: EntityKind,
    from_id: &Id,
    to_id: &Id,
) -> Vec<String> {
    let mut rewritten: Vec<String> = Vec::new();
    let Some(kind) = entity.kind.as_mut() else {
        return rewritten;
    };

    // supersedes: only rewrite if this entity's own kind matches from_kind.
    // The supersedes field is an Id (not an EntityRef), so we match by id
    // alone; the kind constraint is enforced by the fact that supersedes
    // always points at the same kind.
    {
        let self_kind_matches = matches_kind(kind, from_kind);
        let supersedes_opt: Option<&mut Id> = match kind {
            EntityOneof::Event(x) => x.supersedes.as_mut(),
            EntityOneof::Command(x) => x.supersedes.as_mut(),
            EntityOneof::ReadModel(x) => x.supersedes.as_mut(),
            EntityOneof::Processor(x) => x.supersedes.as_mut(),
            EntityOneof::Ui(x) => x.supersedes.as_mut(),
            EntityOneof::Persona(x) => x.supersedes.as_mut(),
            EntityOneof::Swimlane(x) => x.supersedes.as_mut(),
            EntityOneof::CommandSlice(x) => x.supersedes.as_mut(),
            EntityOneof::ReadModelSlice(x) => x.supersedes.as_mut(),
            EntityOneof::AutomationSlice(x) => x.supersedes.as_mut(),
            EntityOneof::Storyboard(x) => x.supersedes.as_mut(),
            EntityOneof::EventModel(x) => x.supersedes.as_mut(),
            EntityOneof::Component(x) => x.supersedes.as_mut(),
            EntityOneof::ExternalSystem(x) => x.supersedes.as_mut(),
            EntityOneof::Tracker(x) => x.supersedes.as_mut(),
            EntityOneof::BoundedContext(x) => x.supersedes.as_mut(),
            EntityOneof::Domain(x) => x.supersedes.as_mut(),
            EntityOneof::Subdomain(x) => x.supersedes.as_mut(),
            EntityOneof::Schema(x) => x.supersedes.as_mut(),
            EntityOneof::Project(x) => x.supersedes.as_mut(),
            EntityOneof::Screen(x) => x.supersedes.as_mut(),
            EntityOneof::Term(x) => x.supersedes.as_mut(),
            EntityOneof::Ambiguity(x) => x.supersedes.as_mut(),
            EntityOneof::UiSlice(x) => x.supersedes.as_mut(),
            EntityOneof::ServiceLevelIndicator(x) => x.supersedes.as_mut(),
            EntityOneof::ServiceLevelObjective(x) => x.supersedes.as_mut(),
            EntityOneof::AlertPolicy(x) => x.supersedes.as_mut(),
            EntityOneof::AlertNotificationTarget(x) => x.supersedes.as_mut(),
            EntityOneof::TypeLibrary(x) => x.supersedes.as_mut(),
        };
        if self_kind_matches {
            if let Some(s) = supersedes_opt {
                if s == from_id {
                    *s = to_id.clone();
                    rewritten.push("supersedes".into());
                }
            }
        }
    }

    // Per-kind structural fields.
    match kind {
        EntityOneof::Event(x) => {
            if x.swimlane.as_ref().and_then(|s| s.id.as_ref()) == Some(from_id)
                && from_kind == EntityKind::Swimlane
            {
                if let Some(sl) = x.swimlane.as_mut() {
                    sl.id = Some(to_id.clone());
                    rewritten.push("swimlane".into());
                }
            }
            retarget_schema_any_field_refs(
                x.schema.as_mut(),
                from_kind,
                from_id,
                to_id,
                &mut rewritten,
            );
        }
        EntityOneof::Command(x) => {
            if x.swimlane.as_ref().and_then(|s| s.id.as_ref()) == Some(from_id)
                && from_kind == EntityKind::Swimlane
            {
                if let Some(sl) = x.swimlane.as_mut() {
                    sl.id = Some(to_id.clone());
                    rewritten.push("swimlane".into());
                }
            }
            retarget_schema_any_field_refs(
                x.schema.as_mut(),
                from_kind,
                from_id,
                to_id,
                &mut rewritten,
            );
        }
        EntityOneof::ReadModel(x) => {
            for (i, ev) in x.source_events.iter_mut().enumerate() {
                if ev.id.as_ref() == Some(from_id) && from_kind == EntityKind::Event {
                    ev.id = Some(to_id.clone());
                    rewritten.push(format!("source_events[{i}]"));
                }
            }
            retarget_schema_any_field_refs(
                x.schema.as_mut(),
                from_kind,
                from_id,
                to_id,
                &mut rewritten,
            );
            if from_kind == EntityKind::ExternalSystem {
                if let Some(src) = x.external_source.as_mut() {
                    if src.system.as_ref().and_then(|s| s.id.as_ref()) == Some(from_id) {
                        if let Some(sys) = src.system.as_mut() {
                            sys.id = Some(to_id.clone());
                            rewritten.push("external_source.system".into());
                        }
                    }
                }
            }
        }
        EntityOneof::Processor(x) => {
            for (i, c) in x.calls.iter_mut().enumerate() {
                if c.id.as_ref() == Some(from_id) && from_kind == EntityKind::ExternalSystem {
                    c.id = Some(to_id.clone());
                    rewritten.push(format!("calls[{i}]"));
                }
            }
        }
        EntityOneof::Ui(x) => {
            for (ti, t) in x.transitions.iter_mut().enumerate() {
                if t.to.as_ref().and_then(|r| r.id.as_ref()) == Some(from_id)
                    && from_kind == EntityKind::Ui
                {
                    if let Some(r) = t.to.as_mut() {
                        r.id = Some(to_id.clone());
                        rewritten.push(format!("transitions[{ti}].to"));
                    }
                }
            }
            if from_kind == EntityKind::Screen
                && x.slot
                    .as_ref()
                    .and_then(|s| s.screen.as_ref())
                    .and_then(|sc| sc.id.as_ref())
                    == Some(from_id)
            {
                if let Some(slot) = x.slot.as_mut() {
                    if let Some(screen) = slot.screen.as_mut() {
                        screen.id = Some(to_id.clone());
                        rewritten.push("slot.screen".into());
                    }
                }
            }
        }
        EntityOneof::Persona(_) | EntityOneof::ExternalSystem(_) | EntityOneof::Screen(_) => {}
        EntityOneof::Domain(x) => {
            if from_kind == EntityKind::Project
                && x.project.as_ref().and_then(|p| p.id.as_ref()) == Some(from_id)
            {
                if let Some(p) = x.project.as_mut() {
                    p.id = Some(to_id.clone());
                    rewritten.push("project".into());
                }
            }
        }
        EntityOneof::Schema(x) => {
            for (i, f) in x.fields.iter_mut().enumerate() {
                retarget_field_refs(
                    f,
                    &format!("fields[{i}]"),
                    from_kind,
                    from_id,
                    to_id,
                    &mut rewritten,
                    0,
                );
            }
        }
        EntityOneof::Swimlane(x) => {
            for (i, f) in x.state.iter_mut().enumerate() {
                retarget_field_refs(
                    f,
                    &format!("state[{i}]"),
                    from_kind,
                    from_id,
                    to_id,
                    &mut rewritten,
                    0,
                );
            }
            for (ti, t) in x.transitions.iter_mut().enumerate() {
                if t.after.as_ref().and_then(|r| r.id.as_ref()) == Some(from_id)
                    && from_kind == EntityKind::Event
                {
                    if let Some(r) = t.after.as_mut() {
                        r.id = Some(to_id.clone());
                        rewritten.push(format!("transitions[{ti}].after"));
                    }
                }
                for (ni, n) in t.next.iter_mut().enumerate() {
                    if n.event.as_ref().and_then(|r| r.id.as_ref()) == Some(from_id)
                        && from_kind == EntityKind::Event
                    {
                        if let Some(r) = n.event.as_mut() {
                            r.id = Some(to_id.clone());
                            rewritten.push(format!("transitions[{ti}].next[{ni}]"));
                        }
                    }
                }
            }
        }
        EntityOneof::CommandSlice(x) => {
            if from_kind == EntityKind::Persona
                && x.persona
                    .as_ref()
                    .and_then(|p| p.persona.as_ref())
                    .and_then(|r| r.id.as_ref())
                    == Some(from_id)
            {
                if let Some(p) = x.persona.as_mut() {
                    if let Some(r) = p.persona.as_mut() {
                        r.id = Some(to_id.clone());
                        rewritten.push("persona".into());
                    }
                }
            }
            if from_kind == EntityKind::Ui
                && x.ui
                    .as_ref()
                    .and_then(|u| u.ui.as_ref())
                    .and_then(|r| r.id.as_ref())
                    == Some(from_id)
            {
                if let Some(u) = x.ui.as_mut() {
                    if let Some(r) = u.ui.as_mut() {
                        r.id = Some(to_id.clone());
                        rewritten.push("ui".into());
                    }
                }
            }
            if from_kind == EntityKind::Command
                && x.command
                    .as_ref()
                    .and_then(|c| c.command.as_ref())
                    .and_then(|r| r.id.as_ref())
                    == Some(from_id)
            {
                if let Some(c) = x.command.as_mut() {
                    if let Some(r) = c.command.as_mut() {
                        r.id = Some(to_id.clone());
                        rewritten.push("command".into());
                    }
                }
            }
            if from_kind == EntityKind::Event {
                for (i, e) in x.emitted_events.iter_mut().enumerate() {
                    if e.event.as_ref().and_then(|r| r.id.as_ref()) == Some(from_id) {
                        if let Some(r) = e.event.as_mut() {
                            r.id = Some(to_id.clone());
                            rewritten.push(format!("emitted_events[{i}]"));
                        }
                    }
                }
            }
            for (si, sc) in x.scenarios.iter_mut().enumerate() {
                if from_kind == EntityKind::Event {
                    for (gi, g) in sc.given.iter_mut().enumerate() {
                        if g.event.as_ref().and_then(|r| r.id.as_ref()) == Some(from_id) {
                            if let Some(r) = g.event.as_mut() {
                                r.id = Some(to_id.clone());
                                rewritten.push(format!("scenarios[{si}].given[{gi}].event"));
                            }
                        }
                    }
                }
                if from_kind == EntityKind::Command
                    && sc
                        .when
                        .as_ref()
                        .and_then(|w| w.command.as_ref())
                        .and_then(|r| r.id.as_ref())
                        == Some(from_id)
                {
                    if let Some(w) = sc.when.as_mut() {
                        if let Some(r) = w.command.as_mut() {
                            r.id = Some(to_id.clone());
                            rewritten.push(format!("scenarios[{si}].when.command"));
                        }
                    }
                }
                if from_kind == EntityKind::Event {
                    if let Some(trogon_atlas_proto::command_scenario::Then::Emit(em)) =
                        sc.then.as_mut()
                    {
                        for (ei, e) in em.events.iter_mut().enumerate() {
                            if e.event.as_ref().and_then(|r| r.id.as_ref()) == Some(from_id) {
                                if let Some(r) = e.event.as_mut() {
                                    r.id = Some(to_id.clone());
                                    rewritten.push(format!(
                                        "scenarios[{si}].then.emit.events[{ei}].event"
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        EntityOneof::ReadModelSlice(x) => {
            if from_kind == EntityKind::Event {
                for (i, e) in x.source_events.iter_mut().enumerate() {
                    if e.event.as_ref().and_then(|r| r.id.as_ref()) == Some(from_id) {
                        if let Some(r) = e.event.as_mut() {
                            r.id = Some(to_id.clone());
                            rewritten.push(format!("source_events[{i}]"));
                        }
                    }
                }
            }
            if from_kind == EntityKind::ReadModel
                && x.read_model
                    .as_ref()
                    .and_then(|rm| rm.read_model.as_ref())
                    .and_then(|r| r.id.as_ref())
                    == Some(from_id)
            {
                if let Some(rm) = x.read_model.as_mut() {
                    if let Some(r) = rm.read_model.as_mut() {
                        r.id = Some(to_id.clone());
                        rewritten.push("read_model".into());
                    }
                }
            }
            for (si, sc) in x.scenarios.iter_mut().enumerate() {
                if from_kind == EntityKind::Event {
                    for (wi, w) in sc.when.iter_mut().enumerate() {
                        if w.event.as_ref().and_then(|r| r.id.as_ref()) == Some(from_id) {
                            if let Some(r) = w.event.as_mut() {
                                r.id = Some(to_id.clone());
                                rewritten.push(format!("scenarios[{si}].when[{wi}].event"));
                            }
                        }
                    }
                }
                if from_kind == EntityKind::ReadModel
                    && sc
                        .then
                        .as_ref()
                        .and_then(|t| t.read_model.as_ref())
                        .and_then(|r| r.id.as_ref())
                        == Some(from_id)
                {
                    if let Some(t) = sc.then.as_mut() {
                        if let Some(r) = t.read_model.as_mut() {
                            r.id = Some(to_id.clone());
                            rewritten.push(format!("scenarios[{si}].then.read_model"));
                        }
                    }
                }
            }
        }
        EntityOneof::AutomationSlice(x) => {
            if from_kind == EntityKind::ReadModel {
                for (i, e) in x.source_read_models.iter_mut().enumerate() {
                    if e.read_model.as_ref().and_then(|r| r.id.as_ref()) == Some(from_id) {
                        if let Some(r) = e.read_model.as_mut() {
                            r.id = Some(to_id.clone());
                            rewritten.push(format!("source_read_models[{i}]"));
                        }
                    }
                }
            }
            if from_kind == EntityKind::Processor
                && x.processor
                    .as_ref()
                    .and_then(|p| p.processor.as_ref())
                    .and_then(|r| r.id.as_ref())
                    == Some(from_id)
            {
                if let Some(p) = x.processor.as_mut() {
                    if let Some(r) = p.processor.as_mut() {
                        r.id = Some(to_id.clone());
                        rewritten.push("processor".into());
                    }
                }
            }
            if from_kind == EntityKind::Command
                && x.emitted_command
                    .as_ref()
                    .and_then(|c| c.command.as_ref())
                    .and_then(|r| r.id.as_ref())
                    == Some(from_id)
            {
                if let Some(c) = x.emitted_command.as_mut() {
                    if let Some(r) = c.command.as_mut() {
                        r.id = Some(to_id.clone());
                        rewritten.push("emitted_command".into());
                    }
                }
            }
            for (si, sc) in x.scenarios.iter_mut().enumerate() {
                if from_kind == EntityKind::ReadModel {
                    for (gi, g) in sc.given.iter_mut().enumerate() {
                        if g.read_model.as_ref().and_then(|r| r.id.as_ref()) == Some(from_id) {
                            if let Some(r) = g.read_model.as_mut() {
                                r.id = Some(to_id.clone());
                                rewritten.push(format!("scenarios[{si}].given[{gi}].read_model"));
                            }
                        }
                    }
                }
                if from_kind == EntityKind::Command
                    && sc
                        .then
                        .as_ref()
                        .and_then(|t| t.command.as_ref())
                        .and_then(|r| r.id.as_ref())
                        == Some(from_id)
                {
                    if let Some(t) = sc.then.as_mut() {
                        if let Some(r) = t.command.as_mut() {
                            r.id = Some(to_id.clone());
                            rewritten.push(format!("scenarios[{si}].then.command"));
                        }
                    }
                }
            }
        }
        EntityOneof::UiSlice(x) => {
            if from_kind == EntityKind::Persona
                && x.persona
                    .as_ref()
                    .and_then(|p| p.persona.as_ref())
                    .and_then(|r| r.id.as_ref())
                    == Some(from_id)
            {
                if let Some(p) = x.persona.as_mut() {
                    if let Some(r) = p.persona.as_mut() {
                        r.id = Some(to_id.clone());
                        rewritten.push("persona".into());
                    }
                }
            }
            if from_kind == EntityKind::ReadModel {
                for (i, e) in x.source_read_models.iter_mut().enumerate() {
                    if e.read_model.as_ref().and_then(|r| r.id.as_ref()) == Some(from_id) {
                        if let Some(r) = e.read_model.as_mut() {
                            r.id = Some(to_id.clone());
                            rewritten.push(format!("source_read_models[{i}]"));
                        }
                    }
                }
            }
            if from_kind == EntityKind::Ui
                && x.ui
                    .as_ref()
                    .and_then(|u| u.ui.as_ref())
                    .and_then(|r| r.id.as_ref())
                    == Some(from_id)
            {
                if let Some(u) = x.ui.as_mut() {
                    if let Some(r) = u.ui.as_mut() {
                        r.id = Some(to_id.clone());
                        rewritten.push("ui".into());
                    }
                }
            }
        }
        EntityOneof::Storyboard(x) => {
            // entry.read_model
            if from_kind == EntityKind::ReadModel
                && x.entry
                    .as_ref()
                    .and_then(|e| e.read_model.as_ref())
                    .and_then(|rm| rm.read_model.as_ref())
                    .and_then(|r| r.id.as_ref())
                    == Some(from_id)
            {
                if let Some(entry) = x.entry.as_mut() {
                    if let Some(rm) = entry.read_model.as_mut() {
                        if let Some(r) = rm.read_model.as_mut() {
                            r.id = Some(to_id.clone());
                            rewritten.push("entry.read_model".into());
                        }
                    }
                }
            }
            // entry.observer.human.persona and entry.observer.human.ui
            if let Some(entry) = x.entry.as_mut() {
                if let Some(obs) = entry.observer.as_mut() {
                    match obs {
                        trogon_atlas_proto::storyboard_entry::Observer::Human(h) => {
                            if from_kind == EntityKind::Persona
                                && h.persona
                                    .as_ref()
                                    .and_then(|p| p.persona.as_ref())
                                    .and_then(|r| r.id.as_ref())
                                    == Some(from_id)
                            {
                                if let Some(p) = h.persona.as_mut() {
                                    if let Some(r) = p.persona.as_mut() {
                                        r.id = Some(to_id.clone());
                                        rewritten.push("entry.observer.human.persona".into());
                                    }
                                }
                            }
                            if from_kind == EntityKind::Ui
                                && h.ui
                                    .as_ref()
                                    .and_then(|u| u.ui.as_ref())
                                    .and_then(|r| r.id.as_ref())
                                    == Some(from_id)
                            {
                                if let Some(u) = h.ui.as_mut() {
                                    if let Some(r) = u.ui.as_mut() {
                                        r.id = Some(to_id.clone());
                                        rewritten.push("entry.observer.human.ui".into());
                                    }
                                }
                            }
                        }
                        trogon_atlas_proto::storyboard_entry::Observer::Automation(a) => {
                            if from_kind == EntityKind::Processor
                                && a.processor
                                    .as_ref()
                                    .and_then(|p| p.processor.as_ref())
                                    .and_then(|r| r.id.as_ref())
                                    == Some(from_id)
                            {
                                if let Some(p) = a.processor.as_mut() {
                                    if let Some(r) = p.processor.as_mut() {
                                        r.id = Some(to_id.clone());
                                        rewritten
                                            .push("entry.observer.automation.processor".into());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // outcome.events and outcome.ui
            if let Some(outcome) = x.outcome.as_mut() {
                if from_kind == EntityKind::Event {
                    for (i, e) in outcome.events.iter_mut().enumerate() {
                        if e.event.as_ref().and_then(|r| r.id.as_ref()) == Some(from_id) {
                            if let Some(r) = e.event.as_mut() {
                                r.id = Some(to_id.clone());
                                rewritten.push(format!("outcome.events[{i}]"));
                            }
                        }
                    }
                }
                if from_kind == EntityKind::Ui
                    && outcome
                        .ui
                        .as_ref()
                        .and_then(|u| u.ui.as_ref())
                        .and_then(|r| r.id.as_ref())
                        == Some(from_id)
                {
                    if let Some(u) = outcome.ui.as_mut() {
                        if let Some(r) = u.ui.as_mut() {
                            r.id = Some(to_id.clone());
                            rewritten.push("outcome.ui".into());
                        }
                    }
                }
            }
            // slices (untyped SliceRef)
            for (i, sr) in x.slices.iter_mut().enumerate() {
                if sr.id.as_ref() == Some(from_id)
                    && matches!(
                        from_kind,
                        EntityKind::CommandSlice
                            | EntityKind::ReadModelSlice
                            | EntityKind::AutomationSlice
                            | EntityKind::UiSlice
                    )
                {
                    sr.id = Some(to_id.clone());
                    rewritten.push(format!("slices[{i}]"));
                }
            }
            // traces[].steps[].slice
            for (ti, t) in x.traces.iter_mut().enumerate() {
                for (si, step) in t.steps.iter_mut().enumerate() {
                    if step.slice.as_ref().and_then(|s| s.id.as_ref()) == Some(from_id)
                        && matches!(
                            from_kind,
                            EntityKind::CommandSlice
                                | EntityKind::ReadModelSlice
                                | EntityKind::AutomationSlice
                                | EntityKind::UiSlice
                        )
                    {
                        if let Some(s) = step.slice.as_mut() {
                            s.id = Some(to_id.clone());
                            rewritten.push(format!("traces[{ti}].steps[{si}].slice"));
                        }
                    }
                }
            }
        }
        EntityOneof::EventModel(x) => {
            for (i, m) in x.members.iter_mut().enumerate() {
                if m.id.as_ref() == Some(from_id) {
                    match EntityKind::try_from(m.kind) {
                        Ok(k) if k == from_kind => {
                            m.id = Some(to_id.clone());
                            rewritten.push(format!("members[{i}]"));
                        }
                        _ => {}
                    }
                }
            }
        }
        EntityOneof::Component(x) => {
            for (i, m) in x.members.iter_mut().enumerate() {
                if m.id.as_ref() == Some(from_id) {
                    match EntityKind::try_from(m.kind) {
                        Ok(k) if k == from_kind => {
                            m.id = Some(to_id.clone());
                            rewritten.push(format!("members[{i}]"));
                        }
                        _ => {}
                    }
                }
            }
        }
        EntityOneof::BoundedContext(x) => {
            if from_kind == EntityKind::Subdomain {
                for (i, r) in x.realizes.iter_mut().enumerate() {
                    if r.id.as_ref() == Some(from_id) {
                        r.id = Some(to_id.clone());
                        rewritten.push(format!("realizes[{i}]"));
                    }
                }
            }
            if from_kind == EntityKind::BoundedContext {
                for (i, r) in x.relationships.iter_mut().enumerate() {
                    if r.upstream.as_ref().and_then(|u| u.id.as_ref()) == Some(from_id) {
                        if let Some(u) = r.upstream.as_mut() {
                            u.id = Some(to_id.clone());
                            rewritten.push(format!("relationships[{i}].upstream"));
                        }
                    }
                }
            }
        }
        EntityOneof::Subdomain(x) => {
            if from_kind == EntityKind::Domain
                && x.domain.as_ref().and_then(|r| r.id.as_ref()) == Some(from_id)
            {
                if let Some(r) = x.domain.as_mut() {
                    r.id = Some(to_id.clone());
                    rewritten.push("domain".into());
                }
            }
        }
        EntityOneof::Tracker(x) => {
            for (i, item) in x.items.iter_mut().enumerate() {
                if let Some(subject) = item.subject.as_mut() {
                    if subject.id.as_ref() == Some(from_id) {
                        if let Ok(k) = EntityKind::try_from(subject.kind) {
                            if k == from_kind {
                                subject.id = Some(to_id.clone());
                                rewritten.push(format!("items[{i}].subject"));
                            }
                        }
                    }
                }
            }
        }
        EntityOneof::Project(x) => {
            let _ = x;
        }
        EntityOneof::Term(x) => {
            for (i, r) in x.embodied_by.iter_mut().enumerate() {
                if r.id.as_ref() == Some(from_id) {
                    if let Ok(k) = EntityKind::try_from(r.kind) {
                        if k == from_kind {
                            r.id = Some(to_id.clone());
                            rewritten.push(format!("embodied_by[{i}]"));
                        }
                    }
                }
            }
        }
        EntityOneof::Ambiguity(x) => {
            if from_kind == EntityKind::Term {
                for (i, r) in x.terms.iter_mut().enumerate() {
                    if r.id.as_ref() == Some(from_id) {
                        r.id = Some(to_id.clone());
                        rewritten.push(format!("terms[{i}]"));
                    }
                }
            }
        }
        EntityOneof::ServiceLevelIndicator(x) => {
            if let Some(conn) = x.connection.as_mut() {
                connection_retarget(
                    conn,
                    "connection",
                    from_kind,
                    from_id,
                    to_id,
                    &mut rewritten,
                );
            }
            if from_kind == EntityKind::Event {
                if let Some(trogon_atlas_proto::service_level_indicator::Measure::OutcomeRatio(
                    or,
                )) = x.measure.as_mut()
                {
                    for (i, r) in or.good.iter_mut().enumerate() {
                        if r.id.as_ref() == Some(from_id) {
                            r.id = Some(to_id.clone());
                            rewritten.push(format!("measure.outcome_ratio.good[{i}]"));
                        }
                    }
                    for (i, r) in or.bad.iter_mut().enumerate() {
                        if r.id.as_ref() == Some(from_id) {
                            r.id = Some(to_id.clone());
                            rewritten.push(format!("measure.outcome_ratio.bad[{i}]"));
                        }
                    }
                }
            }
        }
        EntityOneof::ServiceLevelObjective(x) => {
            if from_kind == EntityKind::Component
                && x.service.as_ref().and_then(|r| r.id.as_ref()) == Some(from_id)
            {
                if let Some(r) = x.service.as_mut() {
                    r.id = Some(to_id.clone());
                    rewritten.push("service".into());
                }
            }
            if from_kind == EntityKind::ServiceLevelIndicator
                && x.indicator.as_ref().and_then(|r| r.id.as_ref()) == Some(from_id)
            {
                if let Some(r) = x.indicator.as_mut() {
                    r.id = Some(to_id.clone());
                    rewritten.push("indicator".into());
                }
            }
            if from_kind == EntityKind::AlertPolicy {
                for (i, r) in x.alert_policies.iter_mut().enumerate() {
                    if r.id.as_ref() == Some(from_id) {
                        r.id = Some(to_id.clone());
                        rewritten.push(format!("alert_policies[{i}]"));
                    }
                }
            }
            if let Some(trogon_atlas_proto::commitment::Kind::External(ext)) =
                x.commitment.as_mut().and_then(|c| c.kind.as_mut())
            {
                match ext.counterparty.as_mut() {
                    Some(trogon_atlas_proto::commitment::external::Counterparty::System(r))
                        if from_kind == EntityKind::ExternalSystem
                            && r.id.as_ref() == Some(from_id) =>
                    {
                        r.id = Some(to_id.clone());
                        rewritten.push("commitment.external.system".into());
                    }
                    Some(trogon_atlas_proto::commitment::external::Counterparty::Persona(r))
                        if from_kind == EntityKind::Persona && r.id.as_ref() == Some(from_id) =>
                    {
                        r.id = Some(to_id.clone());
                        rewritten.push("commitment.external.persona".into());
                    }
                    _ => {}
                }
            }
        }
        EntityOneof::AlertPolicy(x) => {
            if from_kind == EntityKind::AlertNotificationTarget {
                for (i, r) in x.notification_targets.iter_mut().enumerate() {
                    if r.id.as_ref() == Some(from_id) {
                        r.id = Some(to_id.clone());
                        rewritten.push(format!("notification_targets[{i}]"));
                    }
                }
            }
        }
        EntityOneof::AlertNotificationTarget(x) => {
            let _ = x;
        }
        EntityOneof::TypeLibrary(x) => {
            if from_kind == EntityKind::TypeLibrary {
                for (i, r) in x.dependencies.iter_mut().enumerate() {
                    if r.id.as_ref() == Some(from_id) {
                        r.id = Some(to_id.clone());
                        rewritten.push(format!("dependencies[{i}]"));
                    }
                }
            }
        }
    }

    rewritten
}

/// Mutable mirror of `connection_refs`; walks the same sites in the same
/// order so the two cannot drift.
fn connection_retarget(
    conn: &mut trogon_atlas_proto::Connection,
    prefix: &str,
    from_kind: EntityKind,
    from_id: &Id,
    to_id: &Id,
    rewritten: &mut Vec<String>,
) {
    use trogon_atlas_proto::connection::{journey::Scope, Kind as ConnKind};
    let is_slice_kind = matches!(
        from_kind,
        EntityKind::CommandSlice
            | EntityKind::ReadModelSlice
            | EntityKind::AutomationSlice
            | EntityKind::UiSlice
    );
    match conn.kind.as_mut() {
        Some(ConnKind::CommandHandling(ch)) => {
            if is_slice_kind
                && ch.command_slice.as_ref().and_then(|s| s.id.as_ref()) == Some(from_id)
            {
                if let Some(s) = ch.command_slice.as_mut() {
                    s.id = Some(to_id.clone());
                    rewritten.push(format!("{prefix}.command_handling.command_slice"));
                }
            }
            if from_kind == EntityKind::Event {
                for (i, r) in ch.events.iter_mut().enumerate() {
                    if r.id.as_ref() == Some(from_id) {
                        r.id = Some(to_id.clone());
                        rewritten.push(format!("{prefix}.command_handling.events[{i}]"));
                    }
                }
            }
        }
        Some(ConnKind::Projection(p)) => {
            if is_slice_kind
                && p.read_model_slice.as_ref().and_then(|s| s.id.as_ref()) == Some(from_id)
            {
                if let Some(s) = p.read_model_slice.as_mut() {
                    s.id = Some(to_id.clone());
                    rewritten.push(format!("{prefix}.projection.read_model_slice"));
                }
            }
            if from_kind == EntityKind::Event {
                for (i, r) in p.source_events.iter_mut().enumerate() {
                    if r.id.as_ref() == Some(from_id) {
                        r.id = Some(to_id.clone());
                        rewritten.push(format!("{prefix}.projection.source_events[{i}]"));
                    }
                }
            }
        }
        Some(ConnKind::Reaction(r)) => {
            if is_slice_kind
                && r.automation_slice.as_ref().and_then(|s| s.id.as_ref()) == Some(from_id)
            {
                if let Some(s) = r.automation_slice.as_mut() {
                    s.id = Some(to_id.clone());
                    rewritten.push(format!("{prefix}.reaction.automation_slice"));
                }
            }
        }
        Some(ConnKind::Display(d)) => {
            if is_slice_kind && d.ui_slice.as_ref().and_then(|s| s.id.as_ref()) == Some(from_id) {
                if let Some(s) = d.ui_slice.as_mut() {
                    s.id = Some(to_id.clone());
                    rewritten.push(format!("{prefix}.display.ui_slice"));
                }
            }
        }
        Some(ConnKind::Integration(i)) => {
            if from_kind == EntityKind::Processor
                && i.processor.as_ref().and_then(|p| p.id.as_ref()) == Some(from_id)
            {
                if let Some(p) = i.processor.as_mut() {
                    p.id = Some(to_id.clone());
                    rewritten.push(format!("{prefix}.integration.processor"));
                }
            }
            if from_kind == EntityKind::ExternalSystem
                && i.system.as_ref().and_then(|s| s.id.as_ref()) == Some(from_id)
            {
                if let Some(s) = i.system.as_mut() {
                    s.id = Some(to_id.clone());
                    rewritten.push(format!("{prefix}.integration.system"));
                }
            }
        }
        Some(ConnKind::Journey(j)) => {
            match j.scope.as_mut() {
                Some(Scope::Storyboard(r))
                    if from_kind == EntityKind::Storyboard && r.id.as_ref() == Some(from_id) =>
                {
                    r.id = Some(to_id.clone());
                    rewritten.push(format!("{prefix}.journey.storyboard"));
                }
                Some(Scope::EventModel(r))
                    if from_kind == EntityKind::EventModel && r.id.as_ref() == Some(from_id) =>
                {
                    r.id = Some(to_id.clone());
                    rewritten.push(format!("{prefix}.journey.event_model"));
                }
                _ => {}
            }
            if from_kind == EntityKind::Event {
                if j.start.as_ref().and_then(|e| e.id.as_ref()) == Some(from_id) {
                    if let Some(r) = j.start.as_mut() {
                        r.id = Some(to_id.clone());
                        rewritten.push(format!("{prefix}.journey.start"));
                    }
                }
                if j.end.as_ref().and_then(|e| e.id.as_ref()) == Some(from_id) {
                    if let Some(r) = j.end.as_mut() {
                        r.id = Some(to_id.clone());
                        rewritten.push(format!("{prefix}.journey.end"));
                    }
                }
            }
        }
        None => {}
    }
}

/// Returns true when the entity oneof variant corresponds to `kind`.
fn matches_kind(kind: &EntityOneof, ek: EntityKind) -> bool {
    matches!(
        (kind, ek),
        (EntityOneof::Event(_), EntityKind::Event)
            | (EntityOneof::Command(_), EntityKind::Command)
            | (EntityOneof::ReadModel(_), EntityKind::ReadModel)
            | (EntityOneof::Processor(_), EntityKind::Processor)
            | (EntityOneof::Ui(_), EntityKind::Ui)
            | (EntityOneof::Persona(_), EntityKind::Persona)
            | (EntityOneof::Swimlane(_), EntityKind::Swimlane)
            | (EntityOneof::CommandSlice(_), EntityKind::CommandSlice)
            | (EntityOneof::ReadModelSlice(_), EntityKind::ReadModelSlice)
            | (EntityOneof::AutomationSlice(_), EntityKind::AutomationSlice)
            | (EntityOneof::UiSlice(_), EntityKind::UiSlice)
            | (EntityOneof::Storyboard(_), EntityKind::Storyboard)
            | (EntityOneof::EventModel(_), EntityKind::EventModel)
            | (EntityOneof::Component(_), EntityKind::Component)
            | (EntityOneof::ExternalSystem(_), EntityKind::ExternalSystem)
            | (EntityOneof::Tracker(_), EntityKind::Tracker)
            | (EntityOneof::BoundedContext(_), EntityKind::BoundedContext)
            | (EntityOneof::Domain(_), EntityKind::Domain)
            | (EntityOneof::Subdomain(_), EntityKind::Subdomain)
            | (EntityOneof::Schema(_), EntityKind::Schema)
            | (EntityOneof::Project(_), EntityKind::Project)
            | (EntityOneof::Screen(_), EntityKind::Screen)
            | (EntityOneof::Term(_), EntityKind::Term)
            | (EntityOneof::Ambiguity(_), EntityKind::Ambiguity)
            | (
                EntityOneof::ServiceLevelIndicator(_),
                EntityKind::ServiceLevelIndicator
            )
            | (
                EntityOneof::ServiceLevelObjective(_),
                EntityKind::ServiceLevelObjective
            )
            | (EntityOneof::AlertPolicy(_), EntityKind::AlertPolicy)
            | (
                EntityOneof::AlertNotificationTarget(_),
                EntityKind::AlertNotificationTarget
            )
            | (EntityOneof::TypeLibrary(_), EntityKind::TypeLibrary)
    )
}

const MAX_RETARGET_DEPTH: usize = 32;

fn retarget_field_refs(
    field: &mut trogon_atlas_proto::FieldSpec,
    prefix: &str,
    from_kind: EntityKind,
    from_id: &Id,
    to_id: &Id,
    out: &mut Vec<String>,
    depth: usize,
) {
    if depth >= MAX_RETARGET_DEPTH {
        return;
    }
    use trogon_atlas_proto::field_type::Kind as FieldTypeKind;
    // Determine whether the field is Ref or Object without borrowing mutably yet.
    let is_ref = field
        .r#type
        .as_ref()
        .and_then(|t| t.kind.as_ref())
        .is_some_and(|k| matches!(k, FieldTypeKind::Ref(_)));
    let is_object = field
        .r#type
        .as_ref()
        .and_then(|t| t.kind.as_ref())
        .is_some_and(|k| matches!(k, FieldTypeKind::Object(_)));

    if is_ref {
        if let Some(FieldTypeKind::Ref(rt)) = field.r#type.as_mut().and_then(|t| t.kind.as_mut()) {
            if let Some(inner) = rt.r#ref.as_mut().and_then(|r| r.r#ref.as_mut()) {
                let matched = match (&*inner, from_kind) {
                    (trogon_atlas_proto::type_ref::Ref::Event(x), EntityKind::Event) => {
                        x.id.as_ref() == Some(from_id)
                    }
                    (trogon_atlas_proto::type_ref::Ref::Command(x), EntityKind::Command) => {
                        x.id.as_ref() == Some(from_id)
                    }
                    (trogon_atlas_proto::type_ref::Ref::ReadModel(x), EntityKind::ReadModel) => {
                        x.id.as_ref() == Some(from_id)
                    }
                    (trogon_atlas_proto::type_ref::Ref::Processor(x), EntityKind::Processor) => {
                        x.id.as_ref() == Some(from_id)
                    }
                    (trogon_atlas_proto::type_ref::Ref::Ui(x), EntityKind::Ui) => {
                        x.id.as_ref() == Some(from_id)
                    }
                    (trogon_atlas_proto::type_ref::Ref::Persona(x), EntityKind::Persona) => {
                        x.id.as_ref() == Some(from_id)
                    }
                    (trogon_atlas_proto::type_ref::Ref::Swimlane(x), EntityKind::Swimlane) => {
                        x.id.as_ref() == Some(from_id)
                    }
                    _ => false,
                };
                if matched {
                    out.push(format!("{prefix}.ref"));
                    match inner {
                        trogon_atlas_proto::type_ref::Ref::Event(x) => x.id = Some(to_id.clone()),
                        trogon_atlas_proto::type_ref::Ref::Command(x) => x.id = Some(to_id.clone()),
                        trogon_atlas_proto::type_ref::Ref::ReadModel(x) => {
                            x.id = Some(to_id.clone());
                        }
                        trogon_atlas_proto::type_ref::Ref::Processor(x) => {
                            x.id = Some(to_id.clone());
                        }
                        trogon_atlas_proto::type_ref::Ref::Ui(x) => x.id = Some(to_id.clone()),
                        trogon_atlas_proto::type_ref::Ref::Persona(x) => x.id = Some(to_id.clone()),
                        trogon_atlas_proto::type_ref::Ref::Swimlane(x) => {
                            x.id = Some(to_id.clone());
                        }
                    }
                }
            }
        }
    } else if is_object {
        let n = field
            .r#type
            .as_ref()
            .and_then(|t| t.kind.as_ref())
            .and_then(|k| {
                if let FieldTypeKind::Object(o) = k {
                    Some(o.fields.len())
                } else {
                    None
                }
            })
            .unwrap_or(0);
        for i in 0..n {
            if let Some(FieldTypeKind::Object(obj)) =
                field.r#type.as_mut().and_then(|t| t.kind.as_mut())
            {
                retarget_field_refs(
                    &mut obj.fields[i],
                    &format!("{prefix}.fields[{i}]"),
                    from_kind,
                    from_id,
                    to_id,
                    out,
                    depth + 1,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use trogon_atlas_proto::{
        bounded_context::ContextRelationship, field_type::RefType, tracker, *,
    };

    use super::*;

    fn id(ns: &str, slug: &str) -> Id {
        Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn make_entity(kind: entity::Kind) -> Entity {
        Entity {
            system: None,
            kind: Some(kind),
        }
    }

    #[test]
    fn event_outbound_refs_swimlane_and_field_ref() {
        let entity = make_entity(entity::Kind::Event(Event {
            id: Some(id("ns", "order.placed")),
            swimlane: Some(SwimlaneRef {
                id: Some(id("ns", "orders")),
            }),
            schema: Some(crate::schema::pack_fields(vec![FieldSpec {
                name: "order_id".into(),
                r#type: Some(FieldType {
                    kind: Some(field_type::Kind::Ref(RefType {
                        r#ref: Some(TypeRef {
                            r#ref: Some(type_ref::Ref::Event(EventRef {
                                id: Some(id("ns", "other-event")),
                            })),
                        }),
                    })),
                }),
                ..Default::default()
            }])),
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        let kinds: Vec<_> = refs.iter().map(|r| r.to_kind).collect();
        assert!(
            kinds.contains(&Some(EntityKind::Swimlane)),
            "swimlane ref missing"
        );
        assert!(
            kinds.contains(&Some(EntityKind::Event)),
            "field ref missing"
        );
    }

    #[test]
    fn command_outbound_refs_swimlane() {
        let entity = make_entity(entity::Kind::Command(Command {
            id: Some(id("ns", "place-order")),
            swimlane: Some(SwimlaneRef {
                id: Some(id("ns", "orders")),
            }),
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].to_kind, Some(EntityKind::Swimlane));
    }

    #[test]
    fn read_model_outbound_refs_source_events() {
        let entity = make_entity(entity::Kind::ReadModel(ReadModel {
            id: Some(id("ns", "orders-view")),
            source_events: vec![EventRef {
                id: Some(id("ns", "order.placed")),
            }],
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].to_kind, Some(EntityKind::Event));
        assert_eq!(refs[0].from_field, "source_events[0]");
    }

    #[test]
    fn command_slice_outbound_refs() {
        let entity = make_entity(entity::Kind::CommandSlice(CommandSlice {
            id: Some(id("ns", "place-slice")),
            command: Some(CommandEdge {
                command: Some(CommandRef {
                    id: Some(id("ns", "place-order")),
                }),
                ..Default::default()
            }),
            emitted_events: vec![EventEdge {
                event: Some(EventRef {
                    id: Some(id("ns", "order.placed")),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        let fields: Vec<_> = refs.iter().map(|r| r.from_field.as_str()).collect();
        assert!(fields.contains(&"command"), "command ref missing");
        assert!(fields.contains(&"emitted_events[0]"), "event ref missing");
        assert!(refs.iter().all(|r| r.to_kind.is_some()));
    }

    #[test]
    fn read_model_slice_outbound_refs() {
        let entity = make_entity(entity::Kind::ReadModelSlice(ReadModelSlice {
            id: Some(id("ns", "rm-slice")),
            read_model: Some(ReadModelEdge {
                read_model: Some(ReadModelRef {
                    id: Some(id("ns", "orders-view")),
                }),
                ..Default::default()
            }),
            source_events: vec![EventEdge {
                event: Some(EventRef {
                    id: Some(id("ns", "order.placed")),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        let kinds: Vec<_> = refs.iter().map(|r| r.to_kind).collect();
        assert!(kinds.contains(&Some(EntityKind::ReadModel)));
        assert!(kinds.contains(&Some(EntityKind::Event)));
    }

    #[test]
    fn automation_slice_outbound_refs() {
        let entity = make_entity(entity::Kind::AutomationSlice(AutomationSlice {
            id: Some(id("ns", "auto-slice")),
            emitted_command: Some(CommandEdge {
                command: Some(CommandRef {
                    id: Some(id("ns", "do-thing")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].to_kind, Some(EntityKind::Command));
    }

    #[test]
    fn storyboard_slice_refs_are_none_kind() {
        let entity = make_entity(entity::Kind::Storyboard(Storyboard {
            id: Some(id("ns", "sb")),
            slices: vec![SliceRef {
                id: Some(id("ns", "slice-1")),
            }],
            traces: vec![StoryboardTrace {
                steps: vec![TraceStep {
                    slice: Some(SliceRef {
                        id: Some(id("ns", "slice-2")),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 2);
        for r in &refs {
            assert_eq!(r.to_kind, None, "storyboard slice ref must have None kind");
        }
    }

    #[test]
    fn storyboard_to_entity_ref_returns_none_for_slice_ref() {
        let r = OutboundRef {
            to_kind: None,
            to_id: id("ns", "slice-1"),
            from_field: "slices[0]".into(),
        };
        assert!(r.to_entity_ref().is_none());
    }

    #[test]
    fn event_model_unknown_kind_is_skipped() {
        let entity = make_entity(entity::Kind::EventModel(EventModel {
            id: Some(id("ns", "em")),
            members: vec![
                EntityRef {
                    kind: EntityKind::Event as i32,
                    id: Some(id("ns", "ev")),
                },
                EntityRef {
                    kind: 9999,
                    id: Some(id("ns", "bad")),
                },
            ],
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 1, "unknown kind must be skipped, not emitted");
        assert_eq!(refs[0].to_kind, Some(EntityKind::Event));
    }

    #[test]
    fn component_unknown_kind_is_skipped() {
        let entity = make_entity(entity::Kind::Component(Component {
            id: Some(id("ns", "comp")),
            members: vec![
                EntityRef {
                    kind: EntityKind::Event as i32,
                    id: Some(id("ns", "ev")),
                },
                EntityRef {
                    kind: 0,
                    id: Some(id("ns", "bad")),
                },
            ],
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 1, "Unspecified(0) kind must be skipped");
    }

    #[test]
    fn bounded_context_outbound_refs() {
        let entity = make_entity(entity::Kind::BoundedContext(BoundedContext {
            id: Some(id("ns", "orders-ctx")),
            realizes: vec![SubdomainRef {
                id: Some(id("ns", "orders-sub")),
            }],
            relationships: vec![ContextRelationship {
                upstream: Some(BoundedContextRef {
                    id: Some(id("ns", "inventory-ctx")),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 2);
        let kinds: Vec<_> = refs.iter().map(|r| r.to_kind).collect();
        assert!(kinds.contains(&Some(EntityKind::Subdomain)));
        assert!(kinds.contains(&Some(EntityKind::BoundedContext)));
    }

    #[test]
    fn tracker_outbound_refs() {
        let entity = make_entity(entity::Kind::Tracker(Tracker {
            id: Some(id("ns", "track")),
            items: vec![tracker::Item {
                subject: Some(EntityRef {
                    kind: EntityKind::Event as i32,
                    id: Some(id("ns", "ev")),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].to_kind, Some(EntityKind::Event));
    }

    #[test]
    fn term_outbound_refs() {
        let entity = make_entity(entity::Kind::Term(Term {
            id: Some(id("ns", "order")),
            embodied_by: vec![EntityRef {
                kind: EntityKind::Event as i32,
                id: Some(id("ns", "order.placed")),
            }],
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].to_kind, Some(EntityKind::Event));
    }

    #[test]
    fn ambiguity_outbound_refs() {
        let entity = make_entity(entity::Kind::Ambiguity(Ambiguity {
            id: Some(id("ns", "amb")),
            terms: vec![TermRef {
                id: Some(id("ns", "order")),
            }],
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].to_kind, Some(EntityKind::Term));
    }

    #[test]
    fn field_ref_depth_guard() {
        fn nested_object(depth: usize) -> FieldSpec {
            if depth == 0 {
                FieldSpec {
                    name: "leaf".into(),
                    r#type: Some(FieldType {
                        kind: Some(field_type::Kind::Ref(RefType {
                            r#ref: Some(TypeRef {
                                r#ref: Some(type_ref::Ref::Event(EventRef {
                                    id: Some(id("ns", "ev")),
                                })),
                            }),
                        })),
                    }),
                    ..Default::default()
                }
            } else {
                FieldSpec {
                    name: format!("level{depth}"),
                    r#type: Some(FieldType {
                        kind: Some(field_type::Kind::Object(field_type::ObjectType {
                            fields: vec![nested_object(depth - 1)],
                        })),
                    }),
                    ..Default::default()
                }
            }
        }
        let deep = nested_object(MAX_FIELD_DEPTH + 5);
        let mut out = Vec::new();
        collect_field_refs(&deep, "root", &mut out, 0);
        assert!(
            out.is_empty(),
            "depth guard must stop recursion before the leaf ref is emitted"
        );
    }

    #[test]
    fn supersedes_ref_uses_self_kind() {
        let entity = make_entity(entity::Kind::Event(Event {
            id: Some(id("ns", "order.placed")),
            supersedes: Some(id("ns", "order.placed")),
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        let sup: Vec<_> = refs
            .iter()
            .filter(|r| r.from_field == "supersedes")
            .collect();
        assert_eq!(sup.len(), 1);
        assert_eq!(sup[0].to_kind, Some(EntityKind::Event));
    }

    #[test]
    fn entity_id_returns_some_for_entity_with_id() {
        let expected = id("ns", "order.placed");
        let entity = make_entity(entity::Kind::Event(Event {
            id: Some(expected.clone()),
            ..Default::default()
        }));
        let got = entity_id(&entity);
        assert_eq!(got, Some(&expected));
    }

    #[test]
    fn entity_id_returns_none_when_inner_id_is_absent() {
        let entity = make_entity(entity::Kind::Event(Event {
            id: None,
            ..Default::default()
        }));
        assert_eq!(entity_id(&entity), None);
    }

    #[test]
    fn entity_id_returns_none_when_kind_is_absent() {
        let entity = Entity {
            system: None,
            kind: None,
        };
        assert_eq!(entity_id(&entity), None);
    }

    #[test]
    fn retarget_refs_rewrites_event_in_command_slice() {
        let old_id = id("ns", "order.placed");
        let mut new_id_v2 = old_id.clone();
        new_id_v2.version = 2;

        let mut entity = make_entity(entity::Kind::CommandSlice(CommandSlice {
            id: Some(id("ns", "place-slice")),
            emitted_events: vec![EventEdge {
                event: Some(EventRef {
                    id: Some(old_id.clone()),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }));
        let rewritten = retarget_refs(&mut entity, EntityKind::Event, &old_id, &new_id_v2);
        assert_eq!(rewritten, vec!["emitted_events[0]"]);
        let remaining = outbound_refs(&entity);
        assert!(
            remaining.iter().all(|r| r.to_id != old_id),
            "old_id must not remain after retarget"
        );
    }

    #[test]
    fn retarget_refs_rewrites_event_model_member() {
        let old_id = id("ns", "ev");
        let mut new_id = old_id.clone();
        new_id.version = 2;

        let mut entity = make_entity(entity::Kind::EventModel(EventModel {
            id: Some(id("ns", "em")),
            members: vec![
                EntityRef {
                    kind: EntityKind::Event as i32,
                    id: Some(old_id.clone()),
                },
                EntityRef {
                    kind: EntityKind::Command as i32,
                    id: Some(id("ns", "cmd")),
                },
            ],
            ..Default::default()
        }));
        let rewritten = retarget_refs(&mut entity, EntityKind::Event, &old_id, &new_id);
        assert_eq!(rewritten, vec!["members[0]"]);
        let remaining = outbound_refs(&entity);
        assert!(remaining.iter().all(|r| r.to_id != old_id));
    }

    #[test]
    fn retarget_refs_leaves_non_matching_id_unchanged() {
        let target_id = id("ns", "order.placed");
        let other_id = id("ns", "other-event");
        let new_id = id("ns", "order.placed");

        let mut entity = make_entity(entity::Kind::CommandSlice(CommandSlice {
            id: Some(id("ns", "slice")),
            emitted_events: vec![EventEdge {
                event: Some(EventRef {
                    id: Some(other_id.clone()),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }));
        let rewritten = retarget_refs(&mut entity, EntityKind::Event, &target_id, &new_id);
        assert!(rewritten.is_empty(), "unrelated ref must not be rewritten");
    }

    #[test]
    fn retarget_refs_tracker_item_subject() {
        let old_id = id("ns", "ev");
        let mut new_id = old_id.clone();
        new_id.version = 2;

        let mut entity = make_entity(entity::Kind::Tracker(Tracker {
            id: Some(id("ns", "t")),
            items: vec![tracker::Item {
                subject: Some(EntityRef {
                    kind: EntityKind::Event as i32,
                    id: Some(old_id.clone()),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }));
        let rewritten = retarget_refs(&mut entity, EntityKind::Event, &old_id, &new_id);
        assert_eq!(rewritten, vec!["items[0].subject"]);
        let remaining = outbound_refs(&entity);
        assert!(remaining.iter().all(|r| r.to_id != old_id));
    }

    #[test]
    fn retarget_coverage_matches_outbound_refs() {
        let old_event = id("ns", "ev-old");
        let new_event = id("ns", "ev-new");
        let cmd_id = id("ns", "cmd");
        let persona_id = id("ns", "persona");
        let ui_id = id("ns", "ui");

        let mut entity = make_entity(entity::Kind::CommandSlice(CommandSlice {
            id: Some(id("ns", "slice")),
            command: Some(CommandEdge {
                command: Some(CommandRef {
                    id: Some(cmd_id.clone()),
                }),
                ..Default::default()
            }),
            persona: Some(PersonaEdge {
                persona: Some(PersonaRef {
                    id: Some(persona_id.clone()),
                }),
                ..Default::default()
            }),
            ui: Some(UiEdge {
                ui: Some(UiRef {
                    id: Some(ui_id.clone()),
                }),
                ..Default::default()
            }),
            emitted_events: vec![EventEdge {
                event: Some(EventRef {
                    id: Some(old_event.clone()),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }));

        let before = outbound_refs(&entity);
        assert!(before.iter().any(|r| r.to_id == old_event));

        let rewritten = retarget_refs(&mut entity, EntityKind::Event, &old_event, &new_event);
        assert_ne!(rewritten, [] as [std::string::String; 0]);

        let after = outbound_refs(&entity);
        assert!(
            after.iter().all(|r| r.to_id != old_event),
            "outbound_refs still contains old_event after retarget: coverage mismatch"
        );
        assert!(after.iter().any(|r| r.to_id == cmd_id));
        assert!(after.iter().any(|r| r.to_id == persona_id));
        assert!(after.iter().any(|r| r.to_id == ui_id));
    }

    // -----------------------------------------------------------------
    // Gap-fill: outbound_refs + retarget_refs for kinds/ref-sites not
    // covered above (Ui, Swimlane, Processor, ReadModel.external_source,
    // full Storyboard entry/outcome, Component, BoundedContext, Subdomain,
    // Term, Ambiguity).
    // -----------------------------------------------------------------

    #[test]
    fn ui_outbound_refs_transition_and_slot_screen() {
        let target_ui = id("ns", "next-screen");
        let screen_id = id("ns", "checkout-screen");
        let entity = make_entity(entity::Kind::Ui(Ui {
            id: Some(id("ns", "cart-ui")),
            transitions: vec![ui::Transition {
                to: Some(UiRef {
                    id: Some(target_ui.clone()),
                }),
                ..Default::default()
            }],
            slot: Some(ScreenSlotRef {
                screen: Some(ScreenRef {
                    id: Some(screen_id.clone()),
                }),
                slot: "main".into(),
            }),
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 2);
        assert!(refs.iter().any(|r| r.to_kind == Some(EntityKind::Ui)
            && r.to_id == target_ui
            && r.from_field == "transitions[0].to"));
        assert!(refs.iter().any(|r| r.to_kind == Some(EntityKind::Screen)
            && r.to_id == screen_id
            && r.from_field == "slot.screen"));
    }

    #[test]
    fn ui_retarget_refs_transition_to() {
        let old_ui = id("ns", "old-screen");
        let new_ui = id("ns", "new-screen");
        let mut entity = make_entity(entity::Kind::Ui(Ui {
            id: Some(id("ns", "cart-ui")),
            transitions: vec![ui::Transition {
                to: Some(UiRef {
                    id: Some(old_ui.clone()),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }));
        let rewritten = retarget_refs(&mut entity, EntityKind::Ui, &old_ui, &new_ui);
        assert_eq!(rewritten, vec!["transitions[0].to".to_string()]);
        let refs = outbound_refs(&entity);
        assert!(refs.iter().all(|r| r.to_id != old_ui));
        assert!(refs.iter().any(|r| r.to_id == new_ui));
    }

    #[test]
    fn ui_retarget_refs_slot_screen() {
        let old_screen = id("ns", "old-screen");
        let new_screen = id("ns", "new-screen");
        let mut entity = make_entity(entity::Kind::Ui(Ui {
            id: Some(id("ns", "cart-ui")),
            slot: Some(ScreenSlotRef {
                screen: Some(ScreenRef {
                    id: Some(old_screen.clone()),
                }),
                slot: "main".into(),
            }),
            ..Default::default()
        }));
        let rewritten = retarget_refs(&mut entity, EntityKind::Screen, &old_screen, &new_screen);
        assert_eq!(rewritten, vec!["slot.screen".to_string()]);
        let refs = outbound_refs(&entity);
        assert!(refs.iter().all(|r| r.to_id != old_screen));
        assert!(refs.iter().any(|r| r.to_id == new_screen));
    }

    #[test]
    fn swimlane_outbound_refs_transitions_after_and_next() {
        let after_event = id("ns", "reserved");
        let next_event_a = id("ns", "expired");
        let next_event_b = id("ns", "confirmed");
        let entity = make_entity(entity::Kind::Swimlane(Swimlane {
            id: Some(id("ns", "order-stream")),
            transitions: vec![swimlane::Transition {
                after: Some(EventRef {
                    id: Some(after_event.clone()),
                }),
                next: vec![
                    swimlane::transition::Next {
                        event: Some(EventRef {
                            id: Some(next_event_a.clone()),
                        }),
                        ..Default::default()
                    },
                    swimlane::transition::Next {
                        event: Some(EventRef {
                            id: Some(next_event_b.clone()),
                        }),
                        ..Default::default()
                    },
                ],
            }],
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 3);
        assert!(refs.iter().any(|r| r.to_kind == Some(EntityKind::Event)
            && r.to_id == after_event
            && r.from_field == "transitions[0].after"));
        assert!(refs
            .iter()
            .any(|r| r.to_id == next_event_a && r.from_field == "transitions[0].next[0]"));
        assert!(refs
            .iter()
            .any(|r| r.to_id == next_event_b && r.from_field == "transitions[0].next[1]"));
    }

    #[test]
    fn swimlane_retarget_refs_after_and_next() {
        let old_event = id("ns", "reserved");
        let new_event = id("ns", "reserved-v2");
        let mut entity = make_entity(entity::Kind::Swimlane(Swimlane {
            id: Some(id("ns", "order-stream")),
            transitions: vec![swimlane::Transition {
                after: Some(EventRef {
                    id: Some(old_event.clone()),
                }),
                next: vec![swimlane::transition::Next {
                    event: Some(EventRef {
                        id: Some(old_event.clone()),
                    }),
                    ..Default::default()
                }],
            }],
            ..Default::default()
        }));
        let rewritten = retarget_refs(&mut entity, EntityKind::Event, &old_event, &new_event);
        assert_eq!(rewritten.len(), 2);
        assert!(rewritten.contains(&"transitions[0].after".to_string()));
        assert!(rewritten.contains(&"transitions[0].next[0]".to_string()));
        let refs = outbound_refs(&entity);
        assert!(refs.iter().all(|r| r.to_id != old_event));
        assert_eq!(refs.iter().filter(|r| r.to_id == new_event).count(), 2);
    }

    #[test]
    fn processor_outbound_refs_calls() {
        let sys_a = id("ns", "payments-gateway");
        let sys_b = id("ns", "shipping-api");
        let entity = make_entity(entity::Kind::Processor(Processor {
            id: Some(id("ns", "charge-processor")),
            calls: vec![
                ExternalSystemRef {
                    id: Some(sys_a.clone()),
                },
                ExternalSystemRef {
                    id: Some(sys_b.clone()),
                },
            ],
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 2);
        assert!(refs
            .iter()
            .any(|r| r.to_kind == Some(EntityKind::ExternalSystem)
                && r.to_id == sys_a
                && r.from_field == "calls[0]"));
        assert!(refs
            .iter()
            .any(|r| r.to_id == sys_b && r.from_field == "calls[1]"));
    }

    #[test]
    fn processor_retarget_refs_calls() {
        let old_sys = id("ns", "payments-gateway-v1");
        let new_sys = id("ns", "payments-gateway-v2");
        let mut entity = make_entity(entity::Kind::Processor(Processor {
            id: Some(id("ns", "charge-processor")),
            calls: vec![ExternalSystemRef {
                id: Some(old_sys.clone()),
            }],
            ..Default::default()
        }));
        let rewritten = retarget_refs(&mut entity, EntityKind::ExternalSystem, &old_sys, &new_sys);
        assert_eq!(rewritten, vec!["calls[0]".to_string()]);
        let refs = outbound_refs(&entity);
        assert!(refs.iter().all(|r| r.to_id != old_sys));
        assert!(refs.iter().any(|r| r.to_id == new_sys));
    }

    #[test]
    fn read_model_outbound_refs_external_source_system() {
        let sys_id = id("ns", "legacy-crm");
        let entity = make_entity(entity::Kind::ReadModel(ReadModel {
            id: Some(id("ns", "customer-view")),
            external_source: Some(read_model::ExternalSource {
                system: Some(ExternalSystemRef {
                    id: Some(sys_id.clone()),
                }),
                descriptor: "webhook:customer.updated".into(),
            }),
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].to_kind, Some(EntityKind::ExternalSystem));
        assert_eq!(refs[0].to_id, sys_id);
        assert_eq!(refs[0].from_field, "external_source.system");
    }

    #[test]
    fn read_model_retarget_refs_external_source_system() {
        let old_sys = id("ns", "legacy-crm");
        let new_sys = id("ns", "modern-crm");
        let mut entity = make_entity(entity::Kind::ReadModel(ReadModel {
            id: Some(id("ns", "customer-view")),
            external_source: Some(read_model::ExternalSource {
                system: Some(ExternalSystemRef {
                    id: Some(old_sys.clone()),
                }),
                descriptor: "webhook:customer.updated".into(),
            }),
            ..Default::default()
        }));
        let rewritten = retarget_refs(&mut entity, EntityKind::ExternalSystem, &old_sys, &new_sys);
        assert_eq!(rewritten, vec!["external_source.system".to_string()]);
        let refs = outbound_refs(&entity);
        assert!(refs.iter().all(|r| r.to_id != old_sys));
        assert!(refs.iter().any(|r| r.to_id == new_sys));
    }

    /// A Storyboard with every ref site populated: human-observer entry
    /// (read_model + persona + ui), event/ui outcome, and one untyped
    /// slice + one trace step slice ref. Used both as a direct fixture and
    /// as the "kind-complete generated entity" for the exact-set-match
    /// property test.
    fn full_storyboard(
        entry_rm: &Id,
        persona: &Id,
        entry_ui: &Id,
        outcome_event: &Id,
        outcome_ui: &Id,
        slice_ref: &Id,
        trace_slice_ref: &Id,
    ) -> Entity {
        make_entity(entity::Kind::Storyboard(Storyboard {
            id: Some(id("ns", "checkout-story")),
            entry: Some(StoryboardEntry {
                read_model: Some(ReadModelEdge {
                    read_model: Some(ReadModelRef {
                        id: Some(entry_rm.clone()),
                    }),
                    ..Default::default()
                }),
                observer: Some(storyboard_entry::Observer::Human(
                    storyboard_entry::HumanObserver {
                        persona: Some(PersonaEdge {
                            persona: Some(PersonaRef {
                                id: Some(persona.clone()),
                            }),
                            ..Default::default()
                        }),
                        ui: Some(UiEdge {
                            ui: Some(UiRef {
                                id: Some(entry_ui.clone()),
                            }),
                            ..Default::default()
                        }),
                    },
                )),
            }),
            outcome: Some(StoryboardOutcome {
                events: vec![EventEdge {
                    event: Some(EventRef {
                        id: Some(outcome_event.clone()),
                    }),
                    ..Default::default()
                }],
                ui: Some(UiEdge {
                    ui: Some(UiRef {
                        id: Some(outcome_ui.clone()),
                    }),
                    ..Default::default()
                }),
            }),
            slices: vec![SliceRef {
                id: Some(slice_ref.clone()),
            }],
            traces: vec![StoryboardTrace {
                steps: vec![TraceStep {
                    slice: Some(SliceRef {
                        id: Some(trace_slice_ref.clone()),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }))
    }

    #[test]
    fn storyboard_outbound_refs_human_entry_and_outcome() {
        let entry_rm = id("ns", "cart-view");
        let persona = id("ns", "billy-customer");
        let entry_ui = id("ns", "cart-ui");
        let outcome_event = id("ns", "order-placed");
        let outcome_ui = id("ns", "confirmation-ui");
        let slice_ref = id("ns", "place-order-slice");
        let trace_slice_ref = id("ns", "place-order-slice");
        let entity = full_storyboard(
            &entry_rm,
            &persona,
            &entry_ui,
            &outcome_event,
            &outcome_ui,
            &slice_ref,
            &trace_slice_ref,
        );
        let refs = outbound_refs(&entity);
        let field_of = |field: &str| refs.iter().find(|r| r.from_field == field);
        assert_eq!(
            field_of("entry.read_model").map(|r| (r.to_kind, &r.to_id)),
            Some((Some(EntityKind::ReadModel), &entry_rm))
        );
        assert_eq!(
            field_of("entry.observer.human.persona").map(|r| (r.to_kind, &r.to_id)),
            Some((Some(EntityKind::Persona), &persona))
        );
        assert_eq!(
            field_of("entry.observer.human.ui").map(|r| (r.to_kind, &r.to_id)),
            Some((Some(EntityKind::Ui), &entry_ui))
        );
        assert_eq!(
            field_of("outcome.events[0]").map(|r| (r.to_kind, &r.to_id)),
            Some((Some(EntityKind::Event), &outcome_event))
        );
        assert_eq!(
            field_of("outcome.ui").map(|r| (r.to_kind, &r.to_id)),
            Some((Some(EntityKind::Ui), &outcome_ui))
        );
        assert_eq!(
            field_of("slices[0]").map(|r| (r.to_kind, &r.to_id)),
            Some((None, &slice_ref))
        );
        assert_eq!(
            field_of("traces[0].steps[0].slice").map(|r| (r.to_kind, &r.to_id)),
            Some((None, &trace_slice_ref))
        );
        assert_eq!(refs.len(), 7);
    }

    #[test]
    fn storyboard_outbound_refs_automation_entry() {
        let entry_rm = id("ns", "queue-depth-view");
        let processor = id("ns", "reorder-processor");
        let entity = make_entity(entity::Kind::Storyboard(Storyboard {
            id: Some(id("ns", "auto-reorder-story")),
            entry: Some(StoryboardEntry {
                read_model: Some(ReadModelEdge {
                    read_model: Some(ReadModelRef {
                        id: Some(entry_rm.clone()),
                    }),
                    ..Default::default()
                }),
                observer: Some(storyboard_entry::Observer::Automation(
                    storyboard_entry::AutomationObserver {
                        processor: Some(ProcessorEdge {
                            processor: Some(ProcessorRef {
                                id: Some(processor.clone()),
                            }),
                            ..Default::default()
                        }),
                    },
                )),
            }),
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 2);
        assert!(refs
            .iter()
            .any(|r| r.to_id == entry_rm && r.from_field == "entry.read_model"));
        assert!(refs.iter().any(|r| {
            r.to_kind == Some(EntityKind::Processor)
                && r.to_id == processor
                && r.from_field == "entry.observer.automation.processor"
        }));
    }

    #[test]
    fn storyboard_retarget_refs_human_entry_and_outcome_exact() {
        let entry_rm = id("ns", "cart-view");
        let persona = id("ns", "billy-customer");
        let entry_ui = id("ns", "cart-ui");
        let outcome_event = id("ns", "order-placed");
        let outcome_ui = id("ns", "confirmation-ui");
        let slice_ref = id("ns", "place-order-slice");
        let trace_slice_ref = id("ns", "place-order-slice");
        let mut entity = full_storyboard(
            &entry_rm,
            &persona,
            &entry_ui,
            &outcome_event,
            &outcome_ui,
            &slice_ref,
            &trace_slice_ref,
        );

        let new_rm = id("ns", "cart-view-v2");
        let rewritten = retarget_refs(&mut entity, EntityKind::ReadModel, &entry_rm, &new_rm);
        assert_eq!(rewritten, vec!["entry.read_model".to_string()]);

        let new_persona = id("ns", "billy-customer-v2");
        let rewritten = retarget_refs(&mut entity, EntityKind::Persona, &persona, &new_persona);
        assert_eq!(rewritten, vec!["entry.observer.human.persona".to_string()]);

        let new_entry_ui = id("ns", "cart-ui-v2");
        let rewritten = retarget_refs(&mut entity, EntityKind::Ui, &entry_ui, &new_entry_ui);
        // entry.observer.human.ui and outcome.ui share no id here (different
        // ids), so only entry.observer.human.ui should flip.
        assert_eq!(rewritten, vec!["entry.observer.human.ui".to_string()]);

        let new_outcome_event = id("ns", "order-placed-v2");
        let rewritten = retarget_refs(
            &mut entity,
            EntityKind::Event,
            &outcome_event,
            &new_outcome_event,
        );
        assert_eq!(rewritten, vec!["outcome.events[0]".to_string()]);

        let new_outcome_ui = id("ns", "confirmation-ui-v2");
        let rewritten = retarget_refs(&mut entity, EntityKind::Ui, &outcome_ui, &new_outcome_ui);
        assert_eq!(rewritten, vec!["outcome.ui".to_string()]);

        let new_slice = id("ns", "place-order-slice-v2");
        let rewritten = retarget_refs(
            &mut entity,
            EntityKind::CommandSlice,
            &slice_ref,
            &new_slice,
        );
        // Both slices[0] and traces[0].steps[0].slice hold the same id, so
        // both rewrite in one call.
        assert_eq!(rewritten.len(), 2);
        assert!(rewritten.contains(&"slices[0]".to_string()));
        assert!(rewritten.contains(&"traces[0].steps[0].slice".to_string()));

        let refs = outbound_refs(&entity);
        for old in [
            &entry_rm,
            &persona,
            &entry_ui,
            &outcome_event,
            &outcome_ui,
            &slice_ref,
        ] {
            assert!(
                refs.iter().all(|r| r.to_id != *old),
                "stale id {old:?} still present after full retarget pass"
            );
        }
        assert!(refs.iter().any(|r| r.to_id == new_rm));
        assert!(refs.iter().any(|r| r.to_id == new_persona));
        assert!(refs.iter().any(|r| r.to_id == new_entry_ui));
        assert!(refs.iter().any(|r| r.to_id == new_outcome_event));
        assert!(refs.iter().any(|r| r.to_id == new_outcome_ui));
        assert_eq!(refs.iter().filter(|r| r.to_id == new_slice).count(), 2);
    }

    #[test]
    fn storyboard_retarget_refs_slice_ref_rejects_wrong_kind() {
        // slices[] and traces[].steps[].slice only rewrite when from_kind is
        // one of the three concrete slice kinds (SliceRef carries no kind of
        // its own, so retarget_refs uses an explicit allowlist to avoid
        // cross-kind corruption).
        let slice_id = id("ns", "some-slice");
        let mut entity = make_entity(entity::Kind::Storyboard(Storyboard {
            id: Some(id("ns", "story")),
            slices: vec![SliceRef {
                id: Some(slice_id.clone()),
            }],
            ..Default::default()
        }));
        let rewritten = retarget_refs(
            &mut entity,
            EntityKind::Event,
            &slice_id,
            &id("ns", "irrelevant"),
        );
        assert_eq!(rewritten, [] as [std::string::String; 0]);
        let refs = outbound_refs(&entity);
        assert!(refs.iter().any(|r| r.to_id == slice_id));
    }

    #[test]
    fn component_outbound_and_retarget_refs_members() {
        let event_member = id("ns", "order-placed");
        let entity_kind_val = entity::Kind::Component(Component {
            id: Some(id("ns", "checkout-component")),
            members: vec![EntityRef {
                kind: EntityKind::Event as i32,
                id: Some(event_member.clone()),
            }],
            ..Default::default()
        });
        let entity = make_entity(entity_kind_val);
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].to_kind, Some(EntityKind::Event));
        assert_eq!(refs[0].to_id, event_member);
        assert_eq!(refs[0].from_field, "members[0]");

        let mut entity = entity;
        let new_member = id("ns", "order-placed-v2");
        let rewritten = retarget_refs(&mut entity, EntityKind::Event, &event_member, &new_member);
        assert_eq!(rewritten, vec!["members[0]".to_string()]);
        let refs = outbound_refs(&entity);
        assert!(refs.iter().all(|r| r.to_id != event_member));
        assert!(refs.iter().any(|r| r.to_id == new_member));
    }

    #[test]
    fn component_retarget_refs_members_rejects_mismatched_kind() {
        // Same id, wrong from_kind: the member's declared kind (Event) must
        // match from_kind or the retarget must not fire (kind confusion
        // guard mirrored from EventModel's identical member-list shape).
        let member_id = id("ns", "order-placed");
        let mut entity = make_entity(entity::Kind::Component(Component {
            id: Some(id("ns", "checkout-component")),
            members: vec![EntityRef {
                kind: EntityKind::Event as i32,
                id: Some(member_id.clone()),
            }],
            ..Default::default()
        }));
        let rewritten = retarget_refs(
            &mut entity,
            EntityKind::Command,
            &member_id,
            &id("ns", "irrelevant"),
        );
        assert_eq!(rewritten, [] as [std::string::String; 0]);
        let refs = outbound_refs(&entity);
        assert!(refs.iter().any(|r| r.to_id == member_id));
    }

    #[test]
    fn bounded_context_outbound_and_retarget_refs_realizes_and_relationships() {
        let subdomain_id = id("ns", "checkout-subdomain");
        let upstream_bc = id("payments", "payments");
        let entity_kind_val = entity::Kind::BoundedContext(BoundedContext {
            id: Some(id("ns", "ns")),
            realizes: vec![SubdomainRef {
                id: Some(subdomain_id.clone()),
            }],
            relationships: vec![bounded_context::ContextRelationship {
                upstream: Some(BoundedContextRef {
                    id: Some(upstream_bc.clone()),
                }),
                ..Default::default()
            }],
            ..Default::default()
        });
        let entity = make_entity(entity_kind_val);
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 2);
        assert!(refs.iter().any(|r| r.to_kind == Some(EntityKind::Subdomain)
            && r.to_id == subdomain_id
            && r.from_field == "realizes[0]"));
        assert!(refs.iter().any(|r| {
            r.to_kind == Some(EntityKind::BoundedContext)
                && r.to_id == upstream_bc
                && r.from_field == "relationships[0].upstream"
        }));

        let mut entity = entity;
        let new_subdomain = id("ns", "checkout-subdomain-v2");
        let rewritten = retarget_refs(
            &mut entity,
            EntityKind::Subdomain,
            &subdomain_id,
            &new_subdomain,
        );
        assert_eq!(rewritten, vec!["realizes[0]".to_string()]);

        let new_upstream = id("payments", "payments-v2");
        let rewritten = retarget_refs(
            &mut entity,
            EntityKind::BoundedContext,
            &upstream_bc,
            &new_upstream,
        );
        assert_eq!(rewritten, vec!["relationships[0].upstream".to_string()]);

        let refs = outbound_refs(&entity);
        assert!(refs
            .iter()
            .all(|r| r.to_id != subdomain_id && r.to_id != upstream_bc));
        assert!(refs.iter().any(|r| r.to_id == new_subdomain));
        assert!(refs.iter().any(|r| r.to_id == new_upstream));
    }

    #[test]
    fn subdomain_outbound_and_retarget_refs_domain() {
        let domain_id = id("ns", "commerce-domain");
        let entity_kind_val = entity::Kind::Subdomain(Subdomain {
            id: Some(id("ns", "checkout-subdomain")),
            domain: Some(DomainRef {
                id: Some(domain_id.clone()),
            }),
            ..Default::default()
        });
        let mut entity = make_entity(entity_kind_val);
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].to_kind, Some(EntityKind::Domain));
        assert_eq!(refs[0].to_id, domain_id);
        assert_eq!(refs[0].from_field, "domain");

        let new_domain = id("ns", "commerce-domain-v2");
        let rewritten = retarget_refs(&mut entity, EntityKind::Domain, &domain_id, &new_domain);
        assert_eq!(rewritten, vec!["domain".to_string()]);
        let refs = outbound_refs(&entity);
        assert!(refs.iter().all(|r| r.to_id != domain_id));
        assert!(refs.iter().any(|r| r.to_id == new_domain));
    }

    #[test]
    fn term_outbound_and_retarget_refs_embodied_by() {
        let event_id = id("ns", "review-submitted");
        let entity_kind_val = entity::Kind::Term(Term {
            id: Some(id("ns", "review")),
            embodied_by: vec![EntityRef {
                kind: EntityKind::Event as i32,
                id: Some(event_id.clone()),
            }],
            ..Default::default()
        });
        let mut entity = make_entity(entity_kind_val);
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].to_kind, Some(EntityKind::Event));
        assert_eq!(refs[0].to_id, event_id);
        assert_eq!(refs[0].from_field, "embodied_by[0]");

        let new_event = id("ns", "review-submitted-v2");
        let rewritten = retarget_refs(&mut entity, EntityKind::Event, &event_id, &new_event);
        assert_eq!(rewritten, vec!["embodied_by[0]".to_string()]);
        let refs = outbound_refs(&entity);
        assert!(refs.iter().all(|r| r.to_id != event_id));
        assert!(refs.iter().any(|r| r.to_id == new_event));
    }

    #[test]
    fn term_retarget_refs_embodied_by_rejects_mismatched_kind() {
        let member_id = id("ns", "review-submitted");
        let mut entity = make_entity(entity::Kind::Term(Term {
            id: Some(id("ns", "review")),
            embodied_by: vec![EntityRef {
                kind: EntityKind::Event as i32,
                id: Some(member_id.clone()),
            }],
            ..Default::default()
        }));
        let rewritten = retarget_refs(
            &mut entity,
            EntityKind::Command,
            &member_id,
            &id("ns", "irrelevant"),
        );
        assert_eq!(rewritten, [] as [std::string::String; 0]);
    }

    #[test]
    fn ambiguity_outbound_and_retarget_refs_terms() {
        let term_a = id("ns", "review");
        let term_b = id("ns", "critique");
        let entity_kind_val = entity::Kind::Ambiguity(Ambiguity {
            id: Some(id("ns", "review-collision")),
            terms: vec![
                TermRef {
                    id: Some(term_a.clone()),
                },
                TermRef {
                    id: Some(term_b.clone()),
                },
            ],
            ..Default::default()
        });
        let mut entity = make_entity(entity_kind_val);
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 2);
        assert!(refs.iter().any(|r| r.to_kind == Some(EntityKind::Term)
            && r.to_id == term_a
            && r.from_field == "terms[0]"));
        assert!(refs
            .iter()
            .any(|r| r.to_id == term_b && r.from_field == "terms[1]"));

        let new_term_a = id("ns", "review-v2");
        let rewritten = retarget_refs(&mut entity, EntityKind::Term, &term_a, &new_term_a);
        assert_eq!(rewritten, vec!["terms[0]".to_string()]);
        let refs = outbound_refs(&entity);
        assert!(refs.iter().all(|r| r.to_id != term_a));
        assert!(refs.iter().any(|r| r.to_id == new_term_a));
        assert!(
            refs.iter().any(|r| r.to_id == term_b),
            "unrelated term untouched"
        );
    }

    #[test]
    fn service_level_indicator_outbound_and_retarget_refs_connection_and_outcome_ratio() {
        let command_slice = id("ns", "place-order-slice");
        let event_a = id("ns", "order-placed");
        let good_event = id("ns", "order-fulfilled");
        let bad_event = id("ns", "order-failed");
        let entity_kind_val = entity::Kind::ServiceLevelIndicator(ServiceLevelIndicator {
            id: Some(id("ns", "checkout-latency")),
            connection: Some(Connection {
                kind: Some(connection::Kind::CommandHandling(
                    connection::CommandHandling {
                        command_slice: Some(SliceRef {
                            id: Some(command_slice.clone()),
                        }),
                        events: vec![EventRef {
                            id: Some(event_a.clone()),
                        }],
                    },
                )),
            }),
            measure: Some(service_level_indicator::Measure::OutcomeRatio(
                service_level_indicator::OutcomeRatio {
                    good: vec![EventRef {
                        id: Some(good_event.clone()),
                    }],
                    bad: vec![EventRef {
                        id: Some(bad_event.clone()),
                    }],
                },
            )),
            ..Default::default()
        });
        let mut entity = make_entity(entity_kind_val);
        let refs = outbound_refs(&entity);
        assert!(refs.iter().any(|r| r.to_kind.is_none()
            && r.to_id == command_slice
            && r.from_field == "connection.command_handling.command_slice"));
        assert!(refs.iter().any(|r| r.to_kind == Some(EntityKind::Event)
            && r.to_id == event_a
            && r.from_field == "connection.command_handling.events[0]"));
        assert!(refs.iter().any(|r| r.to_kind == Some(EntityKind::Event)
            && r.to_id == good_event
            && r.from_field == "measure.outcome_ratio.good[0]"));
        assert!(refs.iter().any(|r| r.to_kind == Some(EntityKind::Event)
            && r.to_id == bad_event
            && r.from_field == "measure.outcome_ratio.bad[0]"));

        let new_good_event = id("ns", "order-fulfilled-v2");
        let rewritten = retarget_refs(&mut entity, EntityKind::Event, &good_event, &new_good_event);
        assert_eq!(rewritten, vec!["measure.outcome_ratio.good[0]".to_string()]);
        let refs = outbound_refs(&entity);
        assert!(refs.iter().all(|r| r.to_id != good_event));
        assert!(refs.iter().any(|r| r.to_id == new_good_event));
        assert!(
            refs.iter().any(|r| r.to_id == bad_event),
            "unrelated outcome ratio event untouched"
        );

        let new_command_slice = id("ns", "place-order-slice-v2");
        let rewritten = retarget_refs(
            &mut entity,
            EntityKind::CommandSlice,
            &command_slice,
            &new_command_slice,
        );
        assert_eq!(
            rewritten,
            vec!["connection.command_handling.command_slice".to_string()]
        );
        let refs = outbound_refs(&entity);
        assert!(refs.iter().all(|r| r.to_id != command_slice));
        assert!(refs.iter().any(|r| r.to_id == new_command_slice));
    }

    #[test]
    fn service_level_objective_outbound_and_retarget_refs() {
        let service_id = id("ns", "checkout-service");
        let indicator_id = id("ns", "checkout-latency");
        let alert_policy_id = id("ns", "checkout-slo-burn");
        let persona_id = id("ns", "customer-success");
        let entity_kind_val = entity::Kind::ServiceLevelObjective(ServiceLevelObjective {
            id: Some(id("ns", "checkout-availability")),
            service: Some(ComponentRef {
                id: Some(service_id.clone()),
            }),
            indicator: Some(ServiceLevelIndicatorRef {
                id: Some(indicator_id.clone()),
            }),
            alert_policies: vec![AlertPolicyRef {
                id: Some(alert_policy_id.clone()),
            }],
            commitment: Some(Commitment {
                kind: Some(commitment::Kind::External(commitment::External {
                    counterparty: Some(commitment::external::Counterparty::Persona(PersonaRef {
                        id: Some(persona_id.clone()),
                    })),
                    ..Default::default()
                })),
            }),
            ..Default::default()
        });
        let mut entity = make_entity(entity_kind_val);
        let refs = outbound_refs(&entity);
        assert!(refs.iter().any(|r| r.to_kind == Some(EntityKind::Component)
            && r.to_id == service_id
            && r.from_field == "service"));
        assert!(refs
            .iter()
            .any(|r| r.to_kind == Some(EntityKind::ServiceLevelIndicator)
                && r.to_id == indicator_id
                && r.from_field == "indicator"));
        assert!(refs
            .iter()
            .any(|r| r.to_kind == Some(EntityKind::AlertPolicy)
                && r.to_id == alert_policy_id
                && r.from_field == "alert_policies[0]"));
        assert!(refs.iter().any(|r| r.to_kind == Some(EntityKind::Persona)
            && r.to_id == persona_id
            && r.from_field == "commitment.external.persona"));

        let new_indicator = id("ns", "checkout-latency-v2");
        let rewritten = retarget_refs(
            &mut entity,
            EntityKind::ServiceLevelIndicator,
            &indicator_id,
            &new_indicator,
        );
        assert_eq!(rewritten, vec!["indicator".to_string()]);
        let refs = outbound_refs(&entity);
        assert!(refs.iter().all(|r| r.to_id != indicator_id));
        assert!(refs.iter().any(|r| r.to_id == new_indicator));
    }

    #[test]
    fn alert_policy_outbound_and_retarget_refs_notification_targets() {
        let target_id = id("ns", "oncall-pagerduty");
        let entity_kind_val = entity::Kind::AlertPolicy(AlertPolicy {
            id: Some(id("ns", "checkout-slo-burn")),
            notification_targets: vec![AlertNotificationTargetRef {
                id: Some(target_id.clone()),
            }],
            ..Default::default()
        });
        let mut entity = make_entity(entity_kind_val);
        let refs = outbound_refs(&entity);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].to_kind, Some(EntityKind::AlertNotificationTarget));
        assert_eq!(refs[0].to_id, target_id);
        assert_eq!(refs[0].from_field, "notification_targets[0]");

        let new_target = id("ns", "oncall-pagerduty-v2");
        let rewritten = retarget_refs(
            &mut entity,
            EntityKind::AlertNotificationTarget,
            &target_id,
            &new_target,
        );
        assert_eq!(rewritten, vec!["notification_targets[0]".to_string()]);
        let refs = outbound_refs(&entity);
        assert!(refs.iter().all(|r| r.to_id != target_id));
        assert!(refs.iter().any(|r| r.to_id == new_target));
    }

    #[test]
    fn alert_notification_target_outbound_refs_is_empty() {
        let entity = make_entity(entity::Kind::AlertNotificationTarget(
            AlertNotificationTarget {
                id: Some(id("ns", "oncall-pagerduty")),
                target: "pagerduty:service-123".into(),
                ..Default::default()
            },
        ));
        assert!(outbound_refs(&entity).is_empty());
    }

    // -----------------------------------------------------------------
    // Bug proofs (intentionally failing). Do NOT "fix" by weakening
    // these asserts; production `outbound_refs` / `retarget_refs` must
    // grow coverage for the sites below.
    // -----------------------------------------------------------------

    fn field_spec_ref_event(name: &str, target: Id) -> FieldSpec {
        FieldSpec {
            name: name.into(),
            r#type: Some(FieldType {
                kind: Some(field_type::Kind::Ref(RefType {
                    r#ref: Some(TypeRef {
                        r#ref: Some(type_ref::Ref::Event(EventRef { id: Some(target) })),
                    }),
                })),
            }),
            ..Default::default()
        }
    }

    /// BUG: `Swimlane.state` is a load-bearing FieldSpec list (aggregate
    /// state; validator treats it as a field source), but `outbound_refs`
    /// only walks `transitions` and never `state`, so TypeRefs planted
    /// on lane state are invisible to GetIncomingReferences / GetImpact /
    /// RetargetReferences / DANGLING_REF.
    #[test]
    fn bug_swimlane_state_field_ref_must_appear_in_outbound_refs() {
        let target = id("ns", "order-type");
        let entity = make_entity(entity::Kind::Swimlane(Swimlane {
            id: Some(id("ns", "orders")),
            state: vec![field_spec_ref_event("order_type", target.clone())],
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert!(
            refs.iter().any(|r| {
                r.to_kind == Some(EntityKind::Event)
                    && r.to_id == target
                    && r.from_field.starts_with("state[")
            }),
            "Swimlane.state[0] TypeRef must be an outbound ref; got {refs:?}"
        );
    }

    /// BUG: mirror of the outbound gap: RetargetReferences will never
    /// rewrite TypeRefs living on `Swimlane.state`.
    #[test]
    fn bug_swimlane_state_field_ref_must_be_retargetable() {
        let old = id("ns", "order-type");
        let new = id("ns", "order-type-v2");
        let mut entity = make_entity(entity::Kind::Swimlane(Swimlane {
            id: Some(id("ns", "orders")),
            state: vec![field_spec_ref_event("order_type", old.clone())],
            ..Default::default()
        }));
        let rewritten = retarget_refs(&mut entity, EntityKind::Event, &old, &new);
        assert!(
            rewritten.iter().any(|p| p.starts_with("state[")),
            "retarget_refs must rewrite Swimlane.state FieldSpec refs; got {rewritten:?}"
        );
        let EntityOneof::Swimlane(lane) = entity.kind.as_ref().expect("kind") else {
            panic!("expected swimlane");
        };
        let got = lane.state[0]
            .r#type
            .as_ref()
            .and_then(|t| t.kind.as_ref())
            .and_then(|k| match k {
                field_type::Kind::Ref(rt) => rt.r#ref.as_ref(),
                _ => None,
            })
            .and_then(|tr| match tr.r#ref.as_ref() {
                Some(type_ref::Ref::Event(e)) => e.id.clone(),
                _ => None,
            });
        assert_eq!(
            got.as_ref(),
            Some(&new),
            "state field still points at {got:?}"
        );
    }

    /// BUG: `Domain.project` is the only structural Project association
    /// (strategic.proto), but Domain is in the empty arm of
    /// `outbound_refs`, so Project incoming refs / retarget / DANGLING_REF
    /// never see it.
    #[test]
    fn bug_domain_project_must_appear_in_outbound_refs() {
        let project = id("acme", "acme");
        let entity = make_entity(entity::Kind::Domain(Domain {
            id: Some(id("marketplace", "marketplace")),
            project: Some(ProjectRef {
                id: Some(project.clone()),
            }),
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert!(
            refs.iter().any(|r| {
                r.to_kind == Some(EntityKind::Project)
                    && r.to_id == project
                    && r.from_field == "project"
            }),
            "Domain.project must be an outbound ref; got {refs:?}"
        );
    }

    #[test]
    fn bug_domain_project_must_be_retargetable() {
        let old = id("acme", "acme");
        let new = id("acme", "acme-v2");
        let mut entity = make_entity(entity::Kind::Domain(Domain {
            id: Some(id("marketplace", "marketplace")),
            project: Some(ProjectRef {
                id: Some(old.clone()),
            }),
            ..Default::default()
        }));
        let rewritten = retarget_refs(&mut entity, EntityKind::Project, &old, &new);
        assert_eq!(
            rewritten,
            vec!["project".to_string()],
            "retarget_refs must rewrite Domain.project"
        );
        let EntityOneof::Domain(d) = entity.kind.as_ref().expect("kind") else {
            panic!("expected domain");
        };
        assert_eq!(d.project.as_ref().and_then(|p| p.id.as_ref()), Some(&new));
    }

    /// BUG: top-level Schema entities carry `repeated FieldSpec fields`
    /// (Decision #33 named shapes), but Schema is in the empty arm; TypeRefs
    /// inside Schema.fields are invisible to the reverse index and retarget.
    #[test]
    fn bug_schema_fields_ref_must_appear_in_outbound_refs() {
        let target = id("ns", "money");
        let entity = make_entity(entity::Kind::Schema(Schema {
            id: Some(id("ns", "order-payload")),
            fields: vec![field_spec_ref_event("amount", target.clone())],
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert!(
            refs.iter().any(|r| {
                r.to_kind == Some(EntityKind::Event)
                    && r.to_id == target
                    && r.from_field.starts_with("fields[")
            }),
            "Schema.fields TypeRef must be an outbound ref; got {refs:?}"
        );
    }

    /// BUG: CommandSlice.scenarios[].given[].event (and when/then) are real
    /// addressable pointers used by SCENARIO_DANGLING_REF, but
    /// `outbound_refs` never walks scenarios; GetImpact / Retarget /
    /// model-level DANGLING_REF miss them.
    #[test]
    fn bug_command_slice_scenario_given_event_must_appear_in_outbound_refs() {
        let given_event = id("ns", "order.reserved");
        let entity = make_entity(entity::Kind::CommandSlice(CommandSlice {
            id: Some(id("ns", "confirm-slice")),
            scenarios: vec![CommandScenario {
                id: "happy".into(),
                given: vec![EventExample {
                    event: Some(EventRef {
                        id: Some(given_event.clone()),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert!(
            refs.iter().any(|r| {
                r.to_kind == Some(EntityKind::Event)
                    && r.to_id == given_event
                    && r.from_field.contains("scenarios")
            }),
            "CommandSlice.scenarios[].given[].event must be an outbound ref; got {refs:?}"
        );
    }

    #[test]
    fn bug_event_schema_any_field_refs_must_appear_in_outbound_refs() {
        use prost::Message as _;
        let target = id("ns", "customer");
        let schema = Schema {
            fields: vec![field_spec_ref_event("customer_id", target.clone())],
            ..Default::default()
        };
        let entity = make_entity(entity::Kind::Event(Event {
            id: Some(id("ns", "order.placed")),
            schema: Some(prost_types::Any {
                type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Schema".into(),
                value: schema.encode_to_vec(),
            }),
            ..Default::default()
        }));
        let refs = outbound_refs(&entity);
        assert!(
            refs.iter()
                .any(|r| r.to_kind == Some(EntityKind::Event) && r.to_id == target),
            "TypeRef inside Event.schema (Any-packed Schema) must be an outbound ref; got {refs:?}"
        );
    }

    // -----------------------------------------------------------------
    // Property-based tests (proptest). Three invariants requested:
    //   (a) retarget_refs(from -> to) followed by outbound_refs never
    //       reports `from` (unless `to == from`, the no-op case).
    //   (b) retargeting to the same id is a no-op (empty rewritten list,
    //       entity byte-identical before/after).
    //   (c) outbound_refs of a kind-complete generated entity finds
    //       exactly the planted refs (exact-set match), demonstrated here
    //       via the full_storyboard fixture generated with arbitrary ids.
    // -----------------------------------------------------------------

    use proptest::prelude::*;

    fn arb_id() -> impl Strategy<Value = Id> {
        ("[a-z][a-z0-9]{0,10}", "[a-z][a-z0-9-]{0,10}", 0u64..5).prop_map(
            |(namespace, slug, version)| Id {
                namespace,
                slug,
                version,
            },
        )
    }

    /// Distinct (from, to) id pairs to drive the invariants: `to` must
    /// differ from `from` for the "not a no-op" branch, and the two must
    /// serialize the same protobuf field width (both are plain `Id`s here,
    /// so no extra constraint beyond inequality is required).
    fn distinct_id_pair() -> impl Strategy<Value = (Id, Id)> {
        (arb_id(), arb_id()).prop_filter("ids must differ", |(a, b)| a != b)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        /// After retargeting Event refs on a Swimlane's transition table,
        /// outbound_refs never reports the old id again (unless the target
        /// equals it, which distinct_id_pair rules out).
        #[test]
        fn prop_retarget_then_outbound_never_reports_from_swimlane(
            (from_id, to_id) in distinct_id_pair(),
            other in arb_id(),
        ) {
            prop_assume!(other != from_id && other != to_id);
            let mut entity = make_entity(entity::Kind::Swimlane(Swimlane {
                id: Some(id("ns", "stream")),
                transitions: vec![swimlane::Transition {
                    after: Some(EventRef { id: Some(from_id.clone()) }),
                    next: vec![
                        swimlane::transition::Next { event: Some(EventRef { id: Some(from_id.clone()) }), ..Default::default() },
                        swimlane::transition::Next { event: Some(EventRef { id: Some(other.clone()) }), ..Default::default() },
                    ],
                }],
                ..Default::default()
            }));
            retarget_refs(&mut entity, EntityKind::Event, &from_id, &to_id);
            let after = outbound_refs(&entity);
            prop_assert!(after.iter().all(|r| r.to_id != from_id));
            prop_assert!(after.iter().any(|r| r.to_id == other));
        }

        /// Same invariant for BoundedContext.realizes (Subdomain refs) and
        /// relationships.upstream (BoundedContext refs) simultaneously.
        #[test]
        fn prop_retarget_then_outbound_never_reports_from_bounded_context(
            (from_subdomain, to_subdomain) in distinct_id_pair(),
            (from_upstream, to_upstream) in distinct_id_pair(),
        ) {
            let mut entity = make_entity(entity::Kind::BoundedContext(BoundedContext {
                id: Some(id("ns", "ns")),
                realizes: vec![SubdomainRef { id: Some(from_subdomain.clone()) }],
                relationships: vec![bounded_context::ContextRelationship {
                    upstream: Some(BoundedContextRef { id: Some(from_upstream.clone()) }),
                    ..Default::default()
                }],
                ..Default::default()
            }));
            retarget_refs(&mut entity, EntityKind::Subdomain, &from_subdomain, &to_subdomain);
            retarget_refs(&mut entity, EntityKind::BoundedContext, &from_upstream, &to_upstream);
            let after = outbound_refs(&entity);
            prop_assert!(after.iter().all(|r| r.to_id != from_subdomain));
            prop_assert!(after.iter().all(|r| r.to_id != from_upstream));
        }

        /// Retargeting to the SAME id is a semantic no-op: `retarget_refs`
        /// does not special-case `from_id == to_id` (it still reports the
        /// site as "rewritten" since the match condition is met), but the
        /// observable ref set is byte-identical before and after -- the
        /// entity's outbound refs never change (checked on a
        /// Term.embodied_by site, representative of the kind-tagged
        /// EntityRef list shape shared by Component/EventModel members and
        /// Tracker items).
        #[test]
        fn prop_retarget_to_same_id_is_noop(id_val in arb_id()) {
            let mut entity = make_entity(entity::Kind::Term(Term {
                id: Some(id("ns", "term")),
                embodied_by: vec![EntityRef { kind: EntityKind::Event as i32, id: Some(id_val.clone()) }],
                ..Default::default()
            }));
            let before = outbound_refs(&entity);
            retarget_refs(&mut entity, EntityKind::Event, &id_val, &id_val);
            let after = outbound_refs(&entity);
            prop_assert_eq!(before.len(), after.len());
            prop_assert!(after.iter().any(|r| r.to_id == id_val));
        }

        /// Same no-op invariant on the Storyboard's slices[] site (untyped
        /// SliceRef, allowlisted from_kind path): the ref set is unchanged
        /// when retargeting an id to itself.
        #[test]
        fn prop_retarget_to_same_id_is_noop_storyboard_slices(id_val in arb_id()) {
            let mut entity = make_entity(entity::Kind::Storyboard(Storyboard {
                id: Some(id("ns", "story")),
                slices: vec![SliceRef { id: Some(id_val.clone()) }],
                ..Default::default()
            }));
            let before = outbound_refs(&entity);
            retarget_refs(&mut entity, EntityKind::CommandSlice, &id_val, &id_val);
            let after = outbound_refs(&entity);
            prop_assert_eq!(before.len(), after.len());
            prop_assert!(after.iter().any(|r| r.to_id == id_val));
        }

        /// Exact-set match: a kind-complete Storyboard generated from seven
        /// arbitrary distinct ids reports exactly those seven refs (as a
        /// multiset of (to_kind, to_id, from_field) triples), never more,
        /// never fewer -- outbound_refs must not invent or drop sites.
        #[test]
        fn prop_storyboard_outbound_refs_exact_set(
            entry_rm in arb_id(),
            persona in arb_id(),
            entry_ui in arb_id(),
            outcome_event in arb_id(),
            outcome_ui in arb_id(),
            slice_ref in arb_id(),
        ) {
            let trace_slice_ref = slice_ref.clone();
            let entity = full_storyboard(
                &entry_rm, &persona, &entry_ui, &outcome_event, &outcome_ui, &slice_ref, &trace_slice_ref,
            );
            let refs = outbound_refs(&entity);
            let mut got: Vec<(Option<EntityKind>, Id, String)> = refs
                .into_iter()
                .map(|r| (r.to_kind, r.to_id, r.from_field))
                .collect();
            got.sort_by(|a, b| a.2.cmp(&b.2));
            let mut want = vec![
                (Some(EntityKind::ReadModel), entry_rm, "entry.read_model".to_string()),
                (Some(EntityKind::Persona), persona, "entry.observer.human.persona".to_string()),
                (Some(EntityKind::Ui), entry_ui, "entry.observer.human.ui".to_string()),
                (Some(EntityKind::Event), outcome_event, "outcome.events[0]".to_string()),
                (Some(EntityKind::Ui), outcome_ui, "outcome.ui".to_string()),
                (None, slice_ref.clone(), "slices[0]".to_string()),
                (None, slice_ref, "traces[0].steps[0].slice".to_string()),
            ];
            want.sort_by(|a, b| a.2.cmp(&b.2));
            prop_assert_eq!(got, want);
        }
    }
}
