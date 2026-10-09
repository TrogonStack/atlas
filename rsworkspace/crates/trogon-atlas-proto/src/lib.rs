#[rustfmt::skip]
#[allow(clippy::all, clippy::pedantic, clippy::unwrap_used, clippy::expect_used)]
#[path = "gen/mod.rs"]
mod generated;

pub use generated::trogonatlas::{
    annotation::v1alpha1::*, api::eventmodel::v1alpha1::*, catalog::v1alpha1::*,
    eventmodel::v1alpha1::*, r#type::v1alpha1::*,
};

pub const SCHEMA_VERSION: &str = "trogonatlas.eventmodel.v1alpha1";

/// Wire contract revision this build speaks, advertised by the server as
/// `GetServerInfoResponse.contract_revision` and checked by clients before
/// mutating. See `docs/explanation/wire-compatibility.md` for when it changes.
pub const CONTRACT_REVISION: u32 = 8;

/// Oldest server contract revision clients built from this tree mutate
/// against.
pub const MIN_SERVER_CONTRACT_REVISION: u32 = 8;

/// Oldest client contract revision a server built from this tree accepts
/// mutations from.
pub const MIN_CLIENT_CONTRACT_REVISION: u32 = 8;

/// Serialized `FileDescriptorSet` for the schema, for reflection-based
/// tooling (textproto/JSON transcoding).
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!("gen/descriptor.binpb");

/// Hashable identity newtype carrying the (kind, namespace, slug, version)
/// quadruple. Replaces the ad-hoc `(EntityKind, String, String, u64)` tuple
/// used as a HashMap/HashSet key across the validation and graph passes, and
/// gives misorderings a compile error, equips lookups with a `Display`
/// matching `canonical::id_string`, and concentrates conversion from a
/// `(kind, &Id)` pair in one place.
///
/// The `Id` fields (`namespace`, `slug`) must be valid id components as
/// validated by `trogon_atlas_core::validate_id_component` before key
/// construction. This type does not re-validate; callers are responsible.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntityKey {
    pub kind: EntityKind,
    pub namespace: String,
    pub slug: String,
    pub version: u64,
}

impl EntityKey {
    #[must_use]
    pub fn new(kind: EntityKind, id: &Id) -> Self {
        Self {
            kind,
            namespace: id.namespace.clone(),
            slug: id.slug.clone(),
            version: id.version,
        }
    }

    #[must_use]
    pub fn id(&self) -> Id {
        Id {
            namespace: self.namespace.clone(),
            slug: self.slug.clone(),
            version: self.version,
        }
    }
}

impl std::fmt::Display for EntityKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", canonical::id_string(self.kind, &self.id()))
    }
}

impl From<(EntityKind, &Id)> for EntityKey {
    fn from((kind, id): (EntityKind, &Id)) -> Self {
        Self::new(kind, id)
    }
}

/// Kind-less identity newtype carrying the (namespace, slug, version) triple.
/// Used when a HashMap/HashSet is keyed by Id regardless of entity kind
/// (e.g. swimlane-by-stream lookups, per-id processor maps).
///
/// The `Id` fields (`namespace`, `slug`) must be valid id components as
/// validated by `trogon_atlas_core::validate_id_component` before key
/// construction. This type does not re-validate; callers are responsible.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdKey {
    pub namespace: String,
    pub slug: String,
    pub version: u64,
}

impl IdKey {
    #[must_use]
    pub fn new(id: &Id) -> Self {
        Self {
            namespace: id.namespace.clone(),
            slug: id.slug.clone(),
            version: id.version,
        }
    }

    #[must_use]
    pub fn id(&self) -> Id {
        Id {
            namespace: self.namespace.clone(),
            slug: self.slug.clone(),
            version: self.version,
        }
    }
}

impl std::fmt::Display for IdKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}@{}", self.namespace, self.slug, self.version)
    }
}

