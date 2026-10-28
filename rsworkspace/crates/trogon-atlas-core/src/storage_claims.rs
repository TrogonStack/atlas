use trogon_atlas_proto::{self as pb, entity, Entity};

pub const MAX_CLAIMS: usize = 1_024;
pub const MAX_ENCODED_BYTES: usize = 1_048_576;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct StorageClaimError(pub String);
type Result<T> = std::result::Result<T, StorageClaimError>;

#[must_use]
pub fn claims(entity: &Entity) -> &[pb::StorageAccessClaim] {
    match &entity.kind {
        Some(entity::Kind::Component(value)) => &value.storage_claims,
        Some(entity::Kind::ExternalSystem(value)) => &value.storage_claims,
        Some(entity::Kind::DataProduct(value)) => &value.storage_claims,
        _ => &[],
    }
}

pub fn validate_values(_entity: &Entity) -> Result<()> {
    Ok(())
}
