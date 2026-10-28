use trogon_atlas_proto::{self as pb, entity, Entity};

pub const MAX_LINKS: usize = 1_024;
pub const MAX_ENCODED_BYTES: usize = 1_048_576;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct ChannelLinkError(pub String);
type Result<T> = std::result::Result<T, ChannelLinkError>;

#[must_use]
pub fn links(entity: &Entity) -> &[pb::ChannelLink] {
    match &entity.kind {
        Some(entity::Kind::Channel(value)) => &value.links,
        _ => &[],
    }
}

pub fn validate_values(_entity: &Entity) -> Result<()> {
    Ok(())
}
