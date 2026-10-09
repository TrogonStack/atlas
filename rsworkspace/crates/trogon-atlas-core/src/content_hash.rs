//! Addressable content identity: a hash per entity, and a hash per
//! whole-model snapshot.
//!
//! # Why not hash the proto bytes
//!
//! Protobuf serialization is *not* canonical. Map-typed fields have no
//! defined wire order, and `google.protobuf.Any` payloads carry whatever
//! byte order the producer happened to emit (see the
//! `any_packed_struct_map_order_is_ignored` case in [`crate::semantic_eq`]).
//! Hashing bytes would make two semantically identical models hash
//! differently, reintroducing exactly the byte-level noise that entity-level
//! comparison exists to avoid.
//!
//! So the hash is taken over *canonical JSON*: the entity rendered through
//! the embedded descriptor pool as proto3 JSON ([`crate::transcode`]), then
//! re-serialized by [`canonical_json`] with object keys sorted and no
//! insignificant whitespace.
//!
//! # What the identity deliberately ignores
//!
//! * **The server-owned `system` block.** Cleared before hashing, so the
//!   hash agrees with [`crate::semantically_equal`]: two entities are
//!   content-identical iff they hash identically. `uid` and `created_at`
//!   are provenance, not content.
//! * **The difference between an unset field and one explicitly set to its
//!   default.** proto3 JSON omits default-valued fields, and proto3 itself
//!   does not preserve that distinction on the wire, so there is nothing to
//!   preserve here either.
//!
//! # Pinned canonicalization decisions
//!
//! These are the three details that would otherwise drift. Changing any of
//! them changes every hash, which is a format break.
//!
//! 1. **64-bit integers render as JSON strings.** That is proto3 JSON's
//!    rule (`"version": "1"`, not `"version": 1`) and it is preserved
//!    verbatim: no renumbering back to a JSON number.
//! 2. **Object keys sort by UTF-8 byte order**, done explicitly by
//!    [`canonical_json`] rather than inherited from `serde_json::Map`
//!    happening to be a `BTreeMap`. That backing type is a build-time
//!    feature (`preserve_order`) of a shared dependency, and content
//!    identity must not be decided by feature unification.
//! 3. **Floats format as shortest round-trip with a mandatory decimal
//!    point** (`1.0`, never `1`), so a `double` field never collides
//!    textually with an integer. Non-finite floats are rejected;
//!    proto3 JSON renders those as the strings `"NaN"` / `"Infinity"`, so
//!    reaching the number branch with one means the input was not proto3
//!    JSON.
//!
//! # Domain separation
//!
//! Entity hashes and snapshot hashes are both SHA-256, prefixed with
//! distinct domain tags so that a digest computed for one purpose can never
//! be mistaken for the other.

use std::{
    collections::BTreeMap,
    fmt::{self, Write as _},
    str::FromStr,
};

use sha2::{Digest, Sha256};
use trogon_atlas_proto::Entity;

use crate::transcode::{self, TranscodeError};

/// Domain tag for a single entity's content hash.
const ENTITY_DOMAIN: &[u8] = b"trogonatlas.content.entity.v1\0";
/// Domain tag for a whole-model snapshot hash.
const SNAPSHOT_DOMAIN: &[u8] = b"trogonatlas.content.snapshot.v1\0";

/// Width of a [`ContentHash`] in bytes (SHA-256).
pub const CONTENT_HASH_LEN: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum ContentHashError {
    #[error("rendering entity as canonical JSON: {0}")]
    Transcode(#[from] TranscodeError),
    #[error("serializing canonical JSON: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("non-finite number in canonical JSON; proto3 JSON renders these as strings")]
    NonFiniteNumber,
    #[error("duplicate snapshot key {0:?}")]
    DuplicateKey(String),
    #[error("content hash must be {CONTENT_HASH_LEN} lowercase hex bytes, got {0:?}")]
    Parse(String),
}

/// A content-addressable digest: SHA-256 over a domain tag plus canonical
/// bytes. Renders as 64 lowercase hex characters.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash([u8; CONTENT_HASH_LEN]);

impl ContentHash {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; CONTENT_HASH_LEN]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; CONTENT_HASH_LEN] {
        &self.0
    }

    /// Lowercase hex rendering, the wire and storage form.
    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(CONTENT_HASH_LEN * 2);
        for byte in self.0 {
            // Two hex nibbles per byte; `write!` to a String cannot fail.
            out.push(nibble(byte >> 4));
            out.push(nibble(byte & 0x0f));
        }
        out
    }

    /// The first `n` hex characters, for human-facing short forms. Never
    /// use a short form for equality: it is a display convenience only.
    #[must_use]
    pub fn to_short_hex(&self, n: usize) -> String {
        self.to_hex()
            .chars()
            .take(n.min(CONTENT_HASH_LEN * 2))
            .collect()
    }
}

