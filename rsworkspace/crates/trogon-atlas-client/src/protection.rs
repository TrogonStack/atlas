//! Entity protection (see `docs/explanation/entity-protection.md`): an admin
//! freezes an identity so every route refuses to change it, and anyone who
//! can read sees which identities are frozen. Reads keep the server's three
//! answers apart (protected, unprotected, unknown) and keep a listing the
//! server could not finish marked as partial, so a caller never reads a
//! missing row as proof that an identity is free to change.

use std::{fmt, num::NonZeroU64, str::FromStr};

use anyhow::{Context as _, Result};
use serde_json::{json, Value};
use trogon_atlas_proto as pb;

use crate::{client::Client, failure::invalid_input};

/// The unit protection applies to: every version of `(kind, namespace,
/// slug)`, including versions nobody has written yet. Written as
/// `<kind>/<namespace>/<slug>`, the form refusals name it by.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProtectedIdentity {
    kind: pb::EntityKind,
    namespace: String,
    slug: String,
}

impl ProtectedIdentity {
    pub fn new(kind: pb::EntityKind, namespace: &str, slug: &str) -> Result<Self> {
        if kind == pb::EntityKind::Unspecified {
            crate::invalid_input!("entity kind must be set");
        }
        for (what, value) in [("namespace", namespace), ("slug", slug)] {
            trogon_atlas_core::validate_id_component(value)
                .map_err(|e| invalid_input(format!("{what} {e}")))?;
        }
        Ok(Self {
            kind,
            namespace: namespace.to_string(),
            slug: slug.to_string(),
        })
    }

    /// Parse `<kind>/<namespace>/<slug>` (or `<kind>:<namespace>/<slug>`).
    /// A trailing `@version` is refused: protection covers every version, so
    /// accepting one would suggest the others stay writable.
    pub fn parse(s: &str) -> Result<Self> {
        let Some((kind, namespace, slug)) = s
            .split_once([':', '/'])
            .and_then(|(kind, rest)| rest.split_once('/').map(|(ns, slug)| (kind, ns, slug)))
        else {
            crate::invalid_input!("expected <kind>/<namespace>/<slug>, got {s:?}");
        };
        if slug.contains('@') {
            crate::invalid_input!(
                "protection covers every version of an identity; drop the @version from {s:?}"
            );
        }
        let kind = pb::canonical::parse_kind(kind)
            .ok_or_else(|| invalid_input(format!("unknown entity kind: {kind}")))?;
        Self::new(kind, namespace, slug)
    }

    fn from_wire(kind: i32, namespace: &str, slug: &str) -> Self {
        Self {
            kind: pb::EntityKind::try_from(kind).unwrap_or(pb::EntityKind::Unspecified),
            namespace: namespace.to_string(),
            slug: slug.to_string(),
        }
    }

    #[must_use]
    pub fn kind(&self) -> pb::EntityKind {
        self.kind
    }

    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    #[must_use]
    pub fn slug(&self) -> &str {
        &self.slug
    }

    fn fields_json(&self) -> serde_json::Map<String, Value> {
        serde_json::Map::from_iter([
            ("identity".into(), json!(self.to_string())),
            ("kind".into(), json!(pb::canonical::kind_short(self.kind))),
            ("namespace".into(), json!(self.namespace)),
            ("slug".into(), json!(self.slug)),
        ])
    }
}

impl fmt::Display for ProtectedIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{}",
            pb::canonical::kind_short(self.kind),
            self.namespace,
            self.slug
        )
    }
}

