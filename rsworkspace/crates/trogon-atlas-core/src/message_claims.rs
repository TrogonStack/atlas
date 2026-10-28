use std::collections::HashSet;

use prost::Message;
use trogon_atlas_proto::{self as pb, entity, Entity, EntityKind, EntityRef};

use crate::{connections, refs::OutboundRef};

pub const MAX_CLAIMS: usize = 1024;
pub const MAX_OCCURRENCES: usize = 4096;
pub const MAX_SELECTORS: usize = 256;
pub const MAX_ENCODED_BYTES: usize = 1_048_576;
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct ClaimError(pub String);
type Result<T> = std::result::Result<T, ClaimError>;
fn check(ok: bool, msg: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(ClaimError(msg.into()))
    }
}
#[must_use]
pub fn declarations(owner: &Entity) -> Option<&pb::MessageClaims> {
    match &owner.kind {
        Some(entity::Kind::Component(v)) => v.message_claims.as_ref(),
        Some(entity::Kind::ExternalSystem(v)) => v.message_claims.as_ref(),
        Some(entity::Kind::Domain(v)) => v.message_claims.as_ref(),
        _ => None,
    }
}
pub fn declarations_mut(owner: &mut Entity) -> Option<&mut pb::MessageClaims> {
    match &mut owner.kind {
        Some(entity::Kind::Component(v)) => v.message_claims.as_mut(),
        Some(entity::Kind::ExternalSystem(v)) => v.message_claims.as_mut(),
        Some(entity::Kind::Domain(v)) => v.message_claims.as_mut(),
        _ => None,
    }
}
#[must_use]
pub fn has_content(owner: &Entity) -> bool {
    declarations(owner).is_some_and(|v| {
        !v.claims.is_empty()
            || !v.routes.is_empty()
            || !v.triggers.is_empty()
            || !v.field_selections.is_empty()
    })
}
fn exact(id: Option<&pb::Id>) -> Result<()> {
    check(
        id.is_some_and(|v| {
            v.version > 0
                && crate::is_safe_id_component(&v.namespace)
                && crate::is_safe_id_component(&v.slug)
        }),
        "exact target required",
    )
}
fn uid(v: &str) -> Result<()> {
    check(
        uuid::Uuid::parse_str(v).is_ok_and(|u| !u.is_nil() && u.to_string() == v),
        "invalid owner UID",
    )
}
fn local(v: Option<&pb::ConnectionId>) -> Result<&pb::ConnectionId> {
    let v = v.ok_or_else(|| ClaimError("local claim ID required".into()))?;
    v.value
        .parse::<connections::ConnectionId>()
        .map_err(|e| ClaimError(e.to_string()))?;
    Ok(v)
}
fn message(v: Option<&pb::ContractMemberRef>) -> Result<&pb::ContractMemberRef> {
    let v = v.ok_or_else(|| ClaimError("message required".into()))?;
    exact(v.specification.as_ref().and_then(|s| s.id.as_ref()))?;
    uid(&v.owner_uid)?;
    check(
        v.kind == pb::ContractMemberKind::Message as i32 && v.expected_digest.is_none(),
        "exact MESSAGE without digest required",
    )?;
    v.member_id
        .as_ref()
        .ok_or_else(|| ClaimError("member ID required".into()))?
        .value
        .parse::<crate::specification::MemberId>()
        .map_err(|e| ClaimError(e.to_string()))?;
    Ok(v)
}
fn text(v: Option<&pb::CatalogText>) -> Result<()> {
    if v.is_none() {
        return Ok(());
    }
    crate::decision_document::validate_catalog_text(v).map_err(|e| ClaimError(e.to_string()))
}
pub fn validate_values(owner: &Entity) -> Result<()> {
    let Some(v) = declarations(owner) else {
        return Ok(());
    };
    exact(crate::entity_id(owner))?;
    check(
        owner.encoded_len() <= MAX_ENCODED_BYTES,
        "claim owner exceeds encoded byte limit",
    )?;
    check(v.claims.len() <= MAX_CLAIMS, "claim count limit exceeded")?;
    check(
        connections::connection_occurrences(owner)
            .map_err(|e| ClaimError(e.to_string()))?
            .len()
            <= MAX_OCCURRENCES,
        "owner occurrence limit exceeded",
    )?;
    connections::validate_connection_ids(owner).map_err(|e| ClaimError(e.to_string()))?;
    for c in &v.claims {
        check(
            matches!(
                pb::CommunicationDirection::try_from(c.direction),
                Ok(pb::CommunicationDirection::Send | pb::CommunicationDirection::Receive)
            ),
            "claim direction required",
        )?;
        message(c.message.as_ref())?;
        if c.group.is_some() {
            crate::decision_document::validate_catalog_title(c.group.as_ref())
                .map_err(|e| ClaimError(e.to_string()))?;
        }
        text(c.doc.as_ref())?;
    }
    let claim = |id: Option<&pb::ConnectionId>| -> Result<&pb::MessageClaim> {
        let id = local(id)?;
        v.claims
            .iter()
            .find(|c| c.connection_id.as_ref() == Some(id))
            .ok_or_else(|| ClaimError("local claim unavailable".into()))
    };
    for r in &v.routes {
        claim(r.claim_id.as_ref())?;
        exact(r.channel.as_ref().and_then(|c| c.id.as_ref()))?;
        check(
            r.delivery_mode.is_none_or(|m| {
                matches!(
                    pb::ClaimDeliveryMode::try_from(m),
                    Ok(pb::ClaimDeliveryMode::Push
                        | pb::ClaimDeliveryMode::Pull
                        | pb::ClaimDeliveryMode::PushPull)
                )
            }),
            "invalid delivery mode",
        )?;
        text(r.doc.as_ref())?;
    }
    for t in &v.triggers {
        let c = claim(t.claim_id.as_ref())?;
        check(
            c.direction == pb::CommunicationDirection::Receive as i32,
            "trigger requires RECEIVE",
        )?;
        let m = message(t.message.as_ref())?;
        check(
            c.message.as_ref() != Some(m),
            "trigger cannot select received message",
        )?;
        text(t.condition.as_ref())?;
        text(t.doc.as_ref())?;
    }
    let mut pairs = HashSet::new();
    for s in &v.field_selections {
        let c = claim(s.claim_id.as_ref())?;
        let m = message(c.message.as_ref())?;
        let b = s
            .binding
            .as_ref()
            .ok_or_else(|| ClaimError("binding required".into()))?;
        let o = b
            .owner
            .as_ref()
            .ok_or_else(|| ClaimError("binding owner required".into()))?;
        check(
            o.kind == EntityKind::Specification as i32
                && o.id.as_ref() == m.specification.as_ref().and_then(|v| v.id.as_ref())
                && b.owner_uid == m.owner_uid,
            "binding must select same Specification incarnation",
        )?;
        local(b.id.as_ref())?;
        check(
            pairs.insert((local(s.claim_id.as_ref())?.value.clone(), b.encode_to_vec())),
            "duplicate claim binding selection",
        )?;
        check(
            s.selectors.len() <= MAX_SELECTORS,
            "field selector limit exceeded",
        )?;
        for selector in &s.selectors {
            match &selector.selector {
                Some(pb::claim_field_selector::Selector::Path(p)) => check(
                    !p.segments.is_empty()
                        && p.segments.len() <= 32
                        && p.segments.iter().all(|s| !s.is_empty()),
                    "invalid native field path",
                )?,
                Some(pb::claim_field_selector::Selector::Opaque(o)) => check(
                    !o.syntax.trim().is_empty() && !o.value.is_empty() && o.value.len() <= 1024,
                    "invalid opaque selector",
                )?,
                None => return Err(ClaimError("field selector required".into())),
            }
        }
        text(s.doc.as_ref())?;
    }
    Ok(())
}
/// Existing owners select only IDs returned by an earlier guarded write.
pub fn validate_local_selections(previous: Option<&Entity>, proposed: &Entity) -> Result<()> {
    validate_values(proposed)?;
    if let (Some(old), Some(v)) = (previous, declarations(proposed)) {
        let active: HashSet<_> = declarations(old)
            .into_iter()
            .flat_map(|v| &v.claims)
            .filter_map(|c| c.connection_id.as_ref())
            .collect();
        for id in v
            .routes
            .iter()
            .map(|v| v.claim_id.as_ref())
            .chain(v.triggers.iter().map(|v| v.claim_id.as_ref()))
            .chain(v.field_selections.iter().map(|v| v.claim_id.as_ref()))
        {
            check(
                id.is_some_and(|v| active.contains(v)),
                "stage new local claim before selecting returned ID",
            )?;
        }
    }
    Ok(())
}
pub fn resolve_connection_context(
    owner: &Entity,
    reference: &pb::ConnectionRef,
) -> Result<EntityRef> {
    validate_values(owner)?;
    let occurrence =
        connections::resolve_connection(owner, reference).map_err(|e| ClaimError(e.to_string()))?;
    check(
        occurrence.path.contains(".message_claims."),
        "not a claim occurrence",
    )?;
    occurrence
        .target
        .ok_or_else(|| ClaimError("claim endpoint unavailable".into()))
}
pub(crate) fn outbound(owner: &Entity, out: &mut Vec<OutboundRef>) {
    let Some(v) = declarations(owner) else { return };
    for (i, c) in v.claims.iter().enumerate() {
        push_message(
            c.message.as_ref(),
            format!("message_claims.claims[{i}].message.specification"),
            out,
        );
    }
    for (i, t) in v.triggers.iter().enumerate() {
        push_message(
            t.message.as_ref(),
            format!("message_claims.triggers[{i}].message.specification"),
            out,
        );
    }
    for (i, r) in v.routes.iter().enumerate() {
        if let Some(id) = r.channel.as_ref().and_then(|v| v.id.clone()) {
            out.push(OutboundRef {
                to_kind: Some(EntityKind::Channel),
                to_id: id,
                from_field: format!("message_claims.routes[{i}].channel"),
            });
        }
    }
    for (i, s) in v.field_selections.iter().enumerate() {
        if let Some(o) = s.binding.as_ref().and_then(|v| v.owner.as_ref()) {
            if let Some(id) = o.id.clone() {
                out.push(OutboundRef {
                    to_kind: EntityKind::try_from(o.kind).ok(),
                    to_id: id,
                    from_field: format!("message_claims.field_selections[{i}].binding.owner"),
                });
            }
        }
    }
}
fn push_message(m: Option<&pb::ContractMemberRef>, path: String, out: &mut Vec<OutboundRef>) {
    if let Some(id) = m
        .and_then(|v| v.specification.as_ref())
        .and_then(|v| v.id.clone())
    {
        out.push(OutboundRef {
            to_kind: Some(EntityKind::Specification),
            to_id: id,
            from_field: path,
        });
    }
}
/// Member and binding references require explicit reselection, never key-only retargeting.
pub(crate) fn retarget(
    owner: &mut Entity,
    kind: EntityKind,
    from: &pb::Id,
    to: &pb::Id,
    out: &mut Vec<String>,
) {
    if kind != EntityKind::Channel || from == to {
        return;
    }
    let Some(v) = declarations_mut(owner) else {
        return;
    };
    for (i, r) in v.routes.iter_mut().enumerate() {
        if r.channel.as_ref().and_then(|v| v.id.as_ref()) == Some(from) {
            r.channel = Some(pb::ChannelRef {
                id: Some(to.clone()),
            });
            r.connection_id = None;
            out.push(format!("message_claims.routes[{i}].channel"));
        }
    }
}

pub(crate) fn rewrite_local_selections(previous: &Entity, proposed: &mut Entity) {
    let Some(old) = declarations(previous) else { return };
    let Some(new) = declarations_mut(proposed) else { return };
    let changed: std::collections::HashMap<_, _> = old.claims.iter().zip(&new.claims).filter_map(|(a,b)| Some((a.connection_id.as_ref()?.value.clone(), b.connection_id.clone()?))).collect();
    for id in new.routes.iter_mut().map(|v|&mut v.claim_id).chain(new.triggers.iter_mut().map(|v|&mut v.claim_id)).chain(new.field_selections.iter_mut().map(|v|&mut v.claim_id)) {
        if let Some(replacement)=id.as_ref().and_then(|v|changed.get(&v.value)) { *id=Some(replacement.clone()); }
    }
}

pub fn validate_route_target(_route: &pb::ClaimRoute, _channel: &pb::Channel) -> Result<()> { Ok(()) }
pub fn validate_source_write(_previous: Option<&Entity>, _proposed: &Entity, _unit: crate::source_integration::OwnershipUnit) -> Result<()> { Ok(()) }
