//! `SpiceDbAuthorizer` against a real SpiceDB, because the interesting claims
//! are all claims about what SpiceDB does.
//!
//! A mock would let this file assert that the right requests go out, which is
//! not the question. The question is whether the schema in `spicedb::SCHEMA`
//! actually grants what the code assumes it grants: that a member of an
//! organisation reaches every namespace under it, that a share reaches
//! exactly one, and that a move takes the old owner's access away in the same
//! instant it hands it over.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use trogon_atlas_authzed::{
    ObjectId, ObjectRef, ObjectType, PresharedKey, Relation, Relationship, RelationshipOp,
    RelationshipUpdate, SpiceDb, SpiceDbConfig, SubjectRef,
};
use trogon_atlas_core::{NamespaceId, NamespaceName, OwnerId};
use trogon_atlas_server::{
    ownership::{Lens, NamespaceAuthorizer, NamespaceDirectory, Visibility, WriteVerdict},
    spicedb::{Freshness, SpiceDbAuthorizer},
};
use trogon_atlas_store::Store;

/// Everything a single test needs: its own namespace names, its own owners,
/// and its own principals, so tests can share one SpiceDB and one NATS
/// container without seeing each other's relationships.
struct World {
    tag: String,
    store: Arc<dyn Store>,
    authorizer: SpiceDbAuthorizer,
    raw: SpiceDb,
    _serialized: tokio::sync::MutexGuard<'static, ()>,
}

/// The schema is the same for every test and the in-memory datastore has
/// little write throughput, so installing it once keeps ten concurrent tests
/// from stampeding it.
static SCHEMA: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// One test at a time against the shared SpiceDB.
///
/// SpiceDB's default datastore is in-memory and documented as a testing
/// convenience rather than a production one, and under concurrent writes it
/// drops read-your-writes: a check can miss a relationship whose write has
/// already returned, at every consistency level including fully consistent.
/// Measured against this container, a storm of concurrent writers produced a
/// few such misses per five hundred checks. Thirteen tests sharing one
/// container is a mild version of that storm, and it showed up as roughly one
/// spurious failure in twenty-five runs.
///
/// None of these tests are about concurrency, so serialising them removes the
/// confound rather than papering over it. Every assertion is unchanged, and
/// the whole file still runs in a couple of seconds. Retrying the checks
/// instead would have hidden exactly the regressions the file exists to
/// catch.
static SERIALIZED: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl World {
    /// A world whose checks are quorum reads.
    ///
    /// Most tests want this: they grant something through `raw` and check it
    /// through `authorizer`, which are two clients, so anything weaker would
    /// be asserting against SpiceDB's replication timing rather than against
    /// the schema.
    async fn new() -> Self {
        Self::with_freshness(Freshness::FullyConsistent).await
    }

    async fn with_freshness(freshness: Freshness) -> Self {
        let serialized = SERIALIZED.lock().await;
        let tag = format!("t{}", trogon_atlas_testsupport::unique_suffix_for_test());
        let store: Arc<dyn Store> =
            Arc::new(trogon_atlas_testsupport::shared().await.store().await);
        let spicedb = trogon_atlas_testsupport::shared_spicedb().await;
        let config = SpiceDbConfig::new(
            spicedb.endpoint.clone(),
            PresharedKey::parse(trogon_atlas_testsupport::SPICEDB_PRESHARED_KEY).unwrap(),
        );
        let authorizer = SpiceDbAuthorizer::new(
            &config,
            Arc::new(NamespaceDirectory::new()),
            store.clone(),
            freshness,
        )
        .unwrap();
        SCHEMA
            .get_or_init(|| async {
                authorizer.reconcile(&[]).await.expect("schema install");
            })
            .await;
        Self {
            tag,
            store,
            authorizer,
            raw: SpiceDb::connect(&config).unwrap(),
            _serialized: serialized,
        }
    }

    fn owner(&self, label: &str) -> OwnerId {
        OwnerId::parse(&format!("{}-{label}", self.tag)).unwrap()
    }

    fn principal(&self, label: &str) -> Arc<str> {
        Arc::from(format!("{}-{label}", self.tag).as_str())
    }

    /// Register a namespace and publish the grant, which is the pair of
    /// writes the service performs on `RegisterNamespace`.
    async fn register(&self, name: &str, owner: &OwnerId) -> NamespaceId {
        let claim = self
            .store
            .register_namespace(
                &NamespaceName::parse(&format!("{}-{name}", self.tag)).unwrap(),
                owner,
                "test",
            )
            .await
            .unwrap();
        self.authorizer
            .on_registered(&claim.record)
            .await
            .expect("publishing the grant");
        claim.record.id
    }

    async fn join(&self, principal: &Arc<str>, owner: &OwnerId) {
        self.write(
            RelationshipOp::Touch,
            &format!("organization:{}#member@user:{principal}", owner.as_str()),
        )
        .await;
    }

    /// Whether SpiceDB currently believes the principal is in the owner, read
    /// straight from the raw client. Only used to say something useful when
    /// an assertion about a share fails, since a missing membership and a
    /// missing share fail identically.
    async fn membership(&self, principal: &Arc<str>, owner: &OwnerId) -> bool {
        self.raw
            .check_permission(
                &object(&format!("organization:{}", owner.as_str())),
                &Relation::parse("membership").unwrap(),
                &SubjectRef::new(object(&format!("user:{principal}"))),
                &trogon_atlas_authzed::Consistency::FullyConsistent,
            )
            .await
            .expect("checking membership")
            .is_granted()
    }

    /// Write a relationship spelled the way SpiceDB spells it, so the tests
    /// read as the grants they are rather than as four constructor calls.
    async fn write(&self, op: RelationshipOp, spec: &str) {
        let (resource, rest) = spec.split_once('#').expect("resource#relation@subject");
        let (relation, subject) = rest.split_once('@').expect("resource#relation@subject");
        self.raw
            .write_relationships(&[RelationshipUpdate::new(
                op,
                Relationship::new(object(resource), Relation::parse(relation).unwrap(), {
                    match subject.split_once('#') {
                        Some((object_part, subject_relation)) => SubjectRef::with_relation(
                            object(object_part),
                            Relation::parse(subject_relation).unwrap(),
                        ),
                        None => SubjectRef::new(object(subject)),
                    }
                }),
            )])
            .await
            .expect("writing a relationship");
    }
}

