//! `trogon-atlas-core`: schema-derived helpers built on top of
//! `trogon-atlas-proto` types. Lives one layer above the generated
//! bindings and one layer below the storage and server crates so that:
//!
//! * graph-walking primitives (`refs::outbound_refs`,
//!   `refs::entity_id`, `refs::entity_kind`) are reachable by any
//!   third-party storage backend or analytics tool without pulling in
//!   the storage or gRPC layer;
//! * the contract between proto types and downstream consumers has one
//!   home, instead of leaking into `trogon-atlas-store`.
//!
//! Re-export everything that callers historically reached through
//! `trogon_atlas_store::refs` so existing imports keep working with a
//! single `use` swap.

pub mod content_hash;
pub mod id_component;
pub mod namespace;
pub mod operation_id;
pub mod refs;
pub mod schema;
pub mod semantic_eq;
pub mod system_meta;
pub mod transcode;
pub mod writer_lease;

pub use content_hash::{
    canonical_json, entity_content_hash, ContentHash, ContentHashError, SnapshotHasher,
    CONTENT_HASH_LEN,
};
pub use id_component::{is_safe_id_component, validate_id_component, IdComponentError};
pub use namespace::{
    NamespaceError, NamespaceId, NamespaceName, OwnerId, NAMESPACE_ID_PREFIX, OWNER_ID_PREFIX,
};
pub use operation_id::{
    new_operation_id, operation_claim_key, operation_digest, validate_operation_id,
    InvalidOperationId,
};
pub use refs::{entity_id, entity_kind, outbound_refs, retarget_refs, OutboundRef};
pub use semantic_eq::semantically_equal;
pub use system_meta::stamp_system;
pub use writer_lease::{Epoch, ParseWriterRoleError, WriterRole};