impl From<&Id> for IdKey {
    fn from(id: &Id) -> Self {
        Self::new(id)
    }
}

pub mod canonical {
    use super::{field_type::Kind as FieldTypeKind, EntityKind, Id};

    /// Stable label for a field's kind: what tools print and compare.
    #[must_use]
    pub fn field_type_label(t: &super::FieldType) -> &'static str {
        match t.kind.as_ref() {
            Some(FieldTypeKind::String(_)) => "string",
            Some(FieldTypeKind::Bool(_)) => "bool",
            Some(FieldTypeKind::Int(_)) => "int",
            Some(FieldTypeKind::Float(_)) => "float",
            Some(FieldTypeKind::Timestamp(_)) => "timestamp",
            Some(FieldTypeKind::Uuid(_)) => "uuid",
            Some(FieldTypeKind::Bytes(_)) => "bytes",
            Some(FieldTypeKind::Enumeration(_)) => "enum",
            Some(FieldTypeKind::Object(_)) => "object",
            Some(FieldTypeKind::Ref(_)) => "ref",
            None => "unspecified",
        }
    }

    /// Two field types are flow-compatible when their kinds match; the
    /// per-kind configuration refines, it does not re-type.
    ///
    /// Returns `false` when either argument is `None`.
    #[must_use]
    pub fn field_types_compatible(
        a: Option<&super::FieldType>,
        b: Option<&super::FieldType>,
    ) -> bool {
        match (a, b) {
            (Some(x), Some(y)) => field_type_label(x) == field_type_label(y),
            _ => false,
        }
    }

    /// Canonical Id string used as the key in projection maps and as a
    /// stable hashable identifier for entities. Format:
    ///   "<kind>:<namespace>/<slug>@<version>"
    /// `kind` is the lowercase short form of the `EntityKind` enum
    /// (e.g. "event", "`command_slice`"). Callers may construct this from
    /// an Id; do not parse it back; use the entity's own Id field for
    /// semantic decisions.
    pub fn id_string(kind: EntityKind, id: &Id) -> String {
        debug_assert!(
            kind != EntityKind::Unspecified,
            "id_string called with EntityKind::Unspecified; caller must supply a concrete kind"
        );
        if kind == EntityKind::Unspecified {
            tracing::error!(
                namespace = %id.namespace,
                slug = %id.slug,
                version = id.version,
                "id_string called with EntityKind::Unspecified; emitting sentinel key"
            );
        }
        format!(
            "{}:{}/{}@{}",
            kind_short(kind),
            id.namespace,
            id.slug,
            id.version
        )
    }

    #[must_use]
    pub fn kind_short(kind: EntityKind) -> &'static str {
        match kind {
            EntityKind::Unspecified => "unspecified",
            EntityKind::Event => "event",
            EntityKind::Command => "command",
            EntityKind::ReadModel => "read_model",
            EntityKind::Processor => "processor",
            EntityKind::Ui => "ui",
            EntityKind::Persona => "persona",
            EntityKind::Swimlane => "swimlane",
            EntityKind::CommandSlice => "command_slice",
            EntityKind::ReadModelSlice => "read_model_slice",
            EntityKind::AutomationSlice => "automation_slice",
            EntityKind::Storyboard => "storyboard",
            EntityKind::EventModel => "event_model",
            EntityKind::Component => "component",
            EntityKind::ExternalSystem => "external_system",
            EntityKind::Tracker => "tracker",
            EntityKind::BoundedContext => "bounded_context",
            EntityKind::Domain => "domain",
            EntityKind::Subdomain => "subdomain",
            EntityKind::Schema => "schema",
            EntityKind::Project => "project",
            EntityKind::Screen => "screen",
            EntityKind::Term => "term",
            EntityKind::Ambiguity => "ambiguity",
            EntityKind::UiSlice => "ui_slice",
            EntityKind::ServiceLevelIndicator => "service_level_indicator",
            EntityKind::ServiceLevelObjective => "service_level_objective",
            EntityKind::AlertPolicy => "alert_policy",
            EntityKind::AlertNotificationTarget => "alert_notification_target",
            EntityKind::TypeLibrary => "type_library",
        }
    }

    /// Every concrete `EntityKind` in declaration order. Single source of
    /// truth for "the set of kinds": extending the proto enum means
    /// extending this list, and downstream code (`parse_kind`, kind
    /// allow-lists, exhaustive iteration) reads it directly.
    pub const ALL_KINDS: &[EntityKind] = &[
        EntityKind::Event,
        EntityKind::Command,
        EntityKind::ReadModel,
        EntityKind::Processor,
        EntityKind::Ui,
        EntityKind::Persona,
        EntityKind::Swimlane,
        EntityKind::CommandSlice,
        EntityKind::ReadModelSlice,
        EntityKind::AutomationSlice,
        EntityKind::Storyboard,
        EntityKind::EventModel,
        EntityKind::Component,
        EntityKind::ExternalSystem,
        EntityKind::Tracker,
        EntityKind::BoundedContext,
        EntityKind::Domain,
        EntityKind::Subdomain,
        EntityKind::Schema,
        EntityKind::Project,
        EntityKind::Screen,
        EntityKind::Term,
        EntityKind::Ambiguity,
        EntityKind::UiSlice,
        EntityKind::ServiceLevelIndicator,
        EntityKind::ServiceLevelObjective,
        EntityKind::AlertPolicy,
        EntityKind::AlertNotificationTarget,
        EntityKind::TypeLibrary,
    ];

    /// Parse a kind string into an `EntityKind`. Accepts the canonical
    /// `snake_case` form (`read_model`), the alphanumeric collapse
    /// (`readmodel`), the dash-separated form (`read-model`), and the
    /// proto enum / wire name (`ENTITY_KIND_READ_MODEL`). Case
    /// insensitive. Returns the `EntityKind` on a unique match.
    ///
    /// The lookup uses a precomputed static table; no per-call allocation.
    pub fn parse_kind(s: &str) -> Option<EntityKind> {
        use std::{collections::HashMap, sync::LazyLock};
        static TABLE: LazyLock<HashMap<String, EntityKind>> = LazyLock::new(|| {
            let mut m = HashMap::new();
            for &k in ALL_KINDS {
                let canonical = kind_short(k);
                m.insert(canonical.to_string(), k);
                let collapsed = canonical.replace('_', "");
                m.entry(collapsed).or_insert(k);
                let dashed = canonical.replace('_', "-");
                m.entry(dashed).or_insert(k);
            }
            m
        });
        let lowered = s.trim().to_ascii_lowercase().replace('-', "_");
        // Wire / proto-JSON names are `ENTITY_KIND_EVENT`; strip so the same
        // table serves MCP, CLI, manifests, and studio query strings.
        let normalized = lowered
            .strip_prefix("entity_kind_")
            .unwrap_or(lowered.as_str());
        if let Some(&k) = TABLE.get(normalized) {
            return Some(k);
        }
        let collapsed = normalized.replace('_', "");
        TABLE.get(&collapsed).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical::*, EntityKind};

    #[test]
    fn all_kinds_length_matches_highest_variant() {
        let max_discriminant = ALL_KINDS.iter().map(|&k| k as i32).max().unwrap_or(0);
        #[allow(clippy::expect_used)]
        let all_kinds_len =
            i32::try_from(ALL_KINDS.len()).expect("ALL_KINDS is too large to fit in i32");
        assert_eq!(
            all_kinds_len, max_discriminant,
            "ALL_KINDS length must equal the highest enum discriminant; \
             add the new kind to ALL_KINDS when extending the proto enum"
        );
    }

    #[test]
    fn all_kinds_contains_no_duplicates() {
        use std::collections::HashSet;
        let discriminants: HashSet<i32> = ALL_KINDS.iter().map(|&k| k as i32).collect();
        assert_eq!(
            discriminants.len(),
            ALL_KINDS.len(),
            "ALL_KINDS contains duplicate discriminants; each kind must appear exactly once"
        );
    }

    // This test validates the release-mode contract: id_string(Unspecified)
    // must not panic and must produce an "unspecified:" prefixed string so
    // miscalled sites are visible in logs. The debug_assert inside
    // id_string fires in debug builds (including `cargo test`), so this
    // test is gated on release semantics only.
    #[test]
    #[cfg(not(debug_assertions))]
    fn id_string_with_unspecified_is_safe_in_release() {
        let id = super::Id {
            namespace: "ns".into(),
            slug: "slug".into(),
            version: 1,
        };
        let s = id_string(EntityKind::Unspecified, &id);
        assert!(
            s.starts_with("unspecified:"),
            "id_string(Unspecified) must produce an 'unspecified:' prefixed string in release, got: {s}"
        );
    }

    #[test]
    fn all_kinds_round_trip_through_kind_short_and_parse_kind() {
        for &k in ALL_KINDS {
            let short = kind_short(k);
            let parsed = parse_kind(short).unwrap_or_else(|| {
                panic!("parse_kind failed to round-trip kind_short({k:?}) = {short:?}")
            });
            assert_eq!(parsed, k, "round-trip mismatch for {k:?}");
        }
    }

    #[test]
    fn parse_kind_accepts_dash_form() {
        assert_eq!(parse_kind("read-model"), Some(EntityKind::ReadModel));
        assert_eq!(parse_kind("command-slice"), Some(EntityKind::CommandSlice));
        assert_eq!(
            parse_kind("automation-slice"),
            Some(EntityKind::AutomationSlice)
        );
    }

    #[test]
    fn parse_kind_accepts_collapsed_form() {
        assert_eq!(parse_kind("readmodel"), Some(EntityKind::ReadModel));
        assert_eq!(parse_kind("commandslice"), Some(EntityKind::CommandSlice));
        assert_eq!(
            parse_kind("boundedcontext"),
            Some(EntityKind::BoundedContext)
        );
    }

    #[test]
    fn parse_kind_is_case_insensitive() {
        assert_eq!(parse_kind("EVENT"), Some(EntityKind::Event));
        assert_eq!(parse_kind("ReadModel"), Some(EntityKind::ReadModel));
    }

    #[test]
    fn parse_kind_returns_none_for_unknown() {
        assert_eq!(parse_kind("not_a_kind"), None);
        assert_eq!(parse_kind(""), None);
    }

    // Wire / proto-enum names (`ENTITY_KIND_EVENT`) are what gRPC JSON and the
    // studio BFF speak. parse_kind already accepts snake/dash/collapsed forms;
    // rejecting the ENTITY_KIND_ prefix forces callers to translate by hand
    // and breaks copy-paste from wire payloads into MCP/CLI/manifests.
    #[test]
    fn parse_kind_accepts_entity_kind_prefix() {
        assert_eq!(parse_kind("ENTITY_KIND_EVENT"), Some(EntityKind::Event));
        assert_eq!(
            parse_kind("ENTITY_KIND_READ_MODEL"),
            Some(EntityKind::ReadModel)
        );
        assert_eq!(
            parse_kind("entity_kind_command_slice"),
            Some(EntityKind::CommandSlice)
        );
        assert_eq!(parse_kind("ENTITY_KIND_UNSPECIFIED"), None);
        assert_eq!(parse_kind("ENTITY_KIND_NOT_A_KIND"), None);
    }

    #[test]
    fn kind_short_covers_all_kinds() {
        for &k in ALL_KINDS {
            let s = kind_short(k);
            assert!(!s.is_empty(), "kind_short returned empty for {k:?}");
            assert_ne!(s, "unspecified", "ALL_KINDS must not contain Unspecified");
        }
    }
}