fn visibility(principal: &Arc<str>, owner: &OwnerId) -> Visibility {
    Visibility::owned_by(principal.clone(), owner.clone())
}

fn object(spec: &str) -> ObjectRef {
    let (object_type, object_id) = spec.split_once(':').expect("type:id");
    ObjectRef::new(
        ObjectType::parse(object_type).unwrap(),
        ObjectId::parse(object_id).unwrap(),
    )
}

fn seen(lens: &Lens, id: &NamespaceId) -> bool {
    lens.admits(id.as_str())
}

/// The baseline the registry already provided: a principal in an owner sees
/// that owner's namespaces and no others.
#[tokio::test]
async fn a_member_sees_its_own_owners_namespaces_and_nothing_else() {
    let world = World::new().await;
    let acme = world.owner("acme");
    let beta = world.owner("beta");
    let alice = world.principal("alice");
    world.join(&alice, &acme).await;

    let ours = world.register("orders", &acme).await;
    let theirs = world.register("orders", &beta).await;

    let vis = visibility(&alice, &acme);
    assert!(world.authorizer.admits(&vis, &ours).await.unwrap());
    assert!(!world.authorizer.admits(&vis, &theirs).await.unwrap());

    let lens = world.authorizer.lens(&vis).await.unwrap();
    assert!(seen(&lens, &ours));
    assert!(
        !seen(&lens, &theirs),
        "the lens must not leak another owner's namespace of the same name",
    );
}

