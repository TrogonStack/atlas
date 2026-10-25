use std::{fs::File, io::Read, path::PathBuf};

use anyhow::Result;
use trogon_atlas_core::source_bundle::{
    self, LogicalName, MediaType, SuppliedFile, ValidatedBundle,
};

use crate::failure::invalid_input;

pub struct SuppliedFilePath {
    logical_name: LogicalName,
    media_type: MediaType,
    path: PathBuf,
}

impl SuppliedFilePath {
    pub fn new(logical_name: &str, media_type: &str, path: PathBuf) -> Result<Self> {
        Ok(Self {
            logical_name: LogicalName::new(logical_name)
                .map_err(|error| invalid_input(error.to_string()))?,
            media_type: MediaType::new(media_type)
                .map_err(|error| invalid_input(error.to_string()))?,
            path,
        })
    }
}

pub fn read_bounded_file(path: &std::path::Path, limit: usize) -> Result<Vec<u8>> {
    let entry =
        std::fs::symlink_metadata(path).map_err(|_| invalid_input("supplied file unavailable"))?;
    if !entry.is_file() {
        return Err(invalid_input("supplied path must name a regular file"));
    }
    let mut file = File::open(path).map_err(|_| invalid_input("supplied file unavailable"))?;
    let metadata = file
        .metadata()
        .map_err(|_| invalid_input("supplied file unavailable"))?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(invalid_input(
            "supplied file is not regular or exceeds intake limit",
        ));
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid_input("supplied file unreadable"))?;
    if bytes.len() > limit {
        return Err(invalid_input("supplied file exceeds intake limit"));
    }
    Ok(bytes)
}

pub fn read_supplied_bundle(
    inventory_path: &std::path::Path,
    files: &[SuppliedFilePath],
) -> Result<ValidatedBundle> {
    if files.len() > source_bundle::MAX_FILES {
        return Err(invalid_input("supplied file count exceeds intake limit"));
    }
    let inventory = read_bounded_file(inventory_path, source_bundle::MAX_INVENTORY_BYTES)?;
    let mut remaining = source_bundle::MAX_COMBINED_BYTES - inventory.len();
    let mut payloads = Vec::with_capacity(files.len());
    for file in files {
        let bytes = read_bounded_file(&file.path, remaining.min(source_bundle::MAX_FILE_BYTES))?;
        remaining -= bytes.len();
        payloads.push(bytes);
    }
    let supplied = files
        .iter()
        .zip(&payloads)
        .map(|(file, bytes)| SuppliedFile {
            logical_name: file.logical_name.as_str(),
            media_type: file.media_type.as_str(),
            bytes,
        })
        .collect::<Vec<_>>();
    source_bundle::validate_bundle(&inventory, &supplied)
        .map_err(|error| invalid_input(error.to_string()))
}
