//! Ownership decided by SpiceDB instead of by the namespace registry.
//!
//! # What moves and what does not
//!
//! Only the decision moves. The registry stays the source of truth for a
//! namespace's id, its human name, and whether it exists at all; those are
//! storage facts, and a permission system is the wrong place to keep them.
//! What SpiceDB takes over is the single question
//! [`NamespaceAuthorizer`](crate::ownership::NamespaceAuthorizer) asks: may
//! this caller see, or write, this namespace.
//!
//! The gain is not a different answer to today's question. It is that the
//! question can now have answers the registry cannot express, because the
//! registry only ever stores one owner per namespace: sharing a namespace
//! with a second organisation, granting one API key read access to one
//! namespace, or letting a parent organisation reach into its children.
//!
//! # Resource granularity is the namespace
//!
//! Never the entity. A check per entity would be thousands of round trips on
//! a single listing, and it would buy nothing: the filter is per namespace
//! either way. So a read resolves the caller's whole namespace set once
//! (`LookupResources`) and filters locally, and a write checks one namespace
//! (`CheckPermission`).
//!
//! # Where identity comes from
//!
//! The token registry file, still. SpiceDB holds namespace-to-organisation
//! and namespace-sharing edges; the token file holds which principal belongs
//! to which organisation, and [`SpiceDbAuthorizer::reconcile`] copies that
//! into SpiceDB as `organization:<parent>#member@user:<principal>`.
//!
//! The file is authoritative for that edge, not merely a source for it:
//! reconciliation replaces a principal's memberships rather than adding to
//! them, so moving a principal from one organisation to another in the file
//! actually takes the old organisation away. Sharing is expressed with
//! `namespace#viewer` and `namespace#editor`, which reconciliation never
//! touches, so nothing an operator grants there is at risk from this.
//!
//! A principal deleted from the file outright keeps its membership edge,
//! because reconciliation never learns the name to delete. That is safe
//! rather than merely tolerable: authentication happens before any of this,
//! so a principal with no token never presents an identity for the stale edge
//! to match.
//!
//! # Ordering, and the window it leaves
//!
//! A namespace is registered before its grant is written, because the id the
//! grant refers to does not exist until the registry mints it. Between the
//! two, the namespace exists and nobody can see it. That fails closed, and it
//! is repairable two ways: the failing RPC returns an error so the caller
//! retries (registration is idempotent), and `reconcile` re-derives every
//! grant from the registry.
//!
//! The reverse order would be worse in kind rather than in degree: a grant on
//! an id that does not exist is inert, but it would have to be written before
//! the id is known, which is impossible.

use std::{collections::BTreeSet, fmt::Write as _, sync::Arc};

use async_trait::async_trait;
use tonic::Status;
use trogon_atlas_authzed::{
    AuthzedError, Consistency, ObjectId, ObjectRef, ObjectType, Relation, Relationship,
    RelationshipFilter, RelationshipOp, RelationshipUpdate, Revision, SpiceDb, SpiceDbConfig,
    SubjectRef,
};
use trogon_atlas_core::{NamespaceId, OwnerId};
use trogon_atlas_store::{NamespaceRecord, Store};

use crate::ownership::{Lens, NamespaceAuthorizer, NamespaceDirectory, Visibility, WriteVerdict};

/// The schema this authorizer expects, written by
/// [`SpiceDbAuthorizer::reconcile`].
///
/// `view` is defined as `viewer + edit` rather than as its own independent
/// relation so that granting edit cannot leave somebody unable to read what
/// they may write, which is a mistake that is easy to make in data and
/// impossible to make here.
pub const SCHEMA: &str = "\
definition user {}

definition organization {
    relation parent: organization
    relation member: user

    permission membership = member + parent->membership
}

definition namespace {
    relation parent: organization
    relation viewer: user | organization#member
    relation editor: user | organization#member

    permission edit = editor + parent->membership
    permission view = viewer + edit
}
";

/// How fresh a permission check has to be.
///
/// [`MinimizeLatency`](Freshness::MinimizeLatency) is strengthened by this
/// process's own writes: once it has written a relationship it pins every
/// later check to at least that revision, so it always sees its own grants.
/// It still cannot see, within SpiceDB's revision quantization window, a
/// grant some other process just wrote. That is the trade every Zanzibar
/// deployment makes, and [`FullyConsistent`](Freshness::FullyConsistent) is
/// how to decline it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Freshness {
    /// Read from whichever replica answers first, floored by this process's
    /// own writes. The default.
    #[default]
    MinimizeLatency,
    /// Quorum-read every check. Correct under any interleaving, and the
    /// choice for a deployment where a revocation performed elsewhere must
    /// take effect immediately.
    FullyConsistent,
}

