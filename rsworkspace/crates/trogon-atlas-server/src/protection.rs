//! Entity protection: the request annotation a write may carry, and who may
//! act on it. The registry itself lives in the store; the refusal status is
//! `crate::conv::entity_protected`.

use trogon_atlas_proto as pb;

use crate::{auth::Principal, validation::entity_metadata_mut};

const ANNOTATION_TYPE: &str = "trogonatlas.annotation.v1alpha1.ProtectionAnnotation";

/// A write asked for its entity's identity to be protected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionRequest {
    pub reason: String,
}

/// Remove every `ProtectionAnnotation` from the entity's metadata, so the
/// annotation is never stored, and return the request it carried. A payload
/// that does not decode is still removed and counts as a request with no
/// reason.
pub fn take_request(entity: &mut pb::Entity) -> Option<ProtectionRequest> {
    let metadata = entity_metadata_mut(entity)?;
    let mut request = None;
    metadata.retain(|any| {
        if !is_protection_annotation(any) {
            return true;
        }
        if request.is_none() {
            let reason = <pb::ProtectionAnnotation as prost::Message>::decode(any.value.as_slice())
                .map(|ann| ann.reason)
                .unwrap_or_default();
            request = Some(ProtectionRequest { reason });
        }
        false
    });
    request
}

fn is_protection_annotation(any: &prost_types::Any) -> bool {
    any.type_url
        .rsplit('/')
        .next()
        .is_some_and(|name| name == ANNOTATION_TYPE)
}

/// Protection is set and lifted only by a genuinely authenticated admin: the
/// synthetic admin `--insecure-allow-anonymous` inserts does not count, and
/// neither does a server with no auth stack, because neither has an identity
/// to attribute the change to.
pub fn is_authenticated_admin(principal: Option<&Principal>) -> bool {
    principal.is_some_and(|p| p.role == crate::auth::Role::Admin && !p.is_anonymous)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn annotated(reason: &str) -> pb::Entity {
        pb::Entity {
            kind: Some(pb::entity::Kind::Event(pb::Event {
                metadata: vec![
                    prost_types::Any {
                        type_url:
                            "type.googleapis.com/trogonatlas.annotation.v1alpha1.TagAnnotation"
                                .into(),
                        value: Vec::new(),
                    },
                    prost_types::Any {
                        type_url: format!("type.googleapis.com/{ANNOTATION_TYPE}"),
                        value: prost::Message::encode_to_vec(&pb::ProtectionAnnotation {
                            reason: reason.into(),
                        }),
                    },
                ],
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn metadata_len(entity: &pb::Entity) -> usize {
        match entity.kind.as_ref() {
            Some(pb::entity::Kind::Event(e)) => e.metadata.len(),
            _ => 0,
        }
    }

    #[test]
    fn take_request_strips_the_annotation_and_keeps_the_rest() {
        let mut entity = annotated("published contract");
        assert_eq!(
            take_request(&mut entity),
            Some(ProtectionRequest {
                reason: "published contract".into()
            })
        );
        assert_eq!(metadata_len(&entity), 1);
        assert_eq!(take_request(&mut entity), None);
    }

    #[test]
    fn take_request_strips_an_undecodable_annotation() {
        let mut entity = annotated("x");
        if let Some(pb::entity::Kind::Event(e)) = entity.kind.as_mut() {
            e.metadata[1].value = vec![0xff, 0xff];
        }
        assert_eq!(
            take_request(&mut entity),
            Some(ProtectionRequest {
                reason: String::new()
            })
        );
        assert_eq!(metadata_len(&entity), 1);
    }

    #[test]
    fn a_suffix_match_is_not_the_annotation() {
        let mut entity = annotated("x");
        if let Some(pb::entity::Kind::Event(e)) = entity.kind.as_mut() {
            e.metadata[1].type_url = format!("type.googleapis.com/evil.{ANNOTATION_TYPE}");
        }
        assert_eq!(take_request(&mut entity), None);
    }
}
