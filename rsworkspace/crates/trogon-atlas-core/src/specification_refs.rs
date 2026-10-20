use trogon_atlas_proto::{
    contract_dependency, operation_binding, provider_target, schema_binding, schema_definition,
    EntityKind, Id, ProviderTarget, Specification,
};

use crate::refs::OutboundRef;

fn provider(
    target: &mut ProviderTarget,
    path: &str,
    visit: &mut impl FnMut(EntityKind, &mut Option<Id>, String, bool),
) {
    match target.target.as_mut() {
        Some(provider_target::Target::Component(r)) => {
            visit(
                EntityKind::Component,
                &mut r.id,
                format!("{path}.component"),
                false,
            );
        }
        Some(provider_target::Target::ExternalSystem(r)) => {
            visit(
                EntityKind::ExternalSystem,
                &mut r.id,
                format!("{path}.external_system"),
                false,
            );
        }
        None => {}
    }
}

fn walk(
    value: &mut Specification,
    visit: &mut impl FnMut(EntityKind, &mut Option<Id>, String, bool),
) {
    for (i, schema) in value.schemas.iter_mut().enumerate() {
        if let Some(schema_definition::Target::Schema(r)) =
            schema.definition.as_mut().and_then(|d| d.target.as_mut())
        {
            visit(
                EntityKind::Schema,
                &mut r.id,
                format!("schemas[{i}].definition.schema"),
                false,
            );
        }
    }
    for (i, message) in value.messages.iter_mut().enumerate() {
        if let Some(r) = message.mapping.as_mut().and_then(|m| m.read_model.as_mut()) {
            visit(
                EntityKind::ReadModel,
                &mut r.id,
                format!("messages[{i}].mapping.read_model"),
                false,
            );
        }
        for (j, binding) in message.schemas.iter_mut().enumerate() {
            let path = format!("messages[{i}].schemas[{j}]");
            match binding.r#type.as_mut() {
                Some(schema_binding::Type::Schema(r)) => visit(
                    EntityKind::Schema,
                    &mut r.id,
                    format!("{path}.schema"),
                    false,
                ),
                Some(schema_binding::Type::LibraryType(r)) => {
                    if let Some(library) = r.library.as_mut() {
                        visit(
                            EntityKind::TypeLibrary,
                            &mut library.id,
                            format!("{path}.library_type.library"),
                            false,
                        );
                    }
                }
                Some(schema_binding::Type::ContractType(r)) => {
                    if let Some(owner) = r.schema.as_mut().and_then(|s| s.specification.as_mut()) {
                        visit(
                            EntityKind::Specification,
                            &mut owner.id,
                            format!("{path}.contract_type.schema.specification"),
                            true,
                        );
                    }
                }
                _ => {}
            }
        }
    }
    for (i, operation) in value.operations.iter_mut().enumerate() {
        if let Some(operation_binding::Kind::Channel(binding)) =
            operation.binding.as_mut().and_then(|b| b.kind.as_mut())
        {
            if let Some(r) = binding.channel.as_mut() {
                visit(
                    EntityKind::Channel,
                    &mut r.id,
                    format!("operations[{i}].binding.channel.channel"),
                    false,
                );
            }
        }
    }
    for (i, exposure) in value.exposures.iter_mut().enumerate() {
        if let Some(p) = exposure.provider.as_mut() {
            provider(p, &format!("exposures[{i}].provider"), visit);
        }
    }
    for (i, usage) in value.usages.iter_mut().enumerate() {
        if let Some(p) = usage.caller.as_mut() {
            provider(p, &format!("usages[{i}].caller"), visit);
        }
    }
    for (i, dependency) in value.dependencies.iter_mut().enumerate() {
        let path = format!("dependencies[{i}]");
        match dependency.target.as_mut() {
            Some(contract_dependency::Target::Specification(r)) => visit(
                EntityKind::Specification,
                &mut r.id,
                format!("{path}.specification"),
                false,
            ),
            Some(contract_dependency::Target::TypeLibrary(r)) => visit(
                EntityKind::TypeLibrary,
                &mut r.id,
                format!("{path}.type_library"),
                false,
            ),
            None => {}
        }
    }
}

pub(crate) fn outbound(value: &Specification, out: &mut Vec<OutboundRef>) {
    walk(&mut value.clone(), &mut |kind, id, path, _| {
        if let Some(id) = id {
            out.push(OutboundRef {
                to_kind: Some(kind),
                to_id: id.clone(),
                from_field: path,
            });
        }
    });
}

pub(crate) fn retarget(
    value: &mut Specification,
    kind: EntityKind,
    from: &Id,
    to: &Id,
    rewritten: &mut Vec<String>,
) {
    walk(value, &mut |target_kind, id, path, pinned| {
        if !pinned && target_kind == kind && id.as_ref() == Some(from) {
            *id = Some(to.clone());
            rewritten.push(path);
        }
    });
}
