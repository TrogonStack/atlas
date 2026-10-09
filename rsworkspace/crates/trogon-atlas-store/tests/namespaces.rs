//! Namespace registry integration tests against a real NATS server with
//! `JetStream`.
//!
//! The registry exists to make two guarantees that unit tests cannot reach,
//! because both of them are properties of the storage layer rather than of
//! any Rust value:
//!
//! * **Collision-free between owners.** Two owners can hold namespaces with
//!   the same human name, and the ids they get are different, so their
//!   entity keys never overlap.
//! * **Moving ownership is not a rekey.** A move rewrites one registry row.
//!   The namespace id, and therefore every entity key derived from it, is
//!   untouched.
//!
//! Both are asserted here directly rather than inferred from the API shape.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_core::{NamespaceId, NamespaceName, OwnerId};
use trogon_atlas_store::{NamespaceTenure, Store, StoreError};

fn name(value: &str) -> NamespaceName {
    NamespaceName::parse(value).unwrap()
}

fn owner(value: &str) -> OwnerId {
    OwnerId::parse(value).unwrap()
}

/// The headline requirement: two tenants, one human name, no collision.
#[tokio::test]
async fn two_owners_can_hold_the_same_name_with_distinct_ids() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    let acme = store
        .register_namespace(&name("orders"), &owner("acme"), "test")
        .await
        .unwrap();
    let beta = store
        .register_namespace(&name("orders"), &owner("beta"), "test")
        .await
        .unwrap();

    assert!(acme.created);
    assert!(beta.created);
    assert_ne!(
        acme.record.id, beta.record.id,
        "two owners must never be handed the same namespace id",
    );
    assert!(acme.record.id.is_minted());
    assert!(beta.record.id.is_minted());
    assert_eq!(acme.record.name, beta.record.name);
}

/// Resolution is per owner, which is what makes the shared name usable: each
/// tenant types `orders` and reaches its own.
#[tokio::test]
async fn resolution_is_scoped_to_one_owner() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    let acme = store
        .register_namespace(&name("catalog"), &owner("acme"), "test")
        .await
        .unwrap();
    let beta = store
        .register_namespace(&name("catalog"), &owner("beta"), "test")
        .await
        .unwrap();

    assert_eq!(
        store
            .resolve_namespace(&owner("acme"), &name("catalog"))
            .await
            .unwrap(),
        Some(acme.record.id),
    );
    assert_eq!(
        store
            .resolve_namespace(&owner("beta"), &name("catalog"))
            .await
            .unwrap(),
        Some(beta.record.id),
    );
    assert_eq!(
        store
            .resolve_namespace(&owner("nobody"), &name("catalog"))
            .await
            .unwrap(),
        None,
        "an owner that registered nothing must resolve nothing",
    );
}

/// Registering a name an owner already holds returns the existing row rather
/// than minting a second id. Without this, a retried claim would silently
/// strand the first namespace.
#[tokio::test]
async fn registering_twice_is_idempotent() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    let first = store
        .register_namespace(&name("billing"), &owner("acme"), "test")
        .await
        .unwrap();
    let second = store
        .register_namespace(&name("billing"), &owner("acme"), "someone-else")
        .await
        .unwrap();

    assert!(first.created);
    assert!(
        !second.created,
        "the second claim must not create a new row"
    );
    assert_eq!(first.record.id, second.record.id);
    assert_eq!(
        second.record.created_by, "test",
        "the original claimant stays recorded",
    );
}

/// The second requirement: a move is one registry row, and in particular the
/// id -- which every entity key is built from -- does not change.
#[tokio::test]
async fn moving_an_owner_leaves_the_id_alone() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    let claim = store
        .register_namespace(&name("shipping"), &owner("acme"), "test")
        .await
        .unwrap();
    let original_id = claim.record.id.clone();

    let moved = store
        .move_namespace(&original_id, &owner("beta"))
        .await
        .unwrap();

    assert_eq!(moved.id, original_id, "a move must never rekey");
    assert_eq!(moved.parent, owner("beta"));
    assert_eq!(moved.name, name("shipping"));

    // The name index followed the record, in both directions.
    assert_eq!(
        store
            .resolve_namespace(&owner("beta"), &name("shipping"))
            .await
            .unwrap(),
        Some(original_id.clone()),
    );
    assert_eq!(
        store
            .resolve_namespace(&owner("acme"), &name("shipping"))
            .await
            .unwrap(),
        None,
        "the old owner must stop resolving the moved name",
    );
    assert_eq!(
        store
            .get_namespace(&original_id)
            .await
            .unwrap()
            .unwrap()
            .parent,
        owner("beta"),
    );
}

