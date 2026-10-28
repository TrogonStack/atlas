use trogon_atlas_proto::Entity;

#[derive(Debug, thiserror::Error)]
#[error("invalid metadata at {0}")]
pub struct MetadataError(pub String);

pub fn validate(entity: &Entity) -> Result<(), MetadataError> {
    let _ = entity;
    Ok(())
}