/// The thing the registry could not express, and the reason for all of this:
/// one namespace, visible to a second organisation, without moving it.
#[tokio::test]
async fn a_namespace_can_be_shared_with_a_second_organization() {
    let world = World::new().await;
    let acme = world.owner("acme");
    let beta = world.owner("beta");
    let bob = world.principal("bob");
    world.join(&bob, &beta).await;

    let shared = world.register("catalog", &acme).await;
    let private = world.register("payroll", &acme).await;

    let vis = visibility(&bob, &beta);
    assert!(!world.authorizer.admits(&vis, &shared).await.unwrap());

    world
        .write(
            RelationshipOp::Touch,
            &format!(
                "namespace:{}#viewer@organization:{}#member",
                shared.as_str(),
                beta.as_str(),
            ),
        )
        .await;

    assert!(
        world.authorizer.admits(&vis, &shared).await.unwrap(),
        "bob is a member of {beta}: {}",
        world.membership(&bob, &beta).await,
    );
    assert!(
        !world.authorizer.admits(&vis, &private).await.unwrap(),
        "sharing one namespace must not share the rest of the owner's",
    );

    let lens = world.authorizer.lens(&vis).await.unwrap();
    assert!(seen(&lens, &shared));
    assert!(!seen(&lens, &private));
}

/// A viewer may read and must not write, which is the half of the split that
/// the registry's single `parent` column cannot represent at all.
#[tokio::test]
async fn a_viewer_reads_but_does_not_write_and_an_editor_does_both() {
    let world = World::new().await;
    let acme = world.owner("acme");
    let beta = world.owner("beta");
    let reader = world.principal("reader");
    let writer = world.principal("writer");
    world.join(&reader, &beta).await;
    world.join(&writer, &beta).await;

    let id = world.register("ledger", &acme).await;
    world
        .write(
            RelationshipOp::Touch,
            &format!("namespace:{}#viewer@user:{reader}", id.as_str()),
        )
        .await;
    world
        .write(
            RelationshipOp::Touch,
            &format!("namespace:{}#editor@user:{writer}", id.as_str()),
        )
        .await;

    let as_reader = visibility(&reader, &beta);
    let as_writer = visibility(&writer, &beta);

    assert!(world.authorizer.admits(&as_reader, &id).await.unwrap());
    assert_eq!(
        world.authorizer.may_write(&as_reader, &id).await.unwrap(),
        WriteVerdict::Denied {
            owner: acme.clone()
        },
    );

    assert!(world.authorizer.admits(&as_writer, &id).await.unwrap());
    assert_eq!(
        world.authorizer.may_write(&as_writer, &id).await.unwrap(),
        WriteVerdict::Allowed,
    );
}

/// A move hands over read and write together, and takes them away from the
/// old owner in the same transaction, so there is no instant where both or
/// neither can see it.
#[tokio::test]
async fn a_move_transfers_access_without_leaving_the_old_owner_behind() {
    let world = World::new().await;
    let acme = world.owner("acme");
    let beta = world.owner("beta");
    let alice = world.principal("alice");
    let bob = world.principal("bob");
    world.join(&alice, &acme).await;
    world.join(&bob, &beta).await;

    let id = world.register("shipping", &acme).await;
    let as_alice = visibility(&alice, &acme);
    let as_bob = visibility(&bob, &beta);
    assert!(world.authorizer.admits(&as_alice, &id).await.unwrap());
    assert!(!world.authorizer.admits(&as_bob, &id).await.unwrap());

    let moved = world.store.move_namespace(&id, &beta).await.unwrap();
    world.authorizer.on_moved(&moved, &acme).await.unwrap();

    assert!(!world.authorizer.admits(&as_alice, &id).await.unwrap());
    assert!(world.authorizer.admits(&as_bob, &id).await.unwrap());
    assert_eq!(
        world.authorizer.may_write(&as_alice, &id).await.unwrap(),
        WriteVerdict::Denied { owner: beta },
    );
    assert_eq!(moved.id, id, "a move must never rekey");
}

/// The registry, not SpiceDB, answers whether a namespace exists. Conflating
/// "no grant" with "no such namespace" would turn claim-on-first-write into a
/// permission error for a name nobody holds.
#[tokio::test]
async fn a_namespace_with_no_registry_row_is_unclaimed_rather_than_denied() {
    let world = World::new().await;
    let acme = world.owner("acme");
    let alice = world.principal("alice");
    world.join(&alice, &acme).await;

    let unknown = NamespaceId::parse(&format!("{}-never-registered", world.tag)).unwrap();
    assert_eq!(
        world
            .authorizer
            .may_write(&visibility(&alice, &acme), &unknown)
            .await
            .unwrap(),
        WriteVerdict::Unclaimed,
    );
}