/// A move into an owner that already holds that name would make resolution
/// ambiguous, so it is refused and the source is left untouched.
#[tokio::test]
async fn a_move_that_would_collide_is_refused_and_changes_nothing() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    let source = store
        .register_namespace(&name("returns"), &owner("acme"), "test")
        .await
        .unwrap();
    store
        .register_namespace(&name("returns"), &owner("beta"), "test")
        .await
        .unwrap();

    let err = store
        .move_namespace(&source.record.id, &owner("beta"))
        .await
        .expect_err("moving onto an occupied name must fail");
    assert!(
        matches!(err, StoreError::AlreadyExists),
        "expected AlreadyExists, got {err:?}",
    );

    // The source survived intact: a rejected move must not consume the row.
    let after = store
        .get_namespace(&source.record.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.parent, owner("acme"));
    assert_eq!(
        store
            .resolve_namespace(&owner("acme"), &name("returns"))
            .await
            .unwrap(),
        Some(source.record.id),
        "the source owner must still resolve its own namespace",
    );
}

/// Adoption is what the backfill and claim-on-first-write both use: the id
/// equals the name, so entity keys already written under that name stay
/// valid.
#[tokio::test]
async fn adoption_keeps_the_bare_name_as_the_id() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    let claim = store
        .adopt_namespace(
            &name("legacy-context"),
            &owner("default"),
            &NamespaceTenure::Permanent,
        )
        .await
        .unwrap();

    assert!(claim.created);
    assert_eq!(claim.record.id.as_str(), "legacy-context");
    assert!(!claim.record.id.is_minted());
    assert_eq!(claim.record.name.as_str(), "legacy-context");
}

/// Re-running the backfill must not disturb what it already wrote. This is
/// the property that makes the migration safe to run against a live store.
#[tokio::test]
async fn adoption_is_idempotent() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    let first = store
        .adopt_namespace(
            &name("rerun"),
            &owner("default"),
            &NamespaceTenure::Permanent,
        )
        .await
        .unwrap();
    let second = store
        .adopt_namespace(
            &name("rerun"),
            &owner("default"),
            &NamespaceTenure::Permanent,
        )
        .await
        .unwrap();

    assert!(first.created);
    assert!(!second.created);
    assert_eq!(first.record.id, second.record.id);
    assert_eq!(first.record.created_at, second.record.created_at);
}

/// The boundary between the convenience path and the explicit one. Adoption
/// cannot mint, so the second owner to want a bare id is refused, and
/// `register_namespace` is the answer for them.
#[tokio::test]
async fn adoption_refuses_an_id_another_owner_already_holds() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    store
        .adopt_namespace(
            &name("contested"),
            &owner("acme"),
            &NamespaceTenure::Permanent,
        )
        .await
        .unwrap();

    let err = store
        .adopt_namespace(
            &name("contested"),
            &owner("beta"),
            &NamespaceTenure::Permanent,
        )
        .await
        .expect_err("a second owner must not be handed an id that is taken");
    assert!(
        matches!(err, StoreError::AlreadyExists),
        "expected AlreadyExists, got {err:?}",
    );

    // ...and the explicit path still works for them, with a distinct id.
    let minted = store
        .register_namespace(&name("contested"), &owner("beta"), "test")
        .await
        .unwrap();
    assert!(minted.created);
    assert!(minted.record.id.is_minted());
    assert_ne!(minted.record.id.as_str(), "contested");
}

/// Minted and adopted ids share one bucket, and listing returns both. That
/// coexistence is the reason the backfill can leave 3000 entity keys alone.
#[tokio::test]
async fn minted_and_adopted_ids_coexist_in_one_listing() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    let adopted = store
        .adopt_namespace(
            &name("coexist-old"),
            &owner("acme"),
            &NamespaceTenure::Permanent,
        )
        .await
        .unwrap();
    let minted = store
        .register_namespace(&name("coexist-new"), &owner("acme"), "test")
        .await
        .unwrap();

    let listed = store.list_namespaces().await.unwrap();
    let ids: Vec<&str> = listed.iter().map(|r| r.id.as_str()).collect();
    assert!(ids.contains(&adopted.record.id.as_str()));
    assert!(ids.contains(&minted.record.id.as_str()));

    for record in &listed {
        assert_eq!(
            store
                .get_namespace(&record.id)
                .await
                .unwrap()
                .as_ref()
                .map(|r| &r.id),
            Some(&record.id),
            "every listed id must be readable by id",
        );
    }
}

#[tokio::test]
async fn unknown_ids_and_names_read_as_absent() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    assert!(store
        .get_namespace(&NamespaceId::parse("ns_nothinghere").unwrap())
        .await
        .unwrap()
        .is_none());
    assert!(store
        .resolve_namespace(&owner("ghost"), &name("ghost"))
        .await
        .unwrap()
        .is_none());
}

/// A namespace name containing the key separator must not be able to escape
/// its owner's index prefix and impersonate a row under another owner.
#[tokio::test]
async fn dotted_names_stay_inside_their_owner() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    let claim = store
        .register_namespace(&name("orders.v2"), &owner("acme"), "test")
        .await
        .unwrap();

    assert_eq!(
        store
            .resolve_namespace(&owner("acme"), &name("orders.v2"))
            .await
            .unwrap(),
        Some(claim.record.id),
    );
    // `acme` + `orders.v2` must not be reachable as `acme.orders` + `v2`.
    assert_eq!(
        store
            .resolve_namespace(&owner("acme.orders"), &name("v2"))
            .await
            .unwrap(),
        None,
    );
}
