use trogon_atlas_proto::{self as pb, DataProduct, EntityKind, Id};

use crate::refs::OutboundRef;

fn connection(
    reference: &mut Option<pb::ConnectionRef>,
    path: &str,
    visit: &mut impl FnMut(EntityKind, &mut Option<Id>, String, bool),
) {
    if let Some(owner) = reference.as_mut().and_then(|r| r.owner.as_mut()) {
        if let Ok(kind) = EntityKind::try_from(owner.kind) {
            visit(kind, &mut owner.id, format!("{path}.owner"), true);
        }
    }
}
fn walk(
    value: &mut DataProduct,
    visit: &mut impl FnMut(EntityKind, &mut Option<Id>, String, bool),
) {
    for (i, port) in value.inputs.iter_mut().enumerate() {
        match port.data.as_mut().and_then(|d| d.target.as_mut()) {
            Some(pb::input_data_selection::Target::ReadModel(r)) => visit(
                EntityKind::ReadModel,
                &mut r.id,
                format!("inputs[{i}].data.read_model"),
                false,
            ),
            Some(pb::input_data_selection::Target::Event(r)) => visit(
                EntityKind::Event,
                &mut r.id,
                format!("inputs[{i}].data.event"),
                false,
            ),
            None => {}
        }
    }
    for (i, port) in value.outputs.iter_mut().enumerate() {
        if let Some(r) = port.data.as_mut().and_then(|d| d.read_model.as_mut()) {
            visit(
                EntityKind::ReadModel,
                &mut r.id,
                format!("outputs[{i}].data.read_model"),
                false,
            );
        }
        match port.contract.as_mut().and_then(|c| c.r#type.as_mut()) {
            Some(pb::output_contract::Type::Schema(r)) => visit(
                EntityKind::Schema,
                &mut r.id,
                format!("outputs[{i}].contract.schema"),
                false,
            ),
            Some(pb::output_contract::Type::LibraryType(r)) => {
                if let Some(r) = r.library.as_mut() {
                    visit(
                        EntityKind::TypeLibrary,
                        &mut r.id,
                        format!("outputs[{i}].contract.library_type.library"),
                        false,
                    );
                }
            }
            Some(pb::output_contract::Type::ContractType(r)) => {
                if let Some(r) = r.schema.as_mut().and_then(|r| r.specification.as_mut()) {
                    visit(
                        EntityKind::Specification,
                        &mut r.id,
                        format!("outputs[{i}].contract.contract_type.schema.specification"),
                        true,
                    );
                }
            }
            None => {}
        }
        for (j, access) in port.access.iter_mut().enumerate() {
            let path = format!("outputs[{i}].access[{j}]");
            match access.selection.as_mut() {
                Some(pb::output_access::Selection::Store(r)) => {
                    if let Some(r) = r.store.as_mut() {
                        visit(
                            EntityKind::DataStore,
                            &mut r.id,
                            format!("{path}.store.store"),
                            true,
                        );
                    }
                    connection(
                        &mut r.attachment,
                        &format!("{path}.store.attachment"),
                        visit,
                    );
                }
                Some(pb::output_access::Selection::Channel(r)) => {
                    if let Some(r) = r.channel.as_mut() {
                        visit(
                            EntityKind::Channel,
                            &mut r.id,
                            format!("{path}.channel.channel"),
                            true,
                        );
                    }
                    connection(&mut r.binding, &format!("{path}.channel.binding"), visit);
                    connection(
                        &mut r.observation,
                        &format!("{path}.channel.observation"),
                        visit,
                    );
                }
                Some(pb::output_access::Selection::Operation(r)) => {
                    if let Some(r) = r.operation.as_mut().and_then(|r| r.specification.as_mut()) {
                        visit(
                            EntityKind::Specification,
                            &mut r.id,
                            format!("{path}.operation.operation.specification"),
                            true,
                        );
                    }
                    connection(
                        &mut r.response,
                        &format!("{path}.operation.response"),
                        visit,
                    );
                    connection(
                        &mut r.provider_exposure,
                        &format!("{path}.operation.provider_exposure"),
                        visit,
                    );
                }
                None => {}
            }
        }
    }
}
pub(crate) fn outbound(value: &DataProduct, out: &mut Vec<OutboundRef>) {
    walk(&mut value.clone(), &mut |kind, id, path, _| {
        if let Some(id) = id {
            out.push(OutboundRef {
                to_kind: Some(kind),
                to_id: id.clone(),
                from_field: path,
            });
        }
    });
    crate::storage_claims::outbound(&value.storage_claims, out);
}
pub(crate) fn retarget(
    value: &mut DataProduct,
    kind: EntityKind,
    from: &Id,
    to: &Id,
    rewritten: &mut Vec<String>,
) {
    walk(value, &mut |target, id, path, pinned| {
        if !pinned && target == kind && id.as_ref() == Some(from) {
            *id = Some(to.clone());
            rewritten.push(path);
        }
    });
    crate::storage_claims::retarget(&mut value.storage_claims, kind, from, to, rewritten);
}