/// The reason for the `=` escape: a dotted namespace name is a legal
/// `NamespaceId` and an illegal SpiceDB object id, so it has to survive the
/// round trip through both the check and the listing.
#[tokio::test]
async fn dotted_namespace_ids_survive_the_encoding() {
    let world = World::new().await;
    let acme = world.owner("acme");
    let alice = world.principal("alice");
    world.join(&alice, &acme).await;

    let claim = world
        .store
        .adopt_namespace(
            &NamespaceName::parse(&format!("{}.orders.v2", world.tag)).unwrap(),
            &acme,
            &trogon_atlas_store::NamespaceTenure::Permanent,
        )
        .await
        .unwrap();
    assert!(claim.record.id.as_str().contains('.'));
    world.authorizer.on_registered(&claim.record).await.unwrap();

    let vis = visibility(&alice, &acme);
    assert!(world
        .authorizer
        .admits(&vis, &claim.record.id)
        .await
        .unwrap());
    assert!(
        seen(
            &world.authorizer.lens(&vis).await.unwrap(),
            &claim.record.id
        ),
        "the lens must decode back to the dotted id it encoded",
    );
}

/// Nested organisations, which the registry's flat `parent` column cannot
/// express: a member of the parent reaches what the child owns.
#[tokio::test]
async fn an_organization_hierarchy_grants_downwards_only() {
    let world = World::new().await;
    let group = world.owner("group");
    let division = world.owner("division");
    let boss = world.principal("boss");
    let staff = world.principal("staff");
    world.join(&boss, &group).await;
    world.join(&staff, &division).await;
    world
        .write(
            RelationshipOp::Touch,
            &format!(
                "organization:{}#parent@organization:{}",
                division.as_str(),
                group.as_str(),
            ),
        )
        .await;

    let below = world.register("subsidiary-books", &division).await;
    let above = world.register("group-books", &group).await;

    let as_boss = visibility(&boss, &group);
    let as_staff = visibility(&staff, &division);

    assert!(world.authorizer.admits(&as_boss, &below).await.unwrap());
    assert_eq!(
        world.authorizer.may_write(&as_boss, &below).await.unwrap(),
        WriteVerdict::Allowed,
    );
    assert!(
        !world.authorizer.admits(&as_staff, &above).await.unwrap(),
        "membership must not travel upwards",
    );
}

/// `reconcile` is the repair path, so it has to be able to fix a namespace
/// whose registry row landed while its grant did not.
#[tokio::test]
async fn reconcile_repairs_a_registry_row_whose_grant_never_landed() {
    let world = World::new().await;
    let acme = world.owner("acme");
    let alice = world.principal("alice");

    // Registered without publishing the grant, which is exactly the state a
    // crash between the two writes leaves behind.
    let claim = world
        .store
        .register_namespace(
            &NamespaceName::parse(&format!("{}-orphan", world.tag)).unwrap(),
            &acme,
            "test",
        )
        .await
        .unwrap();
    let vis = visibility(&alice, &acme);
    world.join(&alice, &acme).await;
    assert!(
        !world
            .authorizer
            .admits(&vis, &claim.record.id)
            .await
            .unwrap(),
        "the unpublished namespace must fail closed, not open",
    );

    let report = world
        .authorizer
        .reconcile(&[(alice.clone(), acme.clone())])
        .await
        .unwrap();
    assert!(report.namespaces > 0);
    assert_eq!(report.memberships, 1);
    assert!(world
        .authorizer
        .admits(&vis, &claim.record.id)
        .await
        .unwrap());
}

