//! One-off migration of the retired `fields` lists on Event (tag 5),
//! Command (tag 5) and ReadModel (tag 4) into the `schema` Any.
//!
//! The current generated types no longer know those tags, so prost drops
//! them on decode. The stored bytes are therefore read twice: once through
//! the private mirrors below, which only know the retired tags, and once
//! through the current types, which know everything else.
//!
//! Covers the entities bucket and every branch delta row. The revision log
//! is history and keeps the bytes it was written with.

use std::{
    collections::{BTreeMap, HashMap},
    fmt,
};

use bytes::Bytes;
use futures::StreamExt;
use prost::Message as _;
use prost_types::Any;
use trogon_atlas_core::schema::{pack_fields, schema_fields};
use trogon_atlas_proto::{entity, BranchDelta, Entity, FieldSpec};

use super::{kv_entry_timeout, live_entry, write_schema_marker, KvStore, NatsStore, KV_OP_TIMEOUT};
use crate::{
    error::{StoreError, StoreResult},
    key::parse_entity_key,
};

#[derive(Clone, PartialEq, ::prost::Message)]
struct LegacyEntity {
    #[prost(oneof = "LegacyKind", tags = "1, 2, 3")]
    kind: Option<LegacyKind>,
}

#[derive(Clone, PartialEq, ::prost::Oneof)]
enum LegacyKind {
    #[prost(message, tag = "1")]
    Event(LegacyEventFields),
    #[prost(message, tag = "2")]
    Command(LegacyEventFields),
    #[prost(message, tag = "3")]
    ReadModel(LegacyReadModelFields),
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct LegacyEventFields {
    #[prost(message, repeated, tag = "5")]
    fields: Vec<FieldSpec>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct LegacyReadModelFields {
    #[prost(message, repeated, tag = "4")]
    fields: Vec<FieldSpec>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct LegacyBranchDelta {
    #[prost(message, optional, tag = "1")]
    base: Option<LegacyEntity>,
    #[prost(message, optional, tag = "3")]
    ours: Option<LegacyEntity>,
}

impl LegacyEntity {
    fn into_fields(self) -> Vec<FieldSpec> {
        match self.kind {
            Some(LegacyKind::Event(x) | LegacyKind::Command(x)) => x.fields,
            Some(LegacyKind::ReadModel(x)) => x.fields,
            None => Vec::new(),
        }
    }
}

/// Whether a run writes what it finds or only reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationMode {
    DryRun,
    Apply,
}

/// What an apply does with an image whose `fields` and `schema` disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictResolution {
    /// Leave the image untouched and keep the store closed to servers.
    Refuse,
    /// Keep `schema` and discard the retired list.
    KeepSchema,
}

/// What one stored entity image holds, judged against the retired layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LegacyFieldsShape {
    /// A kind that never carried `fields`.
    NotFieldBearing,
    /// An Event, Command or ReadModel with neither `fields` nor `schema`.
    Empty,
    SchemaOnly,
    /// Only `fields`: rewritten into an anonymous `Schema`.
    FieldsOnly,
    /// Both, declaring the same fields: `fields` is dropped.
    BothEqual,
    /// Both, disagreeing: left untouched for a person to resolve.
    BothDifferent,
}

impl LegacyFieldsShape {
    #[must_use]
    pub fn needs_rewrite(self) -> bool {
        matches!(self, Self::FieldsOnly | Self::BothEqual)
    }

    fn rewritten_under(self, conflicts: ConflictResolution) -> bool {
        self.needs_rewrite() || (self.is_conflict() && conflicts == ConflictResolution::KeepSchema)
    }

    #[must_use]
    pub fn is_conflict(self) -> bool {
        self == Self::BothDifferent
    }
}

impl fmt::Display for LegacyFieldsShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFieldBearing => "not-field-bearing",
            Self::Empty => "empty",
            Self::SchemaOnly => "schema-only",
            Self::FieldsOnly => "fields-only",
            Self::BothEqual => "both-equal",
            Self::BothDifferent => "both-different",
        })
    }
}

/// Where an entity image lives in the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LegacyFieldsLocation {
    Baseline { key: String },
    BranchBase { key: String },
    BranchOurs { key: String },
}

impl fmt::Display for LegacyFieldsLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Baseline { key } => write!(f, "baseline {key}"),
            Self::BranchBase { key } => write!(f, "branch {key} (base)"),
            Self::BranchOurs { key } => write!(f, "branch {key} (ours)"),
        }
    }
}

/// One entity image that needs, or needed, attention.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyFieldsFinding {
    pub location: LegacyFieldsLocation,
    pub shape: LegacyFieldsShape,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyFieldsReport {
    pub mode: MigrationMode,
    pub conflicts: ConflictResolution,
    /// Every image seen, by shape.
    pub counts: BTreeMap<LegacyFieldsShape, usize>,
    /// Images that were (or, in a dry run, would be) rewritten, and
    /// conflicts left untouched.
    pub findings: Vec<LegacyFieldsFinding>,
}

