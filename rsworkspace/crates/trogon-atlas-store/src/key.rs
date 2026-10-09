use std::fmt::Write as _;

use trogon_atlas_proto::{EntityKind, Id};

/// NATS `JetStream` KV keys are restricted to `[-/_=.a-zA-Z0-9]+`. Within
/// our encoded form we further reserve `.` as the segment separator, so
/// per-component encoding limits the allowed alphabet to
/// `[-_a-zA-Z0-9]` and escapes everything else with the `=HH` byte
/// scheme (uppercase hex, one escape per UTF-8 byte). `=` itself is
/// escaped as `=3D`. The escape was chosen because `=` is already in
/// the KV-valid set, so the encoded form is always KV-valid.
///
/// Layout:
///
/// ```text
/// "<kind_short>.<namespace>.<slug>.<version>"
/// ```
///
/// # Contract
///
/// `namespace` and `slug` must not be empty. An empty component cannot
/// round-trip through `parse_entity_key` because the empty string encodes
/// as an empty segment, which `splitn` collapses, making it
/// indistinguishable from a missing segment. Callers must enforce
/// non-empty components before construction (the gRPC boundary does this
/// via `validate`).
#[must_use]
pub fn entity_key(kind: EntityKind, id: &Id) -> String {
    format!(
        "{}.{}.{}.{}",
        trogon_atlas_proto::canonical::kind_short(kind),
        encode_component(&id.namespace),
        encode_component(&id.slug),
        id.version
    )
}

// Kept test-only: nothing in the production paths consumes this prefix
// today, and the kind/namespace/slug layout is an internal key-layout
// concern. Promote to `pub(crate)` / `pub` if a real consumer appears.
#[cfg(test)]
pub(crate) fn slug_prefix(kind: EntityKind, namespace: &str, slug: &str) -> String {
    format!(
        "{}.{}.{}.",
        trogon_atlas_proto::canonical::kind_short(kind),
        encode_component(namespace),
        encode_component(slug)
    )
}

#[must_use]
pub fn namespace_prefix(kind: EntityKind, namespace: &str) -> String {
    format!(
        "{}.{}.",
        trogon_atlas_proto::canonical::kind_short(kind),
        encode_component(namespace)
    )
}

#[must_use]
pub fn kind_prefix(kind: EntityKind) -> String {
    format!("{}.", trogon_atlas_proto::canonical::kind_short(kind))
}

// ---------------------------------------------------------------------------
// Namespace registry keys
// ---------------------------------------------------------------------------
//
// The registry bucket holds two rows per namespace, because the two questions
// asked of it need opposite lookups and both must be O(1):
//
//   record  `"id.<ns_id>"`            -> encoded `NamespaceRecord`
//   index   `"name.<parent>.<name>"`  -> the namespace id, as raw UTF-8
//
// The index row is not a cache. It is the uniqueness constraint: claiming a
// name is a KV *create* against this key, so two concurrent claims for the
// same `(parent, name)` cannot both succeed no matter how they interleave.
// Deriving the answer by scanning record rows instead would be a
// read-then-write race, which is exactly the shape of the
// `BC_PROJECT_COLLISION` bug the registry exists to remove.
//
// Scoping the index by `parent` is what makes names collision-free *between*
// owners while still unique *within* one: `name.acme.orders` and
// `name.beta.orders` are different keys.
//
// The `id.` / `name.` discriminators keep the two row families from ever
// colliding under a prefix scan. Neither can be produced by the other's
// builder, because every embedded component is escaped and `.` is the
// segment separator.

/// Key for a namespace's record row: `"id.<escaped-ns-id>"`.
#[must_use]
pub fn namespace_record_key(namespace_id: &str) -> String {
    format!("id.{}", encode_component(namespace_id))
}

/// Prefix matching every namespace record row. Used to enumerate the
/// registry; deliberately excludes the `name.` index rows.
#[must_use]
pub fn namespace_record_prefix() -> &'static str {
    "id."
}

/// Key for the name-uniqueness index row:
/// `"name.<escaped-parent>.<escaped-name>"`.
///
/// Written with KV create (never put) so it doubles as the per-owner
/// uniqueness constraint. See the module note above.
#[must_use]
pub fn namespace_name_key(parent: &str, name: &str) -> String {
    format!(
        "name.{}.{}",
        encode_component(parent),
        encode_component(name)
    )
}

