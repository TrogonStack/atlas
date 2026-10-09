//! Namespace identity, display name, and hierarchy position.
//!
//! Three distinct things that the pre-registry design collapsed into one
//! free-form string:
//!
//! * [`NamespaceId`] is the immutable, globally unique handle that storage
//!   keys are built from. It never changes for the life of the namespace.
//! * [`NamespaceName`] is the human label. It is unique only *within* one
//!   [`OwnerId`], which is what lets two tenants each have a namespace
//!   called `orders`.
//! * [`OwnerId`] is the namespace's position in the ownership hierarchy:
//!   the `parent` of ADR#6310044131. It is mutable, and moving a namespace
//!   to a different owner rewrites this one field and nothing else.
//!
//! Keeping the id out of the name is what buys both properties at once.
//! Because the id is in the storage key and the name is not, a rename or a
//! re-owning touches one registry row and zero entity rows.
//!
//! # Why legacy ids are bare names
//!
//! A store written before the registry existed keyed its entities by the
//! bare namespace name. Those keys are already well-formed [`NamespaceId`]s:
//! `validate_id_component` accepts the same alphabet for both. So the
//! migration registers each existing namespace with `id == name` instead of
//! rewriting 3000-plus entity keys, and newly claimed namespaces get a
//! minted `ns_…` id. Both forms coexist in one bucket because the id is an
//! opaque string to everything below the registry.
//!
//! Only [`NamespaceId::adopt_legacy`] can produce a bare-name id, and it is
//! reachable from the migration path alone. Every other route mints, which
//! is the invariant that keeps ids collision-free: a second tenant can never
//! be handed `orders`, because it can never ask for a specific id at all.

use std::fmt;

use crate::id_component::{validate_id_component, IdComponentError};

/// Prefix on every minted [`NamespaceId`]. Self-describing per ADR#6310044131
/// rule 3, so a bare id in a log line or an error message says what it is.
pub const NAMESPACE_ID_PREFIX: &str = "ns_";

/// Prefix on every minted [`OwnerId`].
pub const OWNER_ID_PREFIX: &str = "own_";

/// Why a namespace identity value was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamespaceError {
    /// Failed the shared id-component charset rule.
    Component(IdComponentError),
    /// A minted id was expected but the value carries no recognised prefix.
    MissingPrefix {
        expected: &'static str,
        value: String,
    },
}

impl From<IdComponentError> for NamespaceError {
    fn from(value: IdComponentError) -> Self {
        Self::Component(value)
    }
}

impl fmt::Display for NamespaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Component(inner) => fmt::Display::fmt(inner, f),
            Self::MissingPrefix { expected, value } => {
                write!(
                    f,
                    "{value:?} is not a minted id (expected {expected}… prefix)"
                )
            }
        }
    }
}

impl std::error::Error for NamespaceError {}

/// Render a UUIDv7 as the compact lowercase-hex body of a minted id.
///
/// UUIDv7 is time-ordered, and hex preserves that order lexicographically,
/// so minted ids sort by creation time. That is not load-bearing today but
/// it costs nothing and makes a raw KV key listing readable in age order.
fn mint(prefix: &str) -> String {
    format!("{prefix}{}", uuid::Uuid::now_v7().simple())
}

// ---------------------------------------------------------------------------
// NamespaceId
// ---------------------------------------------------------------------------

/// The immutable handle a namespace is keyed by.
///
/// Never derived from the display name (except for legacy rows, see the
/// module docs) and never reused. Storage keys embed this, so it must stay
/// stable across renames and ownership moves.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NamespaceId(String);

impl NamespaceId {
    /// Mint a fresh id. The only route new namespaces take.
    #[must_use]
    pub fn generate() -> Self {
        Self(mint(NAMESPACE_ID_PREFIX))
    }

    /// Adopt a pre-registry namespace name as its own id.
    ///
    /// Restricted to the migration: it is the one place where an id is
    /// chosen rather than minted, and letting callers choose ids is exactly
    /// how two tenants would end up sharing one.
    ///
    /// Infallible: a [`NamespaceName`] has already passed the same charset
    /// rule an id must satisfy, which is precisely why the migration can
    /// reuse it verbatim.
    #[must_use]
    pub fn adopt_legacy(name: &NamespaceName) -> Self {
        Self(name.as_str().to_owned())
    }

