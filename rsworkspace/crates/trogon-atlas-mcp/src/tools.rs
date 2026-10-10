use serde::Deserialize;

fn default_page_size() -> u32 {
    100
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntitySpecDe {
    kind: String,
    namespace: String,
    slug: String,
    version: u64,
}

#[derive(Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntitySpec {
    #[schemars(
        description = "Entity kind: event, command, read_model, processor, ui, persona, swimlane, command_slice, read_model_slice, automation_slice, ui_slice, storyboard, event_model"
    )]
    pub kind: String,
    pub namespace: String,
    pub slug: String,
    #[schemars(description = "Schema version number (>0), e.g. 1")]
    pub version: u64,
}

impl<'de> Deserialize<'de> for EntitySpec {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = EntitySpecDe::deserialize(deserializer)?;
        if raw.version == 0 {
            return Err(serde::de::Error::custom("version must be > 0"));
        }
        Ok(Self {
            kind: raw.kind,
            namespace: raw.namespace,
            slug: raw.slug,
            version: raw.version,
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmptyParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListEntitiesParams {
    #[schemars(description = "Restrict by entity kind (optional)")]
    pub kind: Option<String>,
    pub namespace: Option<String>,
    #[schemars(description = "Page size (1-500). Defaults to 100.", range(max = 500))]
    #[serde(default = "default_page_size")]
    pub page_size: u32,
    pub page_token: Option<String>,
    #[schemars(
        description = "Include only entities whose LifecycleAnnotation status (lowercased, trimmed) is in this list. Use the empty string \"\" to match entities with no annotation. Omit to skip lifecycle filtering."
    )]
    #[serde(default)]
    pub lifecycle_status_in: Option<Vec<String>>,
    #[schemars(
        description = "Exclude entities whose LifecycleAnnotation status (lowercased, trimmed) is in this list. Use \"\" to exclude entities with no annotation. Composes with lifecycle_status_in as AND."
    )]
    #[serde(default)]
    pub lifecycle_status_not_in: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchGetParams {
    #[schemars(description = "Entity refs to fetch. Maximum 100 refs per request.")]
    pub refs: Vec<EntitySpec>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListVersionsParams {
    pub kind: String,
    pub namespace: String,
    pub slug: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetLatestVersionParams {
    pub kind: String,
    pub namespace: String,
    pub slug: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SupersessionChainParams {
    pub kind: String,
    pub namespace: String,
    pub slug: String,
    pub version: u64,
    #[serde(default)]
    pub include_descendants: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReferencesParams {
    pub kind: String,
    pub namespace: String,
    pub slug: String,
    pub version: u64,
    #[schemars(description = "Restrict referrers/referents to these kinds (optional list)")]
    pub filter_kinds: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImpactParamsDe {
    kind: String,
    namespace: String,
    slug: String,
    version: u64,
    #[serde(default)]
    max_depth: u32,
    #[serde(default)]
    filter_kinds: Option<Vec<String>>,
}

#[derive(Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactParams {
    pub kind: String,
    pub namespace: String,
    pub slug: String,
    /// Schema version (>0). Version 0 is never a valid entity id.
    pub version: u64,
    #[schemars(
        description = "Max BFS depth from root. 0 = server default (16). Values above 16 are capped at 16."
    )]
    #[serde(default)]
    pub max_depth: u32,
    pub filter_kinds: Option<Vec<String>>,
}

impl<'de> Deserialize<'de> for ImpactParams {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = ImpactParamsDe::deserialize(deserializer)?;
        if raw.version == 0 {
            return Err(serde::de::Error::custom("version must be > 0"));
        }
        Ok(Self {
            kind: raw.kind,
            namespace: raw.namespace,
            slug: raw.slug,
            version: raw.version,
            max_depth: raw.max_depth,
            filter_kinds: raw.filter_kinds,
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectionParams {
    pub namespace: String,
    pub slug: String,
    pub version: u64,
    #[serde(default)]
    pub include_cross_model: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SliceProjectionParams {
    #[schemars(
        description = "Slice kind: command_slice, read_model_slice, automation_slice, ui_slice"
    )]
    pub kind: String,
    pub namespace: String,
    pub slug: String,
    pub version: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtractSubgraphParams {
    /// Root entity that scopes the subgraph extraction. The closure of
    /// references from this root becomes the new (unpersisted) `EventModel`.
    /// The underlying RPC accepts a single `AnalysisScope`, so this tool
    /// takes a single root rather than a list.
    pub root: EntitySpec,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiffEntitiesParams {
    pub a: EntitySpec,
    pub b: EntitySpec,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchParams {
    #[schemars(description = "Full-text search query. Maximum 2048 bytes.")]
    pub q: String,
    pub kinds: Option<Vec<String>>,
    #[schemars(description = "Page size (1-500). Defaults to 100.", range(max = 500))]
    #[serde(default = "default_page_size")]
    pub page_size: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListChangesParams {
    pub since_token: Option<String>,
    #[schemars(description = "Page size (1-500). Defaults to 100.", range(max = 500))]
    #[serde(default = "default_page_size")]
    pub page_size: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListChangesetsParams {
    #[schemars(description = "Id of the oldest changeset already seen. Omit for the newest page.")]
    pub page_token: Option<String>,
    #[schemars(description = "Restrict to changesets recorded on this branch. Omit for all.")]
    pub branch: Option<String>,
    #[schemars(description = "Page size (1-500). Defaults to 100.", range(max = 500))]
    #[serde(default = "default_page_size")]
    pub page_size: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetChangesetParams {
    #[schemars(description = "Changeset id, as returned by list_changesets or a change event.")]
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEntityHistoryParams {
    pub kind: String,
    pub namespace: String,
    pub slug: String,
    pub version: u64,
    #[schemars(
        description = "Changeset id of the oldest revision already seen. Omit for the newest page."
    )]
    pub page_token: Option<String>,
    #[schemars(description = "Page size (1-500). Defaults to 100.", range(max = 500))]
    #[serde(default = "default_page_size")]
    pub page_size: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevertChangesetParams {
    #[schemars(description = "Changeset id to undo, as returned by list_changesets.")]
    pub id: String,
    #[schemars(description = "Report the inverse ops without writing them.")]
    #[serde(default)]
    pub dry_run: bool,
    #[schemars(
        description = "Idempotency key: replaying the same operation_id with the same arguments returns the original outcome instead of reverting twice. Look its status up with get_operation. Must not be set together with dry_run."
    )]
    #[serde(default)]
    pub operation_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidateEventModelParams {
    pub namespace: String,
    pub slug: String,
    pub version: u64,
    /// When true, return the response as proto3-rendered JSON inside a
    /// `{ "json": "..." }` envelope instead of the default
    /// `{ "binpb_base64": "..." }`. Lets clients skip a varint decoder.
    #[serde(default)]
    pub json: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JsonOpts {
    /// When true, return the response as proto3-rendered JSON inside a
    /// `{ "json": "..." }` envelope instead of the default
    /// `{ "binpb_base64": "..." }`.
    #[serde(default)]
    pub json: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidateProjectParamsDe {
    #[serde(default)]
    project_namespace: Option<String>,
    #[serde(default)]
    project_slug: Option<String>,
    #[serde(default)]
    project_version: Option<u64>,
    #[serde(default)]
    domain_namespace: Option<String>,
    #[serde(default)]
    domain_slug: Option<String>,
    #[serde(default)]
    domain_version: Option<u64>,
    #[serde(default)]
    json: Option<bool>,
}

#[derive(Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidateProjectParams {
    #[serde(default)]
    pub project_namespace: Option<String>,
    #[serde(default)]
    pub project_slug: Option<String>,
    /// Required (>0) when project_namespace+project_slug are set. Omitted must not become 0.
    pub project_version: Option<u64>,
    #[serde(default)]
    pub domain_namespace: Option<String>,
    #[serde(default)]
    pub domain_slug: Option<String>,
    /// Required (>0) when domain_namespace+domain_slug are set. Omitted must not become 0.
    pub domain_version: Option<u64>,
    #[serde(default)]
    pub json: Option<bool>,
}

impl<'de> Deserialize<'de> for ValidateProjectParams {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = ValidateProjectParamsDe::deserialize(deserializer)?;
        Self::try_from(raw).map_err(serde::de::Error::custom)
    }
}

impl TryFrom<ValidateProjectParamsDe> for ValidateProjectParams {
    type Error = &'static str;

    fn try_from(raw: ValidateProjectParamsDe) -> Result<Self, Self::Error> {
        let has_project = raw.project_namespace.is_some() || raw.project_slug.is_some();
        let has_domain = raw.domain_namespace.is_some() || raw.domain_slug.is_some();
        if has_project && has_domain {
            return Err(
                "scope is exclusive: set project_namespace+project_slug OR domain_namespace+domain_slug, not both",
            );
        }
        if has_project {
            match raw.project_version {
                Some(v) if v > 0 => {}
                _ => {
                    return Err(
                        "project_version is required and must be > 0 when project scope is set",
                    );
                }
            }
        } else if let Some(0) = raw.project_version {
            return Err("project_version must be > 0");
        }
        if has_domain {
            match raw.domain_version {
                Some(v) if v > 0 => {}
                _ => {
                    return Err(
                        "domain_version is required and must be > 0 when domain scope is set",
                    );
                }
            }
        } else if let Some(0) = raw.domain_version {
            return Err("domain_version must be > 0");
        }
        Ok(Self {
            project_namespace: raw.project_namespace,
            project_slug: raw.project_slug,
            project_version: raw.project_version,
            domain_namespace: raw.domain_namespace,
            domain_slug: raw.domain_slug,
            domain_version: raw.domain_version,
            json: raw.json,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteByQueryParamsDe {
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    max_deletes: Option<u32>,
    #[serde(default)]
    json: Option<bool>,
}

#[derive(Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteByQueryParams {
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[schemars(
        description = "Required safety cap: refuse to delete more than this many entities. Must be > 0 (server rejects uncapped deletes)."
    )]
    pub max_deletes: u32,
    #[serde(default)]
    pub json: Option<bool>,
}

impl<'de> Deserialize<'de> for DeleteByQueryParams {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = DeleteByQueryParamsDe::deserialize(deserializer)?;
        Self::try_from(raw).map_err(serde::de::Error::custom)
    }
}

impl TryFrom<DeleteByQueryParamsDe> for DeleteByQueryParams {
    type Error = &'static str;

    fn try_from(raw: DeleteByQueryParamsDe) -> Result<Self, Self::Error> {
        let max_deletes = match raw.max_deletes {
            Some(v) if v > 0 => v,
            Some(0) => {
                return Err("max_deletes must be > 0 (server refuses uncapped deletes)");
            }
            None | Some(_) => {
                return Err(
                    "max_deletes is required and must be > 0 (server refuses uncapped deletes)",
                );
            }
        };
        Ok(Self {
            project: raw.project,
            namespace: raw.namespace,
            kind: raw.kind,
            slug: raw.slug,
            mode: raw.mode,
            max_deletes,
            json: raw.json,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListEntitiesByDomainParamsDe {
    namespace: String,
    slug: String,
    #[serde(default)]
    version: Option<u64>,
    #[serde(default)]
    kinds: Option<Vec<String>>,
    #[serde(default)]
    page_size: Option<u32>,
    #[serde(default)]
    page_token: Option<String>,
    #[serde(default)]
    latest_versions_only: Option<bool>,
    #[serde(default)]
    json: Option<bool>,
}

#[derive(Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListEntitiesByDomainParams {
    pub namespace: String,
    pub slug: String,
    /// Required schema version (>0). Omitted must not become 0.
    pub version: u64,
    #[serde(default)]
    pub kinds: Option<Vec<String>>,
    #[schemars(
        description = "Page size (1-500, 0 = backend default).",
        range(max = 500)
    )]
    #[serde(default)]
    pub page_size: Option<u32>,
    #[serde(default)]
    pub page_token: Option<String>,
    #[serde(default)]
    pub latest_versions_only: Option<bool>,
    #[serde(default)]
    pub json: Option<bool>,
}

impl<'de> Deserialize<'de> for ListEntitiesByDomainParams {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = ListEntitiesByDomainParamsDe::deserialize(deserializer)?;
        Self::try_from(raw).map_err(serde::de::Error::custom)
    }
}

impl TryFrom<ListEntitiesByDomainParamsDe> for ListEntitiesByDomainParams {
    type Error = &'static str;

    fn try_from(raw: ListEntitiesByDomainParamsDe) -> Result<Self, Self::Error> {
        let version = match raw.version {
            Some(v) if v > 0 => v,
            Some(0) => return Err("version must be > 0"),
            None | Some(_) => return Err("version is required and must be > 0"),
        };
        Ok(Self {
            namespace: raw.namespace,
            slug: raw.slug,
            version,
            kinds: raw.kinds,
            page_size: raw.page_size,
            page_token: raw.page_token,
            latest_versions_only: raw.latest_versions_only,
            json: raw.json,
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisScopeParams {
    #[schemars(
        description = "Scope kind: command_slice, read_model_slice, automation_slice, ui_slice, storyboard, event_model"
    )]
    pub kind: String,
    pub namespace: String,
    pub slug: String,
    pub version: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PutEntityParams {
    #[schemars(description = "Base64-encoded prost binpb of a fully-constructed Entity message")]
    pub entity_b64: String,
    #[serde(default)]
    pub create_only: bool,
    #[serde(default)]
    pub if_match: String,
    #[serde(default)]
    pub validate_only: bool,
    #[serde(default)]
    #[schemars(
        description = "Overwrite even when the stored entity is semantically identical; by default such writes are skipped (response no_op=true)"
    )]
    pub force: bool,
    #[schemars(
        description = "Idempotency key: replaying the same operation_id with the same arguments returns the original outcome instead of writing twice. Look its status up with get_operation. Must not be set together with validate_only."
    )]
    #[serde(default)]
    pub operation_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PutEntityJsonParams {
    #[schemars(
        description = "proto3 JSON of an Entity message, e.g. {\"event\": {\"id\": {\"namespace\": \"shop\", \"slug\": \"order-placed\", \"version\": 1}, \"title\": \"Order placed\"}}"
    )]
    pub entity_json: String,
    #[serde(default)]
    pub create_only: bool,
    #[serde(default)]
    pub if_match: String,
    #[serde(default)]
    pub validate_only: bool,
    #[serde(default)]
    #[schemars(
        description = "Overwrite even when the stored entity is semantically identical; by default such writes are skipped (response no_op=true)"
    )]
    pub force: bool,
    #[schemars(
        description = "Idempotency key: replaying the same operation_id with the same arguments returns the original outcome instead of writing twice. Look its status up with get_operation. Must not be set together with validate_only."
    )]
    #[serde(default)]
    pub operation_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchMutateJsonParams {
    #[schemars(description = "proto3 JSON of a BatchMutateRequest message")]
    pub request_json: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteEntityParams {
    pub kind: String,
    pub namespace: String,
    pub slug: String,
    pub version: u64,
    #[serde(default)]
    pub if_match: String,
    #[schemars(description = "Mode: fail_if_referenced (default), force, dry_run")]
    pub mode: Option<String>,
    #[schemars(
        description = "Idempotency key: replaying the same operation_id with the same arguments returns the original outcome instead of deleting twice. Look its status up with get_operation. Must not be set together with mode=dry_run."
    )]
    #[serde(default)]
    pub operation_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchMutateOpJson {
    #[schemars(description = "Either 'put' or 'delete'")]
    pub op: String,
    pub entity_b64: Option<String>,
    pub kind: Option<String>,
    pub namespace: Option<String>,
    pub slug: Option<String>,
    pub version: Option<u64>,
    #[serde(default)]
    pub create_only: bool,
    #[serde(default)]
    pub if_match: String,
    #[serde(default)]
    pub force: bool,
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchMutateParams {
    #[schemars(
        description = "Mutation operations to execute atomically. Maximum 50 ops per request."
    )]
    pub ops: Vec<BatchMutateOpJson>,
    #[serde(default)]
    pub validate_only: bool,
    #[schemars(
        description = "Idempotency key for the whole batch: replaying the same operation_id with the same ops returns the original outcome instead of applying twice. Look its status up with get_operation. Must not be set together with validate_only, and never set per-op."
    )]
    #[serde(default)]
    pub operation_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetargetReferencesParamsDe {
    from_kind: String,
    from_namespace: String,
    from_slug: String,
    from_version: u64,
    #[serde(default)]
    to_namespace: Option<String>,
    #[serde(default)]
    to_slug: Option<String>,
    to_version: u64,
    #[serde(default)]
    dry_run: bool,
    #[serde(default)]
    filter_kinds: Option<Vec<String>>,
}

#[derive(Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RetargetReferencesParams {
    /// Kind of the entity being replaced (e.g. "event", "command").
    pub from_kind: String,
    pub from_namespace: String,
    pub from_slug: String,
    /// Schema version of the entity being replaced (>0).
    pub from_version: u64,
    /// Namespace of the replacement. Defaults to from_namespace when omitted.
    #[serde(default)]
    pub to_namespace: Option<String>,
    /// Slug of the replacement. Defaults to from_slug when omitted.
    #[serde(default)]
    pub to_slug: Option<String>,
    /// Schema version of the replacement (>0).
    pub to_version: u64,
    /// When true, return what would change without writing.
    #[serde(default)]
    pub dry_run: bool,
    /// Restrict which referrer kinds are rewritten. Empty = all.
    #[serde(default)]
    pub filter_kinds: Option<Vec<String>>,
}

impl<'de> Deserialize<'de> for RetargetReferencesParams {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RetargetReferencesParamsDe::deserialize(deserializer)?;
        if raw.from_version == 0 {
            return Err(serde::de::Error::custom("from_version must be > 0"));
        }
        if raw.to_version == 0 {
            return Err(serde::de::Error::custom("to_version must be > 0"));
        }
        Ok(Self {
            from_kind: raw.from_kind,
            from_namespace: raw.from_namespace,
            from_slug: raw.from_slug,
            from_version: raw.from_version,
            to_namespace: raw.to_namespace,
            to_slug: raw.to_slug,
            to_version: raw.to_version,
            dry_run: raw.dry_run,
            filter_kinds: raw.filter_kinds,
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyManifestsParams {
    #[schemars(
        description = "One or more YAML manifest documents (separated by `---`), each with apiVersion/kind/metadata/spec, as accepted by trogon-atlas apply."
    )]
    pub manifests_yaml: String,
    #[schemars(description = "Run server-side validation for every change without persisting.")]
    #[serde(default)]
    pub dry_run: bool,
    #[schemars(
        description = "Idempotency key for the batch this call sends: replaying the same operation_id with the same manifests returns the original outcome instead of applying twice. Look its status up with get_operation. Must not be set together with dry_run."
    )]
    #[serde(default)]
    pub operation_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiffManifestsParams {
    #[schemars(
        description = "One or more YAML manifest documents (separated by `---`) to diff against live state."
    )]
    pub manifests_yaml: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportNamespaceParams {
    #[schemars(
        description = "Namespace to export every entity from, rendered back as manifest YAML."
    )]
    pub namespace: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportOpensloParams {
    #[schemars(
        description = "Namespace to export: its ServiceLevelIndicators, ServiceLevelObjectives, AlertPolicies, AlertNotificationTargets, and referenced Components, rendered as OpenSLO v1 multi-document YAML."
    )]
    pub namespace: String,
    #[schemars(
        description = "openslo-bindings.yaml content: maps each SLI's Signal to the DataSource and the per-role metricSource query templates (threshold/good/bad/total) OpenSLO needs but the model does not carry."
    )]
    pub bindings_yaml: String,
    #[serde(default)]
    #[schemars(
        description = "Emit a documented placeholder metricSource (type Unbound) for any SLI whose signal has no entry in bindings_yaml, instead of failing the export."
    )]
    pub allow_placeholder_metrics: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterNamespaceParams {
    #[schemars(
        description = "Human label to claim. Unique within the owner, not globally: two owners can each have an `orders`."
    )]
    pub name: String,
    #[schemars(
        description = "Owner to register under. Leave empty for your own, which is the only value a non-admin caller may use."
    )]
    #[serde(default)]
    pub parent: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveNamespaceParams {
    #[schemars(
        description = "Namespace id, not name. A move crosses an ownership boundary and names are only unique inside one; list_namespaces reports the id."
    )]
    pub id: String,
    #[schemars(description = "Owner to move it to.")]
    pub parent: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSnapshotIdParams {
    #[schemars(
        description = "Restrict the snapshot to these namespaces. Omit for every namespace. Two snapshot ids are comparable only when taken over the same scope."
    )]
    #[serde(default)]
    pub namespaces: Option<Vec<String>>,
    #[schemars(
        description = "Include the per-entity content hashes. Off by default; turn on to locate which entities differ between two snapshot ids."
    )]
    #[serde(default)]
    pub include_entries: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateBranchParams {
    #[schemars(
        description = "Branch name: ASCII alphanumeric, '-', '_', '.', max 200 chars per segment, no leading '.', at most one '/' to express an owner prefix, e.g. \"alex/retention-rework\"."
    )]
    pub name: String,
    #[serde(default)]
    #[schemars(description = "Free-form description stored with the branch.")]
    pub doc: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteBranchParams {
    #[schemars(description = "Name of the branch to delete.")]
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiffBranchParams {
    #[schemars(description = "Name of the branch to diff against baseline.")]
    pub name: String,
    /// When true, return the response as proto3-rendered JSON inside a
    /// `{ "json": "..." }` envelope instead of the default
    /// `{ "binpb_base64": "..." }`.
    #[serde(default)]
    pub json: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MergeBranchParams {
    #[schemars(description = "Name of the branch to merge onto baseline.")]
    pub name: String,
    #[serde(default)]
    #[schemars(
        description = "Run full conflict detection and post-merge validation without persisting anything, even on success."
    )]
    pub dry_run: bool,
    #[serde(default)]
    #[schemars(
        description = "Keep the branch (now empty of deltas) after a successful merge instead of deleting it. Ignored when dry_run is true or the merge does not apply."
    )]
    pub keep_branch: bool,
    #[serde(default)]
    #[schemars(
        description = "Land the combined result for edit/edit conflicts whose two sides changed different field paths, instead of reporting them as conflicts. Conflicts where both sides changed the same field path still block, as do edit/delete and delete/edit conflicts."
    )]
    pub auto_merge: bool,
    #[serde(default)]
    pub json: Option<bool>,
    #[schemars(
        description = "Idempotency key: replaying the same operation_id with the same arguments returns the original outcome instead of merging twice. Look its status up with get_operation. Must not be set together with dry_run."
    )]
    #[serde(default)]
    pub operation_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateBranchParams {
    #[schemars(
        description = "Name of the branch to rebase forward: advances non-conflicting deltas' recorded base to the current baseline; conflicting entries are left untouched and returned for resolve_branch_entry."
    )]
    pub name: String,
    #[serde(default)]
    pub json: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolveBranchEntryParams {
    #[schemars(description = "Name of the branch holding the conflicting entry.")]
    pub name: String,
    #[schemars(description = "Entity kind of the conflicting entry.")]
    pub kind: String,
    pub namespace: String,
    pub slug: String,
    #[schemars(description = "Schema version number, e.g. 1")]
    pub version: u64,
    #[schemars(
        description = "\"take_theirs\" overwrites the branch's delta with the current baseline content (or removes the delta entirely if baseline no longer has the key). \"keep_ours\" keeps the branch's current value, rebasing base/base_etag forward to the current baseline so the conflict clears."
    )]
    pub resolution: String,
    #[schemars(
        description = "The entry state the decision was made from: copy the `state` object of this entry from diff_branch or update_branch unchanged. The resolution is refused as stale (data.category \"stale_state\") if any side moved since; diff again and decide from the fresh entry."
    )]
    pub expected_state: ExpectedBranchEntryState,
    #[schemars(
        description = "Idempotency key: replaying the same operation_id with the same arguments returns the original outcome instead of resolving twice. Look its status up with get_operation."
    )]
    #[serde(default)]
    pub operation_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetOperationParams {
    #[schemars(
        description = "The operation_id a prior mutating tool call was given. Status is pending (claimed, not yet settled), applied (persisted; a receipt exists), rejected/not_applied (did not persist; safe to retry under a new operation_id), or unknown (never claimed, settled long enough ago to be forgotten, or belongs to a branch-scoped mutation, which keeps no durable receipt)."
    )]
    pub operation_id: String,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpectedBranchEntryState {
    #[serde(default, alias = "baseEtag")]
    pub base_etag: String,
    #[serde(default, alias = "oursEtag")]
    pub ours_etag: String,
    #[serde(default, alias = "theirsEtag")]
    pub theirs_etag: String,
}