/// Prefix matching every name-index row under one owner:
/// `"name.<escaped-parent>."`.
///
/// The trailing `.` is load-bearing in the same way as
/// [`branch_delta_prefix`]: without it, owner `acme` would also match every
/// row belonging to owner `acme-staging`.
#[must_use]
pub fn namespace_name_prefix(parent: &str) -> String {
    format!("name.{}.", encode_component(parent))
}

/// Recover the namespace id embedded in a [`namespace_record_key`].
#[must_use]
pub fn parse_namespace_record_key(key: &str) -> Option<String> {
    let encoded = key.strip_prefix(namespace_record_prefix())?;
    let decoded = decode_component(encoded)?;
    if decoded.is_empty() {
        return None;
    }
    Some(decoded)
}

/// Branch name reserved for the metadata-row prefix; see
/// [`is_reserved_branch_name`].
pub const RESERVED_BRANCH_NAME: &str = "meta";

/// Rejects the one branch name that would make delta rows ambiguous with
/// metadata rows under a plain `"meta."` prefix scan: a branch literally
/// named `"meta"` encodes (via [`encode_component`], which is the identity
/// function on pure-ASCII-alphanumeric input) to the same `meta.` prefix
/// used by every [`branch_meta_key`] row, so `branch_delta_prefix("meta")`
/// and the metadata-row prefix would overlap. Callers validating a
/// caller-supplied branch name (e.g. `CreateBranch`) must reject this name
/// in addition to the general `trogon_atlas_core::validate_id_component`
/// charset rule.
#[must_use]
pub fn is_reserved_branch_name(branch: &str) -> bool {
    branch.eq_ignore_ascii_case(RESERVED_BRANCH_NAME)
}

/// Key for a branch's metadata row in the branches KV bucket:
/// `"meta.<escaped-branch>"`. The branch name is escaped with the same
/// per-component scheme as [`entity_key`] (including its `/` owner-prefix
/// separator, which escapes like any other special byte).
///
/// Metadata rows are enumerated by a plain `"meta."` prefix scan (see
/// `NatsStore::list_branches`). This is unambiguous with delta rows
/// ([`branch_delta_key`]) only because callers reject
/// [`is_reserved_branch_name`] before ever constructing a branch -- see
/// that function's doc comment for why the literal name `"meta"` is the
/// one case that would otherwise collide.
#[must_use]
pub fn branch_meta_key(branch: &str) -> String {
    format!("meta.{}", encode_component(branch))
}

/// Key for a branch's copy-on-write delta row in the branches KV bucket:
/// `"<escaped-branch>.<kind_short>.<namespace>.<slug>.<version>"`.
#[must_use]
pub fn branch_delta_key(branch: &str, kind: EntityKind, id: &Id) -> String {
    format!("{}.{}", encode_component(branch), entity_key(kind, id))
}

/// Prefix shared by every delta row belonging to `branch`:
/// `"<escaped-branch>."`. Used to enumerate a branch's delta rows without
/// matching rows belonging to a *different* branch, even one whose escaped
/// name is a prefix of this one, because the trailing `.` boundary is part
/// of the prefix.
#[must_use]
pub fn branch_delta_prefix(branch: &str) -> String {
    format!("{}.", encode_component(branch))
}

/// Parse a key produced by [`branch_delta_key`] back into `(kind, id)`,
/// given the (already known) branch name it belongs to. Returns `None` for
/// any malformed input, mirroring [`parse_entity_key`].
#[must_use]
pub fn parse_branch_delta_key(branch: &str, key: &str) -> Option<(EntityKind, Id)> {
    let rest = key.strip_prefix(&branch_delta_prefix(branch))?;
    parse_entity_key(rest)
}

