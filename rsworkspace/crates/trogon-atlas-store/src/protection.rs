use std::fmt;

use trogon_atlas_proto::{EntityKind, Id};

use crate::error::{StoreError, StoreResult};

/// The thing protection applies to: every version of one `(kind, namespace,
/// slug)`, including versions nobody has written yet.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityIdentity {
    kind: EntityKind,
    namespace: String,
    slug: String,
}

impl EntityIdentity {
    /// # Errors
    ///
    /// `InvalidArgument` when the kind is unspecified or a component is empty.
    pub fn new(
        kind: EntityKind,
        namespace: impl Into<String>,
        slug: impl Into<String>,
    ) -> StoreResult<Self> {
        let namespace = namespace.into();
        let slug = slug.into();
        if kind == EntityKind::Unspecified {
            return Err(StoreError::InvalidArgument(
                "entity identity requires a kind".into(),
            ));
        }
        if namespace.is_empty() || slug.is_empty() {
            return Err(StoreError::InvalidArgument(
                "entity identity requires a namespace and a slug".into(),
            ));
        }
        Ok(Self {
            kind,
            namespace,
            slug,
        })
    }

    /// The identity a stored version belongs to. `None` for an unspecified
    /// kind or an empty component, which no stored entity has.
    #[must_use]
    pub fn of(kind: EntityKind, id: &Id) -> Option<Self> {
        Self::new(kind, id.namespace.clone(), id.slug.clone()).ok()
    }

    #[must_use]
    pub fn kind(&self) -> EntityKind {
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
}

impl fmt::Display for EntityIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{}",
            trogon_atlas_proto::canonical::kind_short(self.kind),
            self.namespace,
            self.slug
        )
    }
}

/// Why an identity is protected, as the admin stated it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionReason(String);

impl ProtectionReason {
    pub const MAX_LEN: usize = 1024;

    /// # Errors
    ///
    /// `InvalidArgument` when the reason is blank or longer than
    /// [`Self::MAX_LEN`] bytes.
    pub fn parse(raw: &str) -> StoreResult<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(StoreError::InvalidArgument(
                "protection requires a reason".into(),
            ));
        }
        if trimmed.len() > Self::MAX_LEN {
            return Err(StoreError::InvalidArgument(format!(
                "protection reason exceeds {} bytes",
                Self::MAX_LEN
            )));
        }
        Ok(Self(trimmed.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProtectionReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One row of the protection registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionRecord {
    pub identity: EntityIdentity,
    pub reason: ProtectionReason,
    /// Authenticated principal that set it.
    pub protected_by: String,
    /// RFC3339 timestamp.
    pub protected_at: String,
    /// Changeset that recorded the protection.
    pub changeset_id: String,
}

impl ProtectionRecord {
    #[must_use]
    pub fn to_proto(&self) -> trogon_atlas_proto::EntityProtection {
        trogon_atlas_proto::EntityProtection {
            kind: self.identity.kind as i32,
            namespace: self.identity.namespace.clone(),
            slug: self.identity.slug.clone(),
            reason: self.reason.as_str().to_owned(),
            protected_by: self.protected_by.clone(),
            protected_at: self.protected_at.clone(),
            changeset_id: self.changeset_id.clone(),
            revision: 0,
        }
    }

    /// # Errors
    ///
    /// `Backend` when the stored row does not describe a valid identity.
    pub fn from_proto(proto: trogon_atlas_proto::EntityProtection) -> StoreResult<Self> {
        let corrupt = |e: StoreError| StoreError::Backend(format!("decode EntityProtection: {e}"));
        let kind = EntityKind::try_from(proto.kind).unwrap_or(EntityKind::Unspecified);
        Ok(Self {
            identity: EntityIdentity::new(kind, proto.namespace, proto.slug).map_err(corrupt)?,
            reason: ProtectionReason::parse(&proto.reason).map_err(corrupt)?,
            protected_by: proto.protected_by,
            protected_at: proto.protected_at,
            changeset_id: proto.changeset_id,
        })
    }
}

/// Revision of a stored protection row: the etag a lift compares against so
/// it never removes a row it did not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtectionRevision(u64);

impl ProtectionRevision {
    #[must_use]
    pub fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ProtectionRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A protection row as stored, with the revision it was read at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredProtection {
    pub record: ProtectionRecord,
    pub revision: ProtectionRevision,
}

impl StoredProtection {
    #[must_use]
    pub fn to_proto(&self) -> trogon_atlas_proto::EntityProtection {
        trogon_atlas_proto::EntityProtection {
            revision: self.revision.get(),
            ..self.record.to_proto()
        }
    }
}

/// Why an identity's protection could not be read. Coarse on purpose: the
/// backend detail goes to the server log, not to callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionUnknownCause {
    /// The row exists but does not decode to a protection.
    Undecodable,
    /// The row decodes but names a different identity than its key.
    Mismatched,
    /// The registry did not answer.
    Unreachable,
}

impl fmt::Display for ProtectionUnknownCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Undecodable => "protection row cannot be decoded",
            Self::Mismatched => "protection row names a different identity",
            Self::Unreachable => "protection registry did not answer",
        })
    }
}