impl std::fmt::Display for Freshness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MinimizeLatency => "minimize-latency",
            Self::FullyConsistent => "fully-consistent",
        })
    }
}

impl std::str::FromStr for Freshness {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "minimize-latency" => Ok(Self::MinimizeLatency),
            "fully-consistent" => Ok(Self::FullyConsistent),
            other => Err(format!(
                "unknown freshness {other:?}; expected minimize-latency or fully-consistent",
            )),
        }
    }
}

/// Object ids in SpiceDB are `[a-zA-Z0-9/_|\-=+]`, and this workspace's ids
/// are `[A-Za-z0-9_.-]`. The alphabets overlap in all but one character, so
/// the encoding is the identity for every id that has no dot in it, and
/// `orders.v2` becomes `orders=2Ev2`.
///
/// `=` escapes itself, which is what makes the mapping injective: without
/// that, a literal `orders=2Ev2` and an encoded `orders.v2` would be the same
/// object, and two namespaces would share one set of grants.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'|' | b'-' | b'+') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "={byte:02X}");
        }
    }
    out
}

fn decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            let hex = value.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The object types, relations, and permissions of [`SCHEMA`], parsed once.
#[derive(Debug)]
struct Vocabulary {
    user: ObjectType,
    organization: ObjectType,
    namespace: ObjectType,
    parent: Relation,
    member: Relation,
    view: Relation,
    edit: Relation,
}

impl Vocabulary {
    fn build() -> Result<Self, AuthzedError> {
        Ok(Self {
            user: ObjectType::parse("user")?,
            organization: ObjectType::parse("organization")?,
            namespace: ObjectType::parse("namespace")?,
            parent: Relation::parse("parent")?,
            member: Relation::parse("member")?,
            view: Relation::parse("view")?,
            edit: Relation::parse("edit")?,
        })
    }
}

pub struct SpiceDbAuthorizer {
    client: SpiceDb,
    directory: Arc<NamespaceDirectory>,
    store: Arc<dyn Store>,
    vocab: Vocabulary,
    baseline: Freshness,
    /// The newest revision this process has written.
    ///
    /// Only advanced by writes. A read's `ZedToken` is deliberately not
    /// recorded: tokens are opaque and not ordered against each other, so
    /// "the last one seen" is not necessarily the newest one, and treating it
    /// as a floor could move the floor backwards.
    floor: tokio::sync::RwLock<Option<Revision>>,
}

impl std::fmt::Debug for SpiceDbAuthorizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpiceDbAuthorizer")
            .field("baseline", &self.baseline)
            .finish_non_exhaustive()
    }
}

/// How fresh a read has to be, given the configured baseline and the newest
/// revision this authorizer has written.
///
/// The floor strengthens `MinimizeLatency` into read-your-writes; it would
/// weaken `FullyConsistent`. "At least as fresh as revision R" permits any
/// snapshot at or after R, and SpiceDB quantizes revisions, so a satisfying
/// snapshot can still predate a write some other process made a moment ago.
///
/// Pulled out of the method so it can be tested without a SpiceDB: the
/// integration tests cannot tell these apart, because SpiceDB's in-memory
/// test datastore answers a minimize-latency read from head anyway.
fn resolve(baseline: Freshness, floor: Option<&Revision>) -> Consistency {
    if baseline == Freshness::FullyConsistent {
        return Consistency::FullyConsistent;
    }
    match floor {
        Some(revision) => Consistency::AtLeastAsFresh(revision.clone()),
        None => Consistency::MinimizeLatency,
    }
}

impl SpiceDbAuthorizer {
    /// # Errors
    /// When the endpoint is unusable or the built-in vocabulary does not
    /// parse, which would be a bug in [`SCHEMA`].
    pub fn new(
        config: &SpiceDbConfig,
        directory: Arc<NamespaceDirectory>,
        store: Arc<dyn Store>,
        baseline: Freshness,
    ) -> Result<Self, AuthzedError> {
        Ok(Self {
            client: SpiceDb::connect(config)?,
            directory,
            store,
            vocab: Vocabulary::build()?,
            baseline,
            floor: tokio::sync::RwLock::new(None),
        })
    }

