//! A typed client for the Authzed/SpiceDB v1 API.
//!
//! # Why the client is generated here
//!
//! At the time of writing the `authzed` crate on crates.io is a `0.0.1`
//! placeholder and the only third-party alternative is an unaudited `0.1.x`.
//! This client sits on an authorization boundary, so it generates from the
//! upstream protos directly rather than trusting either. The protos come from
//! the `buf.build/authzed/api` module on the Buf Schema Registry, pinned in
//! the generation template; `mise run proto:generate` regenerates `src/gen/`.
//!
//! # What this crate is not
//!
//! It knows nothing about namespaces, owners, or event models. It is a
//! transport with the API's own validation rules encoded as value objects, so
//! that an illegal object id fails where it is constructed instead of arriving
//! back as an opaque `INVALID_ARGUMENT` from a remote service. The mapping
//! from this workspace's domain onto a SpiceDB schema lives in the server.

pub mod google {
    pub mod rpc {
        pub use tonic_types::pb::Status;
    }
}

#[rustfmt::skip]
#[allow(clippy::all, clippy::pedantic, clippy::unwrap_used, clippy::expect_used)]
#[path = "gen/mod.rs"]
mod generated;

pub use authzed::api::v1;
pub use generated::authzed;

mod client;
mod error;
mod object;
mod relationship;

pub use client::{CheckOutcome, Consistency, LookupPage, Permission, SpiceDb, SpiceDbConfig};
pub use error::AuthzedError;
pub use object::{ObjectId, ObjectRef, ObjectType, PresharedKey, Relation, Revision, SubjectRef};
pub use relationship::{Relationship, RelationshipFilter, RelationshipOp, RelationshipUpdate};