#[cfg(test)]
mod deny_unknown_params_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    // The adapter must reject parameters it does not understand instead of
    // silently doing something else: `kinds` (the gRPC field name) passed to
    // list_entities used to be ignored, returning unfiltered results.
    #[test]
    fn unknown_parameter_is_rejected() {
        let err = serde_json::from_value::<ListEntitiesParams>(serde_json::json!({
            "kinds": ["event_model"],
            "page_size": 10
        }));
        assert!(
            err.is_err(),
            "unknown field `kinds` must fail deserialization"
        );
        let msg = err.unwrap_err().to_string();
        assert!(msg.contains("unknown field"), "{msg}");
    }

    #[test]
    fn known_parameters_still_parse() {
        let ok = serde_json::from_value::<ListEntitiesParams>(serde_json::json!({
            "kind": "event_model",
            "namespace": "account-deletion",
            "page_size": 10
        }));
        assert!(ok.is_ok(), "{ok:?}");
    }

    #[test]
    fn apply_manifests_params_reject_unknown_field() {
        let err = serde_json::from_value::<ApplyManifestsParams>(serde_json::json!({
            "manifests_yaml": "...",
            "dryRun": true
        }));
        assert!(
            err.is_err(),
            "unknown field `dryRun` must fail deserialization"
        );
    }

    #[test]
    fn apply_manifests_params_dry_run_defaults_to_false() {
        let ok = serde_json::from_value::<ApplyManifestsParams>(serde_json::json!({
            "manifests_yaml": "..."
        }))
        .unwrap();
        assert!(!ok.dry_run);
    }

    // validate_project tool description: "set exactly one" of project_* or
    // domain_*. Omitting project_version currently defaults to 0 via
    // serde(default), so agents look up project@0 instead of failing fast.
    #[test]
    fn validate_project_omitted_version_must_not_silently_become_zero() {
        let parsed = serde_json::from_value::<ValidateProjectParams>(serde_json::json!({
            "project_namespace": "acme",
            "project_slug": "core"
        }));
        if let Ok(p) = parsed {
            assert_ne!(
            p.project_version,
            Some(0),
            "omitted project_version silently became 0; agents will look up a non-existent @0 id"
        );
        }
    }

    #[test]
    fn validate_project_params_reject_both_project_and_domain_scope() {
        // Handler description says set exactly one scope. Params currently
        // accept both; the handler then silently prefers project.
        let parsed = serde_json::from_value::<ValidateProjectParams>(serde_json::json!({
            "project_namespace": "acme",
            "project_slug": "core",
            "project_version": 1,
            "domain_namespace": "acme",
            "domain_slug": "commerce",
            "domain_version": 1
        }));
        assert!(
            parsed.is_err(),
            "both project and domain scope must be rejected at the schema boundary; got {parsed:?}"
        );
    }

    // Server DeleteByQuery rejects max_deletes=0 (FailedPrecondition); the MCP
    // schema must not document "0 = unlimited" or let omitted become 0.
    #[test]
    fn delete_by_query_omitted_max_deletes_must_not_silently_become_zero() {
        let parsed = serde_json::from_value::<DeleteByQueryParams>(serde_json::json!({
            "namespace": "shop"
        }));
        if let Ok(p) = parsed {
            assert!(
                p.max_deletes > 0,
                "omitted max_deletes must not become 0 (server rejects 0); got {p:?}"
            );
        }
    }

    #[test]
    fn delete_by_query_zero_max_deletes_must_be_rejected() {
        let parsed = serde_json::from_value::<DeleteByQueryParams>(serde_json::json!({
            "namespace": "shop",
            "max_deletes": 0
        }));
        assert!(
            parsed.is_err(),
            "max_deletes=0 must fail at the schema boundary (server requires >0); got {parsed:?}"
        );
    }

    // Same silent-0 footgun as validate_project: omitted version becomes 0 and
    // the handler looks up domain@0 instead of failing fast.
    #[test]
    fn list_entities_by_domain_omitted_version_must_not_silently_become_zero() {
        let parsed = serde_json::from_value::<ListEntitiesByDomainParams>(serde_json::json!({
            "namespace": "acme",
            "slug": "commerce"
        }));
        if let Ok(p) = parsed {
            assert_ne!(
                p.version, 0,
                "omitted version silently became 0; agents will look up a non-existent domain@0"
            );
        }
    }

    #[test]
    fn search_params_schema_must_advertise_server_query_byte_limit() {
        // Server search_entities rejects queries > 2048 bytes. The MCP tool
        // schema / handler must not claim or accept a higher limit.
        let schema = schemars::schema_for!(SearchParams);
        let text = serde_json::to_string(&schema).unwrap_or_default();
        assert!(
            text.contains("2048"),
            "SearchParams schema must advertise the server 2048-byte query cap; got {text}"
        );
        assert!(
            !text.contains("4096"),
            "SearchParams schema must not advertise the stale 4096-byte cap; got {text}"
        );
    }

    // Server get_impact maps max_depth=0 to ADVERTISED_MAX_IMPACT_DEPTH (16),
    // not an unbounded walk. Advertising "0 = unlimited" misleads agents into
    // thinking they requested a full transitive closure.
    #[test]
    fn impact_params_schema_must_not_claim_max_depth_zero_is_unlimited() {
        let schema = schemars::schema_for!(ImpactParams);
        let text = serde_json::to_string(&schema).unwrap_or_default();
        assert!(
            !text.contains("unlimited"),
            "ImpactParams must not claim max_depth 0 is unlimited (server caps at 16); got {text}"
        );
        assert!(
            text.contains("16"),
            "ImpactParams schema must advertise the server max_depth cap of 16; got {text}"
        );
    }

    // Same silent-0 class as validate_project / list_entities_by_domain:
    // get_impact / retarget / batch_get accept version=0 and look up @0.
    #[test]
    fn impact_params_version_zero_must_be_rejected() {
        let parsed = serde_json::from_value::<ImpactParams>(serde_json::json!({
            "kind": "event",
            "namespace": "shop",
            "slug": "order-placed",
            "version": 0
        }));
        assert!(
            parsed.is_err(),
            "version=0 must fail at the schema boundary; got {parsed:?}"
        );
    }

    #[test]
    fn retarget_params_from_version_zero_must_be_rejected() {
        let parsed = serde_json::from_value::<RetargetReferencesParams>(serde_json::json!({
            "from_kind": "event",
            "from_namespace": "shop",
            "from_slug": "order-placed",
            "from_version": 0,
            "to_version": 2
        }));
        assert!(
            parsed.is_err(),
            "from_version=0 must fail at the schema boundary; got {parsed:?}"
        );
    }

    #[test]
    fn batch_get_entity_spec_version_zero_must_be_rejected() {
        let parsed = serde_json::from_value::<BatchGetParams>(serde_json::json!({
            "refs": [{"kind": "event", "namespace": "shop", "slug": "x", "version": 0}]
        }));
        assert!(
            parsed.is_err(),
            "EntitySpec version=0 must fail at the schema boundary; got {parsed:?}"
        );
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompileTypeLibraryParams {
    #[schemars(
        description = "proto3 JSON of a TypeLibrary message, e.g. {\"id\": {\"namespace\": \"shop\", \"slug\": \"acme.orders\", \"version\": 1}, \"files\": [{\"path\": \"acme/orders/v1/orders.proto\", \"content\": \"syntax = \\\"proto3\\\"; ...\"}]}"
    )]
    pub library_json: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolveTypeParams {
    #[schemars(description = "Namespace whose live type libraries are searched.")]
    pub namespace: String,
    #[schemars(
        description = "Any type_url naming the message, e.g. type.googleapis.com/acme.orders.v1.OrderPlaced"
    )]
    pub type_url: String,
}