/// Prefix shared by every revision row for one entity, in the revision-log
/// bucket:
///
/// ```text
/// baseline: "base.<kind_short>.<namespace>.<slug>.<version>."
/// branch:   "branch.<escaped-branch>.<kind_short>.<namespace>.<slug>.<version>."
/// ```
///
/// The leading literal is what keeps the two apart: a branch row always
/// starts `branch.`, so no branch name can reach the `base.` keyspace no
/// matter how it escapes. Baseline and branch history are separate logs
/// because a branch write never touches baseline.
///
/// The trailing `.` is load-bearing: without it `...slug.1.` would also
/// match `...slug.12.<id>`.
#[must_use]
pub fn revision_prefix(branch: Option<&str>, kind: EntityKind, id: &Id) -> String {
    match branch {
        None => format!("base.{}.", entity_key(kind, id)),
        Some(branch) => format!(
            "branch.{}.{}.",
            encode_component(branch),
            entity_key(kind, id)
        ),
    }
}

/// Key for one entity's revision under one changeset:
/// [`revision_prefix`] followed by the changeset id.
///
/// The id is a UUIDv7, so appending it makes the prefix scan return an
/// entity's history already in chronological order. No secondary index and
/// no timestamp cursor.
#[must_use]
pub fn revision_key(branch: Option<&str>, kind: EntityKind, id: &Id, changeset_id: &str) -> String {
    format!("{}{changeset_id}", revision_prefix(branch, kind, id))
}

fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' => out.push(byte as char),
            other => {
                let _ = write!(out, "={other:02X}");
            }
        }
    }
    out
}

/// Public alias for [`decode_component`], the inverse of `encode_component`
/// used when reversing a `branch_meta_key`'s escaped branch-name segment
/// back into the original branch name.
#[must_use]
pub fn decode_component_pub(s: &str) -> Option<String> {
    decode_component(s)
}

