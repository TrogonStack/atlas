//! Exact supplied-byte intake. Validation establishes local shape and digests only,
//! not native validity, authorization, external completeness or snapshot acceptance.

use std::{cell::Cell, collections::BTreeSet, fmt};

use serde::{
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use sha2::{Digest, Sha256};

pub const MAX_INVENTORY_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_FILES: usize = 256;
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_COMBINED_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_RESOURCES: usize = 1000;
pub const MAX_RELATIONS: usize = 10_000;
pub const MAX_JSON_DEPTH: usize = 32;
pub const PROFILE: &str = "supplied-native-v1";
pub const PROFILE_REVISION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BundleError {
    #[error("input exceeds intake limit")]
    Limit,
    #[error("JSON nesting exceeds intake limit")]
    JsonDepth,
    #[error("invalid inventory JSON or field shape")]
    Inventory,
    #[error("invalid opaque text")]
    Text,
    #[error("invalid logical file locator")]
    Locator,
    #[error("invalid media type")]
    MediaType,
    #[error("unsupported input profile or revision")]
    Profile,
    #[error("duplicate durable input key")]
    Duplicate,
    #[error("inventory declaration is outside its partition")]
    Partition,
    #[error("inventory locator or owner is unavailable in supplied input")]
    Unavailable,
}

macro_rules! text_value {
    ($name:ident, $check:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);
        impl $name {
            pub fn new(value: &str) -> Result<Self, BundleError> {
                ($check)(value)?;
                Ok(Self(value.to_owned()))
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl TryFrom<String> for $name {
            type Error = BundleError;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                ($check)(&value)?;
                Ok(Self(value))
            }
        }
    };
}
fn token(value: &str) -> Result<(), BundleError> {
    if value.len() > 1024 || value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(BundleError::Text);
    }
    Ok(())
}
fn locator(value: &str) -> Result<(), BundleError> {
    if value.is_empty()
        || value.len() > 1024
        || value.contains(['\\', ':'])
        || value.chars().any(char::is_control)
        || value
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
    {
        return Err(BundleError::Locator);
    }
    Ok(())
}
fn media_type(value: &str) -> Result<(), BundleError> {
    if value.len() > 1024
        || !value.is_ascii()
        || value.trim().is_empty()
        || value.bytes().any(|b| b.is_ascii_control())
    {
        return Err(BundleError::MediaType);
    }
    Ok(())
}
fn raw_pointer(value: &str) -> Result<(), BundleError> {
    if value.len() > 16_384 {
        return Err(BundleError::Limit);
    }
    Ok(())
}
text_value!(RegistrationId, token);
text_value!(SourceVersion, token);
text_value!(SourceCategory, token);
text_value!(SourceLocalKey, token);
text_value!(OccurrenceKey, token);
text_value!(RelationRole, token);
text_value!(PointerKind, token);
text_value!(RawPointer, raw_pointer);
text_value!(LogicalName, locator);
text_value!(MediaType, media_type);

