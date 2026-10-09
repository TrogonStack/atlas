//! Value objects for the handful of strings the SpiceDB API is strict about.
//!
//! Every pattern here is transcribed from the `buf.validate` rules on the
//! upstream protos. Prost does not generate validation from them, so the
//! rules are re-stated here as constructors. That is the better place for
//! them anyway: a caller learns its object id is illegal at the call site
//! instead of one network round trip later, and a type in a signature says
//! which of these strings a parameter wants.

use std::fmt;

use crate::{error::AuthzedError, v1};

/// `^([a-z][a-z0-9_]{1,61}[a-z0-9]/)*[a-z][a-z0-9_]{1,62}[a-z0-9]$`, max 128
/// bytes. The optional `/`-separated prefixes are SpiceDB's multi-tenant
/// namespacing of the schema itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectType(String);

/// `^([a-zA-Z0-9/_|\-=+]{1,})$`, max 1024 bytes.
///
/// The `*` wildcard the API also accepts here is deliberately not
/// constructible: it means "every subject of this type", and nothing in this
/// workspace wants to write that by accident.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(String);

/// `^[a-z][a-z0-9_]{1,62}[a-z0-9]$`, max 64 bytes.
///
/// One type for both relations and permissions, because SpiceDB's grammar
/// does not distinguish them and a check may name either.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Relation(String);

fn simple_name(kind: &'static str, value: &str) -> Result<(), AuthzedError> {
    if !(3..=64).contains(&value.len()) {
        return Err(AuthzedError::malformed(
            kind,
            format!("must be 3 to 64 bytes, got {}", value.len()),
        ));
    }
    let bytes = value.as_bytes();
    let head_ok = bytes[0].is_ascii_lowercase();
    let tail_ok =
        bytes[bytes.len() - 1].is_ascii_lowercase() || bytes[bytes.len() - 1].is_ascii_digit();
    let body_ok = bytes[1..bytes.len() - 1]
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_');
    if head_ok && tail_ok && body_ok {
        Ok(())
    } else {
        Err(AuthzedError::malformed(
            kind,
            format!("{value:?} must match ^[a-z][a-z0-9_]{{1,62}}[a-z0-9]$"),
        ))
    }
}