/// An identity whose protection could not be read: the row is undecodable or
/// the store did not answer. Writes treat it as protected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionUnknown {
    pub identity: EntityIdentity,
    pub cause: ProtectionUnknownCause,
    /// Revision of the unreadable row, when there is one. An admin lifts the
    /// row at this revision to repair it; `None` means the store itself could
    /// not be read and nothing can be lifted.
    pub revision: Option<ProtectionRevision>,
}

impl ProtectionUnknown {
    #[must_use]
    pub fn to_proto(&self) -> trogon_atlas_proto::EntityProtectionUnknown {
        trogon_atlas_proto::EntityProtectionUnknown {
            kind: self.identity.kind as i32,
            namespace: self.identity.namespace.clone(),
            slug: self.identity.slug.clone(),
            cause: self.cause.to_string(),
            revision: self.revision.map_or(0, ProtectionRevision::get),
        }
    }
}

/// What a lookup learned about one identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtectionState {
    Unprotected,
    Protected(StoredProtection),
    Unknown(ProtectionUnknown),
}

impl ProtectionState {
    /// The state to act on when the lookup itself failed.
    #[must_use]
    pub fn unreadable(identity: EntityIdentity, error: &StoreError) -> Self {
        tracing::warn!(%identity, %error, "protection lookup failed; treating the identity as protected");
        Self::Unknown(ProtectionUnknown {
            identity,
            cause: ProtectionUnknownCause::Unreachable,
            revision: None,
        })
    }

    #[must_use]
    pub fn is_unprotected(&self) -> bool {
        matches!(self, Self::Unprotected)
    }
}

/// Every protection row the store could enumerate. `complete` is false when
/// the enumeration stopped early, so absence from `protected` and `unknown`
/// proves nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProtectionListing {
    pub protected: Vec<StoredProtection>,
    pub unknown: Vec<ProtectionUnknown>,
    pub complete: bool,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn identity_rejects_an_unspecified_kind() {
        assert!(EntityIdentity::new(EntityKind::Unspecified, "orders", "placed").is_err());
    }

    #[test]
    fn identity_rejects_empty_components() {
        assert!(EntityIdentity::new(EntityKind::Event, "", "placed").is_err());
        assert!(EntityIdentity::new(EntityKind::Event, "orders", "").is_err());
    }

    #[test]
    fn identity_ignores_the_version() {
        let v1 = Id {
            namespace: "orders".into(),
            slug: "placed".into(),
            version: 1,
        };
        let v9 = Id {
            version: 9,
            ..v1.clone()
        };
        assert_eq!(
            EntityIdentity::of(EntityKind::Event, &v1),
            EntityIdentity::of(EntityKind::Event, &v9)
        );
    }

    #[test]
    fn reason_rejects_blank_and_oversized_text() {
        assert!(ProtectionReason::parse("   ").is_err());
        assert!(ProtectionReason::parse(&"x".repeat(ProtectionReason::MAX_LEN + 1)).is_err());
        assert_eq!(
            ProtectionReason::parse("  frozen for audit ")
                .unwrap()
                .as_str(),
            "frozen for audit"
        );
    }

    #[test]
    fn a_failed_lookup_is_unknown_without_a_revision() {
        let identity = EntityIdentity::new(EntityKind::Event, "orders", "placed").unwrap();
        let state =
            ProtectionState::unreadable(identity.clone(), &StoreError::Backend("down".into()));
        let ProtectionState::Unknown(unknown) = state else {
            panic!("expected unknown");
        };
        assert_eq!(unknown.identity, identity);
        assert_eq!(unknown.revision, None);
        assert_eq!(unknown.cause, ProtectionUnknownCause::Unreachable);
    }

    #[test]
    fn record_round_trips_through_proto() {
        let record = ProtectionRecord {
            identity: EntityIdentity::new(EntityKind::Command, "orders", "place").unwrap(),
            reason: ProtectionReason::parse("contract is published").unwrap(),
            protected_by: "alice".into(),
            protected_at: "2026-01-01T00:00:00Z".into(),
            changeset_id: "cs".into(),
        };
        assert_eq!(
            ProtectionRecord::from_proto(record.to_proto()).unwrap(),
            record
        );
    }
}
