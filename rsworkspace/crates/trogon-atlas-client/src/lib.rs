//! Shared semantic layer for every surface that talks to the trogon-atlas
//! gRPC service (trogon-atlas and the MCP adapter both build on this instead of
//! duplicating it).
//!
//! The manifest schema deliberately does not conform to Kubernetes (see
//! the strategic backlog); this crate borrows
//! only the UX: declarative YAML documents with apiVersion/kind/metadata/spec,
//! applied idempotently against the server's own gRPC surface. The manifest
//! spec IS the proto3-JSON body of the entity message, so the schema stays
//! derived from the proto, never hand-maintained.
pub mod apply;
pub mod branch;
pub mod client;
pub mod compat;
pub mod export;
pub mod failure;
pub mod fmt;
pub mod manifest;
pub mod manifest_schema;
pub mod openslo;
pub mod openslo_bindings;
pub mod operation;
pub mod outcome;
pub mod precondition;
pub mod tenant_types;