impl ObjectType {
    /// # Errors
    /// When `value` is not a legal SpiceDB object type.
    pub fn parse(value: &str) -> Result<Self, AuthzedError> {
        if value.len() > 128 {
            return Err(AuthzedError::malformed(
                "object type",
                format!("must be at most 128 bytes, got {}", value.len()),
            ));
        }
        for segment in value.split('/') {
            simple_name("object type", segment)?;
        }
        Ok(Self(value.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ObjectId {
    /// # Errors
    /// When `value` is empty, over 1024 bytes, or contains a byte outside
    /// `[a-zA-Z0-9/_|\-=+]`.
    pub fn parse(value: &str) -> Result<Self, AuthzedError> {
        if value.is_empty() || value.len() > 1024 {
            return Err(AuthzedError::malformed(
                "object id",
                format!("must be 1 to 1024 bytes, got {}", value.len()),
            ));
        }
        if let Some(bad) = value.bytes().find(|b| {
            !(b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'|' | b'-' | b'=' | b'+'))
        }) {
            return Err(AuthzedError::malformed(
                "object id",
                format!(
                    "{value:?} contains {:?}, which is outside [a-zA-Z0-9/_|-=+]",
                    char::from(bad)
                ),
            ));
        }
        Ok(Self(value.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Relation {
    /// # Errors
    /// When `value` is not a legal SpiceDB relation or permission name.
    pub fn parse(value: &str) -> Result<Self, AuthzedError> {
        simple_name("relation", value)?;
        Ok(Self(value.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

macro_rules! displays {
    ($($t:ty),*) => {$(
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    )*};
}
displays!(ObjectType, ObjectId, Relation);

/// A `type:id` pair, the thing permissions are granted on.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectRef {
    pub object_type: ObjectType,
    pub object_id: ObjectId,
}

impl ObjectRef {
    #[must_use]
    pub fn new(object_type: ObjectType, object_id: ObjectId) -> Self {
        Self {
            object_type,
            object_id,
        }
    }

    pub(crate) fn to_proto(&self) -> v1::ObjectReference {
        v1::ObjectReference {
            object_type: self.object_type.0.clone(),
            object_id: self.object_id.0.clone(),
        }
    }
}

impl fmt::Display for ObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.object_type, self.object_id)
    }
}

/// An object, optionally narrowed to one of its relations, as in
/// `org:acme#member` meaning "the members of acme" rather than "acme itself".
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SubjectRef {
    pub object: ObjectRef,
    pub relation: Option<Relation>,
}

impl SubjectRef {
    #[must_use]
    pub fn new(object: ObjectRef) -> Self {
        Self {
            object,
            relation: None,
        }
    }

    #[must_use]
    pub fn with_relation(object: ObjectRef, relation: Relation) -> Self {
        Self {
            object,
            relation: Some(relation),
        }
    }

    pub(crate) fn to_proto(&self) -> v1::SubjectReference {
        v1::SubjectReference {
            object: Some(self.object.to_proto()),
            optional_relation: self
                .relation
                .as_ref()
                .map(|r| r.0.clone())
                .unwrap_or_default(),
        }
    }
}

impl fmt::Display for SubjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.relation {
            Some(relation) => write!(f, "{}#{relation}", self.object),
            None => write!(f, "{}", self.object),
        }
    }
}

/// A `ZedToken`: a point in SpiceDB's history.
///
/// Opaque by contract, and kept opaque here. Its only use is being handed
/// back on a later read to say "at least this fresh", which is how a caller
/// avoids reading a permission it just revoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision(String);

impl Revision {
    #[must_use]
    pub fn new(token: String) -> Option<Self> {
        (!token.is_empty()).then_some(Self(token))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn from_proto(token: Option<v1::ZedToken>) -> Option<Self> {
        token.and_then(|t| Self::new(t.token))
    }

    pub(crate) fn to_proto(&self) -> v1::ZedToken {
        v1::ZedToken {
            token: self.0.clone(),
        }
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A SpiceDB pre-shared key.
///
/// A newtype only so that its [`fmt::Debug`] does not print it. Every
/// structured log line in this workspace derives Debug somewhere up the chain,
/// and a bearer credential that reaches a log is a credential to rotate.
#[derive(Clone, PartialEq, Eq)]
pub struct PresharedKey(String);

impl PresharedKey {
    /// # Errors
    /// When the key is empty, or contains bytes that cannot go in an HTTP
    /// header (which would otherwise fail later, per request).
    pub fn parse(value: &str) -> Result<Self, AuthzedError> {
        if value.is_empty() {
            return Err(AuthzedError::malformed(
                "preshared key",
                "must not be empty",
            ));
        }
        if !value.bytes().all(|b| (0x20..0x7f).contains(&b)) {
            return Err(AuthzedError::malformed(
                "preshared key",
                "must be printable ASCII, so it can be sent as a bearer token",
            ));
        }
        Ok(Self(value.to_string()))
    }

    pub(crate) fn header_value(&self) -> String {
        format!("Bearer {}", self.0)
    }
}

impl fmt::Debug for PresharedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PresharedKey(redacted)")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn object_types_follow_the_upstream_grammar() {
        assert!(ObjectType::parse("namespace").is_ok());
        assert!(ObjectType::parse("event_model/namespace").is_ok());
        assert!(ObjectType::parse("Namespace").is_err(), "uppercase");
        assert!(ObjectType::parse("ns").is_err(), "under three bytes");
        assert!(ObjectType::parse("name_").is_err(), "trailing underscore");
        assert!(ObjectType::parse("1namespace").is_err(), "leading digit");
    }

    #[test]
    fn object_ids_accept_the_full_id_charset_and_nothing_else() {
        assert!(ObjectId::parse("orders").is_ok());
        assert!(ObjectId::parse("ns_0199a-b/c|d=e+f").is_ok());
        assert!(ObjectId::parse("A").is_ok(), "single byte is legal");
        assert!(ObjectId::parse("").is_err());
        assert!(
            ObjectId::parse("orders.v2").is_err(),
            "a dot is not in the charset"
        );
        assert!(ObjectId::parse("a b").is_err());
        assert!(ObjectId::parse(&"a".repeat(1025)).is_err());
    }

    /// The wildcard is a real value in the API and its absence here is a
    /// decision, not an oversight, so it gets a test.
    #[test]
    fn the_subject_wildcard_is_not_constructible() {
        assert!(ObjectId::parse("*").is_err());
    }

    #[test]
    fn relations_are_lowercase_snake_case() {
        assert!(Relation::parse("view").is_ok());
        assert!(Relation::parse("can_administer").is_ok());
        assert!(Relation::parse("View").is_err());
        assert!(Relation::parse("no").is_err(), "under three bytes");
    }

    #[test]
    fn a_subject_renders_the_way_spicedb_writes_it() {
        let object = ObjectRef::new(
            ObjectType::parse("org").unwrap(),
            ObjectId::parse("acme").unwrap(),
        );
        assert_eq!(SubjectRef::new(object.clone()).to_string(), "org:acme");
        assert_eq!(
            SubjectRef::with_relation(object, Relation::parse("member").unwrap()).to_string(),
            "org:acme#member",
        );
    }

    #[test]
    fn an_empty_zed_token_is_not_a_revision() {
        assert!(Revision::new(String::new()).is_none());
        assert!(Revision::from_proto(Some(v1::ZedToken {
            token: String::new()
        }))
        .is_none());
        assert!(Revision::from_proto(None).is_none());
        assert_eq!(
            Revision::from_proto(Some(v1::ZedToken {
                token: "abc".into()
            }))
            .unwrap()
            .as_str(),
            "abc",
        );
    }

    #[test]
    fn a_preshared_key_does_not_print_itself() {
        let key = PresharedKey::parse("somesecret").unwrap();
        assert_eq!(format!("{key:?}"), "PresharedKey(redacted)");
        assert!(!format!("{key:?}").contains("somesecret"));
        assert_eq!(key.header_value(), "Bearer somesecret");
    }
}
