use std::{
    collections::{HashMap, HashSet},
    fmt,
    str::FromStr,
};

use prost::Message;
use trogon_atlas_proto::{self as pb, entity, Entity, EntityKind};

use crate::{
    connections::{self, ConnectionError},
    entity_id, entity_kind, responsibility,
};

pub const MAX_SUBJECTS: usize = 1_024;
pub const MAX_ASSOCIATIONS: usize = 2_048;
pub const MAX_ENCODED_BYTES: usize = 1_048_576;
pub const MAX_DISCOVERY_ROWS: usize = 5_000;
pub const MAX_EXAMINED_OCCURRENCES: usize = 10_000;
pub const MAX_OUTPUT_RECORDS: usize = 4_096;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct CatalogError(pub String);

type Result<T> = std::result::Result<T, CatalogError>;

fn require(valid: bool, message: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(CatalogError(message.into()))
    }
}
fn failure(error: impl fmt::Display) -> CatalogError {
    CatalogError(error.to_string())
}
fn connection_error(error: CatalogError) -> ConnectionError {
    ConnectionError::Catalog(error.0)
}
fn exact(id: Option<&pb::Id>) -> Result<()> {
    let id = id.ok_or_else(|| CatalogError("exact entity ID required".into()))?;
    require(
        id.version > 0
            && crate::is_safe_id_component(&id.namespace)
            && crate::is_safe_id_component(&id.slug),
        "invalid exact entity ID",
    )
}
fn text(value: &str, limit: usize) -> Result<()> {
    require(
        !value.trim().is_empty() && value.len() <= limit,
        "catalog text blank or byte limit exceeded",
    )
}
pub fn validate_catalog_title(value: Option<&pb::CatalogTitle>) -> Result<()> {
    text(
        &value
            .ok_or_else(|| CatalogError("catalog title required".into()))?
            .value,
        256,
    )
}
pub fn validate_catalog_text(value: Option<&pb::CatalogText>) -> Result<()> {
    text(
        &value
            .ok_or_else(|| CatalogError("catalog text required".into()))?
            .value,
        16_384,
    )
}
fn digest(value: &pb::ContentDigest) -> Result<()> {
    require(
        value.sha256.len() == 64
            && value
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid SHA-256 digest",
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalValueId(uuid::Uuid);
impl LocalValueId {
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7())
    }
}
impl Default for LocalValueId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for LocalValueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl FromStr for LocalValueId {
    type Err = CatalogError;
    fn from_str(value: &str) -> Result<Self> {
        let parsed = uuid::Uuid::parse_str(value).map_err(failure)?;
        require(
            !parsed.is_nil() && parsed.to_string() == value,
            "local ID must be non-nil lowercase hyphenated UUID",
        )?;
        Ok(Self(parsed))
    }
}
impl From<LocalValueId> for pb::LocalValueId {
    fn from(value: LocalValueId) -> Self {
        Self {
            value: value.to_string(),
        }
    }
}

