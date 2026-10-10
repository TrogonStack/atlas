use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use prost::Message;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ServerHandler,
};
use trogon_atlas_client::{
    apply::ApplyRejected,
    branch::Resolution,
    client::{connect_with_options, Client, ConnectOptions},
    compat::{self, MutationIntent},
    precondition::{BranchEntryState, StalePlan},
    tenant_types::{self, TypeScope},
};
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

use crate::{
    error::{attach_operation_id, tool_err, McpError},
    tools::{
        AnalysisScopeParams, ApplyManifestsParams, BatchGetParams, BatchMutateJsonParams,
        BatchMutateParams, CompileTypeLibraryParams, CreateBranchParams, DeleteBranchParams,
        DeleteByQueryParams, DeleteEntityParams, DiffBranchParams, DiffEntitiesParams,
        DiffManifestsParams, EmptyParams, EntitySpec, ExportNamespaceParams, ExportOpensloParams,
        ExtractSubgraphParams, GetChangesetParams, GetEntityHistoryParams, GetLatestVersionParams,
        GetOperationParams, GetSnapshotIdParams, ImpactParams, JsonOpts, ListChangesParams,
        ListChangesetsParams, ListEntitiesByDomainParams, ListEntitiesParams, ListVersionsParams,
        MergeBranchParams, MoveNamespaceParams, ProjectionParams, PutEntityJsonParams,
        PutEntityParams, ReferencesParams, RegisterNamespaceParams, ResolveBranchEntryParams,
        ResolveTypeParams, RetargetReferencesParams, RevertChangesetParams, SearchParams,
        SliceProjectionParams, SupersessionChainParams, UpdateBranchParams,
        ValidateEventModelParams, ValidateProjectParams,
    },
};

pub const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;

/// Must match `trogon-atlas-server` `search_entities` MAX_QUERY_BYTES (2048).
pub const MAX_SEARCH_QUERY_BYTES: usize = 2048;

/// ServerHandler instructions. Keep in sync with `get_info`.
pub const SERVER_INSTRUCTIONS: &str = "trogon-atlas-mcp: stdio MCP adapter for the trogonatlas.api.eventmodel.v1alpha1 gRPC service. \
             Read tools return a binpb_base64 envelope you can decode (or use *_json companions). Mutations \
             have two surfaces: JSON (get_entity_json -> edit proto3 JSON -> put_entity_json / \
             batch_mutate_json; no protobuf tooling required) and binpb (get_entity -> modify \
             binpb_base64 -> put_entity / batch_mutate). Call who_am_i if you need to confirm \
             which role, namespaces, or owner you are authenticated as; the tools listed here \
             are already filtered to what that role can call.\n\
             \n\
             STORYBOARD AUTHORING RULES (validated; violations are rejected or flagged):\n\
             1. A storyboard is a TEMPORAL sequence: each slice is one moment. A read-model \
             slice lists only the events that just happened at that moment, never the type's \
             full source list (the ReadModel entity carries the type-level list).\n\
             2. Projection adjacency: every event's read-model projections come directly after \
             the emitting slice, before the next command or automation slice acts. Updates \
             from divergent branches (e.g. approve vs reject) are separate slices, never merged.\n\
             3. One processor issues exactly one command; branching outcomes are separate \
             automation slices with separate processors. Record 'these ship as one worker' \
             with a Component entity listing them as members, not by sharing entities.\n\
             4. One automation observes one consumer-shaped read model; different code paths \
             need different views (an external webhook may project into several ExternalSource \
             read models). External integrations are declared on BOTH seams: the processor \
             that performs the call lists the ExternalSystem in Processor.calls (outbound), \
             and the results land in ExternalSource read models pointing at the same system \
             (inbound); the round trip is structural, never prose-only.\n\
             5. Sub-workflows are SEPARATE storyboards chained implicitly: storyboard A's \
             outcome events project into read models that storyboard B's entry observes. \
             Never build one mega-storyboard; never invent nesting. The EventModel is the \
             workflow umbrella, and its ordered members list sequences the storyboards.\n\
             6. Every storyboard declares an honest entry (the read model observed plus the \
             human persona+ui or automation processor observing it) and an outcome (the \
             terminal events). Clock/schedule triggers are read models observed by automations.\n\
             7. Single-use or entitlement constraints (e.g. review-once) get their own \
             aggregate swimlane; that stream is the consistency boundary.\n\
             8. Swimlane stream_id placeholders must be followable names: each {placeholder} \
             references a FieldSpec on that lane's events/commands, or the name of a \
             Uuidv5IdentityAnnotation registered on the swimlane (composed identities). Never \
             inline expressions like {uuidv5(...)}; register the identity once under a name \
             and write e.g. 'review-eligibility-{eligibility_id}'.\n\
             9. Forks in the road live on the SWIMLANE: declare the stream's lifecycle as \
             Swimlane.transitions (after <event>, next = [<event> + guard doc, ...]). \
             Exclusivity comes from the consistency boundary: competing commands append to \
             the same stream and the aggregate accepts one successor, so alternatives \
             without a shared stream are parallel fan-out, not a fork. Cycles are legal \
             (lifecycles loop). Tools derive the storyboard fork graph from the table, and \
             validation checks every GWT against it: a scenario may only emit a legal \
             successor of the last given event on that stream.\n\
             10. TIME ONLY POINTS FORWARD. Every edge goes from an earlier moment to a later \
             one; recurrence of a type (a screen shown again, a view updated again, a \
             lifecycle re-entered) is a NEW occurrence, never an arrow into the past; a \
             backward arrow destroys the ability to read time off the graph. Order slices \
             so events precede their projections and projections precede their consumers; \
             write scenario given histories as forward walks of each stream's transitions \
             (validated both in given order and in emission).\n\
             11. Bounded contexts touch at exactly ONE seam: an upstream EVENT flowing into \
             a downstream subscription READ MODEL's source_events (the translation pattern). \
             A cross-model link is NOT a continuation: no shared stream means no fork, no \
             guard, no single timeline; it is async pub/sub with fan-out. The DOWNSTREAM \
             declares the dependency (its consumer-shaped view names the upstream event); \
             the upstream never knows its consumers. Any other cross-namespace structural \
             ref is rejected (CROSS_MODEL_BOUNDARY_VIOLATION): translate, don't reference. \
             A namespace IS a bounded context; its root document is the BoundedContext \
             entity (slug == namespace, validated) carrying the context charter, links, \
             and metadata. Event models are free-form curations WITHIN a context (a \
             feature, a flow, a release), and a namespace may hold many.\n\
             12. Project management layers ON TOP of the model, never inside it. Delivery \
             status (todo/in_progress/review/blocked/done) lives in Tracker entities: \
             denormalized lists of {subject EntityRef, TrackStatus, doc} pointing at any \
             entity kind. Design entities carry NO status fields; flipping a status never \
             mutates the model. Several trackers may overlay one model (per team, per \
             milestone); within one tracker each subject appears once. Trackers are NOT \
             event model members. 'What needs work?' = list_entities kind=tracker.\n\
             13. UI.transitions declares PURE navigation only: surface-to-surface links \
             that carry no command and no event (tab bars, back affordances, menu links, \
             deep links). Any hop a storyboard command path already explains is DERIVED \
             from the slices; re-declaring it is managed duplication and warns \
             (UI_TRANSITION_DERIVABLE). Cycles are legal in the data (a navigation map is \
             a graph, like the swimlane lifecycle); renderers keep time forward by \
             unrolling occurrences.\n\
             14. Model DECISIONS AND DATA, never implementation. In scope: \
             strategic design (contexts, charters, seams, investment) and the behavioral \
             contract (facts, order, fields). Out of scope, always: classes, services, \
             repositories, frameworks, persistence, code structure. Litmus test: would it \
             survive reimplementing the system in another language?\n\
             15. The problem space is a knowledge graph: Domain (the \
             business, usually one) contains Subdomains (problems the business must be \
             good at, classified CORE/SUPPORTING/GENERIC, a budget document). They live \
             in a namespace named after the domain, which is NOT a bounded context. A \
             Subdomain never contains contexts: BoundedContext.realizes maps solution to \
             problem (cross-namespace, legal: knowledge, not coupling). One context \
             realizing several subdomains is a monolith confessing. Classification goes \
             on the SUBDOMAIN, never the context.\n\
             16. Seams carry data; relationships carry intent. The DOWNSTREAM \
             BoundedContext declares relationships: upstream context + intent \
             (CUSTOMER_SUPPLIER / CONFORMIST / PARTNERSHIP / SEPARATE_WAYS) + doc. Intent \
             and data must agree: integration intent without a seam warns, SEPARATE_WAYS \
             with a seam errors, a seam with no declared intent nudges (Info).\n\
             17. Screen is composition, UI is the moment. Use UI for every \
             slice and storyboard touchpoint. Use Screen only for URL-addressable \
             surfaces that compose UI contributions through UI.slot. Put Screen \
             entities in a non-context application namespace. Required Screen slots \
             need at least one contributing UI. A Screen with UI contributions from \
             multiple contexts is expected shared-surface composition, not a \
             replacement for context seams.\n\
             18. Terms and Ambiguities are language data. A Term lives in the \
             bounded context whose language it defines and points outward through \
             embodied_by to the entities carrying that meaning. An Ambiguity lives \
             in the problem-space namespace and records a real ruling across two \
             or more Term refs: HOMONYM or SYNONYM, with DELIBERATE, RENAME, or \
             MERGE. Do not store unresolved language work as an Ambiguity.\n\
             19. Disconnects are data when declared. Any model member that no \
             slice produces, consumes, or feeds is DISCONNECTED. Disconnects are not always \
             errors: a subscription view may wait for its consumer, an audit log may have no \
             UI yet, a niche flow may be deferred. Declare the disconnect with OrphanAnnotation \
             (a canonical Any in the entity's metadata) carrying a non-empty `doc` explaining \
             WHY. Unflagged disconnects warn (ORPHAN_NOT_FLAGGED); flag-with-empty-doc errors \
             (ORPHAN_DOC_EMPTY); flag-on-a-wired-entity nudges (ORPHAN_FLAG_OBSOLETE); remove \
             the annotation once the entity is connected.\n\
             20. Open questions and assumptions are first-class, not comments. An \
             OpenQuestionAnnotation names something the model does not yet decide: put it on \
             the entity or scenario where the gap lives, with `question` and `author`. An \
             AssumptionAnnotation names something the design already proceeds as if true but \
             nobody confirmed, with fields `statement`, optional `basis`, and `author`. Both \
             attach via put_entity_json as a canonical Any (\"@type\": \
             \"type.googleapis.com/trogonatlas.annotation.v1alpha1.OpenQuestionAnnotation\" or \
             \"...AssumptionAnnotation\") in the entity's metadata, or a scenario's own metadata. \
             Both surface as Info findings (OPEN_QUESTION_PRESENT, ASSUMPTION_PRESENT), never an \
             error or a warning, so recording one never blocks a mutation.\n\
             21. Reliability overlays point outward like Tracker, never as \
             event model members: a ServiceLevelIndicator names a Connection \
             (CommandHandling, Projection, Reaction, Display, Integration, or \
             Journey) that must resolve to a real slice or edge; a \
             ServiceLevelObjective points at one indicator, carries the \
             threshold and target, and lists the AlertPolicy entities that \
             watch it; an AlertPolicy points at AlertNotificationTarget \
             entities. Validation checks the connection resolves, a Journey \
             is structurally reachable, correlation fields are sourced on \
             both ends, the signal matches the measure kind, and an \
             objective with no alert policy is flagged (warning when \
             externally committed, info otherwise).\n\
             entity.system (uid, created_at) is SERVER-OWNED incarnation identity, Kubernetes \
             metadata.uid style: assigned on create, preserved across updates, regenerated on \
             delete+recreate. Anything you send there is ignored; never try to set it.\n\
             Run validate_event_model after every mutation batch and fix all Errors. Mutations \
             execute concurrently when pipelined: issue ordered workflows (write -> delete -> \
             validate) as separate sequential requests.";

/// The MCP crate's own ordering of the server's RBAC roles, so tool
/// listing can compare a caller's role against a tool's minimum without
/// depending on `trogon-atlas-server` (a dev-only dependency here; see
/// `TOOL_POLICIES` below).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolRole {
    Reader,
    Writer,
    Admin,
}

impl From<pb::Role> for ToolRole {
    fn from(role: pb::Role) -> Self {
        match role {
            pb::Role::Unspecified | pb::Role::Reader => ToolRole::Reader,
            pb::Role::Writer => ToolRole::Writer,
            pb::Role::Admin => ToolRole::Admin,
        }
    }
}