    async fn consistency(&self) -> Consistency {
        resolve(self.baseline, self.floor.read().await.as_ref())
    }

    async fn advance(&self, revision: Revision) {
        *self.floor.write().await = Some(revision);
    }

    fn user(&self, principal: &str) -> Result<SubjectRef, Status> {
        Ok(SubjectRef::new(ObjectRef::new(
            self.vocab.user.clone(),
            ObjectId::parse(&encode(principal)).map_err(|e| fatal("principal", &e))?,
        )))
    }

    fn namespace(&self, id: &NamespaceId) -> Result<ObjectRef, Status> {
        Ok(ObjectRef::new(
            self.vocab.namespace.clone(),
            ObjectId::parse(&encode(id.as_str())).map_err(|e| fatal("namespace id", &e))?,
        ))
    }

    fn organization(&self, owner: &OwnerId) -> Result<ObjectRef, Status> {
        Ok(ObjectRef::new(
            self.vocab.organization.clone(),
            ObjectId::parse(&encode(owner.as_str())).map_err(|e| fatal("owner", &e))?,
        ))
    }

    fn owns(&self, id: &NamespaceId, owner: &OwnerId) -> Result<Relationship, Status> {
        Ok(Relationship::new(
            self.namespace(id)?,
            self.vocab.parent.clone(),
            SubjectRef::new(self.organization(owner)?),
        ))
    }

    fn belongs(&self, principal: &str, owner: &OwnerId) -> Result<Relationship, Status> {
        Ok(Relationship::new(
            self.organization(owner)?,
            self.vocab.member.clone(),
            self.user(principal)?,
        ))
    }

    async fn write(&self, updates: &[RelationshipUpdate]) -> Result<(), Status> {
        if updates.is_empty() {
            return Ok(());
        }
        let revision = self
            .client
            .write_relationships(updates)
            .await
            .map_err(|e| remote("WriteRelationships", &e))?;
        self.advance(revision).await;
        Ok(())
    }

    /// Install [`SCHEMA`] and re-derive every grant the registry implies.
    ///
    /// Idempotent, and the repair path for anything that went wrong midway
    /// through a registration.
    ///
    /// `memberships` is the principal-to-owner mapping from the token
    /// registry, and it replaces rather than extends what SpiceDB holds for
    /// each principal named in it. Pass it whole: a principal left out is a
    /// principal whose memberships are left exactly as they were, which is
    /// the right answer for one that was deleted and the wrong one for one
    /// that merely was not looked up.
    ///
    /// Namespace grants are only ever added here. A namespace whose owner
    /// changed through `MoveNamespace` already had its old grant deleted in
    /// the same transaction that wrote the new one, so there is nothing for
    /// this to clean up, and deleting first would blind every caller for as
    /// long as the re-write took.
    ///
    /// # Errors
    /// When SpiceDB is unreachable, rejects the schema, or refuses a write.
    pub async fn reconcile(
        &self,
        memberships: &[(Arc<str>, OwnerId)],
    ) -> Result<ReconcileReport, Status> {
        let revision = self
            .client
            .write_schema(SCHEMA)
            .await
            .map_err(|e| remote("WriteSchema", &e))?;
        self.advance(revision).await;

        let records = self
            .store
            .list_namespaces()
            .await
            .map_err(|e| Status::unavailable(format!("namespace registry unavailable: {e}")))?;

        let mut updates = Vec::with_capacity(records.len());
        for record in &records {
            updates.push(RelationshipUpdate::new(
                RelationshipOp::Touch,
                self.owns(&record.id, &record.parent)?,
            ));
        }

        // Chunked because SpiceDB bounds the updates in one transaction, and
        // a store with thousands of namespaces would otherwise be refused
        // wholesale. Each chunk is its own transaction, which is fine: the
        // whole operation is idempotent, so a partial run is a shorter run.
        for chunk in updates.chunks(RECONCILE_CHUNK) {
            self.write(chunk).await?;
        }

        // Each principal's delete is followed immediately by its write, and
        // the pair is not batched with anything else, so the moment when
        // somebody belongs to no organisation is one principal wide and two
        // round trips long. Batching the writes would stretch that moment
        // across every principal in the file. Deleting after writing cannot
        // work at all: the filter that finds the stale memberships also
        // finds the one just written.
        for (principal, owner) in memberships {
            let revision = self
                .client
                .delete_relationships(
                    &RelationshipFilter::new(self.vocab.organization.clone())
                        .with_relation(self.vocab.member.clone())
                        .with_subject(self.user(principal)?),
                )
                .await
                .map_err(|e| remote("DeleteRelationships", &e))?;
            self.advance(revision).await;
            self.write(&[RelationshipUpdate::new(
                RelationshipOp::Touch,
                self.belongs(principal, owner)?,
            )])
            .await?;
        }

        Ok(ReconcileReport {
            namespaces: records.len(),
            memberships: memberships.len(),
        })
    }
}

