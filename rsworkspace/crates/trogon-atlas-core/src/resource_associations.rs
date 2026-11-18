//! Shared checks for descriptive resource associations and independent
//! message-to-channel declarations. Claim routes and operation bindings stay
//! outside this helper so a later merge can add them without renumbering.

use std::collections::HashMap;

use trogon_atlas_proto as pb;

/// Validate declared parameter values against a channel address.
/// Omitted parameters keep the address default for this check and are not
/// copied into the declaration. An empty string is a supplied value.
pub fn bind_parameters<S: std::hash::BuildHasher>(
    channel: &pb::Channel,
    values: &HashMap<String, String, S>,
) -> Result<(), &'static str> {
    let Some(address) = channel.address.as_ref() else {
        return if values.is_empty() {
            Ok(())
        } else {
            Err("channel without an address accepts no parameter values")
        };
    };
    let mut parameters = HashMap::new();
    for parameter in &address.parameters {
        let name = parameter
            .name
            .as_ref()
            .map_or("", |name| name.value.as_str());
        if name.is_empty() || parameters.insert(name, parameter).is_some() {
            return Err("channel address parameters are invalid");
        }
    }
    if values
        .keys()
        .any(|name| !parameters.contains_key(name.as_str()))
    {
        return Err("message channel parameter name is not declared");
    }
    for (name, parameter) in &parameters {
        let supplied = values.get(*name);
        let value = supplied.or(parameter.default.as_ref());
        let Some(value) = value else {
            return Err("message channel parameter is required");
        };
        if parameter
            .allowed_values
            .as_ref()
            .is_some_and(|allowed| !allowed.values.iter().any(|candidate| candidate == value))
        {
            return Err("message channel parameter value is not allowed");
        }
    }
    Ok(())
}

#[must_use]
pub fn independent_declarations(entity: &pb::Entity) -> Vec<&pb::MessageChannelDeclaration> {
    match &entity.kind {
        Some(pb::entity::Kind::Specification(specification)) => {
            specification.message_channels.iter().collect()
        }
        _ => Vec::new(),
    }
}

#[must_use]
pub fn declared_message_ref(
    entity: &pb::Entity,
    declaration: &pb::MessageChannelDeclaration,
) -> Option<pb::ContractMemberRef> {
    let pb::entity::Kind::Specification(specification) = entity.kind.as_ref()? else {
        return None;
    };
    Some(pb::ContractMemberRef {
        specification: Some(pb::SpecificationRef {
            id: specification.id.clone(),
        }),
        owner_uid: entity
            .system
            .as_ref()
            .map(|system| system.uid.clone())
            .unwrap_or_default(),
        kind: pb::ContractMemberKind::Message as i32,
        member_id: declaration.member_id.clone(),
        expected_digest: None,
    })
}