#[must_use]
pub fn as_responsibility_subject(subject: &pb::CatalogSubject) -> pb::ResponsibilitySubject {
    use pb::{catalog_subject::Selection as C, responsibility_subject::Selection as R};
    pb::ResponsibilitySubject {
        selection: subject.selection.as_ref().map(|value| match value {
            C::Entity(v) => R::Entity(v.clone()),
            C::Connection(v) => R::Connection(v.clone()),
            C::Member(v) => R::Member(v.clone()),
            C::Port(v) => R::Port(v.clone()),
        }),
    }
}
pub fn subject_owner(subject: &pb::CatalogSubject) -> Result<pb::EntityRef> {
    let owner =
        responsibility::subject_owner(&as_responsibility_subject(subject)).map_err(failure)?;
    require(
        !matches!(
            EntityKind::try_from(owner.kind),
            Ok(EntityKind::Decision | EntityKind::Document)
        ),
        "Decision and Document are excluded from catalog subject inventory",
    )?;
    Ok(owner)
}
pub fn validate_subject(subject: &pb::CatalogSubject) -> Result<()> {
    subject_owner(subject)?;
    responsibility::validate_subject(&as_responsibility_subject(subject)).map_err(failure)
}
pub fn resolve_subject(subject: &pb::CatalogSubject, owner: &Entity) -> Result<()> {
    validate_subject(subject)?;
    responsibility::resolve_subject(&as_responsibility_subject(subject), owner).map_err(failure)
}
pub fn document_owner(selection: &pb::DocumentSelection) -> Result<pb::EntityRef> {
    let reference = match &selection.selection {
        Some(pb::document_selection::Selection::Document(v)) => Some(v),
        Some(pb::document_selection::Selection::Anchor(v)) => v.document.as_ref(),
        None => None,
    }
    .ok_or_else(|| CatalogError("document selection required".into()))?;
    exact(reference.id.as_ref())?;
    Ok(pb::EntityRef {
        kind: EntityKind::Document as i32,
        id: reference.id.clone(),
    })
}
pub fn validate_document_selection(selection: &pb::DocumentSelection) -> Result<()> {
    document_owner(selection)?;
    if let Some(pb::document_selection::Selection::Anchor(v)) = &selection.selection {
        v.owner_uid.parse::<LocalValueId>()?;
        v.anchor_id
            .as_ref()
            .ok_or_else(|| CatalogError("anchor ID required".into()))?
            .value
            .parse::<LocalValueId>()?;
        if let Some(pin) = &v.expected_digest {
            digest(pin)?;
        }
    }
    Ok(())
}
pub fn resolve_document_selection<'a>(
    selection: &pb::DocumentSelection,
    owner: &'a Entity,
) -> Result<Option<&'a pb::DocumentAnchor>> {
    validate_document_selection(selection)?;
    let selected = document_owner(selection)?;
    require(
        entity_kind(owner) == Some(EntityKind::Document)
            && entity_id(owner) == selected.id.as_ref(),
        "document selection does not identify exact owner",
    )?;
    let Some(entity::Kind::Document(document)) = &owner.kind else {
        return Err(CatalogError("document required".into()));
    };
    let Some(pb::document_selection::Selection::Anchor(reference)) = &selection.selection else {
        return Ok(None);
    };
    require(
        owner
            .system
            .as_ref()
            .is_some_and(|s| s.uid == reference.owner_uid),
        "document incarnation mismatch",
    )?;
    validate_stored_values(owner)?;
    connections::validate_stored_connection_ids(owner).map_err(failure)?;
    let Some(pb::document::Body::Retained(body)) = &document.body else {
        return Err(CatalogError("anchor requires retained body".into()));
    };
    if let Some(pin) = &reference.expected_digest {
        digest(pin)?;
        require(
            pin.sha256 == crate::specification::content_digest(&body.content).sha256,
            "document byte pin mismatch",
        )?;
    }
    document
        .anchors
        .iter()
        .find(|a| a.anchor_id == reference.anchor_id)
        .map(Some)
        .ok_or_else(|| CatalogError("document anchor missing".into()))
}

