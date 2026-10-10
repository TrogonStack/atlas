use trogon_atlas_proto as pb;

#[must_use]
pub fn diff_entities(a: &pb::Entity, b: &pb::Entity) -> Vec<pb::DiffOp> {
    let mut ops: Vec<pb::DiffOp> = Vec::new();
    let a_kind = kind_name(a);
    let b_kind = kind_name(b);
    if a_kind != b_kind {
        ops.push(pb::DiffOp {
            kind: pb::DiffOpKind::Changed as i32,
            path: "kind".into(),
            path_b: String::new(),
            before: a_kind.into(),
            after: b_kind.into(),
            doc: "entity oneof kind changed".into(),
        });
        return ops;
    }
    diff_common(a, b, &mut ops);
    ops
}

fn kind_name(e: &pb::Entity) -> &'static str {
    use pb::entity::Kind as K;
    match e.kind.as_ref() {
        Some(K::Event(_)) => "Event",
        Some(K::Command(_)) => "Command",
        Some(K::ReadModel(_)) => "ReadModel",
        Some(K::Processor(_)) => "Processor",
        Some(K::Ui(_)) => "Ui",
        Some(K::Persona(_)) => "Persona",
        Some(K::Swimlane(_)) => "Swimlane",
        Some(K::CommandSlice(_)) => "CommandSlice",
        Some(K::ReadModelSlice(_)) => "ReadModelSlice",
        Some(K::AutomationSlice(_)) => "AutomationSlice",
        Some(K::Storyboard(_)) => "Storyboard",
        Some(K::EventModel(_)) => "EventModel",
        Some(K::Component(_)) => "Component",
        Some(K::ExternalSystem(_)) => "ExternalSystem",
        Some(K::Tracker(_)) => "Tracker",
        Some(K::BoundedContext(_)) => "BoundedContext",
        Some(K::Domain(_)) => "Domain",
        Some(K::Subdomain(_)) => "Subdomain",
        Some(K::Schema(_)) => "Schema",
        Some(K::Project(_)) => "Project",
        Some(K::Screen(_)) => "Screen",
        Some(K::Term(_)) => "Term",
        Some(K::Ambiguity(_)) => "Ambiguity",
        Some(K::UiSlice(_)) => "UiSlice",
        Some(K::ServiceLevelIndicator(_)) => "ServiceLevelIndicator",
        Some(K::ServiceLevelObjective(_)) => "ServiceLevelObjective",
        Some(K::AlertPolicy(_)) => "AlertPolicy",
        Some(K::AlertNotificationTarget(_)) => "AlertNotificationTarget",
        Some(K::TypeLibrary(_)) => "TypeLibrary",
        None => "<empty>",
    }
}