impl LegacyFieldsReport {
    fn new(mode: MigrationMode, conflicts: ConflictResolution) -> Self {
        Self {
            mode,
            conflicts,
            counts: BTreeMap::new(),
            findings: Vec::new(),
        }
    }

    fn record(&mut self, location: LegacyFieldsLocation, shape: LegacyFieldsShape) {
        *self.counts.entry(shape).or_default() += 1;
        if shape.needs_rewrite() || shape.is_conflict() {
            self.findings.push(LegacyFieldsFinding { location, shape });
        }
    }

    #[must_use]
    pub fn count(&self, shape: LegacyFieldsShape) -> usize {
        self.counts.get(&shape).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn has_conflicts(&self) -> bool {
        self.count(LegacyFieldsShape::BothDifferent) > 0
    }

    /// Conflicts this run left in the retired layout.
    #[must_use]
    pub fn unresolved_conflicts(&self) -> usize {
        match self.conflicts {
            ConflictResolution::Refuse => self.count(LegacyFieldsShape::BothDifferent),
            ConflictResolution::KeepSchema => 0,
        }
    }

    /// Images still in the retired layout once this run is done: everything
    /// found on a dry run, only unresolved conflicts after an apply.
    #[must_use]
    pub fn remaining(&self) -> usize {
        match self.mode {
            MigrationMode::DryRun => self
                .counts
                .iter()
                .filter(|(shape, _)| shape.needs_rewrite() || shape.is_conflict())
                .map(|(_, n)| n)
                .sum(),
            MigrationMode::Apply => self.unresolved_conflicts(),
        }
    }
}

fn kind_schema_mut(kind: &mut entity::Kind) -> Option<&mut Option<Any>> {
    match kind {
        entity::Kind::Event(x) => Some(&mut x.schema),
        entity::Kind::Command(x) => Some(&mut x.schema),
        entity::Kind::ReadModel(x) => Some(&mut x.schema),
        _ => None,
    }
}

/// Judge one image and, for `FieldsOnly`, move the legacy list into
/// `current.schema`. `current` is the same bytes decoded with the current
/// types, so re-encoding it drops the retired tag.
fn migrate_image(legacy: LegacyEntity, current: &mut Entity) -> LegacyFieldsShape {
    let Some(schema) = current.kind.as_mut().and_then(kind_schema_mut) else {
        return LegacyFieldsShape::NotFieldBearing;
    };
    let fields = legacy.into_fields();
    let has_schema = schema.as_ref().is_some_and(|any| !any.type_url.is_empty());
    match (fields.is_empty(), has_schema) {
        (true, false) => LegacyFieldsShape::Empty,
        (true, true) => LegacyFieldsShape::SchemaOnly,
        (false, false) => {
            *schema = Some(pack_fields(fields));
            LegacyFieldsShape::FieldsOnly
        }
        (false, true) if schema_fields(schema.as_ref()) == fields => LegacyFieldsShape::BothEqual,
        (false, true) => LegacyFieldsShape::BothDifferent,
    }
}

fn decode_error(key: &str, e: &prost::DecodeError) -> StoreError {
    StoreError::Backend(format!("legacy fields decode {key}: {e}"))
}

fn migrate_entity_bytes(key: &str, bytes: &[u8]) -> StoreResult<(LegacyFieldsShape, Entity)> {
    let legacy = LegacyEntity::decode(bytes).map_err(|e| decode_error(key, &e))?;
    let mut current = Entity::decode(bytes).map_err(|e| decode_error(key, &e))?;
    let shape = migrate_image(legacy, &mut current);
    Ok((shape, current))
}

struct DeltaMigration {
    delta: BranchDelta,
    base: Option<LegacyFieldsShape>,
    ours: Option<LegacyFieldsShape>,
}

impl DeltaMigration {
    fn needs_rewrite(&self, conflicts: ConflictResolution) -> bool {
        [self.base, self.ours]
            .into_iter()
            .flatten()
            .any(|shape| shape.rewritten_under(conflicts))
    }
}

fn migrate_delta_bytes(key: &str, bytes: &[u8]) -> StoreResult<DeltaMigration> {
    let legacy = LegacyBranchDelta::decode(bytes).map_err(|e| decode_error(key, &e))?;
    let mut delta = BranchDelta::decode(bytes).map_err(|e| decode_error(key, &e))?;
    let base = delta
        .base
        .as_mut()
        .map(|current| migrate_image(legacy.base.unwrap_or_default(), current));
    let ours = delta
        .ours
        .as_mut()
        .map(|current| migrate_image(legacy.ours.unwrap_or_default(), current));
    Ok(DeltaMigration { delta, base, ours })
}

async fn all_keys(kv: &KvStore, bucket: &str) -> StoreResult<Vec<String>> {
    let mut stream = tokio::time::timeout(KV_OP_TIMEOUT, kv.keys())
        .await
        .map_err(|_| StoreError::Backend(format!("{bucket} kv.keys: timeout")))?
        .map_err(|e| StoreError::Backend(format!("{bucket} kv.keys: {e}")))?;
    let mut keys = Vec::new();
    while let Some(item) = tokio::time::timeout(KV_OP_TIMEOUT, stream.next())
        .await
        .map_err(|_| StoreError::Backend(format!("{bucket} kv.keys stream: timeout")))?
    {
        keys.push(item.map_err(|e| StoreError::Backend(format!("{bucket} kv.keys stream: {e}")))?);
    }
    keys.sort();
    Ok(keys)
}

async fn put_if_unchanged(
    kv: &KvStore,
    key: &str,
    bytes: Vec<u8>,
    revision: u64,
) -> StoreResult<u64> {
    tokio::time::timeout(KV_OP_TIMEOUT, kv.update(key, Bytes::from(bytes), revision))
        .await
        .map_err(|_| StoreError::Backend(format!("kv.update {key}: timeout")))?
        .map_err(|e| {
            StoreError::Backend(format!(
                "kv.update {key}: {e}; the row changed during the migration, run it again"
            ))
        })
}

impl NatsStore {
    /// Move every retired `fields` list into `schema`, across baseline
    /// entities and branch deltas. Idempotent: a second run finds nothing
    /// to rewrite. Meant to run with no server writing to the store.
    ///
    /// An apply that leaves no image in the retired layout stamps the
    /// store's schema marker, which is what lets a server open it again.
    pub async fn migrate_legacy_fields(
        &self,
        mode: MigrationMode,
        conflicts: ConflictResolution,
    ) -> StoreResult<LegacyFieldsReport> {
        let report = migrate_legacy_rows(&self.kv, &self.branches_kv, mode, conflicts).await?;
        if mode == MigrationMode::Apply && report.remaining() == 0 {
            write_schema_marker(&self.kv).await?;
        }
        Ok(report)
    }
}

/// A branch delta whose `base_etag` pointed at a baseline row this run
/// rewrote is moved to the new revision, so the branch does not read the
/// migration as a baseline change.
pub(super) async fn migrate_legacy_rows(
    kv: &KvStore,
    branches_kv: &KvStore,
    mode: MigrationMode,
    conflicts: ConflictResolution,
) -> StoreResult<LegacyFieldsReport> {
    let mut report = LegacyFieldsReport::new(mode, conflicts);
    let mut moved_revisions: HashMap<String, (String, String)> = HashMap::new();

    for key in all_keys(kv, "entities").await? {
        if parse_entity_key(&key).is_none() {
            continue;
        }
        let Some(entry) = live_entry(kv_entry_timeout(kv, key.clone()).await?) else {
            continue;
        };
        let (shape, migrated) = migrate_entity_bytes(&key, &entry.value)?;
        if shape.rewritten_under(conflicts) && mode == MigrationMode::Apply {
            let new_revision =
                put_if_unchanged(kv, &key, migrated.encode_to_vec(), entry.revision).await?;
            moved_revisions.insert(
                key.clone(),
                (entry.revision.to_string(), new_revision.to_string()),
            );
        }
        report.record(LegacyFieldsLocation::Baseline { key }, shape);
    }

    for key in all_keys(branches_kv, "branches").await? {
        if key.starts_with("meta.") {
            continue;
        }
        let Some(entity_key) = key
            .split_once('.')
            .map(|(_, rest)| rest)
            .filter(|rest| parse_entity_key(rest).is_some())
        else {
            continue;
        };
        let Some(entry) = live_entry(kv_entry_timeout(branches_kv, key.clone()).await?) else {
            continue;
        };
        let mut migration = migrate_delta_bytes(&key, &entry.value)?;
        let rebased = match moved_revisions.get(entity_key) {
            Some((old, new)) if *old == migration.delta.base_etag => {
                migration.delta.base_etag.clone_from(new);
                true
            }
            _ => false,
        };
        if (migration.needs_rewrite(conflicts) || rebased) && mode == MigrationMode::Apply {
            put_if_unchanged(
                branches_kv,
                &key,
                migration.delta.encode_to_vec(),
                entry.revision,
            )
            .await?;
        }
        if let Some(shape) = migration.base {
            report.record(LegacyFieldsLocation::BranchBase { key: key.clone() }, shape);
        }
        if let Some(shape) = migration.ours {
            report.record(LegacyFieldsLocation::BranchOurs { key }, shape);
        }
    }

    Ok(report)
}

#[cfg(test)]
#[path = "legacy_fields_tests.rs"]
mod tests;