/// The token file is authoritative for membership, not merely a source for
/// it: editing a principal's `parent` and re-running the sync has to take the
/// old organisation away, or every reassignment would quietly accumulate
/// access instead of transferring it.
#[tokio::test]
async fn reconcile_revokes_a_membership_the_token_file_no_longer_declares() {
    let world = World::new().await;
    let acme = world.owner("acme");
    let beta = world.owner("beta");
    let alice = world.principal("alice");

    let stays = world.register("acme-side", &acme).await;
    let arrives = world.register("beta-side", &beta).await;

    world
        .authorizer
        .reconcile(&[(alice.clone(), acme.clone())])
        .await
        .unwrap();

    // The lens is read through `acme`, the owner the token file used to
    // declare, so it keeps reporting what SpiceDB believes rather than what
    // the token file now says.
    let stale = visibility(&alice, &acme);
    assert!(world.authorizer.admits(&stale, &stays).await.unwrap());

    world
        .authorizer
        .reconcile(&[(alice.clone(), beta.clone())])
        .await
        .unwrap();

    assert!(
        !world.authorizer.admits(&stale, &stays).await.unwrap(),
        "the old organisation must be gone, not merely joined by the new one",
    );
    assert!(
        world
            .authorizer
            .admits(&visibility(&alice, &beta), &arrives)
            .await
            .unwrap(),
        "and the new organisation must have arrived",
    );
}

/// Revoking one principal's membership must not disturb anybody else's, which
/// is the difference between a per-subject delete and a bulk one.
#[tokio::test]
async fn revocation_is_scoped_to_the_principal_being_reconciled() {
    let world = World::new().await;
    let acme = world.owner("acme");
    let beta = world.owner("beta");
    let alice = world.principal("alice");
    let bob = world.principal("bob");

    let id = world.register("shared", &acme).await;
    world
        .authorizer
        .reconcile(&[(alice.clone(), acme.clone()), (bob.clone(), acme.clone())])
        .await
        .unwrap();

    world
        .authorizer
        .reconcile(&[(alice.clone(), beta.clone())])
        .await
        .unwrap();

    assert!(
        world
            .authorizer
            .admits(&visibility(&bob, &acme), &id)
            .await
            .unwrap(),
        "bob was not named in the second sync and must be untouched",
    );
}

/// Reconciliation owns `organization#member` and nothing else. A namespace
/// shared directly with a principal is an operator's deliberate grant, and a
/// sync that quietly swept it away would make sharing unusable.
#[tokio::test]
async fn reconcile_leaves_direct_shares_alone() {
    let world = World::new().await;
    let acme = world.owner("acme");
    let beta = world.owner("beta");
    let alice = world.principal("alice");

    let id = world.register("shared-directly", &acme).await;
    world
        .write(
            RelationshipOp::Touch,
            &format!("namespace:{}#viewer@user:{alice}", id.as_str()),
        )
        .await;

    world
        .authorizer
        .reconcile(&[(alice.clone(), beta.clone())])
        .await
        .unwrap();

    assert!(
        world
            .authorizer
            .admits(&visibility(&alice, &beta), &id)
            .await
            .unwrap(),
        "the direct share must survive a membership reconciliation",
    );
}

/// An unrestricted principal never reaches SpiceDB at all, which is what
/// keeps an unscoped deployment working with SpiceDB configured but empty.
#[tokio::test]
async fn an_unrestricted_principal_short_circuits() {
    let world = World::new().await;
    let acme = world.owner("acme");
    let id = world.register("anything", &acme).await;
    let vis = Visibility::unrestricted(world.principal("root"));

    assert!(world.authorizer.admits(&vis, &id).await.unwrap());
    assert!(world.authorizer.lens(&vis).await.unwrap().is_unrestricted());
    assert_eq!(
        world.authorizer.may_write(&vis, &id).await.unwrap(),
        WriteVerdict::Allowed,
    );
}

/// A namespace registered at the default freshness is visible to the very
/// next check, which is the property the register-then-read path depends on.
///
/// This asserts the behaviour, not the mechanism. The floor is what delivers
/// it against a real deployment, but SpiceDB's in-memory test datastore
/// answers a minimize-latency read from head, so removing the floor does not
/// fail this test. `resolve` is unit-tested for that.
#[tokio::test]
async fn a_freshly_written_grant_is_visible_to_the_next_check() {
    let world = World::with_freshness(Freshness::MinimizeLatency).await;
    let acme = world.owner("acme");
    let alice = world.principal("alice");
    world.join(&alice, &acme).await;
    let vis = visibility(&alice, &acme);

    for round in 0..5 {
        let id = world.register(&format!("race-{round}"), &acme).await;
        assert!(
            world.authorizer.admits(&vis, &id).await.unwrap(),
            "round {round}: the grant written a moment ago was not visible",
        );
    }
}
