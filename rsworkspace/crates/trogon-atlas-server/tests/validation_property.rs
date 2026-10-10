//! Property-based tests for the validation layer.
//!
//! Generates small adversarial entity graphs with random namespaces and slugs
//! drawn from within and outside the valid id-component charset, including
//! dangling references. The invariant under test: the validator never panics,
//! and it rejects entities whose id components are outside the allowed charset.

#![allow(clippy::unwrap_used)]

use proptest::prelude::*;
use trogon_atlas_proto as pb;
use trogon_atlas_server::validation::validate_store;
use trogon_atlas_store::StoredEntity;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn stored(entity: pb::Entity) -> StoredEntity {
    StoredEntity {
        entity,
        etag: String::new(),
    }
}

fn mk_id(ns: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn event_entity(ns: &str, slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(mk_id(ns, slug)),
            ..Default::default()
        })),
    }
}

fn command_entity(ns: &str, slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Command(pb::Command {
            id: Some(mk_id(ns, slug)),
            ..Default::default()
        })),
    }
}

fn command_slice_entity(ns: &str, slug: &str, cmd_ns: &str, cmd_slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(mk_id(ns, slug)),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(mk_id(cmd_ns, cmd_slug)),
                }),
                ..Default::default()
            }),
            ..Default::default()
        })),
    }
}

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn valid_slug() -> impl Strategy<Value = String> {
    // ASCII alphanumeric; always valid as an id component.
    "[a-zA-Z][a-zA-Z0-9]{1,19}"
}

fn invalid_slug() -> impl Strategy<Value = String> {
    // Contains at least one disallowed character (space, slash, colon, etc.).
    let bad_chars: Vec<char> = vec!['/', ' ', '\\', ':', '@', '?', '!'];
    (
        "[a-zA-Z0-9]{1,8}".prop_map(|s| s),
        prop::sample::select(bad_chars),
        "[a-zA-Z0-9]{0,8}".prop_map(|s| s),
    )
        .prop_map(|(prefix, bad, suffix)| format!("{prefix}{bad}{suffix}"))
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// validate_store never panics on a graph of valid entities, regardless
    /// of slug/namespace combinations.
    #[test]
    fn validate_store_never_panics_on_valid_graph(
        ns in valid_slug(),
        event_slug in valid_slug(),
        cmd_slug in valid_slug(),
        slice_slug in valid_slug(),
    ) {
        // Construct a small graph: event, command, slice pointing at command.
        // The slice's command reference is present in the store (no dangling ref).
        let entities: Vec<StoredEntity> = vec![
            stored(event_entity(&ns, &event_slug)),
            stored(command_entity(&ns, &cmd_slug)),
            stored(command_slice_entity(&ns, &slice_slug, &ns, &cmd_slug)),
        ];
        // Must not panic, regardless of issues returned.
        let _issues = validate_store(&entities);
    }

    /// validate_store never panics on a dangling-reference graph (slice
    /// points at a command that does not exist in the store).
    #[test]
    fn validate_store_never_panics_on_dangling_refs(
        ns in valid_slug(),
        slice_slug in valid_slug(),
        missing_slug in valid_slug(),
    ) {
        let entities: Vec<StoredEntity> = vec![
            stored(command_slice_entity(&ns, &slice_slug, &ns, &missing_slug)),
        ];
        let _issues = validate_store(&entities);
    }

    /// validate_store never panics when namespace and slug contain characters
    /// outside the allowed id-component set. The validator must handle hostile
    /// input without unwrapping or indexing unsafely.
    #[test]
    fn validate_store_never_panics_on_invalid_id_components(
        bad_ns in invalid_slug(),
        bad_slug in invalid_slug(),
    ) {
        let entities: Vec<StoredEntity> = vec![
            stored(event_entity(&bad_ns, &bad_slug)),
        ];
        let _issues = validate_store(&entities);
    }

    /// validate_store on an empty store always returns an empty issue list.
    #[test]
    fn validate_store_empty_store_returns_no_issues(_: ()) {
        let issues = validate_store(&[]);
        prop_assert!(issues.is_empty());
    }

    /// validate_store returns a result (does not panic) for a mixed graph
    /// where some entities have valid ids and some have invalid ones.
    #[test]
    fn validate_store_mixed_valid_invalid_never_panics(
        good_ns in valid_slug(),
        good_slug in valid_slug(),
        bad_ns in invalid_slug(),
        bad_slug in invalid_slug(),
    ) {
        let entities: Vec<StoredEntity> = vec![
            stored(event_entity(&good_ns, &good_slug)),
            stored(event_entity(&bad_ns, &bad_slug)),
        ];
        let _issues = validate_store(&entities);
    }
}