fn diff_common(a: &pb::Entity, b: &pb::Entity, ops: &mut Vec<pb::DiffOp>) {
    use pb::entity::Kind as K;
    match (a.kind.as_ref(), b.kind.as_ref()) {
        (Some(K::Event(x)), Some(K::Event(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_schema(ops, x.schema.as_ref(), y.schema.as_ref());
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
        }
        (Some(K::Command(x)), Some(K::Command(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_schema(ops, x.schema.as_ref(), y.schema.as_ref());
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
        }
        (Some(K::ReadModel(x)), Some(K::ReadModel(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_schema(ops, x.schema.as_ref(), y.schema.as_ref());
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
        }
        (Some(K::Processor(x)), Some(K::Processor(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
        }
        (Some(K::Ui(x)), Some(K::Ui(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
        }
        (Some(K::Persona(x)), Some(K::Persona(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
        }
        (Some(K::Swimlane(x)), Some(K::Swimlane(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
        }
        (Some(K::EventModel(x)), Some(K::EventModel(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            scalar(
                ops,
                "members.len",
                &x.members.len().to_string(),
                &y.members.len().to_string(),
            );
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
        }
        (Some(K::CommandSlice(x)), Some(K::CommandSlice(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_edge_ref(
                ops,
                "command",
                edge_command_slug(x.command.as_ref()),
                edge_command_slug(y.command.as_ref()),
            );
            diff_edge_ref(
                ops,
                "persona",
                edge_persona_slug(x.persona.as_ref()),
                edge_persona_slug(y.persona.as_ref()),
            );
            diff_edge_ref(
                ops,
                "ui",
                edge_ui_slug(x.ui.as_ref()),
                edge_ui_slug(y.ui.as_ref()),
            );
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            diff_event_edge_list(ops, "emitted_events", &x.emitted_events, &y.emitted_events);
        }
        (Some(K::ReadModelSlice(x)), Some(K::ReadModelSlice(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_edge_ref(
                ops,
                "read_model",
                edge_read_model_slug(x.read_model.as_ref()),
                edge_read_model_slug(y.read_model.as_ref()),
            );
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            diff_event_edge_list(ops, "source_events", &x.source_events, &y.source_events);
        }
        (Some(K::UiSlice(x)), Some(K::UiSlice(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_edge_ref(
                ops,
                "persona",
                edge_persona_slug(x.persona.as_ref()),
                edge_persona_slug(y.persona.as_ref()),
            );
            diff_edge_ref(
                ops,
                "ui",
                edge_ui_slug(x.ui.as_ref()),
                edge_ui_slug(y.ui.as_ref()),
            );
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            diff_read_model_edge_list(
                ops,
                "source_read_models",
                &x.source_read_models,
                &y.source_read_models,
            );
        }
        (Some(K::AutomationSlice(x)), Some(K::AutomationSlice(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_edge_ref(
                ops,
                "emitted_command",
                edge_command_slug(x.emitted_command.as_ref()),
                edge_command_slug(y.emitted_command.as_ref()),
            );
            diff_edge_ref(
                ops,
                "processor",
                edge_processor_slug(x.processor.as_ref()),
                edge_processor_slug(y.processor.as_ref()),
            );
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            diff_read_model_edge_list(
                ops,
                "source_read_models",
                &x.source_read_models,
                &y.source_read_models,
            );
        }
        (Some(K::Storyboard(x)), Some(K::Storyboard(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            scalar(
                ops,
                "slices.len",
                &x.slices.len().to_string(),
                &y.slices.len().to_string(),
            );
            scalar(
                ops,
                "traces.len",
                &x.traces.len().to_string(),
                &y.traces.len().to_string(),
            );
        }
        (Some(K::Component(x)), Some(K::Component(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            diff_string_list(ops, "tech", &x.tech, &y.tech);
            scalar(
                ops,
                "members.len",
                &x.members.len().to_string(),
                &y.members.len().to_string(),
            );
        }
        (Some(K::ExternalSystem(x)), Some(K::ExternalSystem(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            diff_string_list(ops, "tech", &x.tech, &y.tech);
        }
        (Some(K::Tracker(x)), Some(K::Tracker(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            scalar(
                ops,
                "items.len",
                &x.items.len().to_string(),
                &y.items.len().to_string(),
            );
        }
        (Some(K::BoundedContext(x)), Some(K::BoundedContext(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            scalar(
                ops,
                "realizes.len",
                &x.realizes.len().to_string(),
                &y.realizes.len().to_string(),
            );
            scalar(
                ops,
                "relationships.len",
                &x.relationships.len().to_string(),
                &y.relationships.len().to_string(),
            );
        }
        (Some(K::Domain(x)), Some(K::Domain(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            diff_edge_ref(
                ops,
                "project",
                id_slug(x.project.as_ref().and_then(|p| p.id.as_ref())),
                id_slug(y.project.as_ref().and_then(|p| p.id.as_ref())),
            );
        }
        (Some(K::Subdomain(x)), Some(K::Subdomain(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            scalar(
                ops,
                "classification",
                &x.classification.to_string(),
                &y.classification.to_string(),
            );
            diff_edge_ref(
                ops,
                "domain",
                id_slug(x.domain.as_ref().and_then(|d| d.id.as_ref())),
                id_slug(y.domain.as_ref().and_then(|d| d.id.as_ref())),
            );
        }
        (Some(K::Schema(x)), Some(K::Schema(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            diff_field_list(ops, "fields", &x.fields, &y.fields);
        }
        (Some(K::Project(x)), Some(K::Project(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            scalar(ops, "source_ref", &x.source_ref, &y.source_ref);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
        }
        (Some(K::Screen(x)), Some(K::Screen(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            scalar(ops, "route", &x.route, &y.route);
            scalar(ops, "audience", &x.audience, &y.audience);
            scalar(ops, "layout", &x.layout, &y.layout);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            scalar(
                ops,
                "slots.len",
                &x.slots.len().to_string(),
                &y.slots.len().to_string(),
            );
        }
        (Some(K::Term(x)), Some(K::Term(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            scalar(
                ops,
                "embodied_by.len",
                &x.embodied_by.len().to_string(),
                &y.embodied_by.len().to_string(),
            );
        }
        (Some(K::Ambiguity(x)), Some(K::Ambiguity(y))) => {
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            scalar(ops, "kind", &x.kind.to_string(), &y.kind.to_string());
            scalar(ops, "ruling", &x.ruling.to_string(), &y.ruling.to_string());
            scalar(
                ops,
                "terms.len",
                &x.terms.len().to_string(),
                &y.terms.len().to_string(),
            );
        }
        (Some(K::ServiceLevelIndicator(x)), Some(K::ServiceLevelIndicator(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            scalar(
                ops,
                "connection.kind",
                connection_kind_label(x.connection.as_ref()),
                connection_kind_label(y.connection.as_ref()),
            );
            scalar(
                ops,
                "metadata.len",
                &x.metadata.len().to_string(),
                &y.metadata.len().to_string(),
            );
        }
        (Some(K::ServiceLevelObjective(x)), Some(K::ServiceLevelObjective(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            diff_edge_ref(
                ops,
                "service",
                id_slug(x.service.as_ref().and_then(|r| r.id.as_ref())),
                id_slug(y.service.as_ref().and_then(|r| r.id.as_ref())),
            );
            diff_edge_ref(
                ops,
                "indicator",
                id_slug(x.indicator.as_ref().and_then(|r| r.id.as_ref())),
                id_slug(y.indicator.as_ref().and_then(|r| r.id.as_ref())),
            );
            scalar(
                ops,
                "objectives.len",
                &x.objectives.len().to_string(),
                &y.objectives.len().to_string(),
            );
            scalar(
                ops,
                "alert_policies.len",
                &x.alert_policies.len().to_string(),
                &y.alert_policies.len().to_string(),
            );
        }
        (Some(K::AlertPolicy(x)), Some(K::AlertPolicy(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            scalar(
                ops,
                "conditions.len",
                &x.conditions.len().to_string(),
                &y.conditions.len().to_string(),
            );
            scalar(
                ops,
                "notification_targets.len",
                &x.notification_targets.len().to_string(),
                &y.notification_targets.len().to_string(),
            );
        }
        (Some(K::AlertNotificationTarget(x)), Some(K::AlertNotificationTarget(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            scalar(ops, "target", &x.target, &y.target);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
        }
        (Some(K::TypeLibrary(x)), Some(K::TypeLibrary(y))) => {
            scalar(ops, "title", &x.title, &y.title);
            scalar(ops, "doc", &x.doc, &y.doc);
            diff_id_ref(
                ops,
                "supersedes",
                x.supersedes.as_ref(),
                y.supersedes.as_ref(),
            );
            for f in &y.files {
                let before = x.files.iter().find(|o| o.path == f.path);
                let a = before.map_or("", |o| o.content.as_str());
                scalar(ops, &format!("files[{}]", f.path), a, &f.content);
            }
            for f in &x.files {
                if !y.files.iter().any(|n| n.path == f.path) {
                    scalar(ops, &format!("files[{}]", f.path), &f.content, "");
                }
            }
            let deps = |l: &pb::TypeLibrary| {
                l.dependencies
                    .iter()
                    .filter_map(|d| d.id.as_ref())
                    .map(|i| i.slug.clone())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            scalar(ops, "dependencies", &deps(x), &deps(y));
        }
        _ => {}
    }
}

fn connection_kind_label(conn: Option<&trogon_atlas_proto::Connection>) -> &'static str {
    use trogon_atlas_proto::connection::Kind as ConnKind;
    match conn.and_then(|c| c.kind.as_ref()) {
        Some(ConnKind::CommandHandling(_)) => "command_handling",
        Some(ConnKind::Projection(_)) => "projection",
        Some(ConnKind::Reaction(_)) => "reaction",
        Some(ConnKind::Display(_)) => "display",
        Some(ConnKind::Integration(_)) => "integration",
        Some(ConnKind::Journey(_)) => "journey",
        None => "<unset>",
    }
}

fn scalar(ops: &mut Vec<pb::DiffOp>, path: &str, a: &str, b: &str) {
    if a != b {
        ops.push(pb::DiffOp {
            kind: pb::DiffOpKind::Changed as i32,
            path: path.into(),
            path_b: String::new(),
            before: a.into(),
            after: b.into(),
            doc: String::new(),
        });
    }
}

fn diff_schema(
    ops: &mut Vec<pb::DiffOp>,
    a: Option<&prost_types::Any>,
    b: Option<&prost_types::Any>,
) {
    let type_name = trogon_atlas_core::schema::type_name;
    scalar(
        ops,
        "schema.type_url",
        a.map_or("", type_name),
        b.map_or("", type_name),
    );
    diff_field_list(
        ops,
        "schema.fields",
        &trogon_atlas_core::schema::schema_fields(a),
        &trogon_atlas_core::schema::schema_fields(b),
    );
}

fn diff_field_list(
    ops: &mut Vec<pb::DiffOp>,
    base_path: &str,
    a: &[pb::FieldSpec],
    b: &[pb::FieldSpec],
) {
    let by_name_a: std::collections::HashMap<&str, &pb::FieldSpec> =
        a.iter().map(|f| (f.name.as_str(), f)).collect();
    let by_name_b: std::collections::HashMap<&str, &pb::FieldSpec> =
        b.iter().map(|f| (f.name.as_str(), f)).collect();
    for (name, fa) in &by_name_a {
        match by_name_b.get(name) {
            None => ops.push(pb::DiffOp {
                kind: pb::DiffOpKind::Removed as i32,
                path: format!("{base_path}[{name}]"),
                path_b: String::new(),
                before: format!(
                    "kind={} doc={}",
                    fa.r#type.as_ref().map_or(
                        "unspecified",
                        trogon_atlas_proto::canonical::field_type_label
                    ),
                    fa.doc
                ),
                after: String::new(),
                doc: String::new(),
            }),
            Some(fb) => {
                if fa.r#type != fb.r#type {
                    let label = |f: &pb::FieldSpec| {
                        f.r#type
                            .as_ref()
                            .map_or(
                                "unspecified",
                                trogon_atlas_proto::canonical::field_type_label,
                            )
                            .to_string()
                    };
                    ops.push(pb::DiffOp {
                        kind: pb::DiffOpKind::Changed as i32,
                        path: format!("{base_path}[{name}].kind"),
                        path_b: String::new(),
                        before: label(fa),
                        after: label(fb),
                        doc: String::new(),
                    });
                }
                if fa.doc != fb.doc {
                    ops.push(pb::DiffOp {
                        kind: pb::DiffOpKind::Changed as i32,
                        path: format!("{base_path}[{name}].doc"),
                        path_b: String::new(),
                        before: fa.doc.clone(),
                        after: fb.doc.clone(),
                        doc: String::new(),
                    });
                }
            }
        }
    }
    for (name, fb) in &by_name_b {
        if !by_name_a.contains_key(name) {
            ops.push(pb::DiffOp {
                kind: pb::DiffOpKind::Added as i32,
                path: format!("{base_path}[{name}]"),
                path_b: String::new(),
                before: String::new(),
                after: format!(
                    "kind={} doc={}",
                    fb.r#type.as_ref().map_or(
                        "unspecified",
                        trogon_atlas_proto::canonical::field_type_label
                    ),
                    fb.doc
                ),
                doc: String::new(),
            });
        }
    }
}

fn id_slug(id: Option<&pb::Id>) -> &str {
    id.map_or("", |i| i.slug.as_str())
}

fn edge_command_slug(edge: Option<&pb::CommandEdge>) -> &str {
    edge.and_then(|e| e.command.as_ref()?.id.as_ref().map(|i| i.slug.as_str()))
        .unwrap_or("")
}

fn edge_persona_slug(edge: Option<&pb::PersonaEdge>) -> &str {
    edge.and_then(|e| e.persona.as_ref()?.id.as_ref().map(|i| i.slug.as_str()))
        .unwrap_or("")
}

fn edge_ui_slug(edge: Option<&pb::UiEdge>) -> &str {
    edge.and_then(|e| e.ui.as_ref()?.id.as_ref().map(|i| i.slug.as_str()))
        .unwrap_or("")
}

fn edge_read_model_slug(edge: Option<&pb::ReadModelEdge>) -> &str {
    edge.and_then(|e| e.read_model.as_ref()?.id.as_ref().map(|i| i.slug.as_str()))
        .unwrap_or("")
}

fn edge_processor_slug(edge: Option<&pb::ProcessorEdge>) -> &str {
    edge.and_then(|e| e.processor.as_ref()?.id.as_ref().map(|i| i.slug.as_str()))
        .unwrap_or("")
}

fn diff_edge_ref(ops: &mut Vec<pb::DiffOp>, path: &str, a: &str, b: &str) {
    if a != b {
        ops.push(pb::DiffOp {
            kind: pb::DiffOpKind::Changed as i32,
            path: path.into(),
            path_b: String::new(),
            before: a.into(),
            after: b.into(),
            doc: String::new(),
        });
    }
}

fn diff_id_ref(ops: &mut Vec<pb::DiffOp>, path: &str, a: Option<&pb::Id>, b: Option<&pb::Id>) {
    let a_str = a
        .map(|i| format!("{}/{}@{}", i.namespace, i.slug, i.version))
        .unwrap_or_default();
    let b_str = b
        .map(|i| format!("{}/{}@{}", i.namespace, i.slug, i.version))
        .unwrap_or_default();
    if a_str != b_str {
        ops.push(pb::DiffOp {
            kind: pb::DiffOpKind::Changed as i32,
            path: path.into(),
            path_b: String::new(),
            before: a_str,
            after: b_str,
            doc: String::new(),
        });
    }
}

fn diff_string_list(ops: &mut Vec<pb::DiffOp>, base_path: &str, a: &[String], b: &[String]) {
    let set_a: std::collections::HashSet<&str> =
        a.iter().map(std::string::String::as_str).collect();
    let set_b: std::collections::HashSet<&str> =
        b.iter().map(std::string::String::as_str).collect();
    for item in &set_a {
        if !set_b.contains(item) {
            ops.push(pb::DiffOp {
                kind: pb::DiffOpKind::Removed as i32,
                path: format!("{base_path}[{item}]"),
                path_b: String::new(),
                before: (*item).into(),
                after: String::new(),
                doc: String::new(),
            });
        }
    }
    for item in &set_b {
        if !set_a.contains(item) {
            ops.push(pb::DiffOp {
                kind: pb::DiffOpKind::Added as i32,
                path: format!("{base_path}[{item}]"),
                path_b: String::new(),
                before: String::new(),
                after: (*item).into(),
                doc: String::new(),
            });
        }
    }
}

fn diff_event_edge_list(
    ops: &mut Vec<pb::DiffOp>,
    base_path: &str,
    a: &[pb::EventEdge],
    b: &[pb::EventEdge],
) {
    let slugs_a: std::collections::HashSet<&str> = a
        .iter()
        .filter_map(|e| e.event.as_ref()?.id.as_ref().map(|i| i.slug.as_str()))
        .collect();
    let slugs_b: std::collections::HashSet<&str> = b
        .iter()
        .filter_map(|e| e.event.as_ref()?.id.as_ref().map(|i| i.slug.as_str()))
        .collect();
    for slug in &slugs_a {
        if !slugs_b.contains(slug) {
            ops.push(pb::DiffOp {
                kind: pb::DiffOpKind::Removed as i32,
                path: format!("{base_path}[{slug}]"),
                path_b: String::new(),
                before: (*slug).into(),
                after: String::new(),
                doc: String::new(),
            });
        }
    }
    for slug in &slugs_b {
        if !slugs_a.contains(slug) {
            ops.push(pb::DiffOp {
                kind: pb::DiffOpKind::Added as i32,
                path: format!("{base_path}[{slug}]"),
                path_b: String::new(),
                before: String::new(),
                after: (*slug).into(),
                doc: String::new(),
            });
        }
    }
}

fn diff_read_model_edge_list(
    ops: &mut Vec<pb::DiffOp>,
    base_path: &str,
    a: &[pb::ReadModelEdge],
    b: &[pb::ReadModelEdge],
) {
    let slugs_a: std::collections::HashSet<&str> = a
        .iter()
        .filter_map(|e| e.read_model.as_ref()?.id.as_ref().map(|i| i.slug.as_str()))
        .collect();
    let slugs_b: std::collections::HashSet<&str> = b
        .iter()
        .filter_map(|e| e.read_model.as_ref()?.id.as_ref().map(|i| i.slug.as_str()))
        .collect();
    for slug in &slugs_a {
        if !slugs_b.contains(slug) {
            ops.push(pb::DiffOp {
                kind: pb::DiffOpKind::Removed as i32,
                path: format!("{base_path}[{slug}]"),
                path_b: String::new(),
                before: (*slug).into(),
                after: String::new(),
                doc: String::new(),
            });
        }
    }
    for slug in &slugs_b {
        if !slugs_a.contains(slug) {
            ops.push(pb::DiffOp {
                kind: pb::DiffOpKind::Added as i32,
                path: format!("{base_path}[{slug}]"),
                path_b: String::new(),
                before: String::new(),
                after: (*slug).into(),
                doc: String::new(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn event(ns: &str, slug: &str, title: &str, fields: &[(&str, &str)]) -> pb::Entity {
        let id = pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: 0,
        };
        let ft = |_label: &str| pb::FieldType {
            kind: Some(pb::field_type::Kind::String(pb::field_type::StringType {})),
        };
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id),
                title: title.into(),
                schema: Some(trogon_atlas_core::schema::pack_fields(
                    fields
                        .iter()
                        .map(|(name, label)| pb::FieldSpec {
                            name: (*name).into(),
                            r#type: Some(ft(label)),
                            ..Default::default()
                        })
                        .collect(),
                )),
                ..Default::default()
            })),
        }
    }

    fn command(ns: &str, slug: &str, title: &str) -> pb::Entity {
        let id = pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: 0,
        };
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Command(pb::Command {
                id: Some(id),
                title: title.into(),
                ..Default::default()
            })),
        }
    }

    fn command_slice(ns: &str, slug: &str) -> pb::Entity {
        let id = pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: 0,
        };
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id),
                ..Default::default()
            })),
        }
    }

    #[test]
    fn no_diff_for_identical_entities() {
        let a = event(
            "shop",
            "order-placed",
            "Order placed",
            &[("order_id", "string")],
        );
        let ops = diff_entities(&a, &a);
        assert!(ops.is_empty(), "expected no ops for identical entities");
    }

    #[test]
    fn detects_title_change() {
        let a = event("shop", "order-placed", "Old title", &[]);
        let b = event("shop", "order-placed", "New title", &[]);
        let ops = diff_entities(&a, &b);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].path, "title");
        assert_eq!(ops[0].before, "Old title");
        assert_eq!(ops[0].after, "New title");
        assert_eq!(ops[0].kind, pb::DiffOpKind::Changed as i32);
    }

    /// Event/Command/ReadModel (and other core kinds) carry `supersedes`, and
    /// slices already surface it in DiffEntities. A supersession-only change
    /// must not collapse to an empty op list: that is a silent wrong-empty
    /// for version navigation and branch review.
    #[test]
    fn detects_event_supersedes_change() {
        let a = event("shop", "order-placed", "Order placed", &[]);
        let mut b = event("shop", "order-placed", "Order placed", &[]);
        match b.kind.as_mut() {
            Some(pb::entity::Kind::Event(e)) => {
                e.supersedes = Some(pb::Id {
                    namespace: "shop".into(),
                    slug: "order-placed".into(),
                    version: 1,
                });
                e.id.as_mut().unwrap().version = 2;
            }
            other => panic!("expected Event, got {other:?}"),
        }
        let ops = diff_entities(&a, &b);
        assert!(
            ops.iter().any(|op| op.path == "supersedes"),
            "Event supersedes change must produce a DiffOp; got {ops:?}"
        );
    }

    /// `diff_id_ref` must include namespace. Same slug@version in a different
    /// namespace is a real retarget; omitting namespace yields a wrong-empty diff.
    #[test]
    fn detects_supersedes_namespace_retarget() {
        let mut a = event("shop", "order-placed", "Order placed", &[]);
        let mut b = event("shop", "order-placed", "Order placed", &[]);
        match a.kind.as_mut() {
            Some(pb::entity::Kind::Event(e)) => {
                e.supersedes = Some(pb::Id {
                    namespace: "shop".into(),
                    slug: "legacy".into(),
                    version: 1,
                });
            }
            other => panic!("expected Event, got {other:?}"),
        }
        match b.kind.as_mut() {
            Some(pb::entity::Kind::Event(e)) => {
                e.supersedes = Some(pb::Id {
                    namespace: "other".into(),
                    slug: "legacy".into(),
                    version: 1,
                });
            }
            other => panic!("expected Event, got {other:?}"),
        }
        let ops = diff_entities(&a, &b);
        let op = ops
            .iter()
            .find(|op| op.path == "supersedes")
            .expect("namespace-only supersedes retarget must produce a DiffOp");
        assert_eq!(op.before, "shop/legacy@1");
        assert_eq!(op.after, "other/legacy@1");
    }

    #[test]
    fn detects_field_added() {
        let a = event("shop", "order-placed", "T", &[]);
        let b = event("shop", "order-placed", "T", &[("new_field", "string")]);
        let ops = diff_entities(&a, &b);
        assert_eq!(ops.len(), 1, "expected one add op");
        assert_eq!(ops[0].kind, pb::DiffOpKind::Added as i32);
        assert!(ops[0].path.contains("new_field"));
    }

    #[test]
    fn detects_field_removed() {
        let a = event("shop", "order-placed", "T", &[("old_field", "string")]);
        let b = event("shop", "order-placed", "T", &[]);
        let ops = diff_entities(&a, &b);
        assert_eq!(ops.len(), 1, "expected one remove op");
        assert_eq!(ops[0].kind, pb::DiffOpKind::Removed as i32);
        assert!(ops[0].path.contains("old_field"));
    }

    #[test]
    fn kind_change_produces_single_changed_op_and_returns() {
        let a = event("shop", "x", "T", &[]);
        let b = command("shop", "x", "T");
        let ops = diff_entities(&a, &b);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].path, "kind");
        assert_eq!(ops[0].kind, pb::DiffOpKind::Changed as i32);
    }

    #[test]
    fn command_slice_title_change_is_detected() {
        let a = command_slice("shop", "s");
        let mut b = command_slice("shop", "s");
        if let Some(pb::entity::Kind::CommandSlice(ref mut s)) = b.kind {
            s.title = "changed".into();
        }
        let ops = diff_entities(&a, &b);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].path, "title");
        assert_eq!(ops[0].kind, pb::DiffOpKind::Changed as i32);
    }

    #[test]
    fn command_slice_identical_produces_no_ops() {
        let a = command_slice("shop", "s");
        let ops = diff_entities(&a, &a);
        assert_eq!(ops, [] as [trogon_atlas_proto::DiffOp; 0]);
    }

    fn make_id(ns: &str, slug: &str) -> pb::Id {
        pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn read_model_slice(ns: &str, slug: &str, title: &str, source_events: Vec<&str>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
                id: Some(make_id(ns, slug)),
                title: title.into(),
                source_events: source_events
                    .into_iter()
                    .map(|ev| pb::EventEdge {
                        event: Some(pb::EventRef {
                            id: Some(make_id(ns, ev)),
                        }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn automation_slice(ns: &str, slug: &str, title: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
                id: Some(make_id(ns, slug)),
                title: title.into(),
                ..Default::default()
            })),
        }
    }

    fn domain_entity(ns: &str, slug: &str, title: &str, doc: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Domain(pb::Domain {
                id: Some(make_id(ns, slug)),
                title: title.into(),
                doc: doc.into(),
                ..Default::default()
            })),
        }
    }

    fn schema_entity(ns: &str, slug: &str, fields: &[(&str, &str)]) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Schema(pb::Schema {
                id: Some(make_id(ns, slug)),
                fields: fields
                    .iter()
                    .map(|(name, _)| pb::FieldSpec {
                        name: (*name).into(),
                        r#type: Some(pb::FieldType {
                            kind: Some(pb::field_type::Kind::String(pb::field_type::StringType {})),
                        }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    #[test]
    fn read_model_slice_detects_title_and_source_event_changes() {
        let a = read_model_slice("shop", "orders-rm-slice", "Orders", vec!["order-placed"]);
        let b = read_model_slice(
            "shop",
            "orders-rm-slice",
            "Orders v2",
            vec!["order-placed", "order-updated"],
        );
        let ops = diff_entities(&a, &b);
        let title_op = ops.iter().find(|o| o.path == "title");
        assert!(title_op.is_some(), "expected title change op");
        assert_eq!(title_op.unwrap().before, "Orders");
        assert_eq!(title_op.unwrap().after, "Orders v2");
        let added_op = ops
            .iter()
            .find(|o| o.kind == pb::DiffOpKind::Added as i32 && o.path.contains("order-updated"));
        assert!(
            added_op.is_some(),
            "expected added source_event op for order-updated"
        );
    }

    #[test]
    fn read_model_slice_identical_produces_no_ops() {
        let a = read_model_slice("shop", "orders-rm-slice", "Orders", vec!["order-placed"]);
        let ops = diff_entities(&a, &a);
        assert!(
            ops.is_empty(),
            "expected no ops for identical read model slices"
        );
    }

    #[test]
    fn automation_slice_detects_title_change() {
        let a = automation_slice("shop", "notify-slice", "Notify on order");
        let mut b = automation_slice("shop", "notify-slice", "Notify on order v2");
        if let Some(pb::entity::Kind::AutomationSlice(ref mut s)) = b.kind {
            s.title = "Notify on order v2".into();
        }
        let ops = diff_entities(&a, &b);
        let title_op = ops.iter().find(|o| o.path == "title");
        assert!(title_op.is_some(), "expected title change op");
        assert_eq!(title_op.unwrap().before, "Notify on order");
        assert_eq!(title_op.unwrap().after, "Notify on order v2");
    }

    #[test]
    fn automation_slice_identical_produces_no_ops() {
        let a = automation_slice("shop", "notify-slice", "Notify");
        let ops = diff_entities(&a, &a);
        assert!(
            ops.is_empty(),
            "expected no ops for identical automation slices"
        );
    }

    #[test]
    fn domain_detects_doc_change() {
        let a = domain_entity("biz", "ecommerce", "Ecommerce", "old doc");
        let b = domain_entity("biz", "ecommerce", "Ecommerce", "new doc");
        let ops = diff_entities(&a, &b);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].path, "doc");
        assert_eq!(ops[0].before, "old doc");
        assert_eq!(ops[0].after, "new doc");
        assert_eq!(ops[0].kind, pb::DiffOpKind::Changed as i32);
    }

    #[test]
    fn domain_identical_produces_no_ops() {
        let a = domain_entity("biz", "ecommerce", "Ecommerce", "doc");
        let ops = diff_entities(&a, &a);
        assert!(
            ops.is_empty(),
            "expected no ops for identical domain entities"
        );
    }

    #[test]
    fn schema_detects_field_added() {
        let a = schema_entity("shop", "order-schema", &[("order_id", "string")]);
        let b = schema_entity(
            "shop",
            "order-schema",
            &[("order_id", "string"), ("amount", "int")],
        );
        let ops = diff_entities(&a, &b);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].kind, pb::DiffOpKind::Added as i32);
        assert!(
            ops[0].path.contains("amount"),
            "expected added field 'amount' in path"
        );
    }

    #[test]
    fn schema_detects_field_removed() {
        let a = schema_entity(
            "shop",
            "order-schema",
            &[("order_id", "string"), ("amount", "int")],
        );
        let b = schema_entity("shop", "order-schema", &[("order_id", "string")]);
        let ops = diff_entities(&a, &b);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].kind, pb::DiffOpKind::Removed as i32);
        assert!(
            ops[0].path.contains("amount"),
            "expected removed field 'amount' in path"
        );
    }

    #[test]
    fn schema_identical_produces_no_ops() {
        let a = schema_entity("shop", "order-schema", &[("order_id", "string")]);
        let ops = diff_entities(&a, &a);
        assert!(
            ops.is_empty(),
            "expected no ops for identical schema entities"
        );
    }
}
