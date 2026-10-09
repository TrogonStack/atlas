//! Semantic entity equality: the policy behind idempotent puts.
//!
//! Two entities are semantically equal when their content matches ignoring
//! the server-owned `system` block. Byte equality is the fast path, but
//! bytes over-report difference: proto map encodings are nondeterministic
//! (notably `google.protobuf.Struct` scenario payloads packed inside
//! `google.protobuf.Any`, whose `value` bytes re-encode in arbitrary map
//! order). The fallback renders both entities as canonical proto JSON via
//! the embedded descriptor pool and compares the JSON values, which is
//! key-order independent.
//!
//! Entities that cannot be rendered as JSON (e.g. `Any` payloads packed
//! under a retired type url absent from the descriptor pool) never compare
//! equal through the fallback; callers treat them as changed, which is
//! the safe direction (a write proceeds rather than being wrongly skipped).

use trogon_atlas_proto::Entity;

use crate::transcode;

/// Content equality ignoring the server-owned `system` block.
#[must_use]
pub fn semantically_equal(a: &Entity, b: &Entity) -> bool {
    let mut left = a.clone();
    left.system = None;
    let mut right = b.clone();
    right.system = None;
    if left == right {
        return true;
    }
    match (
        transcode::entity_json_value(&left),
        transcode::entity_json_value(&right),
    ) {
        (Ok(left_json), Ok(right_json)) => left_json == right_json,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

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
    fn identical_content_is_equal_regardless_of_system() {
        let a = event("T");
        let mut b = event("T");
        b.system = Some(pb::SystemMeta {
            uid: "u".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        });
        assert!(semantically_equal(&a, &b));
    }

    #[test]
    fn different_content_is_not_equal() {
        assert!(!semantically_equal(&event("A"), &event("B")));
    }

    #[test]
    fn any_packed_struct_map_order_is_ignored() {
        use prost::Message as _;
        // Two Structs with the same fields serialized in opposite entry
        // order: byte-different, semantically identical.
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
        let bytes_forward = s1.encode_to_vec();
        // Hand-build reversed encoding: field b then field a.
        let mut s2 = prost_types::Struct::default();
        s2.fields.extend([field("b", "2")]);
        let mut bytes_reversed = s2.encode_to_vec();
        let mut s3 = prost_types::Struct::default();
        s3.fields.extend([field("a", "1")]);
        bytes_reversed.extend(s3.encode_to_vec());

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
        let (a, b) = (wrap(bytes_forward.clone()), wrap(bytes_reversed.clone()));
        assert_ne!(bytes_forward, bytes_reversed, "encodings must differ");
        assert!(
            semantically_equal(&a, &b),
            "same Struct content must be equal"
        );
    }

    #[test]
    fn unresolvable_any_payload_is_not_equal_when_bytes_differ() {
        let wrap = |v: u8| Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: "ns".into(),
                    slug: "a".into(),
                    version: 1,
                }),
                metadata: vec![prost_types::Any {
                    type_url: "type.googleapis.com/retired.package.Gone".into(),
                    value: vec![v],
                }],
                ..Default::default()
            })),
        };
        assert!(!semantically_equal(&wrap(1), &wrap(2)));
        assert!(
            semantically_equal(&wrap(1), &wrap(1)),
            "byte-equal fast path"
        );
    }

    // -----------------------------------------------------------------
    // Property-based tests (proptest). Mirrors the idioms in
    // trogon-atlas-server/tests/validation_property.rs: bounded string
    // strategies, `proptest_config` case counts, `prop_assert!` rather
    // than plain `assert!` inside the macro body.
    // -----------------------------------------------------------------

    use proptest::prelude::*;

    fn bounded_text() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9 _.-]{0,24}"
    }

    fn system_meta() -> impl Strategy<Value = Option<pb::SystemMeta>> {
        prop_oneof![
            Just(None),
            bounded_text().prop_map(|uid| Some(pb::SystemMeta {
                uid,
                created_at: "2026-01-01T00:00:00Z".into(),
            })),
        ]
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
            system_meta(),
        )
            .prop_map(|(ns, slug, title, doc, system)| event_with(ns, slug, title, doc, system))
    }

    /// A packed `google.protobuf.Struct` with the given string fields,
    /// encoded verbatim in the given order (no re-sorting), wrapped as the
    /// `schema` payload of a `CommandSlice` scenario -- the same shape
    /// exercised by `any_packed_struct_map_order_is_ignored` above, but
    /// generated instead of hand-built for a property test over arbitrary
    /// field sets and orderings.
    fn command_slice_with_struct_fields(fields: &[(String, String)]) -> Entity {
        use prost::Message as _;
        let mut value = Vec::new();
        for (k, v) in fields {
            let mut s = prost_types::Struct::default();
            s.fields.insert(
                k.clone(),
                prost_types::Value {
                    kind: Some(prost_types::value::Kind::StringValue(v.clone())),
                },
            );
            value.extend(s.encode_to_vec());
        }
        Entity {
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
        }
    }

    fn distinct_field_key() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9]{0,8}"
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Reflexivity: every generated entity is semantically equal to
        /// itself, regardless of `system` or content.
        #[test]
        fn semantic_eq_is_reflexive(e in arb_event()) {
            prop_assert!(semantically_equal(&e, &e));
        }

        /// Symmetry: order of arguments never changes the verdict.
        #[test]
        fn semantic_eq_is_symmetric(a in arb_event(), b in arb_event()) {
            prop_assert_eq!(semantically_equal(&a, &b), semantically_equal(&b, &a));
        }

        /// `system` never affects the verdict: two entities with identical
        /// content but different (or absent/present) `SystemMeta` are
        /// always semantically equal, and mutating only `system` on an
        /// otherwise-fixed entity never flips the verdict against itself.
        #[test]
        fn semantic_eq_ignores_system_meta(
            ns in "[a-z][a-z0-9]{0,12}",
            slug in "[a-z][a-z0-9]{0,12}",
            title in bounded_text(),
            doc in bounded_text(),
            sys_a in system_meta(),
            sys_b in system_meta(),
        ) {
            let a = event_with(ns.clone(), slug.clone(), title.clone(), doc.clone(), sys_a);
            let b = event_with(ns, slug, title, doc, sys_b);
            prop_assert!(semantically_equal(&a, &b));
        }

        /// Struct-field insertion-order independence: packing the same set
        /// of distinct string fields into a `google.protobuf.Struct`
        /// payload in two different orders must never change byte content
        /// enough to matter -- `semantically_equal` must still hold, since
        /// map-typed protobuf fields have no canonical wire order.
        #[test]
        fn semantic_eq_struct_field_order_is_ignored(
            k1 in distinct_field_key(),
            v1 in bounded_text(),
            k2 in distinct_field_key(),
            v2 in bounded_text(),
        ) {
            prop_assume!(k1 != k2);
            let forward = command_slice_with_struct_fields(&[
                (k1.clone(), v1.clone()),
                (k2.clone(), v2.clone()),
            ]);
            let reversed = command_slice_with_struct_fields(&[(k2, v2), (k1, v1)]);
            prop_assert!(semantically_equal(&forward, &reversed));
        }
    }
}