const fn nibble(v: u8) -> char {
    match v {
        0..=9 => (b'0' + v) as char,
        _ => (b'a' + (v - 10)) as char,
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({})", self.to_hex())
    }
}

impl FromStr for ContentHash {
    type Err = ContentHashError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != CONTENT_HASH_LEN * 2 {
            return Err(ContentHashError::Parse(s.to_string()));
        }
        let mut out = [0u8; CONTENT_HASH_LEN];
        let raw = s.as_bytes();
        for (i, slot) in out.iter_mut().enumerate() {
            let hi =
                from_nibble(raw[i * 2]).ok_or_else(|| ContentHashError::Parse(s.to_string()))?;
            let lo = from_nibble(raw[i * 2 + 1])
                .ok_or_else(|| ContentHashError::Parse(s.to_string()))?;
            *slot = (hi << 4) | lo;
        }
        Ok(Self(out))
    }
}

/// Lowercase only: an uppercase digest would parse to the same bytes but
/// render differently, so two spellings of one identity could be stored.
const fn from_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Content hash of a single entity, ignoring the server-owned `system`
/// block.
///
/// Agrees with [`crate::semantically_equal`]: whenever both entities render
/// as JSON, equal hashes and semantic equality are the same predicate.
/// Entities that cannot render (an `Any` packed under a type url absent
/// from the descriptor pool) have no content identity and return an error,
/// rather than falling back to a byte hash that would report spurious
/// difference.
pub fn entity_content_hash(entity: &Entity) -> Result<ContentHash, ContentHashError> {
    let mut stripped = entity.clone();
    stripped.system = None;
    let value = transcode::entity_json_value(&stripped)?;
    let canonical = canonical_json(&value)?;
    let mut hasher = Sha256::new();
    hasher.update(ENTITY_DOMAIN);
    hasher.update(canonical.as_bytes());
    Ok(ContentHash(hasher.finalize().into()))
}

/// Serialize a JSON value canonically: object keys in UTF-8 byte order, no
/// insignificant whitespace, floats always carrying a decimal point.
///
/// Injective on `serde_json::Value` (modulo the non-finite floats it
/// rejects), which is what lets hash equality stand in for value equality.
pub fn canonical_json(value: &serde_json::Value) -> Result<String, ContentHashError> {
    let mut out = String::new();
    write_canonical(value, &mut out)?;
    Ok(out)
}