/// SpiceDB's default `maxUpdatesPerWrite` is 1000.
const RECONCILE_CHUNK: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconcileReport {
    pub namespaces: usize,
    pub memberships: usize,
}

/// A value this process built and SpiceDB would reject. Not the caller's
/// fault and not retryable, so it is `INTERNAL` rather than
/// `INVALID_ARGUMENT`.
fn fatal(what: &str, err: &AuthzedError) -> Status {
    Status::internal(format!("cannot express {what} as a SpiceDB object: {err}"))
}

/// Never denies. An authorization service that cannot be reached has not said
/// no, it has said nothing, and reporting nothing as a denial would turn a
/// SpiceDB outage into a silent, total data loss from the caller's point of
/// view.
fn remote(operation: &str, err: &AuthzedError) -> Status {
    if err.is_transient() {
        Status::unavailable(format!(
            "authorization service unavailable ({operation}): {err}"
        ))
    } else {
        Status::internal(format!("authorization check failed ({operation}): {err}"))
    }
}

#[async_trait]
impl NamespaceAuthorizer for SpiceDbAuthorizer {
    async fn admits(
        &self,
        visibility: &Visibility,
        namespace: &NamespaceId,
    ) -> Result<bool, Status> {
        if visibility.is_unrestricted() {
            return Ok(true);
        }
        let outcome = self
            .client
            .check_permission(
                &self.namespace(namespace)?,
                &self.vocab.view,
                &self.user(visibility.principal())?,
                &self.consistency().await,
            )
            .await
            .map_err(|e| remote("CheckPermission", &e))?;
        Ok(outcome.is_granted())
    }

    async fn lens(&self, visibility: &Visibility) -> Result<Lens, Status> {
        if visibility.is_unrestricted() {
            return Ok(Lens::Everything);
        }
        let page = self
            .client
            .lookup_resources(
                &self.vocab.namespace,
                &self.vocab.view,
                &self.user(visibility.principal())?,
                &self.consistency().await,
            )
            .await
            .map_err(|e| remote("LookupResources", &e))?;

        let mut ids = BTreeSet::new();
        for object in &page.resources {
            // Dropped rather than fatal: an undecodable id is a relationship
            // somebody wrote by hand against something this workspace could
            // never have produced. Dropping it costs that one grant; failing
            // the request would cost the tenant every other one.
            if let Some(id) = decode(object.as_str()).and_then(|raw| NamespaceId::parse(&raw).ok())
            {
                ids.insert(id);
            } else {
                tracing::warn!(
                    object = %object,
                    "SpiceDB returned a namespace object id that is not a NamespaceId; ignoring",
                );
            }
        }
        Ok(Lens::Only(Arc::new(ids)))
    }

    async fn may_write(
        &self,
        visibility: &Visibility,
        namespace: &NamespaceId,
    ) -> Result<WriteVerdict, Status> {
        if visibility.is_unrestricted() {
            return Ok(WriteVerdict::Allowed);
        }
        // Existence is the registry's answer, not SpiceDB's. Asking SpiceDB
        // would conflate "no grant" with "no such namespace", and those lead
        // to opposite actions: refuse, versus claim it for the caller.
        let view = self.directory.view(&*self.store).await?;
        let Some(record) = view.get(namespace) else {
            return Ok(WriteVerdict::Unclaimed);
        };
        let outcome = self
            .client
            .check_permission(
                &self.namespace(namespace)?,
                &self.vocab.edit,
                &self.user(visibility.principal())?,
                &self.consistency().await,
            )
            .await
            .map_err(|e| remote("CheckPermission", &e))?;
        if outcome.is_granted() {
            Ok(WriteVerdict::Allowed)
        } else {
            Ok(WriteVerdict::Denied {
                owner: record.parent.clone(),
            })
        }
    }

    async fn on_registered(&self, record: &NamespaceRecord) -> Result<(), Status> {
        self.write(&[RelationshipUpdate::new(
            RelationshipOp::Touch,
            self.owns(&record.id, &record.parent)?,
        )])
        .await
    }