impl FromStr for ProtectedIdentity {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

/// Namespace a listing is restricted to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceFilter(String);

impl NamespaceFilter {
    pub fn new(namespace: &str) -> Result<Self> {
        trogon_atlas_core::validate_id_component(namespace)
            .map_err(|e| invalid_input(format!("namespace {e}")))?;
        Ok(Self(namespace.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why an identity is frozen. Required and never blank, because it is what
/// a refused writer is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionReason(String);

impl ProtectionReason {
    pub fn new(reason: &str) -> Result<Self> {
        let reason = reason.trim();
        if reason.is_empty() {
            crate::invalid_input!("a protection reason is required");
        }
        Ok(Self(reason.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ProtectionReason {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::new(s)
    }
}

/// Revision of a stored protection row, as `revision` reports it. A lift
/// guarded by one is refused with `STALE_PRECONDITION` once the row moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowRevision(NonZeroU64);

impl RowRevision {
    #[must_use]
    pub fn new(revision: u64) -> Option<Self> {
        NonZeroU64::new(revision).map(Self)
    }

    #[must_use]
    pub fn get(self) -> u64 {
        self.0.get()
    }
}

impl FromStr for RowRevision {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        s.parse::<u64>()
            .ok()
            .and_then(Self::new)
            .ok_or_else(|| invalid_input(format!("revision must be a positive integer, got {s:?}")))
    }
}

impl fmt::Display for RowRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Whether a call should change anything, or only check that it could.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Effect {
    #[default]
    Apply,
    ValidateOnly,
}

impl Effect {
    #[must_use]
    pub fn from_validate_only(validate_only: bool) -> Self {
        if validate_only {
            Self::ValidateOnly
        } else {
            Self::Apply
        }
    }

    #[must_use]
    pub fn is_validate_only(self) -> bool {
        self == Self::ValidateOnly
    }
}

/// What `ProtectEntity` answered.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtectOutcome {
    pub identity: ProtectedIdentity,
    pub effect: Effect,
    /// False when the identity was already protected; `protection` is then
    /// the existing row, whose reason may differ from the one requested.
    pub changed: bool,
    pub protection: Option<pb::EntityProtection>,
}

impl ProtectOutcome {
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut fields = self.identity.fields_json();
        fields.insert("changed".into(), json!(self.changed));
        fields.insert(
            "validate_only".into(),
            json!(self.effect.is_validate_only()),
        );
        fields.insert(
            "protection".into(),
            self.protection
                .as_ref()
                .map_or(Value::Null, protection_json),
        );
        Value::Object(fields)
    }

    #[must_use]
    pub fn render(&self) -> String {
        let row = self
            .protection
            .as_ref()
            .map(|p| format!(" ({}) by {}", p.reason, p.protected_by))
            .unwrap_or_default();
        match (self.effect, self.changed) {
            (Effect::ValidateOnly, true) => format!("{} would be protected{row}", self.identity),
            (_, false) => format!("{} already protected{row}", self.identity),
            (Effect::Apply, true) => {
                let revision = self
                    .protection
                    .as_ref()
                    .map(|p| format!(", revision {}", p.revision))
                    .unwrap_or_default();
                format!("{} protected{row}{revision}", self.identity)
            }
        }
    }
}

/// What `UnprotectEntity` answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnprotectOutcome {
    pub identity: ProtectedIdentity,
    pub effect: Effect,
    /// False when the identity was not protected.
    pub changed: bool,
    /// Changeset that recorded the lift; `None` when nothing changed.
    pub changeset_id: Option<String>,
}

impl UnprotectOutcome {
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut fields = self.identity.fields_json();
        fields.insert("changed".into(), json!(self.changed));
        fields.insert(
            "validate_only".into(),
            json!(self.effect.is_validate_only()),
        );
        fields.insert("changeset_id".into(), json!(self.changeset_id));
        Value::Object(fields)
    }

    #[must_use]
    pub fn render(&self) -> String {
        match (self.effect, self.changed) {
            (_, false) => format!("{} was not protected", self.identity),
            (Effect::ValidateOnly, true) => format!("{} would be unprotected", self.identity),
            (Effect::Apply, true) => match &self.changeset_id {
                Some(id) => format!("{} unprotected in changeset {id}", self.identity),
                None => format!("{} unprotected", self.identity),
            },
        }
    }
}

/// One identity's protection state. `Unknown` is an answer, not an error:
/// writes treat it as protected, and a reader must not treat it as free.
#[derive(Debug, Clone, PartialEq)]
pub enum ProtectionLookup {
    Unprotected(ProtectedIdentity),
    Protected(pb::EntityProtection),
    Unknown(pb::EntityProtectionUnknown),
}

impl ProtectionLookup {
    fn of(identity: &ProtectedIdentity, resp: pb::GetEntityProtectionResponse) -> Self {
        let unknown = |cause: &str| {
            Self::Unknown(pb::EntityProtectionUnknown {
                kind: identity.kind as i32,
                namespace: identity.namespace.clone(),
                slug: identity.slug.clone(),
                cause: cause.to_string(),
                revision: 0,
            })
        };
        match pb::ProtectionState::try_from(resp.state) {
            Ok(pb::ProtectionState::Unprotected) => Self::Unprotected(identity.clone()),
            Ok(pb::ProtectionState::Protected) => match resp.protection {
                Some(protection) => Self::Protected(protection),
                None => unknown("the server reported the identity protected without its row"),
            },
            Ok(pb::ProtectionState::Unknown) => match resp.unknown {
                Some(row) => Self::Unknown(row),
                None => unknown("the server reported the state unknown without a cause"),
            },
            Ok(pb::ProtectionState::Unspecified) | Err(_) => {
                unknown("the server did not report a protection state")
            }
        }
    }

    #[must_use]
    pub fn state_label(&self) -> &'static str {
        match self {
            Self::Unprotected(_) => "unprotected",
            Self::Protected(_) => "protected",
            Self::Unknown(_) => "unknown",
        }
    }

    #[must_use]
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        let (identity, protection, unknown) = match self {
            Self::Unprotected(identity) => (identity.clone(), Value::Null, Value::Null),
            Self::Protected(p) => (
                ProtectedIdentity::from_wire(p.kind, &p.namespace, &p.slug),
                protection_json(p),
                Value::Null,
            ),
            Self::Unknown(u) => (
                ProtectedIdentity::from_wire(u.kind, &u.namespace, &u.slug),
                Value::Null,
                unknown_json(u),
            ),
        };
        let mut fields = identity.fields_json();
        fields.insert("state".into(), json!(self.state_label()));
        fields.insert("protection".into(), protection);
        fields.insert("unknown".into(), unknown);
        Value::Object(fields)
    }

    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Unprotected(identity) => format!("{identity}: unprotected"),
            Self::Protected(p) => format!("{}: protected {}", wire_identity(p), protection_line(p)),
            Self::Unknown(u) => format!("{}: unknown {}", unknown_identity(u), unknown_line(u)),
        }
    }
}