fn write_canonical(value: &serde_json::Value, out: &mut String) -> Result<(), ContentHashError> {
    use serde_json::Value;
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string());
            } else if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else {
                let f = n.as_f64().ok_or(ContentHashError::NonFiniteNumber)?;
                if !f.is_finite() {
                    return Err(ContentHashError::NonFiniteNumber);
                }
                // Debug for f64 is shortest-round-trip and always emits a
                // decimal point, so `1.0` never renders as `1`. Writing to
                // a String is infallible; the Result is the fmt::Write
                // trait's, not a failure this code can encounter.
                let _ = write!(out, "{f:?}");
            }
        }
        Value::String(s) => out.push_str(&write_string(s)?),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            // Sorted explicitly rather than trusting the Map backing type:
            // `serde_json/preserve_order` is a feature any crate in the
            // dependency graph could turn on, and content identity must not
            // depend on feature unification.
            let sorted: BTreeMap<&String, &serde_json::Value> = map.iter().collect();
            out.push('{');
            for (i, (key, val)) in sorted.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&write_string(key)?);
                out.push(':');
                write_canonical(val, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn write_string(s: &str) -> Result<String, ContentHashError> {
    serde_json::to_string(s).map_err(ContentHashError::Serialize)
}

/// Accumulates `(key, entity hash)` pairs into one whole-model snapshot
/// identity.
///
/// The digest is order-independent by construction (entries are kept
/// sorted) so two callers walking the store in different orders reach the
/// same snapshot hash. Entries are length-prefixed before hashing, so no
/// key containing a separator character can be crafted to collide with a
/// different entry set.
///
/// Keys are opaque here. Storage supplies its own entity key form; this
/// layer only requires that they be unique within a snapshot, and that a
/// given caller keep using the same form (changing the key form changes
/// every snapshot hash).
#[derive(Debug, Clone, Default)]
pub struct SnapshotHasher {
    entries: BTreeMap<String, ContentHash>,
}

impl SnapshotHasher {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one entry. A repeated key is an error rather than a silent
    /// overwrite: a snapshot with two rows for one key is a bug in the
    /// caller's enumeration, and silently keeping the last one would hash
    /// successfully while describing a model that never existed.
    pub fn insert(
        &mut self,
        key: impl Into<String>,
        hash: ContentHash,
    ) -> Result<(), ContentHashError> {
        let key = key.into();
        if self.entries.contains_key(&key) {
            return Err(ContentHashError::DuplicateKey(key));
        }
        self.entries.insert(key, hash);
        Ok(())
    }

    /// [`insert`](Self::insert) with the entity hashed in place.
    pub fn insert_entity(
        &mut self,
        key: impl Into<String>,
        entity: &Entity,
    ) -> Result<(), ContentHashError> {
        let hash = entity_content_hash(entity)?;
        self.insert(key, hash)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The snapshot identity. An empty snapshot has a well-defined hash
    /// (the domain tag plus a zero count), which is what makes "the model
    /// is empty" a state a caller can compare against rather than a special
    /// case.
    #[must_use]
    pub fn finish(&self) -> ContentHash {
        let mut hasher = Sha256::new();
        hasher.update(SNAPSHOT_DOMAIN);
        hasher.update(self.entries.len().to_string().as_bytes());
        hasher.update(b"\0");
        for (key, hash) in &self.entries {
            let key = key.as_bytes();
            hasher.update(key.len().to_string().as_bytes());
            hasher.update(b":");
            hasher.update(key);
            hasher.update(hash.as_bytes());
        }
        ContentHash(hasher.finalize().into())
    }
}

impl FromIterator<(String, ContentHash)> for SnapshotHasher {
    /// Last write wins, unlike [`SnapshotHasher::insert`]. Only use this
    /// when the source is already known to hold unique keys (a map, a
    /// deduplicated scan).
    fn from_iter<I: IntoIterator<Item = (String, ContentHash)>>(iter: I) -> Self {
        Self {
            entries: iter.into_iter().collect(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use prost::Message as _;
    use trogon_atlas_proto as pb;

    use super::*;

    fn event(title: &str) -> Entity {
        Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: "ns".into(),
                    slug: "a".into(),
                    version: 1,
                }),
                title: title.into(),
                ..Default::default()
            })),
        }
    }

    #[test]
    fn hash_is_stable_across_calls() {
        let e = event("T");
        assert_eq!(
            entity_content_hash(&e).unwrap(),
            entity_content_hash(&e).unwrap()
        );
    }

    #[test]
    fn hash_ignores_system_meta() {
        let a = event("T");
        let mut b = event("T");
        b.system = Some(pb::SystemMeta {
            uid: "u".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        });
        assert_eq!(
            entity_content_hash(&a).unwrap(),
            entity_content_hash(&b).unwrap()
        );
    }

    #[test]
    fn different_content_hashes_differently() {
        assert_ne!(
            entity_content_hash(&event("A")).unwrap(),
            entity_content_hash(&event("B")).unwrap()
        );
    }

    /// The reason this module exists: byte-different encodings of the same
    /// `google.protobuf.Struct` must reach one identity. Mirrors
    /// `semantic_eq::tests::any_packed_struct_map_order_is_ignored`.
    #[test]
    fn any_packed_struct_map_order_does_not_change_the_hash() {
        let field = |k: &str, v: &str| {
            (
                k.to_string(),
                prost_types::Value {
                    kind: Some(prost_types::value::Kind::StringValue(v.into())),
                },
            )
        };
        let mut s1 = prost_types::Struct::default();
        s1.fields.extend([field("a", "1"), field("b", "2")]);
        let forward = s1.encode_to_vec();

        let mut s2 = prost_types::Struct::default();
        s2.fields.extend([field("b", "2")]);
        let mut reversed = s2.encode_to_vec();
        let mut s3 = prost_types::Struct::default();
        s3.fields.extend([field("a", "1")]);
        reversed.extend(s3.encode_to_vec());

        let wrap = |value: Vec<u8>| Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(pb::Id {
                    namespace: "ns".into(),
                    slug: "s".into(),
                    version: 1,
                }),
                scenarios: vec![pb::CommandScenario {
                    id: "sc".into(),
                    when: Some(pb::CommandExample {
                        payload: Some(prost_types::Any {
                            type_url: "type.googleapis.com/google.protobuf.Struct".into(),
                            value,
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
        };
        assert_ne!(forward, reversed, "encodings must differ");
        assert_eq!(
            entity_content_hash(&wrap(forward)).unwrap(),
            entity_content_hash(&wrap(reversed)).unwrap(),
        );
    }

    #[test]
    fn unrenderable_entity_has_no_content_identity() {
        let e = Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: "ns".into(),
                    slug: "a".into(),
                    version: 1,
                }),
                metadata: vec![prost_types::Any {
                    type_url: "type.googleapis.com/retired.package.Gone".into(),
                    value: vec![1],
                }],
                ..Default::default()
            })),
        };
        assert!(entity_content_hash(&e).is_err());
    }

    #[test]
    fn hex_round_trips() {
        let h = entity_content_hash(&event("T")).unwrap();
        let text = h.to_hex();
        assert_eq!(text.len(), 64);
        assert_eq!(ContentHash::from_str(&text).unwrap(), h);
        assert_eq!(h.to_short_hex(8), text[..8]);
        assert_eq!(h.to_short_hex(500), text);
    }

    #[test]
    fn hex_parsing_rejects_bad_input() {
        assert!(ContentHash::from_str("").is_err());
        assert!(ContentHash::from_str(&"z".repeat(64)).is_err());
        assert!(ContentHash::from_str(&"a".repeat(63)).is_err());
        // Uppercase is rejected: it would parse to the same bytes but
        // render differently, giving one identity two spellings.
        assert!(ContentHash::from_str(&"A".repeat(64)).is_err());
    }

    #[test]
    fn canonical_json_sorts_keys_and_drops_whitespace() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{ "b": 1, "a": [1, 2], "A": null }"#).unwrap();
        assert_eq!(canonical_json(&v).unwrap(), r#"{"A":null,"a":[1,2],"b":1}"#);
    }

    #[test]
    fn canonical_json_pins_float_formatting() {
        let v = serde_json::json!({ "f": 1.0, "g": 0.1, "i": 1 });
        assert_eq!(canonical_json(&v).unwrap(), r#"{"f":1.0,"g":0.1,"i":1}"#);
    }

    #[test]
    fn canonical_json_escapes_strings_and_keys() {
        let v = serde_json::json!({ "a\"b": "x\ny" });
        assert_eq!(canonical_json(&v).unwrap(), r#"{"a\"b":"x\ny"}"#);
    }

    #[test]
    fn canonical_json_is_key_order_independent() {
        let a: serde_json::Value = serde_json::from_str(r#"{"x":1,"y":2}"#).unwrap();
        let b: serde_json::Value = serde_json::from_str(r#"{"y":2,"x":1}"#).unwrap();
        assert_eq!(canonical_json(&a).unwrap(), canonical_json(&b).unwrap());
    }

    #[test]
    fn snapshot_is_insertion_order_independent() {
        let a = entity_content_hash(&event("A")).unwrap();
        let b = entity_content_hash(&event("B")).unwrap();

        let mut forward = SnapshotHasher::new();
        forward.insert("ev.ns.a.1", a).unwrap();
        forward.insert("ev.ns.b.1", b).unwrap();

        let mut reversed = SnapshotHasher::new();
        reversed.insert("ev.ns.b.1", b).unwrap();
        reversed.insert("ev.ns.a.1", a).unwrap();

        assert_eq!(forward.finish(), reversed.finish());
        assert_eq!(forward.len(), 2);
    }

    #[test]
    fn snapshot_changes_when_any_entity_changes() {
        let mut before = SnapshotHasher::new();
        before.insert_entity("ev.ns.a.1", &event("A")).unwrap();
        let mut after = SnapshotHasher::new();
        after.insert_entity("ev.ns.a.1", &event("A!")).unwrap();
        assert_ne!(before.finish(), after.finish());
    }

    #[test]
    fn snapshot_distinguishes_key_from_content() {
        let h = entity_content_hash(&event("A")).unwrap();
        let mut one = SnapshotHasher::new();
        one.insert("ev.ns.a.1", h).unwrap();
        let mut other = SnapshotHasher::new();
        other.insert("ev.ns.b.1", h).unwrap();
        assert_ne!(one.finish(), other.finish());
    }

    /// Length-prefix framing: a key that embeds the separator must not be
    /// able to impersonate a different entry set.
    #[test]
    fn snapshot_framing_resists_separator_injection() {
        let h = entity_content_hash(&event("A")).unwrap();
        let mut one = SnapshotHasher::new();
        one.insert("a:b", h).unwrap();
        let mut other = SnapshotHasher::new();
        other.insert("a", h).unwrap();
        other.insert("b", h).unwrap();
        assert_ne!(one.finish(), other.finish());
    }

    #[test]
    fn empty_snapshot_has_a_defined_identity() {
        let empty = SnapshotHasher::new();
        assert!(empty.is_empty());
        assert_eq!(empty.finish(), SnapshotHasher::new().finish());

        let mut one = SnapshotHasher::new();
        one.insert_entity("ev.ns.a.1", &event("A")).unwrap();
        assert_ne!(empty.finish(), one.finish());
    }

    #[test]
    fn duplicate_snapshot_key_is_rejected() {
        let h = entity_content_hash(&event("A")).unwrap();
        let mut s = SnapshotHasher::new();
        s.insert("ev.ns.a.1", h).unwrap();
        let err = s.insert("ev.ns.a.1", h).unwrap_err();
        assert!(err.to_string().contains("duplicate"), "{err}");
    }

    #[test]
    fn entity_and_snapshot_domains_do_not_collide() {
        // A one-entry snapshot must not equal the entity hash it wraps.
        let h = entity_content_hash(&event("A")).unwrap();
        let mut s = SnapshotHasher::new();
        s.insert("ev.ns.a.1", h).unwrap();
        assert_ne!(s.finish(), h);
    }

    // -----------------------------------------------------------------
    // Property-based tests, mirroring the idioms in `semantic_eq`.
    // -----------------------------------------------------------------

    use proptest::prelude::*;

    fn bounded_text() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9 _.-]{0,24}"
    }

    fn event_with(
        ns: String,
        slug: String,
        title: String,
        doc: String,
        system: Option<pb::SystemMeta>,
    ) -> Entity {
        Entity {
            system,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: ns,
                    slug,
                    version: 1,
                }),
                title,
                doc,
                ..Default::default()
            })),
        }
    }

    fn arb_event() -> impl Strategy<Value = Entity> {
        (
            "[a-z][a-z0-9]{0,12}",
            "[a-z][a-z0-9]{0,12}",
            bounded_text(),
            bounded_text(),
            prop_oneof![
                Just(None),
                bounded_text().prop_map(|uid| Some(pb::SystemMeta {
                    uid,
                    created_at: "2026-01-01T00:00:00Z".into(),
                })),
            ],
        )
            .prop_map(|(ns, slug, title, doc, system)| event_with(ns, slug, title, doc, system))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// The contract that lets a hash stand in for the entity: for any
        /// two renderable entities, hash equality and semantic equality are
        /// the same predicate.
        #[test]
        fn hash_equality_matches_semantic_equality(a in arb_event(), b in arb_event()) {
            let ha = entity_content_hash(&a).unwrap();
            let hb = entity_content_hash(&b).unwrap();
            prop_assert_eq!(ha == hb, crate::semantically_equal(&a, &b));
        }

        /// Hex is a faithful encoding, not a lossy display form.
        #[test]
        fn hash_hex_round_trips(e in arb_event()) {
            let h = entity_content_hash(&e).unwrap();
            prop_assert_eq!(ContentHash::from_str(&h.to_hex()).unwrap(), h);
        }

        /// Snapshot identity depends on the entry *set*, never on the order
        /// the caller enumerated it in.
        #[test]
        fn snapshot_is_order_independent(
            entities in prop::collection::vec(arb_event(), 0..8),
        ) {
            let keyed: Vec<(String, Entity)> = entities
                .into_iter()
                .enumerate()
                .map(|(i, e)| (format!("k{i}"), e))
                .collect();

            let mut forward = SnapshotHasher::new();
            for (k, e) in &keyed {
                forward.insert_entity(k.clone(), e).unwrap();
            }
            let mut reversed = SnapshotHasher::new();
            for (k, e) in keyed.iter().rev() {
                reversed.insert_entity(k.clone(), e).unwrap();
            }
            prop_assert_eq!(forward.finish(), reversed.finish());
        }
    }
}