/// One row of the tool policy table: the short gRPC method names (no
/// leading service path) a tool invokes, directly or through
/// `trogon_atlas_client` (`apply`, `export`, `openslo`, `branch`), and the
/// minimum role a caller needs before this adapter offers the tool at all.
///
/// `min_role` is hand-maintained here rather than derived at runtime from
/// `trogon_atlas_server::auth::required_role`, because that crate is only a
/// dev-dependency of `trogon-atlas-mcp` (the role map is server-internal;
/// this adapter should not need the server crate to run). The `tests`
/// module below, which already depends on `trogon-atlas-server`, asserts
/// every row's `min_role` equals the max role `required_role` reports over
/// `rpcs`, so the two can never drift silently.
pub struct ToolPolicy {
    pub tool: &'static str,
    pub rpcs: &'static [&'static str],
    pub min_role: ToolRole,
}

pub const TOOL_POLICIES: &[ToolPolicy] = &[
    ToolPolicy {
        tool: "get_server_info",
        rpcs: &["GetServerInfo"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "who_am_i",
        rpcs: &["WhoAmI"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "compile_type_library",
        rpcs: &["CompileTypeLibrary"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "resolve_type",
        rpcs: &["ResolveType"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "list_namespaces",
        rpcs: &["ListNamespaces"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "register_namespace",
        rpcs: &["RegisterNamespace"],
        min_role: ToolRole::Writer,
    },
    ToolPolicy {
        tool: "move_namespace",
        rpcs: &["MoveNamespace"],
        min_role: ToolRole::Admin,
    },
    ToolPolicy {
        tool: "get_snapshot_id",
        rpcs: &["GetSnapshotId"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "list_entities",
        rpcs: &["ListEntities"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "get_entity",
        rpcs: &["GetEntity"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "batch_get_entities",
        rpcs: &["BatchGetEntities"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "search_entities",
        rpcs: &["SearchEntities"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "list_versions",
        rpcs: &["ListVersions"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "get_latest_version",
        rpcs: &["GetLatestVersion"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "get_supersession_chain",
        rpcs: &["GetSupersessionChain"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "get_incoming_references",
        rpcs: &["GetIncomingReferences"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "get_outgoing_references",
        rpcs: &["GetOutgoingReferences"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "get_impact",
        rpcs: &["GetImpact"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "retarget_references",
        rpcs: &["RetargetReferences"],
        min_role: ToolRole::Writer,
    },
    ToolPolicy {
        tool: "get_slice_projection",
        rpcs: &["GetSliceProjection"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "get_storyboard_projection",
        rpcs: &["GetStoryboardProjection"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "get_event_model_projection",
        rpcs: &["GetEventModelProjection"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "extract_subgraph",
        rpcs: &["ExtractSubgraph"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "diff_entities",
        rpcs: &["DiffEntities"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "put_entity",
        rpcs: &["PutEntity"],
        min_role: ToolRole::Writer,
    },
    ToolPolicy {
        tool: "get_entity_json",
        rpcs: &["GetEntity"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "put_entity_json",
        rpcs: &["PutEntity"],
        min_role: ToolRole::Writer,
    },
    ToolPolicy {
        tool: "batch_mutate_json",
        rpcs: &["BatchMutate"],
        min_role: ToolRole::Writer,
    },
    ToolPolicy {
        tool: "delete_entity",
        rpcs: &["DeleteEntity"],
        min_role: ToolRole::Writer,
    },
    ToolPolicy {
        tool: "batch_mutate",
        rpcs: &["BatchMutate"],
        min_role: ToolRole::Writer,
    },
    ToolPolicy {
        tool: "list_changes",
        rpcs: &["ListChanges"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "list_changesets",
        rpcs: &["ListChangesets"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "get_changeset",
        rpcs: &["GetChangeset"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "get_entity_history",
        rpcs: &["GetEntityHistory"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "revert_changeset",
        rpcs: &["RevertChangeset"],
        min_role: ToolRole::Writer,
    },
    ToolPolicy {
        tool: "validate_event_model",
        rpcs: &["ValidateEventModel"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "list_validation_rules",
        rpcs: &["ListValidationRules"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "validate_project",
        rpcs: &["ValidateProject"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "delete_by_query",
        rpcs: &["DeleteByQuery"],
        min_role: ToolRole::Admin,
    },
    ToolPolicy {
        tool: "list_entity_kinds",
        rpcs: &["ListEntityKinds"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "list_entities_by_domain",
        rpcs: &["ListEntitiesByDomain"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "infer_data_flow",
        rpcs: &["InferDataFlow"],
        min_role: ToolRole::Writer,
    },
    ToolPolicy {
        tool: "check_information_completeness",
        rpcs: &["CheckInformationCompleteness"],
        min_role: ToolRole::Writer,
    },
    // apply_manifests plans with BatchGetEntities, then writes through
    // BatchMutate (trogon_atlas_client::apply::plan / apply).
    ToolPolicy {
        tool: "apply_manifests",
        rpcs: &["BatchGetEntities", "BatchMutate"],
        min_role: ToolRole::Writer,
    },
    // diff_manifests only plans (trogon_atlas_client::apply::plan); it
    // never calls apply, so it never reaches BatchMutate.
    ToolPolicy {
        tool: "diff_manifests",
        rpcs: &["BatchGetEntities"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "export_namespace",
        rpcs: &["ListEntities"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "export_openslo",
        rpcs: &["ListEntities"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "create_branch",
        rpcs: &["CreateBranch"],
        min_role: ToolRole::Writer,
    },
    ToolPolicy {
        tool: "list_branches",
        rpcs: &["ListBranches"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "delete_branch",
        rpcs: &["DeleteBranch"],
        min_role: ToolRole::Writer,
    },
    ToolPolicy {
        tool: "diff_branch",
        rpcs: &["DiffBranch"],
        min_role: ToolRole::Reader,
    },
    ToolPolicy {
        tool: "merge_branch",
        rpcs: &["MergeBranch"],
        min_role: ToolRole::Writer,
    },
    ToolPolicy {
        tool: "update_branch",
        rpcs: &["UpdateBranch"],
        min_role: ToolRole::Writer,
    },
    ToolPolicy {
        tool: "resolve_branch_entry",
        rpcs: &["ResolveBranchEntry"],
        min_role: ToolRole::Writer,
    },
    ToolPolicy {
        tool: "get_operation",
        rpcs: &["GetOperation"],
        min_role: ToolRole::Writer,
    },
];

fn policy_for(tool: &str) -> Option<&'static ToolPolicy> {
    TOOL_POLICIES.iter().find(|p| p.tool == tool)
}

/// Whether `role` may be offered `tool` at all. A tool with no policy row is
/// treated as admin-only rather than silently listed, so a tool added
/// without a matching row fails toward hiding it, not exposing it; the
/// `every_tool_has_annotations_matching_the_role_map` test keeps this table
/// complete in practice.
fn tool_allowed_for(tool: &str, role: ToolRole) -> bool {
    let Some(policy) = policy_for(tool) else {
        tracing::error!(
            tool = tool,
            "list_tools: no ToolPolicy row for this tool; treating as admin-only"
        );
        return role >= ToolRole::Admin;
    };
    policy.min_role <= role
}

#[derive(Clone)]
pub struct McpServer {
    /// `tonic::Channel` multiplexes requests internally and is `Clone +
    /// Send`. Wrapping the client in `Mutex` (the previous shape) made every
    /// MCP tool call serialize through one global lock, defeating concurrency
    /// and adding head-of-line blocking. We hand each call its own `clone()`
    /// of the client; the underlying channel is shared.
    client: Client,
    /// Whether every request carries `x-trogon-atlas-branch`, which a server
    /// must honor before this adapter lets an agent mutate through it.
    branch_scoped: bool,
    /// The caller's own role, learned from `WhoAmI` at connect time, used to
    /// filter `list_tools` down to what this principal could actually call.
    /// `None` when the server predates the `who_am_i` feature flag or the
    /// call failed, in which case every tool is listed unfiltered: the
    /// server remains the actual enforcement point regardless.
    role: Option<ToolRole>,
    // Required for `#[tool_router]` codegen; the macro reads this field
    // off `self` at dispatch time but the compiler can't prove the read
    // because the dispatch flows through trait-object indirection. The
    // attribute silences that diagnostic without papering over real
    // dead code elsewhere.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl McpServer {
    pub async fn connect(
        endpoint: &str,
        auth_token: Option<&str>,
        max_message_bytes: Option<usize>,
        rpc_timeout_secs: u64,
        branch: Option<String>,
    ) -> Result<Self> {
        // Match the server's 4 MiB cap so a `listEntities` page that's
        // legal server-side doesn't blow up on the MCP client with a
        // surprise `ResourceExhausted`. The limit is configured via
        // --max-message-bytes / TROGON_ATLAS_MCP_MAX_MESSAGE_BYTES.
        let branch_scoped = branch.is_some();
        let client = connect_with_options(
            endpoint,
            auth_token,
            rpc_timeout_secs,
            ConnectOptions {
                request_id_prefix: Some("mcp"),
                max_message_bytes,
                branch,
            },
        )
        .await?;
        let role = Self::learn_role(&client).await;
        Ok(Self {
            client,
            branch_scoped,
            role,
            tool_router: Self::tool_router(),
        })
    }

    /// Learn the caller's own role via `WhoAmI`, gated on the server
    /// advertising the `who_am_i` feature flag. Any absence or failure
    /// degrades to `None` (list every tool unfiltered) rather than
    /// refusing to connect: the server enforces the role on every call
    /// regardless, so a client-side listing miss is never a safety gap,
    /// only a worse-than-ideal tool menu.
    async fn learn_role(client: &Client) -> Option<ToolRole> {
        let mut client = client.clone();
        let info = match client.get_server_info(pb::GetServerInfoRequest {}).await {
            Ok(resp) => resp.into_inner(),
            Err(e) => {
                tracing::warn!(error = %e, "learn_role: GetServerInfo failed; listing every tool");
                return None;
            }
        };
        if !info.features.unwrap_or_default().who_am_i {
            tracing::warn!("learn_role: server does not advertise who_am_i; listing every tool");
            return None;
        }
        match client.who_am_i(pb::WhoAmIRequest {}).await {
            Ok(resp) => {
                let role = resp.into_inner().role;
                Some(ToolRole::from(
                    pb::Role::try_from(role).unwrap_or(pb::Role::Unspecified),
                ))
            }
            Err(e) => {
                tracing::warn!(error = %e, "learn_role: WhoAmI failed; listing every tool");
                None
            }
        }
    }

    /// Hand each tool invocation its own client clone so concurrent calls
    /// multiplex over the shared `tonic::Channel` instead of serializing
    /// behind a single mutex.
    fn client(&self) -> Client {
        self.client.clone()
    }

    async fn ensure_can_mutate(
        &self,
        client: &mut Client,
        validate_only: bool,
    ) -> Result<(), rmcp::ErrorData> {
        self.ensure_intent(
            client,
            MutationIntent {
                validate_only,
                branch_scoped: self.branch_scoped,
                state_preconditions: false,
            },
        )
        .await
    }

    async fn ensure_can_apply_plan(
        &self,
        client: &mut Client,
        validate_only: bool,
    ) -> Result<(), rmcp::ErrorData> {
        self.ensure_intent(
            client,
            MutationIntent {
                validate_only,
                branch_scoped: self.branch_scoped,
                state_preconditions: true,
            },
        )
        .await
    }

    async fn ensure_intent(
        &self,
        client: &mut Client,
        intent: MutationIntent,
    ) -> Result<(), rmcp::ErrorData> {
        compat::ensure_can_mutate(client, intent)
            .await
            .map_err(|e| McpError::from(e).into())
    }
}

// JSON transcoding delegates to the ONE implementation in
// trogon-atlas-core::transcode; these adapters only lift the error into
// anyhow for the ~30 tool handlers below.
fn message_from_json<M: Message + Default>(message_name: &str, json: &str) -> Result<M> {
    Ok(trogon_atlas_core::transcode::message_from_json(
        message_name,
        json,
    )?)
}

fn message_to_json<M: Message>(message_name: &str, msg: &M) -> Result<String> {
    Ok(trogon_atlas_core::transcode::message_to_json(
        message_name,
        msg,
    )?)
}

async fn tenant_types_for(
    client: &mut Client,
    scope: &TypeScope,
) -> Result<TranscodePool, rmcp::ErrorData> {
    tenant_types::transcode_pool(client, scope)
        .await
        .map_err(|e| McpError::from(e).into())
}

fn json_type_scope(json: &str) -> TypeScope {
    serde_json::from_str(json)
        .map(|value| TypeScope::of_json(&value))
        .unwrap_or_default()
}

fn principal_kind_name(kind: pb::PrincipalKind) -> &'static str {
    match kind {
        pb::PrincipalKind::Unspecified => "unspecified",
        pb::PrincipalKind::User => "user",
        pb::PrincipalKind::Agent => "agent",
    }
}

fn role_name(role: pb::Role) -> &'static str {
    match role {
        pb::Role::Unspecified => "unspecified",
        pb::Role::Reader => "reader",
        pb::Role::Writer => "writer",
        pb::Role::Admin => "admin",
    }
}

fn parse_kind(s: &str) -> Result<pb::EntityKind, McpError> {
    pb::canonical::parse_kind(s).ok_or_else(|| McpError::UnknownKind(s.to_string()))
}

fn kinds_to_i32(kinds: Option<&Vec<String>>) -> Result<Vec<i32>, McpError> {
    match kinds {
        None => Ok(Vec::new()),
        Some(list) => list
            .iter()
            .map(|s| parse_kind(s).map(|k| k as i32))
            .collect(),
    }
}

fn parse_delete_mode(
    s: Option<&String>,
) -> Result<pb::delete_entity_request::Mode, rmcp::ErrorData> {
    let Some(raw) = s else {
        return Ok(pb::delete_entity_request::Mode::FailIfReferenced);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "fail_if_referenced" | "failifreferenced" => {
            Ok(pb::delete_entity_request::Mode::FailIfReferenced)
        }
        "force" => Ok(pb::delete_entity_request::Mode::Force),
        "dry_run" | "dryrun" => Ok(pb::delete_entity_request::Mode::DryRun),
        other => Err(rmcp::ErrorData::invalid_params(
            format!("mode must be fail_if_referenced, force, or dry_run; got {other:?}"),
            None,
        )),
    }
}

/// Validate an `operation_id` param at the MCP boundary and resolve it to
/// the value a request should actually carry. Dropped to empty whenever
/// `dry_run_like` is set: the server rejects `operation_id` paired with
/// `validate_only`/`dry_run`, since a dry run never produces a write for a
/// later replay to recover (see `operation_receipts::reject_operation_id_with_dry_run`).
fn resolve_operation_id(
    operation_id: Option<String>,
    dry_run_like: bool,
) -> Result<String, rmcp::ErrorData> {
    let Some(id) = operation_id else {
        return Ok(String::new());
    };
    trogon_atlas_core::validate_operation_id(&id)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("operation_id {e}"), None))?;
    Ok(if dry_run_like { String::new() } else { id })
}

/// Validate a namespace or slug at the MCP boundary before issuing any RPC.
///
/// Replicates the charset rule from `trogon_atlas_core::validate_id_component`
/// (see trogon-atlas-server/src/conv.rs): ASCII alphanumerics plus `-`, `_`, `.`;
/// not empty, not `.` or `..`, not leading `.`, max 200 chars.
/// Checked locally so callers get a structured `invalid_params` instead of a
/// cryptic transport error from the server.
fn validate_id_component(label: &str, value: &str) -> Result<(), rmcp::ErrorData> {
    if value.is_empty() {
        return Err(rmcp::ErrorData::invalid_params(
            format!("{label} must not be empty"),
            None,
        ));
    }
    if value.len() > 200 {
        return Err(rmcp::ErrorData::invalid_params(
            format!("{label} exceeds 200 chars"),
            None,
        ));
    }
    if value == "." || value == ".." {
        return Err(rmcp::ErrorData::invalid_params(
            format!("{label} must not be '.' or '..'"),
            None,
        ));
    }
    if value.starts_with('.') {
        return Err(rmcp::ErrorData::invalid_params(
            format!("{label} must not start with '.'"),
            None,
        ));
    }
    for ch in value.chars() {
        let safe = ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.');
        if !safe {
            return Err(rmcp::ErrorData::invalid_params(
                format!("{label} contains invalid character {ch:?}; allowed: [A-Za-z0-9_.-]"),
                None,
            ));
        }
    }
    Ok(())
}

fn id_from(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.to_string(),
        slug: slug.to_string(),
        version,
    }
}

/// Validates namespace, slug, and version before building an Id, returning a
/// structured `invalid_params` error instead of forwarding bad input to gRPC.
fn validated_id_from(ns: &str, slug: &str, version: u64) -> Result<pb::Id, rmcp::ErrorData> {
    validate_id_component("namespace", ns)?;
    validate_id_component("slug", slug)?;
    if version == 0 {
        return Err(rmcp::ErrorData::invalid_params("version must be > 0", None));
    }
    Ok(id_from(ns, slug, version))
}

fn validate_id_component_mcp(field: &'static str, value: &str) -> Result<(), McpError> {
    validate_id_component(field, value).map_err(|e| McpError::InvalidIdComponent {
        field,
        detail: e.message.to_string(),
    })
}

fn scope_from(
    kind: &str,
    ns: &str,
    slug: &str,
    version: u64,
) -> Result<pb::AnalysisScope, McpError> {
    let k = parse_kind(kind)?;
    validate_id_component_mcp("namespace", ns)?;
    validate_id_component_mcp("slug", slug)?;
    let entity_ref = pb::EntityRef {
        kind: k as i32,
        id: Some(id_from(ns, slug, version)),
    };
    let scope = match k {
        pb::EntityKind::CommandSlice
        | pb::EntityKind::ReadModelSlice
        | pb::EntityKind::AutomationSlice
        | pb::EntityKind::UiSlice => pb::analysis_scope::Scope::Slice(entity_ref),
        pb::EntityKind::Storyboard => pb::analysis_scope::Scope::Storyboard(entity_ref),
        pb::EntityKind::EventModel => pb::analysis_scope::Scope::EventModel(entity_ref),
        other => return Err(McpError::InvalidScopeKind { kind: other }),
    };
    Ok(pb::AnalysisScope { scope: Some(scope) })
}

fn decode_entity_b64(b64: &str) -> Result<pb::Entity, McpError> {
    let bytes = B64
        .decode(b64)
        .map_err(|e| McpError::InvalidBase64(e.to_string()))?;
    pb::Entity::decode(bytes.as_slice()).map_err(|source| McpError::InvalidBinpb {
        message: "Entity",
        source,
    })
}

fn encode_b64<M: Message>(msg: &M) -> String {
    // `encode_to_vec` allocates a `Vec` sized to `encoded_len` and cannot
    // fail (writing to a Vec is infallible). Avoids the cosmetic `.expect`
    // a `grep -n expect` would otherwise surface.
    B64.encode(msg.encode_to_vec())
}

/// Wire-stable envelope for proto responses returned through MCP. The body
/// is a JSON object with a single `binpb_base64` field carrying the
/// base64-encoded protobuf; callers decode it (or use the `*_json`
/// companion tools when JSON is wanted directly). Rust `Debug` output is
/// deliberately NOT included: it would couple consumers to a format that
/// changes whenever prost regenerates field ordering or struct shape.
fn ok_envelope<M: Message>(msg: &M) -> CallToolResult {
    let body = serde_json::json!({ "binpb_base64": encode_b64(msg) }).to_string();
    CallToolResult::success(vec![ContentBlock::text(body)])
}

/// `BatchMutateResponse.status == STATUS_FAILED` is an RPC that returned
/// `Ok` while carrying a domain failure in its body; `ok_envelope` alone
/// would report it to the agent as a successful tool call. Classify it the
/// same way `trogon-atlas-client::apply` classifies the identical response
/// shape, so the agent gets an MCP tool error with the same category data
/// any other failure carries instead of a binpb blob it has to decode to
/// discover the batch did not apply.
fn batch_mutate_failure(resp: &pb::BatchMutateResponse) -> anyhow::Error {
    if let Some(failure) = &resp.precondition_failure {
        return match StalePlan::from_wire(failure) {
            Ok(stale) => anyhow::Error::new(stale),
            Err(e) => e,
        };
    }
    anyhow::Error::new(ApplyRejected {
        failed: format!("op[{}]", resp.failed_op_index),
        issues: resp.failure.clone(),
    })
}

/// Universal response shaper. When `want_json` is true and the descriptor pool
/// knows the message, return the JSON-rendered proto in a
/// `{ "format": "json", "json": "..." }` envelope. On any serialization
/// failure (descriptor missing, transcoding fault) fall back to the
/// `binpb_base64` envelope so the tool call never fails purely over the response
/// shape, but log at warn and include `"format": "binpb_base64"` so callers
/// can detect the fallback without inspecting field presence.
fn ok_response<M: Message>(message_name: &str, msg: &M, want_json: bool) -> CallToolResult {
    if want_json {
        match message_to_json(message_name, msg) {
            Ok(s) => {
                let body = serde_json::json!({ "format": "json", "json": s }).to_string();
                return CallToolResult::success(vec![ContentBlock::text(body)]);
            }
            Err(e) => {
                tracing::warn!(
                    message_name = message_name,
                    error = %e,
                    "ok_response: JSON serialization failed; falling back to binpb_base64"
                );
            }
        }
    }
    let body = serde_json::json!({
        "format": "binpb_base64",
        "binpb_base64": encode_b64(msg)
    })
    .to_string();
    CallToolResult::success(vec![ContentBlock::text(body)])
}

#[tool_router]
impl McpServer {
    #[tool(
        description = "Server identity, schema version, feature flags, and limits.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_server_info(
        &self,
        Parameters(_p): Parameters<EmptyParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut client = self.client();
        let resp = client
            .get_server_info(pb::GetServerInfoRequest {})
            .await
            .map_err(tool_err("get_server_info"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Who you are authenticated as: name, kind, role, writable namespaces, \
                        owner, and whether you're the anonymous principal.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn who_am_i(
        &self,
        Parameters(_p): Parameters<EmptyParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut client = self.client();
        let resp = client
            .who_am_i(pb::WhoAmIRequest {})
            .await
            .map_err(tool_err("who_am_i"))?
            .into_inner();
        let kind = pb::PrincipalKind::try_from(resp.kind).unwrap_or(pb::PrincipalKind::Unspecified);
        let role = pb::Role::try_from(resp.role).unwrap_or(pb::Role::Unspecified);
        let body = serde_json::json!({
            "name": resp.name,
            "kind": principal_kind_name(kind),
            "role": role_name(role),
            "namespaces": resp.namespaces,
            "owner": resp.owner,
            "anonymous": resp.anonymous,
        })
        .to_string();
        Ok(CallToolResult::success(vec![ContentBlock::text(body)]))
    }

    #[tool(
        description = "List known namespaces with approximate entity counts.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_namespaces(
        &self,
        Parameters(_p): Parameters<EmptyParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut client = self.client();
        let resp = client
            .list_namespaces(pb::ListNamespacesRequest {})
            .await
            .map_err(tool_err("list_namespaces"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Claim a namespace name and get back its id. Always succeeds for a name you do not already hold, because the id is minted rather than taken from the name: this is the answer when a write is refused because the bare name is unavailable. Idempotent, so re-registering a name you hold returns the existing row with created=false.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn register_namespace(
        &self,
        Parameters(p): Parameters<RegisterNamespaceParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut client = self.client();
        self.ensure_can_mutate(&mut client, false).await?;
        let resp = client
            .register_namespace(pb::RegisterNamespaceRequest {
                name: p.name,
                parent: p.parent.unwrap_or_default(),
            })
            .await
            .map_err(tool_err("register_namespace"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Re-parent a namespace. Admin only: it is the one operation that crosses a tenancy boundary. Writes the registry row and nothing else, so no entity is re-keyed and no reference changes.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn move_namespace(
        &self,
        Parameters(p): Parameters<MoveNamespaceParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut client = self.client();
        self.ensure_can_mutate(&mut client, false).await?;
        let resp = client
            .move_namespace(pb::MoveNamespaceRequest {
                id: p.id,
                parent: p.parent,
            })
            .await
            .map_err(tool_err("move_namespace"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Compile a type library's proto source without writing it. Returns the compiler diagnostics (path, line, column, message) and any wire-compatibility violations against the library's live version; both empty means put_entity_json would accept it.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn compile_type_library(
        &self,
        Parameters(p): Parameters<CompileTypeLibraryParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if p.library_json.len() > MAX_JSON_BYTES {
            return Err(rmcp::ErrorData::invalid_params(
                format!(
                    "library_json exceeds the 4 MiB limit ({} bytes)",
                    p.library_json.len()
                ),
                None,
            ));
        }
        let library: pb::TypeLibrary = message_from_json(
            "trogonatlas.eventmodel.v1alpha1.TypeLibrary",
            &p.library_json,
        )
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
        let mut client = self.client();
        let resp = client
            .compile_type_library(pb::CompileTypeLibraryRequest {
                library: Some(library),
            })
            .await
            .map_err(tool_err("compile_type_library"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Resolve a message type against a namespace's live type libraries. Returns the library that declares it (absent for built-in types) and a FileDescriptorSet holding its file and every import, enough to decode payloads of that type.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn resolve_type(
        &self,
        Parameters(p): Parameters<ResolveTypeParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut client = self.client();
        let resp = client
            .resolve_type(pb::ResolveTypeRequest {
                namespace: p.namespace,
                type_url: p.type_url,
            })
            .await
            .map_err(tool_err("resolve_type"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Content-addressable id naming the current model state. Take one before an analysis and one after to tell whether anything changed, without walking the change feed; cite one to pin the state a report described. Ids are comparable only across the same namespace scope, and only when truncated is false.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_snapshot_id(
        &self,
        Parameters(p): Parameters<GetSnapshotIdParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut client = self.client();
        let resp = client
            .get_snapshot_id(pb::GetSnapshotIdRequest {
                namespaces: p.namespaces.unwrap_or_default(),
                include_entries: p.include_entries,
            })
            .await
            .map_err(tool_err("get_snapshot_id"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "List entities filtered by optional kind/namespace/lifecycle status with paging. Use lifecycle_status_not_in with [\"draft\", \"proposed\"] to hide drafts, or lifecycle_status_in with [\"draft\"] to show only drafts.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_entities(
        &self,
        Parameters(p): Parameters<ListEntitiesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kinds = match p.kind.as_deref() {
            Some(k) if !k.is_empty() => {
                vec![parse_kind(k).map_err(rmcp::ErrorData::from)? as i32]
            }
            _ => Vec::new(),
        };
        let namespaces = p.namespace.into_iter().collect::<Vec<_>>();
        let page_size = i32::try_from(p.page_size.clamp(1, 500))
            .map_err(|_| rmcp::ErrorData::invalid_params("page_size overflows i32", None))?;
        let req = pb::ListEntitiesRequest {
            kinds,
            namespaces,
            latest_versions_only: false,
            page_size,
            page_token: p.page_token.unwrap_or_default(),
            lifecycle_status_in: p.lifecycle_status_in.unwrap_or_default(),
            lifecycle_status_not_in: p.lifecycle_status_not_in.unwrap_or_default(),
        };
        let mut client = self.client();
        let resp = client
            .list_entities(req)
            .await
            .map_err(tool_err("list_entities"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Fetch a single entity by (kind, namespace, slug, version). Returns an opaque etag for use with put_entity / delete_entity if_match.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_entity(
        &self,
        Parameters(p): Parameters<EntitySpec>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind = parse_kind(&p.kind).map_err(rmcp::ErrorData::from)?;
        let req = pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(validated_id_from(&p.namespace, &p.slug, p.version)?),
        };
        let mut client = self.client();
        let resp = client
            .get_entity(req)
            .await
            .map_err(tool_err("get_entity"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Batch fetch entities by (kind, namespace, slug, version) tuples. Returns the entities in arbitrary order (missing entries omitted).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn batch_get_entities(
        &self,
        Parameters(p): Parameters<BatchGetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        const MAX_REFS: usize = 100;
        if p.refs.len() > MAX_REFS {
            return Err(rmcp::ErrorData::invalid_params(
                format!(
                    "batch_get_entities accepts at most {} refs, got {}",
                    MAX_REFS,
                    p.refs.len()
                ),
                None,
            ));
        }
        let mut keys = Vec::with_capacity(p.refs.len());
        for r in p.refs {
            let kind = parse_kind(&r.kind).map_err(rmcp::ErrorData::from)?;
            keys.push(pb::EntityRef {
                kind: kind as i32,
                id: Some(validated_id_from(&r.namespace, &r.slug, r.version)?),
            });
        }
        let req = pb::BatchGetEntitiesRequest { keys, dense: false };
        let mut client = self.client();
        let resp = client
            .batch_get_entities(req)
            .await
            .map_err(tool_err("batch_get_entities"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Full-text search across entity title/doc. Returns results with relevance scores and optional excerpts.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn search_entities(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kinds = kinds_to_i32(p.kinds.as_ref()).map_err(rmcp::ErrorData::from)?;
        if p.q.len() > MAX_SEARCH_QUERY_BYTES {
            return Err(rmcp::ErrorData::invalid_params(
                format!(
                    "q exceeds the {MAX_SEARCH_QUERY_BYTES}-byte limit ({} bytes)",
                    p.q.len()
                ),
                None,
            ));
        }
        let limit = i32::try_from(p.page_size.clamp(1, 500))
            .map_err(|_| rmcp::ErrorData::invalid_params("page_size overflows i32", None))?;
        let req = pb::SearchEntitiesRequest {
            query: p.q,
            kinds,
            namespaces: Vec::new(),
            limit,
        };
        let mut client = self.client();
        let resp = client
            .search_entities(req)
            .await
            .map_err(tool_err("search_entities"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "List every stored version of (kind, namespace, slug), ordered ascending.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_versions(
        &self,
        Parameters(p): Parameters<ListVersionsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind = parse_kind(&p.kind).map_err(rmcp::ErrorData::from)?;
        validate_id_component("namespace", &p.namespace)?;
        validate_id_component("slug", &p.slug)?;
        let req = pb::ListVersionsRequest {
            kind: kind as i32,
            namespace: p.namespace,
            slug: p.slug,
            page_size: 0,
            page_token: String::new(),
        };
        let mut client = self.client();
        let resp = client
            .list_versions(req)
            .await
            .map_err(tool_err("list_versions"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Return the highest-version entity for (kind, namespace, slug).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_latest_version(
        &self,
        Parameters(p): Parameters<GetLatestVersionParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind = parse_kind(&p.kind).map_err(rmcp::ErrorData::from)?;
        validate_id_component("namespace", &p.namespace)?;
        validate_id_component("slug", &p.slug)?;
        let req = pb::GetLatestVersionRequest {
            kind: kind as i32,
            namespace: p.namespace,
            slug: p.slug,
        };
        let mut client = self.client();
        let resp = client
            .get_latest_version(req)
            .await
            .map_err(tool_err("get_latest_version"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Walk the supersedes/superseded-by chain. include_descendants=true returns both ancestors and descendants; otherwise only ancestors.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_supersession_chain(
        &self,
        Parameters(p): Parameters<SupersessionChainParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind = parse_kind(&p.kind).map_err(rmcp::ErrorData::from)?;
        let direction = if p.include_descendants {
            pb::get_supersession_chain_request::Direction::Both
        } else {
            pb::get_supersession_chain_request::Direction::Ancestors
        };
        let req = pb::GetSupersessionChainRequest {
            kind: kind as i32,
            id: Some(validated_id_from(&p.namespace, &p.slug, p.version)?),
            direction: direction as i32,
            page_size: 0,
            page_token: String::new(),
        };
        let mut client = self.client();
        let resp = client
            .get_supersession_chain(req)
            .await
            .map_err(tool_err("get_supersession_chain"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Find entities that reference this one (incoming pointer hits).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_incoming_references(
        &self,
        Parameters(p): Parameters<ReferencesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind = parse_kind(&p.kind).map_err(rmcp::ErrorData::from)?;
        let filter_kinds = kinds_to_i32(p.filter_kinds.as_ref()).map_err(rmcp::ErrorData::from)?;
        let req = pb::GetReferencesRequest {
            kind: kind as i32,
            id: Some(validated_id_from(&p.namespace, &p.slug, p.version)?),
            filter_kinds,
            page_size: 0,
            page_token: String::new(),
        };
        let mut client = self.client();
        let resp = client
            .get_incoming_references(req)
            .await
            .map_err(tool_err("get_incoming_references"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Find entities this one references (outgoing pointer hits).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_outgoing_references(
        &self,
        Parameters(p): Parameters<ReferencesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind = parse_kind(&p.kind).map_err(rmcp::ErrorData::from)?;
        let filter_kinds = kinds_to_i32(p.filter_kinds.as_ref()).map_err(rmcp::ErrorData::from)?;
        let req = pb::GetReferencesRequest {
            kind: kind as i32,
            id: Some(validated_id_from(&p.namespace, &p.slug, p.version)?),
            filter_kinds,
            page_size: 0,
            page_token: String::new(),
        };
        let mut client = self.client();
        let resp = client
            .get_outgoing_references(req)
            .await
            .map_err(tool_err("get_outgoing_references"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Transitive impact analysis: every entity reachable from root, with depth and the path of references that led there.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_impact(
        &self,
        Parameters(p): Parameters<ImpactParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind = parse_kind(&p.kind).map_err(rmcp::ErrorData::from)?;
        let filter_kinds = kinds_to_i32(p.filter_kinds.as_ref()).map_err(rmcp::ErrorData::from)?;
        let req = pb::GetImpactRequest {
            root: Some(pb::EntityRef {
                kind: kind as i32,
                id: Some(validated_id_from(&p.namespace, &p.slug, p.version)?),
            }),
            max_depth: p.max_depth,
            filter_kinds,
            page_size: 0,
            page_token: String::new(),
        };
        let mut client = self.client();
        let resp = client
            .get_impact(req)
            .await
            .map_err(tool_err("get_impact"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Rewrite every stored reference to `from` so it points at `to`. \
                       The common case is a version promotion: create v+1 with supersedes, \
                       call retarget_references (dry_run=true first to preview), \
                       then drain residual SLICE_STALE_REF findings. \
                       For partial migrations use get_incoming_references + batch_mutate instead.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn retarget_references(
        &self,
        Parameters(p): Parameters<RetargetReferencesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind = parse_kind(&p.from_kind).map_err(rmcp::ErrorData::from)?;
        let to_ns = p.to_namespace.as_deref().unwrap_or(&p.from_namespace);
        let to_slug = p.to_slug.as_deref().unwrap_or(&p.from_slug);
        let filter_kinds = kinds_to_i32(p.filter_kinds.as_ref()).map_err(rmcp::ErrorData::from)?;
        let req = pb::RetargetReferencesRequest {
            from: Some(pb::EntityRef {
                kind: kind as i32,
                id: Some(validated_id_from(
                    &p.from_namespace,
                    &p.from_slug,
                    p.from_version,
                )?),
            }),
            to: Some(pb::EntityRef {
                kind: kind as i32,
                id: Some(validated_id_from(to_ns, to_slug, p.to_version)?),
            }),
            dry_run: p.dry_run,
            filter_kinds,
        };
        let mut client = self.client();
        self.ensure_can_mutate(&mut client, req.dry_run).await?;
        let resp = client
            .retarget_references(req)
            .await
            .map_err(tool_err("retarget_references"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Slice projection: hydrate a slice plus the one-hop entities it references.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_slice_projection(
        &self,
        Parameters(p): Parameters<SliceProjectionParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind = parse_kind(&p.kind).map_err(rmcp::ErrorData::from)?;
        let req = pb::GetSliceProjectionRequest {
            kind: kind as i32,
            id: Some(validated_id_from(&p.namespace, &p.slug, p.version)?),
        };
        let mut client = self.client();
        let resp = client
            .get_slice_projection(req)
            .await
            .map_err(tool_err("get_slice_projection"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Storyboard projection: storyboard + inlined slices + every transitively referenced entity.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_storyboard_projection(
        &self,
        Parameters(p): Parameters<ProjectionParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = pb::GetStoryboardProjectionRequest {
            id: Some(validated_id_from(&p.namespace, &p.slug, p.version)?),
        };
        let mut client = self.client();
        let resp = client
            .get_storyboard_projection(req)
            .await
            .map_err(tool_err("get_storyboard_projection"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Event-model projection: the model plus every transitively reachable entity. Set include_cross_model=true to follow cross-namespace references.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_event_model_projection(
        &self,
        Parameters(p): Parameters<ProjectionParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = pb::GetEventModelProjectionRequest {
            id: Some(validated_id_from(&p.namespace, &p.slug, p.version)?),
            include_cross_model: p.include_cross_model,
        };
        let mut client = self.client();
        let resp = client
            .get_event_model_projection(req)
            .await
            .map_err(tool_err("get_event_model_projection"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Extract a subgraph: synthesizes a new (unpersisted) EventModel covering the closure of the given root entities.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn extract_subgraph(
        &self,
        Parameters(p): Parameters<ExtractSubgraphParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let scope = scope_from(
            &p.root.kind,
            &p.root.namespace,
            &p.root.slug,
            p.root.version,
        )
        .map_err(rmcp::ErrorData::from)?;
        let req = pb::ExtractSubgraphRequest {
            scope: Some(scope),
            new_event_model_id: None,
            include_cross_model: false,
        };
        let mut client = self.client();
        let resp = client
            .extract_subgraph(req)
            .await
            .map_err(tool_err("extract_subgraph"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Structural diff between two entities (typically two versions of the same entity).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn diff_entities(
        &self,
        Parameters(p): Parameters<DiffEntitiesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind_a = parse_kind(&p.a.kind).map_err(rmcp::ErrorData::from)?;
        let kind_b = parse_kind(&p.b.kind).map_err(rmcp::ErrorData::from)?;
        let req = pb::DiffEntitiesRequest {
            a: Some(pb::EntityRef {
                kind: kind_a as i32,
                id: Some(validated_id_from(&p.a.namespace, &p.a.slug, p.a.version)?),
            }),
            b: Some(pb::EntityRef {
                kind: kind_b as i32,
                id: Some(validated_id_from(&p.b.namespace, &p.b.slug, p.b.version)?),
            }),
        };
        let mut client = self.client();
        let resp = client
            .diff_entities(req)
            .await
            .map_err(tool_err("diff_entities"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Create or replace an entity. The entity payload is a base64-encoded prost binpb of an Entity message (obtain via get_entity, edit binpb_base64, and pass back). A failed write is an MCP tool error whose data carries category/next_action (and code/next_action_detail when applicable), never a success envelope.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn put_entity(
        &self,
        Parameters(p): Parameters<PutEntityParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        const MAX_B64_BYTES: usize = MAX_JSON_BYTES * 4 / 3 + 4;
        if p.entity_b64.len() > MAX_B64_BYTES {
            return Err(rmcp::ErrorData::invalid_params(
                format!(
                    "entity_b64 exceeds the size limit ({} bytes)",
                    p.entity_b64.len()
                ),
                None,
            ));
        }
        let entity = decode_entity_b64(&p.entity_b64).map_err(rmcp::ErrorData::from)?;
        let operation_id = resolve_operation_id(p.operation_id, p.validate_only)?;
        let req = pb::PutEntityRequest {
            entity: Some(entity),
            create_only: p.create_only,
            if_match: p.if_match,
            validate_only: p.validate_only,
            force: p.force,
            operation_id: operation_id.clone(),
        };
        let mut client = self.client();
        self.ensure_can_mutate(&mut client, req.validate_only)
            .await?;
        let resp = client
            .put_entity(req)
            .await
            .map_err(tool_err("put_entity"))
            .map_err(|e| attach_operation_id(e, &operation_id))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Fetch a single entity as proto3 JSON (editable without protobuf tooling). The canonical JSON workflow is get_entity_json -> edit -> put_entity_json.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_entity_json(
        &self,
        Parameters(p): Parameters<EntitySpec>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind = parse_kind(&p.kind).map_err(rmcp::ErrorData::from)?;
        let req = pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(validated_id_from(&p.namespace, &p.slug, p.version)?),
        };
        let mut client = self.client();
        let resp = client
            .get_entity(req)
            .await
            .map_err(tool_err("get_entity_json"))?
            .into_inner();
        let entity_json = match resp.entity.as_ref() {
            Some(entity) => {
                let mut scope = TypeScope::default();
                scope.add_namespace(p.namespace.clone());
                tenant_types_for(&mut client, &scope)
                    .await?
                    .entity_json_value(entity)
                    .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))?
            }
            None => serde_json::Value::Null,
        };
        let body = serde_json::json!({ "etag": resp.etag, "json": entity_json }).to_string();
        Ok(CallToolResult::success(vec![ContentBlock::text(body)]))
    }

    #[tool(
        description = "Create or replace an entity from proto3 JSON (no protobuf tooling required). entity_json is an Entity message, e.g. {\"event\": {\"id\": {\"namespace\": \"shop\", \"slug\": \"order-placed\", \"version\": 1}, \"title\": \"Order placed\"}}.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn put_entity_json(
        &self,
        Parameters(p): Parameters<PutEntityJsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if p.entity_json.len() > MAX_JSON_BYTES {
            return Err(rmcp::ErrorData::invalid_params(
                format!(
                    "entity_json exceeds the 4 MiB limit ({} bytes)",
                    p.entity_json.len()
                ),
                None,
            ));
        }
        let mut client = self.client();
        let entity: pb::Entity = tenant_types_for(&mut client, &json_type_scope(&p.entity_json))
            .await?
            .message_from_json("trogonatlas.eventmodel.v1alpha1.Entity", &p.entity_json)
            .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
        let operation_id = resolve_operation_id(p.operation_id, p.validate_only)?;
        let req = pb::PutEntityRequest {
            entity: Some(entity),
            create_only: p.create_only,
            if_match: p.if_match,
            validate_only: p.validate_only,
            force: p.force,
            operation_id: operation_id.clone(),
        };
        self.ensure_can_mutate(&mut client, req.validate_only)
            .await?;
        let resp = client
            .put_entity(req)
            .await
            .map_err(tool_err("put_entity_json"))
            .map_err(|e| attach_operation_id(e, &operation_id))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Atomic batch of put/delete operations expressed as a proto3 JSON BatchMutateRequest (no protobuf tooling required). Failure rolls back the whole batch and is reported as an MCP tool error whose data carries category/next_action (and code/next_action_detail when applicable), never as a success envelope with status=STATUS_FAILED inside it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn batch_mutate_json(
        &self,
        Parameters(p): Parameters<BatchMutateJsonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if p.request_json.len() > MAX_JSON_BYTES {
            return Err(rmcp::ErrorData::invalid_params(
                format!(
                    "request_json exceeds the 4 MiB limit ({} bytes)",
                    p.request_json.len()
                ),
                None,
            ));
        }
        let mut client = self.client();
        let req: pb::BatchMutateRequest =
            tenant_types_for(&mut client, &json_type_scope(&p.request_json))
                .await?
                .message_from_json(
                    "trogonatlas.api.eventmodel.v1alpha1.BatchMutateRequest",
                    &p.request_json,
                )
                .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
        self.ensure_can_mutate(&mut client, req.validate_only)
            .await?;
        let resp = client
            .batch_mutate(req)
            .await
            .map_err(tool_err("batch_mutate_json"))?
            .into_inner();
        if resp.status == pb::batch_mutate_response::Status::Failed as i32 {
            return Err(McpError::from(batch_mutate_failure(&resp)).into());
        }
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Delete an entity. mode = 'fail_if_referenced' (default), 'force', or 'dry_run'. A refused delete is an MCP tool error whose data carries category/next_action (and code/next_action_detail when applicable), never a success envelope.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn delete_entity(
        &self,
        Parameters(p): Parameters<DeleteEntityParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind = parse_kind(&p.kind).map_err(rmcp::ErrorData::from)?;
        let mode = parse_delete_mode(p.mode.as_ref())?;
        let dry_run = mode == pb::delete_entity_request::Mode::DryRun;
        let operation_id = resolve_operation_id(p.operation_id, dry_run)?;
        let req = pb::DeleteEntityRequest {
            kind: kind as i32,
            id: Some(validated_id_from(&p.namespace, &p.slug, p.version)?),
            mode: mode as i32,
            if_match: p.if_match,
            operation_id: operation_id.clone(),
        };
        let mut client = self.client();
        self.ensure_can_mutate(&mut client, dry_run).await?;
        let resp = client
            .delete_entity(req)
            .await
            .map_err(tool_err("delete_entity"))
            .map_err(|e| attach_operation_id(e, &operation_id))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Atomic batch of put/delete operations. Each op specifies op='put' (with entity_b64) or op='delete' (with kind/namespace/slug/version). Failure rolls back the whole batch and is reported as an MCP tool error whose data carries category/next_action (and code/next_action_detail when applicable), never as a success envelope with status=STATUS_FAILED inside it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn batch_mutate(
        &self,
        Parameters(p): Parameters<BatchMutateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        const MAX_OPS: usize = 50;
        if p.ops.len() > MAX_OPS {
            return Err(rmcp::ErrorData::invalid_params(
                format!(
                    "batch_mutate accepts at most {} ops, got {}",
                    MAX_OPS,
                    p.ops.len()
                ),
                None,
            ));
        }
        const MAX_B64_BYTES: usize = MAX_JSON_BYTES * 4 / 3 + 4;
        // Cap the total encoded proto payload across all ops at the gRPC max
        // message size (4 MiB, matching --max-message-bytes / the channel cap
        // set in McpServer::connect). Without this the server rejects the
        // request with an opaque ResourceExhausted instead of a clear error.
        const MAX_BATCH_PAYLOAD_BYTES: usize = MAX_JSON_BYTES;
        let mut batch_payload_bytes: usize = 0;
        let mut ops = Vec::with_capacity(p.ops.len());
        for op in p.ops {
            let inner = match op.op.trim().to_ascii_lowercase().as_str() {
                "put" => {
                    let b64 = op.entity_b64.ok_or_else(|| {
                        rmcp::ErrorData::invalid_params("put op requires entity_b64", None)
                    })?;
                    if b64.len() > MAX_B64_BYTES {
                        return Err(rmcp::ErrorData::invalid_params(
                            format!(
                                "entity_b64 in batch op exceeds the size limit ({} bytes)",
                                b64.len()
                            ),
                            None,
                        ));
                    }
                    let entity = decode_entity_b64(&b64).map_err(rmcp::ErrorData::from)?;
                    let encoded_len = entity.encoded_len();
                    batch_payload_bytes = batch_payload_bytes.saturating_add(encoded_len);
                    if batch_payload_bytes > MAX_BATCH_PAYLOAD_BYTES {
                        return Err(rmcp::ErrorData::invalid_params(
                            format!(
                                "batch total decoded payload exceeds the {MAX_BATCH_PAYLOAD_BYTES} byte cap; \
                                 split into smaller batches"
                            ),
                            None,
                        ));
                    }
                    pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        entity: Some(entity),
                        create_only: op.create_only,
                        if_match: op.if_match,
                        force: op.force,
                        validate_only: false,
                        operation_id: String::new(),
                    })
                }
                "delete" => {
                    let kind_str = op.kind.ok_or_else(|| {
                        rmcp::ErrorData::invalid_params("delete op requires kind", None)
                    })?;
                    let ns = op.namespace.ok_or_else(|| {
                        rmcp::ErrorData::invalid_params("delete op requires namespace", None)
                    })?;
                    let slug = op.slug.ok_or_else(|| {
                        rmcp::ErrorData::invalid_params("delete op requires slug", None)
                    })?;
                    let version = op.version.ok_or_else(|| {
                        rmcp::ErrorData::invalid_params("delete op requires version", None)
                    })?;
                    if version == 0 {
                        return Err(rmcp::ErrorData::invalid_params(
                            "delete op version must be > 0",
                            None,
                        ));
                    }
                    let kind = parse_kind(&kind_str).map_err(rmcp::ErrorData::from)?;
                    let mode = parse_delete_mode(op.mode.as_ref())?;
                    pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                        kind: kind as i32,
                        id: Some(validated_id_from(&ns, &slug, version)?),
                        mode: mode as i32,
                        if_match: op.if_match,
                        operation_id: String::new(),
                    })
                }
                other => {
                    return Err(rmcp::ErrorData::invalid_params(
                        format!("unknown op {other}"),
                        None,
                    ))
                }
            };
            ops.push(pb::BatchMutateOp { op: Some(inner) });
        }
        let operation_id = resolve_operation_id(p.operation_id, p.validate_only)?;
        let req = pb::BatchMutateRequest {
            ops,
            validate_only: p.validate_only,
            operation_id: operation_id.clone(),
        };
        let mut client = self.client();
        self.ensure_can_mutate(&mut client, req.validate_only)
            .await?;
        let resp = client
            .batch_mutate(req)
            .await
            .map_err(tool_err("batch_mutate"))
            .map_err(|e| attach_operation_id(e, &operation_id))?
            .into_inner();
        if resp.status == pb::batch_mutate_response::Status::Failed as i32 {
            let err: rmcp::ErrorData = McpError::from(batch_mutate_failure(&resp)).into();
            return Err(attach_operation_id(err, &operation_id));
        }
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Paged change feed. Empty since_token returns an opaque cursor positioned at 'now'; pass it on subsequent calls to read changes from that point.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_changes(
        &self,
        Parameters(p): Parameters<ListChangesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let page_size = i32::try_from(p.page_size.clamp(1, 500))
            .map_err(|_| rmcp::ErrorData::invalid_params("page_size overflows i32", None))?;
        let req = pb::ListChangesRequest {
            since_token: p.since_token.unwrap_or_default(),
            scopes: Vec::new(),
            page_size,
        };
        let mut client = self.client();
        let resp = client
            .list_changes(req)
            .await
            .map_err(tool_err("list_changes"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Durable changeset log, newest first: what landed together, who did it, when. Unlike list_changes these records never expire. Omit page_token for the newest page, then pass the returned next_page_token to walk backwards.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_changesets(
        &self,
        Parameters(p): Parameters<ListChangesetsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let page_size = i32::try_from(p.page_size.clamp(1, 500))
            .map_err(|_| rmcp::ErrorData::invalid_params("page_size overflows i32", None))?;
        let req = pb::ListChangesetsRequest {
            page_token: p.page_token.unwrap_or_default(),
            page_size,
            branch: p.branch.unwrap_or_default(),
        };
        let mut client = self.client();
        let resp = client
            .list_changesets(req)
            .await
            .map_err(tool_err("list_changesets"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Fetch one changeset by id, including every entity it touched. Use the changeset_id carried on a change event to attribute that change.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_changeset(
        &self,
        Parameters(p): Parameters<GetChangesetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut client = self.client();
        let resp = client
            .get_changeset(pb::GetChangesetRequest { id: p.id })
            .await
            .map_err(tool_err("get_changeset"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Edit log for one entity, newest first: every changeset that touched it, with the entity's content before and after. Answers 'when did this field change and who changed it'. Writes made without changeset attribution (server-side import) are absent.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_entity_history(
        &self,
        Parameters(p): Parameters<GetEntityHistoryParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind = parse_kind(&p.kind).map_err(rmcp::ErrorData::from)?;
        let page_size = i32::try_from(p.page_size.clamp(1, 500))
            .map_err(|_| rmcp::ErrorData::invalid_params("page_size overflows i32", None))?;
        let req = pb::GetEntityHistoryRequest {
            r#ref: Some(pb::EntityRef {
                kind: kind as i32,
                id: Some(validated_id_from(&p.namespace, &p.slug, p.version)?),
            }),
            page_token: p.page_token.unwrap_or_default(),
            page_size,
        };
        let mut client = self.client();
        let resp = client
            .get_entity_history(req)
            .await
            .map_err(tool_err("get_entity_history"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Undo a changeset by writing its inverse as a new changeset; nothing is erased. Refuses when an entity it touched has been edited since, or when the original write carried no attribution. Use dry_run to see the inverse ops first.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn revert_changeset(
        &self,
        Parameters(p): Parameters<RevertChangesetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let operation_id = resolve_operation_id(p.operation_id, p.dry_run)?;
        let mut client = self.client();
        self.ensure_can_mutate(&mut client, p.dry_run).await?;
        let resp = client
            .revert_changeset(pb::RevertChangesetRequest {
                id: p.id,
                dry_run: p.dry_run,
                operation_id: operation_id.clone(),
            })
            .await
            .map_err(tool_err("revert_changeset"))
            .map_err(|e| attach_operation_id(e, &operation_id))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Run structural validation over a stored EventModel and its members.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn validate_event_model(
        &self,
        Parameters(p): Parameters<ValidateEventModelParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let req = pb::ValidateEventModelRequest {
            event_model: None,
            event_model_id: Some(validated_id_from(&p.namespace, &p.slug, p.version)?),
        };
        let mut client = self.client();
        let resp = client
            .validate_event_model(req)
            .await
            .map_err(tool_err("validate_event_model"))?
            .into_inner();
        Ok(ok_response(
            "trogonatlas.api.eventmodel.v1alpha1.ValidateEventModelResponse",
            &resp,
            p.json.unwrap_or(false),
        ))
    }

    #[tool(
        description = "Static catalog of every validation rule the validator can emit (code, default severity, title, doc, subject_kind/subject_field hints, category). Use this to render legends, link findings to docs, or to power CI severity gates without reverse-engineering the validator.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_validation_rules(
        &self,
        Parameters(p): Parameters<JsonOpts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut client = self.client();
        let resp = client
            .list_validation_rules(pb::ListValidationRulesRequest {})
            .await
            .map_err(tool_err("list_validation_rules"))?
            .into_inner();
        Ok(ok_response(
            "trogonatlas.api.eventmodel.v1alpha1.ListValidationRulesResponse",
            &resp,
            p.json.unwrap_or(false),
        ))
    }

    #[tool(
        description = "Validate every EventModel in a project (or domain) in one call instead of N. Returns per-model issue lists plus aggregate Error/Warning/Info counts. Scope by project_namespace+project_slug OR by domain_namespace+domain_slug -- set exactly one.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn validate_project(
        &self,
        Parameters(p): Parameters<ValidateProjectParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let has_project = p.project_namespace.is_some() || p.project_slug.is_some();
        let has_domain = p.domain_namespace.is_some() || p.domain_slug.is_some();
        if has_project && has_domain {
            return Err(rmcp::ErrorData::invalid_params(
                "scope is exclusive: set project_namespace+project_slug OR domain_namespace+domain_slug, not both",
                None,
            ));
        }
        let scope = if let (Some(ns), Some(slug)) =
            (p.project_namespace.as_ref(), p.project_slug.as_ref())
        {
            let version = p.project_version.ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "project_version is required and must be > 0 when project scope is set",
                    None,
                )
            })?;
            if version == 0 {
                return Err(rmcp::ErrorData::invalid_params(
                    "project_version must be > 0",
                    None,
                ));
            }
            pb::validate_project_request::Scope::ProjectId(validated_id_from(ns, slug, version)?)
        } else if let (Some(ns), Some(slug)) = (p.domain_namespace.as_ref(), p.domain_slug.as_ref())
        {
            let version = p.domain_version.ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "domain_version is required and must be > 0 when domain scope is set",
                    None,
                )
            })?;
            if version == 0 {
                return Err(rmcp::ErrorData::invalid_params(
                    "domain_version must be > 0",
                    None,
                ));
            }
            pb::validate_project_request::Scope::DomainId(validated_id_from(ns, slug, version)?)
        } else {
            return Err(rmcp::ErrorData::invalid_params(
                "scope is required: set project_namespace+project_slug OR domain_namespace+domain_slug",
                None,
            ));
        };
        let req = pb::ValidateProjectRequest { scope: Some(scope) };
        let mut client = self.client();
        let resp = client
            .validate_project(req)
            .await
            .map_err(tool_err("validate_project"))?
            .into_inner();
        Ok(ok_response(
            "trogonatlas.api.eventmodel.v1alpha1.ValidateProjectResponse",
            &resp,
            p.json.unwrap_or(false),
        ))
    }

    #[tool(
        description = "Bulk delete entities matching a structural query (project / namespace / kind / slug). Replaces the list-then-batch client-side pattern. Mode = 'fail_if_referenced' (default), 'force', or 'dry_run'. Pass max_deletes>0 to refuse runaway deletes.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn delete_by_query(
        &self,
        Parameters(p): Parameters<DeleteByQueryParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind = match p.kind.as_deref() {
            Some(s) if !s.is_empty() => parse_kind(s).map_err(rmcp::ErrorData::from)? as i32,
            _ => pb::EntityKind::Unspecified as i32,
        };
        let mode = parse_delete_mode(p.mode.as_ref())? as i32;
        if p.max_deletes == 0 {
            return Err(rmcp::ErrorData::invalid_params(
                "max_deletes must be > 0 (server refuses uncapped deletes)",
                None,
            ));
        }
        let max_deletes = i32::try_from(p.max_deletes)
            .map_err(|_| rmcp::ErrorData::invalid_params("max_deletes overflows i32", None))?;
        let req = pb::DeleteByQueryRequest {
            project: p.project.unwrap_or_default(),
            namespace: p.namespace.unwrap_or_default(),
            kind,
            slug: p.slug.unwrap_or_default(),
            mode,
            max_deletes,
        };
        let mut client = self.client();
        self.ensure_can_mutate(
            &mut client,
            mode == pb::delete_entity_request::Mode::DryRun as i32,
        )
        .await?;
        let resp = client
            .delete_by_query(req)
            .await
            .map_err(tool_err("delete_by_query"))?
            .into_inner();
        Ok(ok_response(
            "trogonatlas.api.eventmodel.v1alpha1.DeleteByQueryResponse",
            &resp,
            p.json.unwrap_or(false),
        ))
    }

    #[tool(
        description = "Static catalog of every EntityKind the system stores: numeric enum, proto enum name, JSON discriminator (the field in the Entity oneof), human title, plural, and doc. Avoids hardcoding the three-name map in clients.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_entity_kinds(
        &self,
        Parameters(p): Parameters<JsonOpts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut client = self.client();
        let resp = client
            .list_entity_kinds(pb::ListEntityKindsRequest {})
            .await
            .map_err(tool_err("list_entity_kinds"))?
            .into_inner();
        Ok(ok_response(
            "trogonatlas.api.eventmodel.v1alpha1.ListEntityKindsResponse",
            &resp,
            p.json.unwrap_or(false),
        ))
    }

    #[tool(
        description = "List every entity that traces back to a Domain via the BoundedContext.realizes chain. The server walks EM->BC->Subdomain->Domain so clients don't have to. Optional `kinds` filters to one or more entity kinds.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_entities_by_domain(
        &self,
        Parameters(p): Parameters<ListEntitiesByDomainParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if p.version == 0 {
            return Err(rmcp::ErrorData::invalid_params("version must be > 0", None));
        }
        let domain_id = validated_id_from(&p.namespace, &p.slug, p.version)?;
        let kinds = match p.kinds.as_ref() {
            Some(ks) if !ks.is_empty() => ks
                .iter()
                .map(|s| parse_kind(s).map(|k| k as i32))
                .collect::<Result<Vec<_>, _>>()
                .map_err(rmcp::ErrorData::from)?,
            _ => Vec::new(),
        };
        let page_size = match p.page_size {
            None | Some(0) => 0i32,
            Some(v) => i32::try_from(v.min(500))
                .map_err(|_| rmcp::ErrorData::invalid_params("page_size overflows i32", None))?,
        };
        let req = pb::ListEntitiesByDomainRequest {
            domain_id: Some(domain_id),
            kinds,
            page_size,
            page_token: p.page_token.unwrap_or_default(),
            latest_versions_only: p.latest_versions_only.unwrap_or(true),
        };
        let mut client = self.client();
        let resp = client
            .list_entities_by_domain(req)
            .await
            .map_err(tool_err("list_entities_by_domain"))?
            .into_inner();
        Ok(ok_response(
            "trogonatlas.api.eventmodel.v1alpha1.ListEntitiesByDomainResponse",
            &resp,
            p.json.unwrap_or(false),
        ))
    }

    #[tool(
        description = "Infer field-level data flow across a slice / storyboard / event_model. Returns InferredFieldMapping entries with confidence and rationale.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn infer_data_flow(
        &self,
        Parameters(p): Parameters<AnalysisScopeParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let scope =
            scope_from(&p.kind, &p.namespace, &p.slug, p.version).map_err(rmcp::ErrorData::from)?;
        let req = pb::InferDataFlowRequest { scope: Some(scope) };
        let mut client = self.client();
        let resp = client
            .infer_data_flow(req)
            .await
            .map_err(tool_err("infer_data_flow"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Surface information-completeness gaps (missing source, ambiguous source, dangling input, undeclared fields, etc.) for a slice / storyboard / event_model scope.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn check_information_completeness(
        &self,
        Parameters(p): Parameters<AnalysisScopeParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let scope =
            scope_from(&p.kind, &p.namespace, &p.slug, p.version).map_err(rmcp::ErrorData::from)?;
        let req = pb::CheckInformationCompletenessRequest { scope: Some(scope) };
        let mut client = self.client();
        let resp = client
            .check_information_completeness(req)
            .await
            .map_err(tool_err("check_information_completeness"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }

    #[tool(
        description = "Apply one or more YAML manifests (same format as trogon-atlas apply): create missing entities, update changed ones, skip unchanged ones, as one atomic batch. Set dry_run to validate without persisting. Each write is bound to the revision it was planned against: if another writer changed or created an entity in between, nothing is written and the call fails with data.category \"stale_state\" and data.next_action \"replan\", naming the entity with expected_etag and actual_etag (null means absent); call again to plan from fresh state.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn apply_manifests(
        &self,
        Parameters(p): Parameters<ApplyManifestsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let documents =
            trogon_atlas_client::manifest::read_str(&p.manifests_yaml, "apply_manifests")
                .map_err(|e| rmcp::ErrorData::invalid_params(format!("{e:#}"), None))?;
        let operation_id = resolve_operation_id(p.operation_id, p.dry_run)?;
        let mut client = self.client();
        self.ensure_can_apply_plan(&mut client, p.dry_run).await?;
        let resolved =
            trogon_atlas_client::manifest::load_with_tenant_types(&mut client, documents)
                .await
                .map_err(|e| rmcp::ErrorData::from(McpError::from(e)))?;
        let plan = trogon_atlas_client::apply::plan(&mut client, resolved.manifests)
            .await
            .map_err(|e| rmcp::ErrorData::from(McpError::from(e)))?;
        let outcome = trogon_atlas_client::apply::apply(
            &mut client,
            plan,
            p.dry_run,
            (!operation_id.is_empty()).then_some(operation_id.as_str()),
        )
        .await
        .map_err(|e| {
            attach_operation_id(rmcp::ErrorData::from(McpError::from(e)), &operation_id)
        })?;
        let body = serde_json::json!({
            "lines": outcome.lines,
            "issues": outcome.issues.iter().map(trogon_atlas_client::apply::format_issue).collect::<Vec<_>>(),
            "operation_id": (!operation_id.is_empty()).then_some(operation_id.as_str()),
        })
        .to_string();
        Ok(CallToolResult::success(vec![ContentBlock::text(body)]))
    }

    #[tool(
        description = "Diff one or more YAML manifests (same format as trogon-atlas apply) against live state without changing anything. Returns a unified diff and whether anything would change.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn diff_manifests(
        &self,
        Parameters(p): Parameters<DiffManifestsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let documents =
            trogon_atlas_client::manifest::read_str(&p.manifests_yaml, "diff_manifests")
                .map_err(|e| rmcp::ErrorData::invalid_params(format!("{e:#}"), None))?;
        let mut client = self.client();
        let resolved =
            trogon_atlas_client::manifest::load_with_tenant_types(&mut client, documents)
                .await
                .map_err(|e| rmcp::ErrorData::from(McpError::from(e)))?;
        let plan = trogon_atlas_client::apply::plan(&mut client, resolved.manifests)
            .await
            .map_err(|e| rmcp::ErrorData::from(McpError::from(e)))?;
        let (diff, changed) = trogon_atlas_client::apply::render_diff(&plan, &resolved.types)
            .map_err(|e| rmcp::ErrorData::from(McpError::from(e)))?;
        let body = serde_json::json!({ "changed": changed, "diff": diff }).to_string();
        Ok(CallToolResult::success(vec![ContentBlock::text(body)]))
    }

    #[tool(
        description = "Export every entity in a namespace back into manifest YAML (same format as trogon-atlas export), one file per entity kind.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn export_namespace(
        &self,
        Parameters(p): Parameters<ExportNamespaceParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        validate_id_component("namespace", &p.namespace)?;
        let mut client = self.client();
        let outcome = trogon_atlas_client::export::export_namespace(&mut client, &p.namespace)
            .await
            .map_err(|e| rmcp::ErrorData::from(McpError::from(e)))?;
        let body = serde_json::json!({
            "files": outcome.files,
            "skipped": outcome.skipped,
        })
        .to_string();
        Ok(CallToolResult::success(vec![ContentBlock::text(body)]))
    }

    #[tool(
        description = "Export a namespace's ServiceLevelIndicators, ServiceLevelObjectives, AlertPolicies, AlertNotificationTargets, and referenced Components as vendor-neutral OpenSLO v1 multi-document YAML (same behavior as trogon-atlas openslo export). bindings_yaml supplies the DataSource/metricSource query each SLI's Signal needs, which the model does not carry; a Signal missing from it fails the export naming the indicator, unless allow_placeholder_metrics is set.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn export_openslo(
        &self,
        Parameters(p): Parameters<ExportOpensloParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        validate_id_component("namespace", &p.namespace)?;
        let bindings =
            trogon_atlas_client::openslo_bindings::BindingsConfig::parse(&p.bindings_yaml)
                .map_err(|e| rmcp::ErrorData::invalid_params(format!("{e:#}"), None))?;
        let mut client = self.client();
        let outcome = trogon_atlas_client::openslo::export_namespace_openslo(
            &mut client,
            &p.namespace,
            &bindings,
            p.allow_placeholder_metrics,
        )
        .await
        .map_err(|e| rmcp::ErrorData::from(McpError::from(e)))?;
        let body = serde_json::json!({
            "yaml": outcome.yaml,
            "skipped": outcome.skipped,
        })
        .to_string();
        Ok(CallToolResult::success(vec![ContentBlock::text(body)]))
    }

    #[tool(
        description = "Create a new branch (Phase 1: Isolation): a server-side overlay of copy-on-write deltas over the baseline store. Fails if a branch with this name already exists. Scope subsequent tool calls to it by restarting trogon-atlas-mcp with --branch/TROGON_ATLAS_BRANCH set to the branch name.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn create_branch(
        &self,
        Parameters(p): Parameters<CreateBranchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut client = self.client();
        self.ensure_can_mutate(&mut client, false).await?;
        let info = trogon_atlas_client::branch::create_branch(&mut client, &p.name, &p.doc)
            .await
            .map_err(|e| rmcp::ErrorData::from(McpError::from(e)))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "{info:#?}"
        ))]))
    }

    #[tool(
        description = "List every registered branch (Phase 1: Isolation).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_branches(
        &self,
        Parameters(_p): Parameters<EmptyParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut client = self.client();
        let branches = trogon_atlas_client::branch::list_branches(&mut client)
            .await
            .map_err(|e| rmcp::ErrorData::from(McpError::from(e)))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "{branches:#?}"
        ))]))
    }

    #[tool(
        description = "Delete a branch and every delta it holds (Phase 1: Isolation). Entities previously visible only through this branch's deltas become unreachable (or fall through to baseline, if baseline has the key).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn delete_branch(
        &self,
        Parameters(p): Parameters<DeleteBranchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut client = self.client();
        self.ensure_can_mutate(&mut client, false).await?;
        trogon_atlas_client::branch::delete_branch(&mut client, &p.name)
            .await
            .map_err(|e| rmcp::ErrorData::from(McpError::from(e)))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "{} deleted",
            p.name
        ))]))
    }

    #[tool(
        description = "Diff a branch's live deltas against the current baseline (Phase 2: Review and merge). Read-only. Each entry carries a status (ADDED/CHANGED/DELETED/CONVERGED/CONFLICT_EDIT_EDIT/CONFLICT_EDIT_DELETE/CONFLICT_DELETE_EDIT); CONFLICT_EDIT_EDIT entries also carry conflict_field_paths.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn diff_branch(
        &self,
        Parameters(p): Parameters<DiffBranchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut client = self.client();
        let resp = client
            .diff_branch(pb::DiffBranchRequest { name: p.name })
            .await
            .map_err(tool_err("diff_branch"))?
            .into_inner();
        Ok(ok_response(
            "trogonatlas.api.eventmodel.v1alpha1.DiffBranchResponse",
            &resp,
            p.json.unwrap_or(false),
        ))
    }

    #[tool(
        description = "Merge a branch onto baseline (Phase 2: Review and merge). All-or-nothing: any conflict (see conflicts) or post-merge validation Error (see validation) persists nothing. Set dry_run to run full detection and validation without persisting even on success. keep_branch controls whether a successful, non-dry-run merge deletes the branch afterward (default: deletes it). Set auto_merge to land the combined result for edit/edit conflicts whose two sides changed different fields.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn merge_branch(
        &self,
        Parameters(p): Parameters<MergeBranchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let operation_id = resolve_operation_id(p.operation_id, p.dry_run)?;
        let mut client = self.client();
        self.ensure_can_mutate(&mut client, p.dry_run).await?;
        let resp = client
            .merge_branch(pb::MergeBranchRequest {
                name: p.name,
                dry_run: p.dry_run,
                keep_branch: p.keep_branch,
                auto_merge: p.auto_merge,
                operation_id: operation_id.clone(),
            })
            .await
            .map_err(tool_err("merge_branch"))
            .map_err(|e| attach_operation_id(e, &operation_id))?
            .into_inner();
        Ok(ok_response(
            "trogonatlas.api.eventmodel.v1alpha1.MergeBranchResponse",
            &resp,
            p.json.unwrap_or(false),
        ))
    }

    #[tool(
        description = "Rebase a branch's non-conflicting deltas forward to the current baseline (Phase 2: Review and merge, the pull/rebase analog): CONVERGED entries are dropped, others advance base/base_etag. Conflicting entries are left untouched and returned in conflicts for resolve_branch_entry.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_branch(
        &self,
        Parameters(p): Parameters<UpdateBranchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut client = self.client();
        self.ensure_can_mutate(&mut client, false).await?;
        let resp = client
            .update_branch(pb::UpdateBranchRequest { name: p.name })
            .await
            .map_err(tool_err("update_branch"))?
            .into_inner();
        Ok(ok_response(
            "trogonatlas.api.eventmodel.v1alpha1.UpdateBranchResponse",
            &resp,
            p.json.unwrap_or(false),
        ))
    }

    #[tool(
        description = "Resolve a single conflicting entry on a branch (Phase 2: Review and merge). resolution must be \"take_theirs\" (discard the branch's edit, adopting current baseline content), \"keep_ours\" (keep the branch's value, rebasing forward so the conflict clears), or \"take_merged\" (take both sides' edits combined, available only when the two sides changed different fields). expected_state binds the decision to the entry state it was made from; if the entry moved since, the call fails with data.category \"stale_state\" and data.next_action \"replan\", carrying expected_state and actual_state, and nothing changes.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn resolve_branch_entry(
        &self,
        Parameters(p): Parameters<ResolveBranchEntryParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let kind = parse_kind(&p.kind).map_err(rmcp::ErrorData::from)?;
        let resolution = match p.resolution.as_str() {
            "take_theirs" => Resolution::TakeTheirs,
            "keep_ours" => Resolution::KeepOurs,
            "take_merged" => Resolution::TakeMerged,
            other => {
                return Err(rmcp::ErrorData::invalid_params(
                    format!(
                        "resolution must be \"take_theirs\", \"keep_ours\" or \"take_merged\", got \
                         {other:?}"
                    ),
                    None,
                ))
            }
        };
        let expected = BranchEntryState::from_wire(&pb::BranchEntryState {
            base_etag: p.expected_state.base_etag,
            ours_etag: p.expected_state.ours_etag,
            theirs_etag: p.expected_state.theirs_etag,
        })
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("expected_state: {e:#}"), None))?;
        let entity_ref = pb::EntityRef {
            kind: kind as i32,
            id: Some(validated_id_from(&p.namespace, &p.slug, p.version)?),
        };
        let operation_id = resolve_operation_id(p.operation_id, false)?;
        let mut client = self.client();
        self.ensure_can_apply_plan(&mut client, false).await?;
        trogon_atlas_client::branch::resolve_branch_entry(
            &mut client,
            &p.name,
            entity_ref,
            resolution,
            &expected,
            (!operation_id.is_empty()).then_some(operation_id.as_str()),
        )
        .await
        .map_err(|e| {
            attach_operation_id(rmcp::ErrorData::from(McpError::from(e)), &operation_id)
        })?;
        Ok(CallToolResult::success(vec![ContentBlock::text(
            "resolved",
        )]))
    }

    #[tool(
        description = "Report the status of a previously used operation_id: pending (claimed, not yet settled), applied (persisted; a receipt exists), rejected/not_applied (did not persist; safe to retry under a new operation_id), or unknown (never claimed, settled long enough ago to be forgotten, or belongs to a branch-scoped mutation, which keeps no durable receipt).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_operation(
        &self,
        Parameters(p): Parameters<GetOperationParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        trogon_atlas_core::validate_operation_id(&p.operation_id)
            .map_err(|e| rmcp::ErrorData::invalid_params(format!("operation_id {e}"), None))?;
        let mut client = self.client();
        let resp = client
            .get_operation(pb::GetOperationRequest {
                operation_id: p.operation_id,
            })
            .await
            .map_err(tool_err("get_operation"))?
            .into_inner();
        Ok(ok_envelope(&resp))
    }
}

#[allow(clippy::unused_async_trait_impl)]
#[tool_handler]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(SERVER_INSTRUCTIONS)
    }

    /// Filters the tool list down to what `self.role` could actually call,
    /// so an agent never offers itself a tool the server would answer with
    /// `PERMISSION_DENIED`. `call_tool` is left for `#[tool_handler]` to
    /// generate unfiltered: the server is the real enforcement point, and
    /// this listing is only a courtesy that degrades to "list everything"
    /// whenever `self.role` is unknown (see `McpServer::learn_role`).
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
        let tools = self.tool_router.list_all();
        let tools = match self.role {
            None => tools,
            Some(role) => tools
                .into_iter()
                .filter(|tool| tool_allowed_for(&tool.name, role))
                .collect(),
        };
        Ok(rmcp::model::ListToolsResult::with_all_items(tools))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn parse_kind_accepts_all_variants() {
        for (s, expected) in [
            ("event", pb::EntityKind::Event),
            ("Command", pb::EntityKind::Command),
            ("read_model", pb::EntityKind::ReadModel),
            ("readmodel", pb::EntityKind::ReadModel),
            ("command_slice", pb::EntityKind::CommandSlice),
            ("event_model", pb::EntityKind::EventModel),
        ] {
            assert_eq!(parse_kind(s).unwrap(), expected, "kind {s}");
        }
        assert!(parse_kind("nonsense").is_err());
    }

    #[test]
    fn entity_b64_roundtrip() {
        let entity = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id_from("shop", "order-placed", 1)),
                title: "Order placed".into(),
                ..Default::default()
            })),
        };
        let b64 = encode_b64(&entity);
        let decoded = decode_entity_b64(&b64).unwrap();
        assert_eq!(decoded, entity);
        assert!(decode_entity_b64("!!!not-base64!!!").is_err());
    }

    #[test]
    fn batch_mutate_failure_with_precondition_failure_is_a_stale_plan() {
        let resp = pb::BatchMutateResponse {
            status: pb::batch_mutate_response::Status::Failed as i32,
            precondition_failure: Some(pb::PreconditionFailure {
                subject: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id_from("shop", "order-placed", 1)),
                }),
                expected_etag: String::new(),
                actual_etag: "7".into(),
            }),
            ..Default::default()
        };
        let err = batch_mutate_failure(&resp);
        let failure = trogon_atlas_client::failure::Failure::classify(&err);
        assert_eq!(
            failure.category,
            trogon_atlas_client::failure::FailureCategory::StaleState
        );
    }

    #[test]
    fn batch_mutate_failure_with_a_known_issue_code_classifies_from_it() {
        let resp = pb::BatchMutateResponse {
            status: pb::batch_mutate_response::Status::Failed as i32,
            failed_op_index: 0,
            failure: vec![pb::ValidationIssue {
                code: "ENTITY_NOT_FOUND".into(),
                message: "entity does not exist".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let err = batch_mutate_failure(&resp);
        let failure = trogon_atlas_client::failure::Failure::classify(&err);
        assert_eq!(
            failure.category,
            trogon_atlas_client::failure::FailureCategory::NotFound
        );
        assert_eq!(
            failure
                .code
                .as_ref()
                .map(trogon_atlas_client::failure::FailureCode::as_str),
            Some("ENTITY_NOT_FOUND")
        );
    }

    #[test]
    fn entity_json_roundtrip() {
        let entity = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id_from("shop", "order-placed", 1)),
                title: "Order placed".into(),
                ..Default::default()
            })),
        };
        let json = message_to_json("trogonatlas.eventmodel.v1alpha1.Entity", &entity).unwrap();
        let back: pb::Entity =
            message_from_json("trogonatlas.eventmodel.v1alpha1.Entity", &json).unwrap();
        assert_eq!(back, entity);
    }

    #[test]
    fn entity_json_accepts_handwritten_payload() {
        let json = r#"{"event": {"id": {"namespace": "shop", "slug": "order-placed", "version": 1}, "title": "Order placed"}}"#;
        let entity: pb::Entity =
            message_from_json("trogonatlas.eventmodel.v1alpha1.Entity", json).unwrap();
        match entity.kind {
            Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Order placed"),
            other => panic!("expected event, got {other:?}"),
        }
    }

    #[test]
    fn entity_json_rejects_unknown_shape() {
        let err: Result<pb::Entity> =
            message_from_json("trogonatlas.eventmodel.v1alpha1.Entity", r#"{"bogus": 1}"#);
        assert!(err.is_err());
    }

    #[test]
    fn scope_from_rejects_non_scope_kinds() {
        assert!(scope_from("event", "shop", "x", 1).is_err());
        assert!(scope_from("command_slice", "shop", "x", 1).is_ok());
        assert!(scope_from("storyboard", "shop", "x", 1).is_ok());
    }

    #[test]
    fn scope_from_picks_slice_for_each_slice_kind() {
        for kind in [
            "command_slice",
            "read_model_slice",
            "automation_slice",
            "ui_slice",
        ] {
            let scope = scope_from(kind, "shop", "x", 1).unwrap();
            assert!(
                matches!(scope.scope, Some(pb::analysis_scope::Scope::Slice(_))),
                "{kind} did not map to a Slice scope"
            );
        }
    }

    #[test]
    fn scope_from_storyboard_and_event_model_are_distinct_variants() {
        let sb = scope_from("storyboard", "shop", "x", 1).unwrap();
        assert!(matches!(
            sb.scope,
            Some(pb::analysis_scope::Scope::Storyboard(_))
        ));

        let em = scope_from("event_model", "shop", "x", 1).unwrap();
        assert!(matches!(
            em.scope,
            Some(pb::analysis_scope::Scope::EventModel(_))
        ));
    }

    #[test]
    fn parse_delete_mode_accepts_aliases() {
        use pb::delete_entity_request::Mode;
        assert!(matches!(
            parse_delete_mode(Some("force".to_string()).as_ref()),
            Ok(Mode::Force)
        ));
        assert!(matches!(
            parse_delete_mode(Some("FORCE".to_string()).as_ref()),
            Ok(Mode::Force)
        ));
        assert!(matches!(
            parse_delete_mode(Some("dry_run".to_string()).as_ref()),
            Ok(Mode::DryRun)
        ));
        assert!(matches!(
            parse_delete_mode(Some("dryrun".to_string()).as_ref()),
            Ok(Mode::DryRun)
        ));
        assert!(matches!(
            parse_delete_mode(Some("fail_if_referenced".to_string()).as_ref()),
            Ok(Mode::FailIfReferenced)
        ));
        assert!(matches!(
            parse_delete_mode(None),
            Ok(Mode::FailIfReferenced)
        ));
    }

    // Tool description enumerates fail_if_referenced / force / dry_run. A typo
    // like "froce" must not silently become FailIfReferenced (agents think they
    // forced a delete when they did not).
    #[test]
    fn parse_delete_mode_rejects_unknown_mode() {
        let err = parse_delete_mode(Some("garbage".to_string()).as_ref()).unwrap_err();
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("garbage") || err.message.contains("mode"),
            "{}",
            err.message
        );
    }

    #[test]
    fn search_query_byte_limit_matches_server() {
        assert_eq!(
            MAX_SEARCH_QUERY_BYTES, 2048,
            "MCP search must use the server 2048-byte query cap, not a higher local limit"
        );
    }

    #[test]
    fn server_instructions_must_not_claim_debug_envelope() {
        assert!(
            !SERVER_INSTRUCTIONS.contains("human-readable Debug"),
            "ok_envelope no longer returns Debug; instructions must not claim it"
        );
        assert!(
            SERVER_INSTRUCTIONS.contains("binpb_base64"),
            "instructions should still describe the binpb_base64 envelope"
        );
    }

    #[test]
    fn kinds_to_i32_translates_and_short_circuits_on_first_unknown() {
        let ok = kinds_to_i32(Some(&vec!["event".into(), "command".into()])).unwrap();
        assert_eq!(
            ok,
            vec![pb::EntityKind::Event as i32, pb::EntityKind::Command as i32]
        );
        assert!(kinds_to_i32(Some(&vec!["event".into(), "nonsense".into()])).is_err());
        assert_eq!(kinds_to_i32(None).unwrap(), [] as [i32; 0]);
    }

    #[test]
    fn id_from_assembles_id() {
        let id = id_from("shop", "order", 7);
        assert_eq!(id.namespace, "shop");
        assert_eq!(id.slug, "order");
        assert_eq!(id.version, 7);
    }

    #[test]
    fn descriptor_pool_resolves_canonical_messages() {
        // Sanity: every message type the MCP layer mentions by FQN string
        // must resolve through the cached descriptor pool. If a future
        // proto rename misses one of these strings, the matching tool
        // handler would return garbage at runtime; this test catches it.
        for name in [
            "trogonatlas.eventmodel.v1alpha1.Entity",
            "trogonatlas.api.eventmodel.v1alpha1.BatchMutateRequest",
        ] {
            assert!(
                trogon_atlas_core::transcode::message_descriptor(name).is_ok(),
                "message_descriptor({name}) failed: proto rename / drift?",
            );
        }
    }

    #[test]
    fn mcp_error_unknown_kind_is_invalid_params() {
        let err: rmcp::ErrorData = McpError::UnknownKind("ghost".into()).into();
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("ghost"));
    }

    #[test]
    fn mcp_error_internal_is_internal_error() {
        let err: rmcp::ErrorData = McpError::Internal("boom".into()).into();
        assert_eq!(err.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        assert!(err.message.contains("boom"));
    }

    // Item 5: page_size clamp boundaries. The `clamp(1, 500)` applied before
    // conversion to i32 means zero input becomes 1 and inputs above 500 become 500.
    #[test]
    fn page_size_clamp_zero_becomes_one() {
        assert_eq!(0u32.clamp(1, 500), 1);
    }

    #[test]
    fn page_size_clamp_above_max_becomes_five_hundred() {
        assert_eq!(501u32.clamp(1, 500), 500);
        assert_eq!(u32::MAX.clamp(1, 500), 500);
    }

    #[test]
    fn page_size_clamp_boundaries_are_inclusive() {
        assert_eq!(1u32.clamp(1, 500), 1);
        assert_eq!(500u32.clamp(1, 500), 500);
    }

    // Item 1: validate_id_component rejects the same characters as
    // trogon_atlas_core::validate_id_component (see trogon-atlas-core/src/id_component.rs).
    #[test]
    fn validate_id_component_accepts_valid_slugs() {
        for v in ["orders", "order-placed", "order.placed", "v2", "a_b"] {
            assert!(
                validate_id_component("test", v).is_ok(),
                "rejected valid slug {v:?}"
            );
        }
    }

    #[test]
    fn validate_id_component_rejects_empty() {
        assert!(validate_id_component("test", "").is_err());
    }

    #[test]
    fn validate_id_component_rejects_dot_and_dotdot() {
        assert!(validate_id_component("test", ".").is_err());
        assert!(validate_id_component("test", "..").is_err());
    }

    #[test]
    fn validate_id_component_rejects_leading_dot() {
        assert!(validate_id_component("test", ".git").is_err());
    }

    #[test]
    fn validate_id_component_rejects_spaces_and_control_chars() {
        assert!(validate_id_component("test", "bad slug").is_err());
        assert!(validate_id_component("test", "a\nb").is_err());
    }

    #[test]
    fn validate_id_component_rejects_overlong() {
        let long = "a".repeat(201);
        assert!(validate_id_component("test", &long).is_err());
    }

    #[test]
    fn mcp_error_invalid_id_component_is_invalid_params() {
        let err: rmcp::ErrorData = McpError::InvalidIdComponent {
            field: "namespace",
            detail: "must not be empty".into(),
        }
        .into();
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("namespace"));
    }

    // ------------------------------------------------------------------
    // ToolAnnotations coverage
    // ------------------------------------------------------------------
    //
    // Every #[tool(...)] above must declare all four `ToolAnnotations` hints
    // explicitly, and `read_only_hint` must agree with the server's RBAC
    // role map (`trogon_atlas_server::auth::required_role`) for every gRPC
    // method the tool invokes. This catches a tool added without
    // annotations, a tool invoking an RPC the role map does not know about,
    // and an annotation that drifts from what the server actually requires
    // to call the tool.

    use std::collections::HashSet;

    use trogon_atlas_server::auth::{required_role, Role};

    const EVENT_MODEL_SERVICE: &str = "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/";

    impl From<Role> for ToolRole {
        fn from(role: Role) -> Self {
            match role {
                Role::Reader => ToolRole::Reader,
                Role::Writer => ToolRole::Writer,
                Role::Admin => ToolRole::Admin,
            }
        }
    }

    /// `get_operation` is the only tool whose `read_only_hint` does not
    /// equal `required_role(..) == Role::Reader`. `auth::required_role`
    /// classifies GetOperation as `Role::Writer` only because operation
    /// receipts are scoped to the caller's own principal, not because the
    /// RPC mutates anything (see the comment above that match arm,
    /// crates/trogon-atlas-server/src/auth.rs:246-249). The tool itself
    /// only reads, so it is deliberately annotated read_only_hint = true.
    const READ_ONLY_ROLE_EXEMPT: &[&str] = &["get_operation"];

    #[test]
    fn every_tool_has_annotations_matching_the_role_map() {
        let live_tools = McpServer::tool_router().list_all();
        let live_names: HashSet<String> = live_tools.iter().map(|t| t.name.to_string()).collect();
        let table_names: HashSet<&str> = TOOL_POLICIES.iter().map(|r| r.tool).collect();

        let live_without_a_row: Vec<&String> = live_names
            .iter()
            .filter(|n| !table_names.contains(n.as_str()))
            .collect();
        let rows_without_a_live_tool: Vec<&&str> = table_names
            .iter()
            .filter(|n| !live_names.contains(**n))
            .collect();
        assert!(
            live_without_a_row.is_empty() && rows_without_a_live_tool.is_empty(),
            "tool set mismatch: live tools missing a TOOL_POLICIES row: {live_without_a_row:?}; \
             TOOL_POLICIES rows with no matching live tool: {rows_without_a_live_tool:?}"
        );

        for tool in &live_tools {
            let name = tool.name.as_ref();
            let row = TOOL_POLICIES
                .iter()
                .find(|r| r.tool == name)
                .unwrap_or_else(|| panic!("tool {name}: no TOOL_POLICIES row (checked above)"));
            assert!(!row.rpcs.is_empty(), "tool {name}: empty rpcs list");

            let annotations = tool
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("tool {name}: missing ToolAnnotations"));
            let read_only_hint = annotations
                .read_only_hint
                .unwrap_or_else(|| panic!("tool {name}: missing read_only_hint"));
            let destructive_hint = annotations
                .destructive_hint
                .unwrap_or_else(|| panic!("tool {name}: missing destructive_hint"));
            let _idempotent_hint = annotations
                .idempotent_hint
                .unwrap_or_else(|| panic!("tool {name}: missing idempotent_hint"));
            let _open_world_hint = annotations
                .open_world_hint
                .unwrap_or_else(|| panic!("tool {name}: missing open_world_hint"));

            let max_role = row
                .rpcs
                .iter()
                .map(|rpc| {
                    let path = format!("{EVENT_MODEL_SERVICE}{rpc}");
                    required_role(&path).unwrap_or_else(|| {
                        panic!(
                            "tool {name}: RPC {rpc} is not in auth::required_role \
                             (new RPC missing from the server's role map?)"
                        )
                    })
                })
                .max()
                .expect("non-empty, checked above");

            if READ_ONLY_ROLE_EXEMPT.contains(&name) {
                assert!(
                    read_only_hint,
                    "tool {name} is in READ_ONLY_ROLE_EXEMPT and must have read_only_hint = true"
                );
            } else {
                assert_eq!(
                    read_only_hint,
                    max_role == Role::Reader,
                    "tool {name}: read_only_hint ({read_only_hint}) does not match the role \
                     map (max role {max_role:?} over {:?})",
                    row.rpcs
                );
            }

            assert!(
                !(read_only_hint && destructive_hint),
                "tool {name}: read_only_hint and destructive_hint are both true"
            );

            assert_eq!(
                row.min_role,
                ToolRole::from(max_role),
                "tool {name}: ToolPolicy.min_role ({:?}) does not match the max role \
                 required_role reports over {:?} ({max_role:?})",
                row.min_role,
                row.rpcs
            );
        }
    }
}