/// Whether a listing can be read as the whole registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListingVerdict {
    /// Every row was enumerated and read.
    Complete,
    /// Some rows could not be read, or the registry could not be enumerated
    /// to the end; an identity missing from the listing may still be
    /// protected.
    Partial,
}

impl ListingVerdict {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
        }
    }
}

/// What `ListEntityProtections` answered, with the server's `complete` flag
/// kept verbatim next to the verdict a caller acts on.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtectionListing {
    pub namespace: Option<String>,
    pub protections: Vec<pb::EntityProtection>,
    pub unknown: Vec<pb::EntityProtectionUnknown>,
    pub complete: bool,
}

impl ProtectionListing {
    #[must_use]
    pub fn verdict(&self) -> ListingVerdict {
        if self.complete && self.unknown.is_empty() {
            ListingVerdict::Complete
        } else {
            ListingVerdict::Partial
        }
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "namespace": self.namespace,
            "verdict": self.verdict().as_str(),
            "complete": self.complete,
            "protections": self.protections.iter().map(protection_json).collect::<Vec<_>>(),
            "unknown": self.unknown.iter().map(unknown_json).collect::<Vec<_>>(),
        })
    }

    #[must_use]
    pub fn render(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        if self.protections.is_empty() && self.unknown.is_empty() {
            out.push_str("no protected identities\n");
        }
        for p in &self.protections {
            let _ = writeln!(
                out,
                "{}\tprotected {}",
                wire_identity(p),
                protection_line(p)
            );
        }
        for u in &self.unknown {
            let _ = writeln!(out, "{}\tunknown {}", unknown_identity(u), unknown_line(u));
        }
        if !self.complete {
            out.push_str(
                "incomplete: the registry could not be enumerated to the end; an identity missing here may still be protected\n",
            );
        }
        if !self.unknown.is_empty() {
            out.push_str(
                "incomplete: some protection rows could not be read; writes to those identities are refused until an admin lifts them\n",
            );
        }
        out
    }
}