#[must_use]
pub fn subjects(entity: &Entity) -> &[pb::SubjectAssociation] {
    match &entity.kind {
        Some(entity::Kind::Decision(v)) => &v.subjects,
        Some(entity::Kind::Document(v)) => &v.subjects,
        _ => &[],
    }
}
fn validate_subjects(entity: &Entity) -> Result<()> {
    let values = subjects(entity);
    require(
        !values.is_empty() && values.len() <= MAX_SUBJECTS,
        "subject collection outside bounds",
    )?;
    let mut seen = HashSet::new();
    for value in values {
        let subject = value
            .subject
            .as_ref()
            .ok_or_else(|| CatalogError("subject required".into()))?;
        validate_subject(subject)?;
        require(
            seen.insert(subject.encode_to_vec()),
            "duplicate full catalog subject",
        )?;
        if value.doc.is_some() {
            validate_catalog_text(value.doc.as_ref())?;
        }
    }
    Ok(())
}
fn validate_selection_list<'a>(
    values: impl Iterator<
        Item = (
            &'a Option<pb::DocumentSelection>,
            Option<&'a pb::CatalogText>,
        ),
    >,
    owner: Option<&pb::Id>,
    reject_self: bool,
) -> Result<()> {
    let mut seen = HashSet::new();
    for (selection, doc) in values {
        let selection = selection
            .as_ref()
            .ok_or_else(|| CatalogError("document selection required".into()))?;
        validate_document_selection(selection)?;
        if reject_self {
            require(
                document_owner(selection)?.id.as_ref() != owner,
                "self navigation forbidden",
            )?;
        }
        require(
            seen.insert(selection.encode_to_vec()),
            "duplicate full document selection",
        )?;
        if doc.is_some() {
            validate_catalog_text(doc)?;
        }
    }
    Ok(())
}
fn validate_decision(value: &pb::Decision) -> Result<()> {
    validate_catalog_title(value.title.as_ref())?;
    validate_catalog_text(value.context.as_ref())?;
    validate_catalog_text(value.rationale.as_ref())?;
    let status = pb::DecisionStatus::try_from(value.status).map_err(failure)?;
    require(
        status != pb::DecisionStatus::Unspecified,
        "decision status required",
    )?;
    let adopted = matches!(
        status,
        pb::DecisionStatus::Accepted | pb::DecisionStatus::Superseded
    );
    require(
        value.selected_option.is_some() == adopted,
        "selected option required exactly for adopted statuses",
    )?;
    require(
        value.replaced_by.is_some() == (status == pb::DecisionStatus::Superseded),
        "replacement required exactly for superseded status",
    )?;
    require(
        !value.options.is_empty() && value.options.len() <= 64,
        "option collection outside bounds",
    )?;
    for option in &value.options {
        validate_catalog_title(option.title.as_ref())?;
        validate_catalog_text(option.description.as_ref())?;
    }
    if let Some(selected) = &value.selected_option {
        selected.value.parse::<LocalValueId>()?;
        require(
            value
                .options
                .iter()
                .any(|o| o.option_id.as_ref() == Some(selected)),
            "selected option missing",
        )?;
    }
    require(
        value.consequences.len() <= 64 && (!adopted || !value.consequences.is_empty()),
        "consequence collection outside bounds",
    )?;
    let mut seen = HashSet::new();
    for consequence in &value.consequences {
        require(
            matches!(
                pb::decision_consequence::Kind::try_from(consequence.kind),
                Ok(pb::decision_consequence::Kind::Benefit
                    | pb::decision_consequence::Kind::Cost
                    | pb::decision_consequence::Kind::Risk)
            ),
            "unknown consequence kind",
        )?;
        validate_catalog_text(consequence.text.as_ref())?;
        require(
            seen.insert(consequence.encode_to_vec()),
            "duplicate consequence",
        )?;
    }
    if let Some(replacement) = &value.replaced_by {
        let target = replacement
            .decision
            .as_ref()
            .ok_or_else(|| CatalogError("replacement decision required".into()))?;
        exact(target.id.as_ref())?;
        require(target.id != value.id, "self replacement forbidden")?;
    }
    require(
        value.related_decisions.len() <= 128 && value.documents.len() <= 128,
        "decision relationship limit exceeded",
    )?;
    seen.clear();
    for related in &value.related_decisions {
        let target = related
            .decision
            .as_ref()
            .ok_or_else(|| CatalogError("related decision required".into()))?;
        exact(target.id.as_ref())?;
        require(
            target.id != value.id && seen.insert(target.encode_to_vec()),
            "self or duplicate related decision",
        )?;
        if related.doc.is_some() {
            validate_catalog_text(related.doc.as_ref())?;
        }
    }
    validate_selection_list(
        value
            .documents
            .iter()
            .map(|v| (&v.document, v.doc.as_ref())),
        None,
        false,
    )?;
    require(
        value.subjects.len()
            + value.related_decisions.len()
            + value.documents.len()
            + usize::from(value.replaced_by.is_some())
            <= MAX_ASSOCIATIONS,
        "association limit exceeded",
    )
}
fn validate_document(value: &pb::Document) -> Result<()> {
    validate_catalog_title(value.title.as_ref())?;
    match &value.body {
        Some(pb::document::Body::Retained(body)) => {
            require(
                matches!(
                    body.media_type.as_ref().map(|m| m.value.as_str()),
                    Some("text/plain" | "text/markdown")
                ),
                "unsupported document media type",
            )?;
            require(
                !body.content.is_empty() && body.content.len() <= 262_144,
                "retained text outside byte bounds",
            )?;
            let supplied = body
                .digest
                .as_ref()
                .ok_or_else(|| CatalogError("document digest required".into()))?;
            digest(supplied)?;
            require(
                supplied.sha256 == crate::specification::content_digest(&body.content).sha256,
                "document digest does not match exact bytes",
            )?;
        }
        Some(pb::document::Body::Link(link)) => {
            require(
                value.anchors.is_empty(),
                "link document cannot declare anchors",
            )?;
            let contact = pb::ContactPoint {
                channel: Some(pb::contact_point::Channel::Web(pb::ContactUrl {
                    value: link.value.clone(),
                })),
                label: None,
            };
            responsibility::validate_contact(&contact).map_err(failure)?;
        }
        None => return Err(CatalogError("document body required".into())),
    }
    require(
        value.anchors.len() <= 256 && value.navigation.len() <= 128,
        "document collection limit exceeded",
    )?;
    let mut seen = HashSet::new();
    for anchor in &value.anchors {
        validate_catalog_title(anchor.label.as_ref())?;
        let fragment = &anchor
            .fragment
            .as_ref()
            .ok_or_else(|| CatalogError("anchor fragment required".into()))?
            .value;
        require(
            fragment.len() <= 128
                && fragment
                    .bytes()
                    .next()
                    .is_some_and(|b| b.is_ascii_alphabetic())
                && fragment
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b)),
            "invalid anchor fragment",
        )?;
        require(seen.insert(fragment), "duplicate anchor fragment")?;
    }
    validate_selection_list(
        value.navigation.iter().map(|v| (&v.document, None)),
        value.id.as_ref(),
        true,
    )?;
    for nav in &value.navigation {
        if nav.label.is_some() {
            validate_catalog_title(nav.label.as_ref())?;
        }
    }
    require(
        value.subjects.len() + value.navigation.len() <= MAX_ASSOCIATIONS,
        "association limit exceeded",
    )
}
pub fn validate_values(entity: &Entity) -> Result<()> {
    match &entity.kind {
        Some(entity::Kind::Decision(value)) => validate_decision(value)?,
        Some(entity::Kind::Document(value)) => validate_document(value)?,
        _ => return Ok(()),
    }
    exact(entity_id(entity))?;
    require(
        entity.encoded_len() <= MAX_ENCODED_BYTES,
        "catalog entity byte limit exceeded",
    )?;
    validate_subjects(entity)?;
    inventory(entity, false)?;
    connections::validate_connection_ids(entity).map_err(failure)
}