    async fn on_moved(&self, record: &NamespaceRecord, from: &OwnerId) -> Result<(), Status> {
        if from == &record.parent {
            return Ok(());
        }
        // One transaction, so there is no instant where the namespace has two
        // owners or none.
        self.write(&[
            RelationshipUpdate::new(RelationshipOp::Delete, self.owns(&record.id, from)?),
            RelationshipUpdate::new(
                RelationshipOp::Touch,
                self.owns(&record.id, &record.parent)?,
            ),
        ])
        .await
    }

    async fn on_released(&self, record: &NamespaceRecord) -> Result<(), Status> {
        self.write(&[RelationshipUpdate::new(
            RelationshipOp::Delete,
            self.owns(&record.id, &record.parent)?,
        )])
        .await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn ids_without_a_dot_encode_to_themselves() {
        for raw in ["orders", "ns_0199abc", "a-b_c", "default", "acme"] {
            assert_eq!(encode(raw), raw, "{raw} should not need escaping");
            assert_eq!(decode(&encode(raw)).as_deref(), Some(raw));
        }
    }

    #[test]
    fn a_dot_is_escaped_and_survives_the_round_trip() {
        assert_eq!(encode("orders.v2"), "orders=2Ev2");
        assert_eq!(decode("orders=2Ev2").as_deref(), Some("orders.v2"));
    }

    /// The property that keeps two namespaces from sharing one grant.
    #[test]
    fn the_escape_character_escapes_itself() {
        assert_eq!(encode("orders=2Ev2"), "orders=3D2Ev2");
        assert_ne!(encode("orders=2Ev2"), encode("orders.v2"));
        assert_eq!(
            decode(&encode("orders=2Ev2")).as_deref(),
            Some("orders=2Ev2")
        );
    }

    #[test]
    fn arbitrary_principal_names_survive_the_round_trip() {
        for raw in ["ci@example.com", "svc account", "ünïcode", "", "%%%"] {
            assert_eq!(decode(&encode(raw)).as_deref(), Some(raw), "{raw:?}");
        }
    }

    /// Everything the encoder emits has to be a legal SpiceDB object id, or
    /// the escaping would have moved the failure rather than removed it.
    #[test]
    fn every_encoded_id_is_a_legal_object_id() {
        for raw in ["orders.v2", "ci@example.com", "a.b.c.d", "ünïcode", "x"] {
            assert!(
                ObjectId::parse(&encode(raw)).is_ok(),
                "{raw:?} encoded to something SpiceDB would reject",
            );
        }
    }

    #[test]
    fn truncated_or_invalid_escapes_decode_to_nothing() {
        assert!(decode("orders=2").is_none());
        assert!(decode("orders=").is_none());
        assert!(decode("orders=ZZ").is_none());
    }

    #[test]
    fn a_write_pins_later_reads_to_at_least_that_revision() {
        let revision = Revision::new("zed-token".to_string()).unwrap();
        assert_eq!(
            resolve(Freshness::MinimizeLatency, None),
            Consistency::MinimizeLatency,
        );
        assert_eq!(
            resolve(Freshness::MinimizeLatency, Some(&revision)),
            Consistency::AtLeastAsFresh(revision),
        );
    }

    /// The floor must never be allowed to answer for a deployment that asked
    /// for quorum reads: at-least-as-fresh is satisfied by any snapshot from
    /// that revision onwards, which can still miss another writer's newer
    /// one. This shipped wrong once.
    #[test]
    fn a_write_never_downgrades_a_fully_consistent_deployment() {
        let revision = Revision::new("zed-token".to_string()).unwrap();
        assert_eq!(
            resolve(Freshness::FullyConsistent, Some(&revision)),
            Consistency::FullyConsistent,
        );
        assert_eq!(
            resolve(Freshness::FullyConsistent, None),
            Consistency::FullyConsistent,
        );
    }

    #[test]
    fn the_vocabulary_matches_the_schema() {
        let vocab = Vocabulary::build().unwrap();
        for object_type in [&vocab.user, &vocab.organization, &vocab.namespace] {
            assert!(
                SCHEMA.contains(&format!("definition {object_type} ")),
                "{object_type} is not defined in SCHEMA",
            );
        }
        for relation in [&vocab.parent, &vocab.member] {
            assert!(SCHEMA.contains(&format!("relation {relation}:")));
        }
        for permission in [&vocab.view, &vocab.edit] {
            assert!(SCHEMA.contains(&format!("permission {permission} =")));
        }
    }
}