fn wire_identity(p: &pb::EntityProtection) -> ProtectedIdentity {
    ProtectedIdentity::from_wire(p.kind, &p.namespace, &p.slug)
}

fn unknown_identity(u: &pb::EntityProtectionUnknown) -> ProtectedIdentity {
    ProtectedIdentity::from_wire(u.kind, &u.namespace, &u.slug)
}

fn protection_line(p: &pb::EntityProtection) -> String {
    format!(
        "({}) by {} at {}, revision {}, changeset {}",
        p.reason, p.protected_by, p.protected_at, p.revision, p.changeset_id
    )
}

fn unknown_line(u: &pb::EntityProtectionUnknown) -> String {
    match RowRevision::new(u.revision) {
        Some(revision) => format!("({}), unreadable row at revision {revision}", u.cause),
        None => format!("({}), registry did not answer", u.cause),
    }
}

fn protection_json(p: &pb::EntityProtection) -> Value {
    let mut fields = wire_identity(p).fields_json();
    fields.insert("reason".into(), json!(p.reason));
    fields.insert("protected_by".into(), json!(p.protected_by));
    fields.insert("protected_at".into(), json!(p.protected_at));
    fields.insert("changeset_id".into(), json!(p.changeset_id));
    fields.insert("revision".into(), json!(p.revision));
    Value::Object(fields)
}

/// `revision` is `null` when the registry itself did not answer: there is
/// no row to lift, so `liftable` is false and retrying is the only remedy.
fn unknown_json(u: &pb::EntityProtectionUnknown) -> Value {
    let revision = RowRevision::new(u.revision);
    let mut fields = unknown_identity(u).fields_json();
    fields.insert("cause".into(), json!(u.cause));
    fields.insert("revision".into(), json!(revision.map(RowRevision::get)));
    fields.insert("liftable".into(), json!(revision.is_some()));
    Value::Object(fields)
}

pub async fn protect(
    client: &mut Client,
    identity: &ProtectedIdentity,
    reason: &ProtectionReason,
    effect: Effect,
) -> Result<ProtectOutcome> {
    let resp = client
        .protect_entity(pb::ProtectEntityRequest {
            kind: identity.kind as i32,
            namespace: identity.namespace.clone(),
            slug: identity.slug.clone(),
            reason: reason.as_str().to_string(),
            validate_only: effect.is_validate_only(),
        })
        .await
        .context("ProtectEntity failed")?
        .into_inner();
    Ok(ProtectOutcome {
        identity: identity.clone(),
        effect,
        changed: resp.changed,
        protection: resp.protection,
    })
}

/// Lift protection. With `if_match`, only the row at that revision is
/// lifted, which is also how an admin removes an unreadable row.
pub async fn unprotect(
    client: &mut Client,
    identity: &ProtectedIdentity,
    if_match: Option<RowRevision>,
    effect: Effect,
) -> Result<UnprotectOutcome> {
    let resp = client
        .unprotect_entity(pb::UnprotectEntityRequest {
            kind: identity.kind as i32,
            namespace: identity.namespace.clone(),
            slug: identity.slug.clone(),
            validate_only: effect.is_validate_only(),
            if_match: if_match.map_or(0, RowRevision::get),
        })
        .await
        .context("UnprotectEntity failed")?
        .into_inner();
    Ok(UnprotectOutcome {
        identity: identity.clone(),
        effect,
        changed: resp.changed,
        changeset_id: Some(resp.changeset_id).filter(|id| !id.is_empty()),
    })
}

pub async fn get_protection(
    client: &mut Client,
    identity: &ProtectedIdentity,
) -> Result<ProtectionLookup> {
    let resp = client
        .get_entity_protection(pb::GetEntityProtectionRequest {
            kind: identity.kind as i32,
            namespace: identity.namespace.clone(),
            slug: identity.slug.clone(),
        })
        .await
        .context("GetEntityProtection failed")?
        .into_inner();
    Ok(ProtectionLookup::of(identity, resp))
}