macro_rules! accessors {
    ($($field:ident : $ty:ty),* $(,)?) => { $(pub fn $field(&self) -> &$ty { &self.$field })* };
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceKey {
    category: SourceCategory,
    key: SourceLocalKey,
}
impl ResourceKey {
    pub fn new(category: SourceCategory, key: SourceLocalKey) -> Self {
        Self { category, key }
    }
    accessors!(category: SourceCategory, key: SourceLocalKey);
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePointer {
    kind: PointerKind,
    raw: RawPointer,
}
impl SourcePointer {
    accessors!(kind: PointerKind, raw: RawPointer);
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum DeclaredCoverage {
    #[serde(rename = "COMPLETE")]
    Complete,
    #[default]
    #[serde(rename = "PARTIAL")]
    Partial,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryPartition {
    categories: Vec<SourceCategory>,
    logical_roots: Vec<LogicalName>,
    selected_keys: Vec<ResourceKey>,
}
impl InventoryPartition {
    pub fn categories(&self) -> &[SourceCategory] {
        &self.categories
    }
    pub fn logical_roots(&self) -> &[LogicalName] {
        &self.logical_roots
    }
    pub fn selected_keys(&self) -> &[ResourceKey] {
        &self.selected_keys
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceResource {
    key: ResourceKey,
    file: LogicalName,
    pointer: SourcePointer,
}
impl SourceResource {
    accessors!(key: ResourceKey, file: LogicalName, pointer: SourcePointer);
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationOccurrence {
    owner: ResourceKey,
    occurrence: OccurrenceKey,
    role: RelationRole,
    position: u64,
    target: SourcePointer,
    file: LogicalName,
}
impl RelationOccurrence {
    accessors!(owner: ResourceKey, occurrence: OccurrenceKey, role: RelationRole, target: SourcePointer, file: LogicalName);
    pub fn position(&self) -> u64 {
        self.position
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    profile: String,
    profile_revision: u32,
    registration: RegistrationId,
    source_version: SourceVersion,
    partition: InventoryPartition,
    #[serde(default)]
    coverage: DeclaredCoverage,
    resources: Vec<SourceResource>,
    relations: Vec<RelationOccurrence>,
}
impl Inventory {
    accessors!(registration: RegistrationId, source_version: SourceVersion, partition: InventoryPartition);
    pub fn profile(&self) -> &str {
        &self.profile
    }
    pub fn profile_revision(&self) -> u32 {
        self.profile_revision
    }
    pub fn declared_coverage(&self) -> DeclaredCoverage {
        self.coverage
    }
    pub fn resources(&self) -> &[SourceResource] {
        &self.resources
    }
    pub fn relations(&self) -> &[RelationOccurrence] {
        &self.relations
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SuppliedFile<'a> {
    pub logical_name: &'a str,
    pub media_type: &'a str,
    pub bytes: &'a [u8],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestPurpose {
    InventoryV1,
    BundleV1,
    FileContentSha256,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDigest {
    purpose: DigestPurpose,
    bytes: [u8; 32],
}
impl SourceDigest {
    pub fn purpose(&self) -> DigestPurpose {
        self.purpose
    }
    pub fn bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
    pub fn hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        self.bytes
            .iter()
            .flat_map(|b| {
                [
                    char::from(HEX[usize::from(b >> 4)]),
                    char::from(HEX[usize::from(b & 15)]),
                ]
            })
            .collect()
    }
}
#[derive(Debug)]
pub struct ValidatedFile {
    logical_name: LogicalName,
    media_type: MediaType,
    bytes: Vec<u8>,
    digest: SourceDigest,
}
impl ValidatedFile {
    accessors!(logical_name: LogicalName, media_type: MediaType, digest: SourceDigest);
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
#[derive(Debug)]
pub struct ValidatedBundle {
    inventory: Inventory,
    inventory_bytes: Vec<u8>,
    files: Vec<ValidatedFile>,
    inventory_digest: SourceDigest,
    bundle_digest: SourceDigest,
}
impl ValidatedBundle {
    accessors!(inventory: Inventory, inventory_digest: SourceDigest, bundle_digest: SourceDigest);
    pub fn inventory_bytes(&self) -> &[u8] {
        &self.inventory_bytes
    }
    /// Files are ordered by exact logical-name UTF-8 bytes.
    pub fn files(&self) -> &[ValidatedFile] {
        &self.files
    }
}

#[derive(Clone, Copy)]
enum JsonShape {
    Any,
    Inventory,
    Partition,
    Resource,
    ResourceKey,
    Relation,
    Pointer,
    Resources,
    Relations,
    ResourceKeys,
    Coverage,
}
impl JsonShape {
    fn field(self, key: &str) -> Self {
        match (self, key) {
            (Self::Inventory, "partition") => Self::Partition,
            (Self::Inventory, "resources") => Self::Resources,
            (Self::Inventory, "relations") => Self::Relations,
            (Self::Inventory, "coverage") => Self::Coverage,
            (Self::Partition, "selected_keys") => Self::ResourceKeys,
            (Self::Resource, "key") | (Self::Relation, "owner") => Self::ResourceKey,
            (Self::Resource, "pointer") | (Self::Relation, "target") => Self::Pointer,
            _ => Self::Any,
        }
    }
    fn element(self) -> Option<Self> {
        match self {
            Self::Any => Some(Self::Any),
            Self::Resources => Some(Self::Resource),
            Self::Relations => Some(Self::Relation),
            Self::ResourceKeys => Some(Self::ResourceKey),
            _ => None,
        }
    }
    fn permits_object(self) -> bool {
        matches!(
            self,
            Self::Any
                | Self::Inventory
                | Self::Partition
                | Self::Resource
                | Self::ResourceKey
                | Self::Relation
                | Self::Pointer
        )
    }
    fn scalar<E: de::Error>(self) -> Result<(), E> {
        if matches!(self, Self::Any) {
            Ok(())
        } else {
            Err(de::Error::custom("shape"))
        }
    }
}

// Reject Serde's alternate positional and enum shapes before typed allocation.
struct JsonCheck<'a> {
    shape: JsonShape,
    depth_exceeded: &'a Cell<bool>,
    depth: usize,
    list_limit: usize,
}
impl<'de> DeserializeSeed<'de> for JsonCheck<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for JsonCheck<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bounded JSON")
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<(), E> {
        self.shape.scalar()
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<(), E> {
        self.shape.scalar()
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<(), E> {
        self.shape.scalar()
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<(), E> {
        self.shape.scalar()
    }
    fn visit_str<E: de::Error>(self, _: &str) -> Result<(), E> {
        if matches!(self.shape, JsonShape::Coverage) {
            Ok(())
        } else {
            self.shape.scalar()
        }
    }
    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        self.shape.scalar()
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<(), A::Error> {
        let element_shape = self
            .shape
            .element()
            .ok_or_else(|| de::Error::custom("shape"))?;
        if self.depth >= MAX_JSON_DEPTH {
            self.depth_exceeded.set(true);
            return Err(de::Error::custom("depth"));
        }
        let mut n = 0usize;
        while a
            .next_element_seed(JsonCheck {
                shape: element_shape,
                depth_exceeded: self.depth_exceeded,
                depth: self.depth + 1,
                list_limit: MAX_RELATIONS,
            })?
            .is_some()
        {
            n = n.checked_add(1).ok_or_else(|| de::Error::custom("count"))?;
            if n > self.list_limit {
                return Err(de::Error::custom("count"));
            }
        }
        Ok(())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<(), A::Error> {
        if !self.shape.permits_object() {
            return Err(de::Error::custom("shape"));
        }
        if self.depth >= MAX_JSON_DEPTH {
            self.depth_exceeded.set(true);
            return Err(de::Error::custom("depth"));
        }
        let mut keys = BTreeSet::new();
        while let Some(key) = a.next_key::<String>()? {
            let limit = match key.as_str() {
                "resources" | "categories" | "logical_roots" | "selected_keys" => MAX_RESOURCES,
                _ => MAX_RELATIONS,
            };
            let shape = self.shape.field(&key);
            if !keys.insert(key) {
                return Err(de::Error::custom("duplicate"));
            }
            a.next_value_seed(JsonCheck {
                shape,
                depth_exceeded: self.depth_exceeded,
                depth: self.depth + 1,
                list_limit: limit,
            })?;
        }
        Ok(())
    }
}
fn frame(hasher: &mut Sha256, bytes: &[u8]) -> Result<(), BundleError> {
    let length = u64::try_from(bytes.len()).map_err(|_| BundleError::Limit)?;
    hasher.update(length.to_be_bytes());
    hasher.update(bytes);
    Ok(())
}
fn finish(hasher: Sha256, purpose: DigestPurpose) -> SourceDigest {
    SourceDigest {
        purpose,
        bytes: hasher.finalize().into(),
    }
}
fn unique<T: Ord>(items: &[T]) -> bool {
    items.iter().collect::<BTreeSet<_>>().len() == items.len()
}
fn in_roots(file: &LogicalName, roots: &[LogicalName]) -> bool {
    roots.iter().any(|root| {
        file == root
            || file
                .as_str()
                .strip_prefix(root.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

pub fn validate_bundle(
    inventory_bytes: &[u8],
    files: &[SuppliedFile<'_>],
) -> Result<ValidatedBundle, BundleError> {
    if inventory_bytes.len() > MAX_INVENTORY_BYTES || files.len() > MAX_FILES {
        return Err(BundleError::Limit);
    }
    let mut total = inventory_bytes.len();
    for file in files {
        if file.bytes.len() > MAX_FILE_BYTES {
            return Err(BundleError::Limit);
        }
        total = total
            .checked_add(file.bytes.len())
            .ok_or(BundleError::Limit)?;
        if total > MAX_COMBINED_BYTES {
            return Err(BundleError::Limit);
        }
        locator(file.logical_name)?;
        media_type(file.media_type)?;
    }
    let mut decoder = serde_json::Deserializer::from_slice(inventory_bytes);
    let depth_exceeded = Cell::new(false);
    JsonCheck {
        shape: JsonShape::Inventory,
        depth_exceeded: &depth_exceeded,
        depth: 0,
        list_limit: MAX_RELATIONS,
    }
    .deserialize(&mut decoder)
    .map_err(|_| {
        if depth_exceeded.get() {
            BundleError::JsonDepth
        } else {
            BundleError::Inventory
        }
    })?;
    decoder.end().map_err(|_| BundleError::Inventory)?;
    let inventory: Inventory =
        serde_json::from_slice(inventory_bytes).map_err(|_| BundleError::Inventory)?;
    if inventory.profile != PROFILE || inventory.profile_revision != PROFILE_REVISION {
        return Err(BundleError::Profile);
    }
    let partition = &inventory.partition;
    if partition.categories.is_empty() || partition.logical_roots.is_empty() {
        return Err(BundleError::Partition);
    }
    if !unique(&partition.categories)
        || !unique(&partition.logical_roots)
        || !unique(&partition.selected_keys)
    {
        return Err(BundleError::Duplicate);
    }
    let categories: BTreeSet<_> = partition.categories.iter().collect();
    let selected: BTreeSet<_> = partition.selected_keys.iter().collect();
    if selected
        .iter()
        .any(|key| !categories.contains(&key.category))
    {
        return Err(BundleError::Partition);
    }
    let mut sorted_files: Vec<_> = files.iter().collect();
    sorted_files.sort_by(|a, b| a.logical_name.as_bytes().cmp(b.logical_name.as_bytes()));
    if sorted_files
        .windows(2)
        .any(|pair| pair[0].logical_name == pair[1].logical_name)
    {
        return Err(BundleError::Duplicate);
    }
    let names: BTreeSet<_> = files.iter().map(|f| f.logical_name).collect();
    let check_file = |file: &LogicalName| {
        if !in_roots(file, &partition.logical_roots) {
            return Err(BundleError::Partition);
        }
        if !names.contains(file.as_str()) {
            return Err(BundleError::Unavailable);
        }
        Ok(())
    };
    let mut keys = BTreeSet::new();
    for resource in &inventory.resources {
        if !keys.insert(&resource.key) {
            return Err(BundleError::Duplicate);
        }
        if !categories.contains(&resource.key.category)
            || (!selected.is_empty() && !selected.contains(&resource.key))
        {
            return Err(BundleError::Partition);
        }
        check_file(&resource.file)?;
    }
    let mut occurrences = BTreeSet::new();
    for relation in &inventory.relations {
        if !occurrences.insert((&relation.owner, &relation.occurrence)) {
            return Err(BundleError::Duplicate);
        }
        if !keys.contains(&relation.owner) {
            return Err(BundleError::Unavailable);
        }
        check_file(&relation.file)?;
    }
    let mut inventory_hash = Sha256::new();
    inventory_hash.update(b"atlas.source.inventory.v1\0");
    frame(&mut inventory_hash, inventory_bytes)?;
    let mut bundle_hash = Sha256::new();
    bundle_hash.update(b"atlas.source.bundle.v1\0");
    frame(&mut bundle_hash, inventory_bytes)?;
    bundle_hash.update(
        u64::try_from(files.len())
            .map_err(|_| BundleError::Limit)?
            .to_be_bytes(),
    );
    let mut retained = Vec::with_capacity(files.len());
    for file in sorted_files {
        frame(&mut bundle_hash, file.logical_name.as_bytes())?;
        frame(&mut bundle_hash, file.media_type.as_bytes())?;
        frame(&mut bundle_hash, file.bytes)?;
        let mut file_hash = Sha256::new();
        file_hash.update(file.bytes);
        retained.push(ValidatedFile {
            logical_name: LogicalName::new(file.logical_name)?,
            media_type: MediaType::new(file.media_type)?,
            bytes: file.bytes.to_vec(),
            digest: finish(file_hash, DigestPurpose::FileContentSha256),
        });
    }
    Ok(ValidatedBundle {
        inventory,
        inventory_bytes: inventory_bytes.to_vec(),
        files: retained,
        inventory_digest: finish(inventory_hash, DigestPurpose::InventoryV1),
        bundle_digest: finish(bundle_hash, DigestPurpose::BundleV1),
    })
}