fn each_local(
    entity: &mut Entity,
    mut visit: impl FnMut(&mut Option<pb::LocalValueId>, &[u8]) -> Result<()>,
) -> Result<()> {
    match &mut entity.kind {
        Some(entity::Kind::Decision(value)) => {
            for option in &mut value.options {
                visit(&mut option.option_id, b"option")?;
            }
        }
        Some(entity::Kind::Document(value)) => {
            for anchor in &mut value.anchors {
                let mut fingerprint = b"anchor:".to_vec();
                fingerprint.extend(
                    anchor
                        .fragment
                        .as_ref()
                        .map_or(&[][..], |f| f.value.as_bytes()),
                );
                visit(&mut anchor.anchor_id, &fingerprint)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn inventory(entity: &Entity, persisted: bool) -> Result<HashMap<LocalValueId, Vec<u8>>> {
    let mut out = HashMap::new();
    each_local(&mut entity.clone(), |id, signature| {
        if let Some(value) = id {
            let id = value.value.parse::<LocalValueId>()?;
            require(
                out.insert(id, signature.to_vec()).is_none(),
                "duplicate local value ID",
            )?;
        } else {
            require(!persisted, "persisted local ID missing")?;
        }
        Ok(())
    })?;
    Ok(out)
}
pub fn validate_stored_values(entity: &Entity) -> Result<()> {
    inventory(entity, true)?;
    if let Some(entity::Kind::Document(value)) = &entity.kind {
        let mut seen = HashSet::new();
        require(
            value.anchors.len() <= 256,
            "anchor inventory limit exceeded",
        )?;
        for anchor in &value.anchors {
            let fragment = &anchor
                .fragment
                .as_ref()
                .ok_or_else(|| CatalogError("anchor fragment required".into()))?
                .value;
            require(
                fragment.len() <= 128
                    && fragment
                        .bytes()
                        .next()
                        .is_some_and(|b| b.is_ascii_alphabetic())
                    && fragment
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b)),
                "invalid anchor fragment",
            )?;
            require(seen.insert(fragment), "duplicate anchor fragment")?;
        }
    }
    Ok(())
}

pub(crate) fn prepare_values(
    previous: Option<&Entity>,
    proposed: &mut Entity,
    trusted: bool,
) -> std::result::Result<(), ConnectionError> {
    let old = previous
        .map(|e| inventory(e, true))
        .transpose()
        .map_err(connection_error)?
        .unwrap_or_default();
    inventory(proposed, false).map_err(connection_error)?;
    let stripped = match (previous.and_then(|e| e.kind.as_ref()), &proposed.kind) {
        (Some(entity::Kind::Decision(prior)), Some(entity::Kind::Decision(value))) => {
            value.options.iter().any(|o| {
                o.option_id.is_none()
                    && prior.options.iter().any(|old| {
                        old.title == o.title
                            && old.description == o.description
                            && !value
                                .options
                                .iter()
                                .any(|current| current.option_id == old.option_id)
                    })
            })
        }
        (Some(entity::Kind::Document(prior)), Some(entity::Kind::Document(value))) => {
            value.anchors.iter().any(|a| {
                a.anchor_id.is_none() && prior.anchors.iter().any(|old| old.fragment == a.fragment)
            })
        }
        _ => false,
    };
    require(!stripped, "edited manifest cannot strip local identities")
        .map_err(connection_error)?;
    each_local(proposed, |value, signature| {
        if let Some(value) = value {
            let id = value.value.parse::<LocalValueId>()?;
            require(
                previous.is_none() || trusted || old.contains_key(&id),
                "local ID not active; omit IDs for additions",
            )?;
            require(
                old.get(&id).is_none_or(|old| old == signature),
                "local value fingerprint changed; use a new ID",
            )?;
        } else {
            *value = Some(LocalValueId::new().into());
        }
        Ok(())
    })
    .map_err(connection_error)
}
pub fn equivalent_manifest(
    previous: &Entity,
    proposed: &Entity,
) -> std::result::Result<bool, ConnectionError> {
    if !matches!(
        proposed.kind,
        Some(entity::Kind::Decision(_) | entity::Kind::Document(_))
    ) {
        return Ok(false);
    }
    validate_stored_values(previous).map_err(connection_error)?;
    connections::validate_stored_connection_ids(previous)?;
    let supplied = inventory(proposed, false).map_err(connection_error)?;
    let mut comparison = previous.clone();
    if supplied.is_empty() {
        each_local(&mut comparison, |id, _| {
            *id = None;
            Ok(())
        })
        .map_err(connection_error)?;
    }
    if crate::semantically_equal(&comparison, proposed) {
        return Ok(true);
    }
    connections::equivalent_with_unassigned_ids(&comparison, proposed)
}
pub(crate) fn removed_values(
    base: Option<&Entity>,
    opposing: Option<&Entity>,
    selected: &Entity,
) -> std::result::Result<HashSet<LocalValueId>, ConnectionError> {
    for owner in base
        .into_iter()
        .chain(opposing)
        .chain(std::iter::once(selected))
    {
        validate_stored_values(owner).map_err(connection_error)?;
    }
    let same = |e: &&Entity| {
        e.system
            .as_ref()
            .zip(selected.system.as_ref())
            .is_some_and(|(a, b)| !a.uid.is_empty() && a.uid == b.uid)
    };
    let (Some(base), Some(opposing)) = (base.filter(same), opposing.filter(same)) else {
        return Ok(HashSet::new());
    };
    let current = inventory(opposing, true).map_err(connection_error)?;
    Ok(inventory(base, true)
        .map_err(connection_error)?
        .into_keys()
        .filter(|id| !current.contains_key(id))
        .collect())
}
pub(crate) fn reject_removed(
    entity: &Entity,
    removed: &HashSet<LocalValueId>,
) -> std::result::Result<(), ConnectionError> {
    require(
        !inventory(entity, true)
            .map_err(connection_error)?
            .keys()
            .any(|id| removed.contains(id)),
        "local ID from branch base removed on opposing side",
    )
    .map_err(connection_error)
}
pub(crate) fn restore_values(
    entity: &mut Entity,
    removed: &HashSet<LocalValueId>,
) -> std::result::Result<(), ConnectionError> {
    let mut map = HashMap::new();
    each_local(entity, |value, _| {
        let id = value
            .as_ref()
            .ok_or_else(|| CatalogError("local ID missing".into()))?
            .value
            .parse::<LocalValueId>()?;
        if removed.contains(&id) {
            let fresh: pb::LocalValueId = LocalValueId::new().into();
            map.insert(id, fresh.clone());
            *value = Some(fresh);
        }
        Ok(())
    })
    .map_err(connection_error)?;
    if let Some(entity::Kind::Decision(value)) = &mut entity.kind {
        if let Some(selected) = &mut value.selected_option {
            if let Some(fresh) = map.get(
                &selected
                    .value
                    .parse::<LocalValueId>()
                    .map_err(connection_error)?,
            ) {
                *selected = fresh.clone();
            }
        }
    }
    Ok(())
}

pub fn decorate_reference(entity: &Entity, reference: &mut pb::Reference) {
    crate::decision_document_refs::decorate_reference(entity, reference);
}