    /// Re-hydrate an id read back out of storage or off the wire.
    ///
    /// Accepts both minted and legacy forms, because both are real ids that
    /// already exist in the bucket.
    ///
    /// # Errors
    ///
    /// Rejects any value that is not a valid id component.
    pub fn parse(value: &str) -> Result<Self, NamespaceError> {
        validate_id_component(value)?;
        Ok(Self(value.to_owned()))
    }

    /// Whether this id was minted rather than adopted from a legacy name.
    #[must_use]
    pub fn is_minted(&self) -> bool {
        self.0.starts_with(NAMESPACE_ID_PREFIX)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for NamespaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// NamespaceName
// ---------------------------------------------------------------------------

/// The human label for a namespace, unique within one [`OwnerId`].
///
/// This is what appears in an `Id.namespace` on the wire and what a modeller
/// types. It is deliberately *not* globally unique: that constraint is the
/// thing that made two tenants collide.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NamespaceName(String);

impl NamespaceName {
    /// # Errors
    ///
    /// Rejects any value that is not a valid id component.
    pub fn parse(value: &str) -> Result<Self, NamespaceError> {
        validate_id_component(value)?;
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for NamespaceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// OwnerId
// ---------------------------------------------------------------------------

/// A namespace's position in the ownership hierarchy: its `parent`.
///
/// Named for what it is rather than what it contains. ADR#6310044131 rule 1
/// reserves the bare field name `parent` for hierarchy position, and rule 2
/// only requires qualification between values of the *same* type. A
/// namespace's parent is an owner, not another namespace, so `parent` is the
/// correct field name wherever this type appears.
///
/// Unlike [`NamespaceId`], operator-chosen values are fine here. Owners come
/// from the token registry, which an operator writes by hand, and a readable
/// `acme` beats an opaque `own_…` in a config file. [`OwnerId::generate`]
/// exists for the day owners are provisioned programmatically.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OwnerId(String);

impl OwnerId {
    #[must_use]
    pub fn generate() -> Self {
        Self(mint(OWNER_ID_PREFIX))
    }

    /// # Errors
    ///
    /// Rejects any value that is not a valid id component.
    pub fn parse(value: &str) -> Result<Self, NamespaceError> {
        validate_id_component(value)?;
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for OwnerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn minted_ids_carry_their_prefix_and_are_key_safe() {
        let id = NamespaceId::generate();
        assert!(id.as_str().starts_with(NAMESPACE_ID_PREFIX));
        assert!(id.is_minted());
        // Must survive the storage-key alphabet unescaped, or every key
        // grows an `=HH` escape for no reason.
        assert!(crate::is_safe_id_component(id.as_str()));
        assert!(id
            .as_str()
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }

    #[test]
    fn minted_ids_are_unique() {
        let a = NamespaceId::generate();
        let b = NamespaceId::generate();
        assert_ne!(a, b);
    }

    #[test]
    fn minted_ids_sort_by_creation_time() {
        let first = NamespaceId::generate();
        let second = NamespaceId::generate();
        assert!(first < second, "{first} should sort before {second}");
    }

    #[test]
    fn legacy_adoption_keeps_the_bare_name() {
        let name = NamespaceName::parse("account-deletion").unwrap();
        let id = NamespaceId::adopt_legacy(&name);
        assert_eq!(id.as_str(), "account-deletion");
        assert!(!id.is_minted());
    }

    /// The migration's whole premise: an existing namespace name is already
    /// a well-formed id, so adopting it rewrites no keys.
    #[test]
    fn every_legal_namespace_name_is_a_legal_id() {
        for raw in ["orders", "order.placed", "a-b_c.d", "x", "agent-platform"] {
            let name = NamespaceName::parse(raw).unwrap();
            let id = NamespaceId::adopt_legacy(&name);
            assert_eq!(NamespaceId::parse(raw).unwrap(), id);
        }
    }

    #[test]
    fn rejects_values_outside_the_id_alphabet() {
        for raw in ["", ".", "..", ".hidden", "has space", "has/slash"] {
            assert!(NamespaceName::parse(raw).is_err(), "accepted {raw:?}");
            assert!(NamespaceId::parse(raw).is_err(), "accepted {raw:?}");
            assert!(OwnerId::parse(raw).is_err(), "accepted {raw:?}");
        }
    }

    #[test]
    fn owner_ids_accept_operator_chosen_names_and_mint_when_asked() {
        assert_eq!(OwnerId::parse("acme").unwrap().as_str(), "acme");
        assert!(OwnerId::generate().as_str().starts_with(OWNER_ID_PREFIX));
    }
}