/// List protections, restricted to `namespace` when given.
pub async fn list_protections(
    client: &mut Client,
    namespace: Option<&NamespaceFilter>,
) -> Result<ProtectionListing> {
    let resp = client
        .list_entity_protections(pb::ListEntityProtectionsRequest {
            namespace: namespace
                .map(NamespaceFilter::as_str)
                .unwrap_or_default()
                .to_string(),
        })
        .await
        .context("ListEntityProtections failed")?
        .into_inner();
    Ok(ProtectionListing {
        namespace: namespace.map(|ns| ns.as_str().to_string()),
        protections: resp.protections,
        unknown: resp.unknown,
        complete: resp.complete,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn identity() -> ProtectedIdentity {
        ProtectedIdentity::parse("event/orders/placed").unwrap()
    }

    fn row(revision: u64) -> pb::EntityProtectionUnknown {
        pb::EntityProtectionUnknown {
            kind: pb::EntityKind::Event as i32,
            namespace: "orders".into(),
            slug: "placed".into(),
            cause: "row does not decode".into(),
            revision,
        }
    }

    #[test]
    fn identity_parses_both_address_forms_and_prints_the_refusal_form() {
        assert_eq!(identity().to_string(), "event/orders/placed");
        assert_eq!(
            ProtectedIdentity::parse("event:orders/placed").unwrap(),
            identity()
        );
    }

    #[test]
    fn identity_refuses_a_version_unknown_kind_and_bad_components() {
        for bad in [
            "event/orders/placed@1",
            "nope/orders/placed",
            "event/orders",
            "event/../placed",
            "event/orders/pla ced",
        ] {
            let err = ProtectedIdentity::parse(bad).unwrap_err();
            assert!(
                crate::failure::Failure::classify(&err).category
                    == crate::failure::FailureCategory::Validation,
                "{bad}: {err:#}"
            );
        }
    }

    #[test]
    fn reason_and_revision_reject_blank_and_zero() {
        assert!(ProtectionReason::new("  ").is_err());
        assert_eq!(
            ProtectionReason::new(" published ").unwrap().as_str(),
            "published"
        );
        assert!("0".parse::<RowRevision>().is_err());
        assert!("x".parse::<RowRevision>().is_err());
        assert_eq!("7".parse::<RowRevision>().unwrap().get(), 7);
    }

    #[test]
    fn a_lookup_without_a_state_reads_as_unknown_not_unprotected() {
        let lookup = ProtectionLookup::of(&identity(), pb::GetEntityProtectionResponse::default());
        assert!(lookup.is_unknown(), "{lookup:?}");
        let protected_without_row = ProtectionLookup::of(
            &identity(),
            pb::GetEntityProtectionResponse {
                state: pb::ProtectionState::Protected as i32,
                ..Default::default()
            },
        );
        assert!(protected_without_row.is_unknown());
        assert_eq!(lookup.to_json()["state"], "unknown");
        assert_eq!(lookup.to_json()["unknown"]["liftable"], false);
    }

    #[test]
    fn a_listing_is_partial_when_incomplete_or_holding_unreadable_rows() {
        let complete = ProtectionListing {
            namespace: None,
            protections: Vec::new(),
            unknown: Vec::new(),
            complete: true,
        };
        assert_eq!(complete.verdict(), ListingVerdict::Complete);
        let unreadable = ProtectionListing {
            unknown: vec![row(9)],
            ..complete.clone()
        };
        assert_eq!(unreadable.verdict(), ListingVerdict::Partial);
        let truncated = ProtectionListing {
            complete: false,
            ..complete
        };
        assert_eq!(truncated.verdict(), ListingVerdict::Partial);
        assert!(truncated.render().contains("incomplete:"));

        let json = unreadable.to_json();
        assert_eq!(json["verdict"], "partial");
        assert_eq!(json["complete"], true);
        assert_eq!(json["unknown"][0]["identity"], "event/orders/placed");
        assert_eq!(json["unknown"][0]["revision"], 9);
        assert_eq!(json["unknown"][0]["liftable"], true);
    }

    #[test]
    fn an_unknown_row_without_a_revision_is_not_liftable() {
        let json = unknown_json(&row(0));
        assert_eq!(json["revision"], Value::Null);
        assert_eq!(json["liftable"], false);
    }
}