fn decode_component(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'=' {
            if i + 3 > bytes.len() {
                return None;
            }
            let hi = (bytes[i + 1] as char).to_digit(16)?;
            let lo = (bytes[i + 2] as char).to_digit(16)?;
            #[allow(clippy::cast_possible_truncation)]
            out.push(((hi << 4) | lo) as u8); // safe: hi,lo are each 0..=0xF from to_digit(16)
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Parse the key produced by [`entity_key`]. Returns `None` for any
/// malformed input (unknown kind label, missing segments, invalid escape,
/// non-numeric version, or empty namespace/slug).
///
/// Empty namespace or slug components are rejected here even though
/// `entity_key` will encode them as empty segments. The gRPC boundary
/// rejects empty components before they reach storage, so a key with an
/// empty component is either malformed or from an older schema version
/// that violated the contract. Treating it as unparseable prevents it
/// from silently appearing as a live entity.
#[must_use]
pub fn parse_entity_key(key: &str) -> Option<(EntityKind, Id)> {
    let mut parts = key.splitn(4, '.');
    let kind_short = parts.next()?;
    let namespace_enc = parts.next()?;
    let slug_enc = parts.next()?;
    let version_str = parts.next()?;
    let kind = kind_from_short(kind_short)?;
    let namespace = decode_component(namespace_enc)?;
    let slug = decode_component(slug_enc)?;
    if namespace.is_empty() || slug.is_empty() {
        return None;
    }
    let version = version_str.parse::<u64>().ok()?;
    Some((
        kind,
        Id {
            namespace,
            slug,
            version,
        },
    ))
}

fn kind_from_short(s: &str) -> Option<EntityKind> {
    Some(match s {
        "event" => EntityKind::Event,
        "command" => EntityKind::Command,
        "read_model" => EntityKind::ReadModel,
        "processor" => EntityKind::Processor,
        "ui" => EntityKind::Ui,
        "persona" => EntityKind::Persona,
        "swimlane" => EntityKind::Swimlane,
        "command_slice" => EntityKind::CommandSlice,
        "read_model_slice" => EntityKind::ReadModelSlice,
        "automation_slice" => EntityKind::AutomationSlice,
        "ui_slice" => EntityKind::UiSlice,
        "storyboard" => EntityKind::Storyboard,
        "event_model" => EntityKind::EventModel,
        "component" => EntityKind::Component,
        "external_system" => EntityKind::ExternalSystem,
        "tracker" => EntityKind::Tracker,
        "bounded_context" => EntityKind::BoundedContext,
        "domain" => EntityKind::Domain,
        "subdomain" => EntityKind::Subdomain,
        "schema" => EntityKind::Schema,
        "project" => EntityKind::Project,
        "screen" => EntityKind::Screen,
        "term" => EntityKind::Term,
        "ambiguity" => EntityKind::Ambiguity,
        "service_level_indicator" => EntityKind::ServiceLevelIndicator,
        "service_level_objective" => EntityKind::ServiceLevelObjective,
        "alert_policy" => EntityKind::AlertPolicy,
        "alert_notification_target" => EntityKind::AlertNotificationTarget,
        "type_library" => EntityKind::TypeLibrary,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn id(ns: &str, slug: &str, version: u64) -> Id {
        Id {
            namespace: ns.into(),
            slug: slug.into(),
            version,
        }
    }

    #[test]
    fn roundtrips_simple_ascii() {
        let key = entity_key(EntityKind::Event, &id("shop", "order-placed", 3));
        assert_eq!(key, "event.shop.order-placed.3");
        let (k, parsed) = parse_entity_key(&key).unwrap();
        assert_eq!(k, EntityKind::Event);
        assert_eq!(parsed, id("shop", "order-placed", 3));
    }

    #[test]
    fn escapes_dot_in_slug() {
        let key = entity_key(EntityKind::Command, &id("shop", "order.placed", 1));
        assert!(key.starts_with("command.shop."));
        // `.` is 0x2E
        assert!(key.contains("=2E"));
        let (_, parsed) = parse_entity_key(&key).unwrap();
        assert_eq!(parsed.slug, "order.placed");
    }

    #[test]
    fn escapes_slash_in_namespace() {
        let key = entity_key(EntityKind::ReadModel, &id("team/shop", "x", 0));
        // `/` is 0x2F
        assert!(key.contains("=2F"));
        let (_, parsed) = parse_entity_key(&key).unwrap();
        assert_eq!(parsed.namespace, "team/shop");
        assert_eq!(parsed.slug, "x");
    }

    #[test]
    fn escapes_equals_in_input() {
        // `=` is the escape char itself; must round-trip.
        let key = entity_key(EntityKind::Event, &id("ns=x", "s", 0));
        assert!(key.contains("=3D"));
        let (_, parsed) = parse_entity_key(&key).unwrap();
        assert_eq!(parsed.namespace, "ns=x");
    }

    #[test]
    fn escapes_non_ascii_utf8() {
        let key = entity_key(EntityKind::Event, &id("café", "s", 0));
        // é = U+00E9 = 0xC3 0xA9 in UTF-8
        assert!(key.contains("=C3=A9"));
        let (_, parsed) = parse_entity_key(&key).unwrap();
        assert_eq!(parsed.namespace, "café");
    }

    #[test]
    fn empty_namespace_returns_none() {
        let key = entity_key(EntityKind::Event, &id("", "x", 0));
        assert_eq!(key, "event..x.0");
        // parse_entity_key rejects empty namespace/slug components. The gRPC
        // boundary enforces non-empty components before they reach storage, so
        // keys with empty components are either malformed or from a schema
        // violation; rejecting them prevents silent data corruption.
        assert!(
            parse_entity_key(&key).is_none(),
            "empty namespace must not parse as a valid entity key"
        );
    }

    #[test]
    fn prefixes_compose() {
        let p = slug_prefix(EntityKind::Event, "shop", "order");
        assert!(entity_key(EntityKind::Event, &id("shop", "order", 0)).starts_with(&p));
        let p = namespace_prefix(EntityKind::Event, "shop");
        assert!(entity_key(EntityKind::Event, &id("shop", "any", 0)).starts_with(&p));
        let p = kind_prefix(EntityKind::Event);
        assert!(entity_key(EntityKind::Event, &id("any", "any", 0)).starts_with(&p));
    }

    #[test]
    fn rejects_unknown_kind_short() {
        assert!(parse_entity_key("bogus.ns.slug.0").is_none());
    }

    #[test]
    fn branch_delta_key_roundtrips() {
        let key = branch_delta_key(
            "feature-x",
            EntityKind::Event,
            &id("shop", "order-placed", 3),
        );
        assert_eq!(key, "feature-x.event.shop.order-placed.3");
        let (kind, parsed) = parse_branch_delta_key("feature-x", &key).unwrap();
        assert_eq!(kind, EntityKind::Event);
        assert_eq!(parsed, id("shop", "order-placed", 3));
    }

    #[test]
    fn branch_delta_key_escapes_branch_name() {
        let key = branch_delta_key("team/feature", EntityKind::Command, &id("ns", "s", 0));
        assert!(key.contains("=2F"));
        let (kind, parsed) = parse_branch_delta_key("team/feature", &key).unwrap();
        assert_eq!(kind, EntityKind::Command);
        assert_eq!(parsed, id("ns", "s", 0));
    }

    #[test]
    fn branch_delta_key_wrong_branch_does_not_parse() {
        let key = branch_delta_key("branch-a", EntityKind::Event, &id("ns", "s", 0));
        assert!(parse_branch_delta_key("branch-b", &key).is_none());
    }

    #[test]
    fn reserved_branch_name_is_meta_case_insensitive() {
        assert!(is_reserved_branch_name("meta"));
        assert!(is_reserved_branch_name("META"));
        assert!(is_reserved_branch_name("MeTa"));
        assert!(!is_reserved_branch_name("meta2"));
        assert!(!is_reserved_branch_name("my-meta"));
    }

    #[test]
    fn branch_meta_key_and_delta_key_do_not_share_prefix_for_non_reserved_names() {
        // Metadata rows are enumerated by a "meta." prefix scan; every
        // delta-key prefix for a non-reserved branch name must not start
        // with "meta." (the one case that would, "meta", is rejected by
        // `is_reserved_branch_name` before it ever reaches key construction).
        for name in ["feature-x", "abc", "metaphor", "ametal"] {
            assert!(!is_reserved_branch_name(name));
            let prefix = branch_delta_prefix(name);
            assert!(
                !prefix.starts_with("meta."),
                "branch {name:?} produced delta prefix {prefix:?} colliding with meta. scan"
            );
        }
    }

    // -----------------------------------------------------------------
    // Namespace registry keys
    // -----------------------------------------------------------------

    #[test]
    fn namespace_record_key_roundtrips() {
        for id in ["orders", "ns_0199a1b2c3d4", "order.placed", "a-b_c"] {
            let key = namespace_record_key(id);
            assert!(key.starts_with(namespace_record_prefix()));
            assert_eq!(parse_namespace_record_key(&key).as_deref(), Some(id));
        }
    }

    /// A minted id must survive the key alphabet without picking up a single
    /// `=HH` escape, or every entity key in the store grows for no reason.
    #[test]
    fn minted_namespace_ids_need_no_escaping() {
        let id = trogon_atlas_core::NamespaceId::generate();
        assert_eq!(
            namespace_record_key(id.as_str()),
            format!("id.{id}"),
            "minted ids must be identity-encoded"
        );
        assert_eq!(
            namespace_prefix(EntityKind::Event, id.as_str()),
            format!("event.{id}.")
        );
    }

    /// The two row families share one bucket, so neither builder may ever
    /// produce a key the other's prefix scan would pick up.
    #[test]
    fn record_rows_and_name_index_rows_never_collide() {
        let record = namespace_record_key("name");
        assert!(!record.starts_with("name."));
        let index = namespace_name_key("id", "x");
        assert!(!index.starts_with(namespace_record_prefix()));
        assert!(parse_namespace_record_key(&index).is_none());
    }

    /// Same name under two owners is the whole point: distinct keys, so the
    /// create-based uniqueness constraint binds per owner rather than
    /// globally.
    #[test]
    fn same_name_under_different_owners_is_a_different_key() {
        assert_ne!(
            namespace_name_key("acme", "orders"),
            namespace_name_key("beta", "orders")
        );
    }

    /// Without the trailing `.` the owner prefix would leak rows belonging to
    /// any owner whose name it prefixes.
    #[test]
    fn owner_prefix_does_not_leak_into_a_longer_owner_name() {
        let prefix = namespace_name_prefix("acme");
        assert!(namespace_name_key("acme", "orders").starts_with(&prefix));
        assert!(!namespace_name_key("acme-staging", "orders").starts_with(&prefix));
    }

    /// Owner and name segments are escaped independently, so a `.` in either
    /// cannot forge a different owner's key.
    #[test]
    fn name_index_escapes_both_segments() {
        let sneaky = namespace_name_key("acme.beta", "orders");
        let honest = namespace_name_key("acme", "beta.orders");
        assert_ne!(sneaky, honest);
        assert!(sneaky.contains("=2E"));
        assert!(honest.contains("=2E"));
    }

    #[test]
    fn rejects_non_numeric_version() {
        assert!(parse_entity_key("event.ns.slug.x").is_none());
    }

    /// Every `EntityKind` variant, except `Unspecified`, which is the
    /// proto zero-value sentinel and intentionally has no on-disk
    /// representation, must round-trip through `kind_short` →
    /// `kind_from_short`. This stops the recurring "added a new entity
    /// kind, forgot to register it in the parser" bug class.
    #[test]
    fn kind_from_short_covers_every_variant() {
        use trogon_atlas_proto::canonical::ALL_KINDS;
        for &kind in ALL_KINDS {
            let short = trogon_atlas_proto::canonical::kind_short(kind);
            let parsed = super::kind_from_short(short).unwrap_or_else(|| {
                panic!("kind_from_short missing entry for {kind:?} (short={short})")
            });
            assert_eq!(parsed, kind, "round-trip failed for {kind:?}");
        }
    }

    // -----------------------------------------------------------------
    // Property-based test: branch_delta_key / parse_branch_delta_key
    // round-trip over arbitrary valid branch names and ids. The example
    // tests above already cover specific escape cases (dot, slash,
    // equals, non-ASCII, wrong branch); this generalizes across the
    // grammar accepted by trogon_atlas_server::conv::validate_branch_name
    // (single segment, or two segments joined by one '/'), plus
    // arbitrary namespace/slug/version content including characters that
    // require escaping.
    // -----------------------------------------------------------------

    mod branch_delta_key_property_tests {
        use proptest::prelude::*;

        use super::*;

        fn valid_segment() -> impl Strategy<Value = String> {
            "[a-zA-Z0-9][a-zA-Z0-9_.-]{0,20}"
        }

        fn valid_branch_name() -> impl Strategy<Value = String> {
            prop_oneof![
                valid_segment(),
                (valid_segment(), valid_segment()).prop_map(|(a, b)| format!("{a}/{b}")),
            ]
        }

        // Arbitrary text including characters outside the key-safe
        // alphabet (space, slash, dot, equals, non-ASCII) to exercise the
        // escape/unescape path, not just the identity-mapped fast path.
        fn arbitrary_component() -> impl Strategy<Value = String> {
            "[-_./= a-zA-Z0-9éü日]{1,16}"
        }

        fn arb_id() -> impl Strategy<Value = Id> {
            (arbitrary_component(), arbitrary_component(), any::<u64>()).prop_map(
                |(namespace, slug, version)| Id {
                    namespace,
                    slug,
                    version,
                },
            )
        }

        fn arb_kind() -> impl Strategy<Value = EntityKind> {
            prop::sample::select(trogon_atlas_proto::canonical::ALL_KINDS.to_vec())
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(256))]

            #[test]
            fn branch_delta_key_roundtrips_arbitrary_valid_names_and_ids(
                branch in valid_branch_name(),
                kind in arb_kind(),
                id in arb_id(),
            ) {
                let key = branch_delta_key(&branch, kind, &id);
                let parsed = parse_branch_delta_key(&branch, &key);
                prop_assert_eq!(parsed, Some((kind, id)));
            }

            /// A different (also valid) branch name must never parse the
            /// first branch's delta key, since the trailing '.' boundary on
            /// the prefix rules out one escaped name being a prefix of
            /// another's.
            #[test]
            fn branch_delta_key_does_not_parse_under_a_different_branch(
                branch_a in valid_branch_name(),
                branch_b in valid_branch_name(),
                kind in arb_kind(),
                id in arb_id(),
            ) {
                prop_assume!(branch_a != branch_b);
                let key = branch_delta_key(&branch_a, kind, &id);
                prop_assert_eq!(parse_branch_delta_key(&branch_b, &key), None);
            }
        }
    }
}
