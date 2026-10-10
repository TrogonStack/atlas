use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt,
};

use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    refs::{entity_id, entity_kind, outbound_refs},
    StoredEntity,
};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
struct IssuerId {
    namespace: String,
    slug: String,
    version: u64,
}

impl From<&pb::Id> for IssuerId {
    fn from(id: &pb::Id) -> Self {
        Self {
            namespace: id.namespace.clone(),
            slug: id.slug.clone(),
            version: id.version,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
enum CommandIssuer {
    Ui(IssuerId),
    Processor(IssuerId),
}

impl CommandIssuer {
    fn ui(id: &pb::Id) -> Self {
        Self::Ui(IssuerId::from(id))
    }

    fn processor(id: &pb::Id) -> Self {
        Self::Processor(IssuerId::from(id))
    }
}

impl fmt::Display for CommandIssuer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandIssuer::Ui(id) => {
                write!(f, "ui:{}/{}@{}", id.namespace, id.slug, id.version)
            }
            CommandIssuer::Processor(id) => {
                write!(f, "processor:{}/{}@{}", id.namespace, id.slug, id.version)
            }
        }
    }
}

// Lifecycle statuses that mean "not yet landed". A draft version must not
// nag the whole model into migrating to it. When the annotation is removed or
// flipped to another status, the full migration checklist surfaces.
const DRAFT_LIFECYCLE_STATUSES: &[&str] = &["draft", "proposed"];

// Static catalog of every rule the validator can emit. The `code`
// matches what `validate_model` / `validate_scenarios` push into
// `ValidationIssue.code`; `default_severity` is the canonical severity
// at the rule's primary emission site (rules whose severity is
// conditional (e.g. READ_MODEL_SHARED_CONSUMERS downgrades to Info
// when SharedConsumersAnnotation is present), record the base case.
//
// A unit test below diffs the catalog against the literal codes
// referenced in this file so the two cannot drift.
// Static catalog of every EntityKind the system stores. Same shape and
// motivation as `rule_catalog`: gives consumers (LLM agents, the
// studio, MCP scripts) a single source of truth instead of grepping
// the proto for the {numeric, proto_name, json_key} triple.
//
// A drift test below asserts every EntityKind enum variant has a
// catalog entry (except UNSPECIFIED, which is the proto sentinel).
#[must_use]
pub fn entity_kind_catalog() -> Vec<pb::EntityKindInfo> {
    use pb::EntityKind::{
        AlertNotificationTarget, AlertPolicy, Ambiguity, AutomationSlice, BoundedContext, Command,
        CommandSlice, Component, Domain, Event, EventModel, ExternalSystem, Persona, Processor,
        Project, ReadModel, ReadModelSlice, Schema, Screen, ServiceLevelIndicator,
        ServiceLevelObjective, Storyboard, Subdomain, Swimlane, Term, Tracker, TypeLibrary, Ui,
        UiSlice,
    };
    let k = |kind: pb::EntityKind,
             proto_name: &str,
             json_key: &str,
             title: &str,
             plural: &str,
             doc: &str| {
        pb::EntityKindInfo {
            kind: kind as i32,
            proto_name: proto_name.into(),
            json_key: json_key.into(),
            title: title.into(),
            plural: plural.into(),
            doc: doc.into(),
        }
    };
    vec![
        k(Event, "ENTITY_KIND_EVENT", "event", "Event", "Events",
          "A past-tense fact. The append-only ground truth of the system."),
        k(Command, "ENTITY_KIND_COMMAND", "command", "Command", "Commands",
          "An imperative intent issued by a persona or processor. Wrapped by a CommandSlice that lists the events it emits."),
        k(ReadModel, "ENTITY_KIND_READ_MODEL", "readModel", "Read Model", "Read Models",
          "A projection of one or more events shaped for a single consumer (UI, automation, search). Decision #34: one read model per consumer."),
        k(Processor, "ENTITY_KIND_PROCESSOR", "processor", "Processor", "Processors",
          "An automated actor. Reads a read model, issues exactly one command. Each branching outcome is its own processor."),
        k(Ui, "ENTITY_KIND_UI", "ui", "UI", "UIs",
          "A surface where humans observe a read model or trigger a command. One UI per moment per role (Decision: split form-vs-display)."),
        k(Persona, "ENTITY_KIND_PERSONA", "persona", "Persona", "Personas",
          "A human role (or system actor) the model attributes commands and observations to."),
        k(Swimlane, "ENTITY_KIND_SWIMLANE", "swimlane", "Swimlane", "Swimlanes",
          "A stream's lane on the board. Carries a stream_id template and the lifecycle transitions for events that live on this stream."),
        k(CommandSlice, "ENTITY_KIND_COMMAND_SLICE", "commandSlice", "Command Slice", "Command Slices",
          "A write-side moment: a command emits events. Persona + UI make it human-issued; an AutomationSlice can issue the same command without those human edges."),
        k(ReadModelSlice, "ENTITY_KIND_READ_MODEL_SLICE", "readModelSlice", "Read Model Slice", "Read Model Slices",
          "A moment: events project into a read model. It covers the projection alone; who displays it is a UiSlice, and who acts on it is an AutomationSlice."),
        k(UiSlice, "ENTITY_KIND_UI_SLICE", "uiSlice", "UI Slice", "UI Slices",
          "A moment: a persona sees read models rendered on a UI. The human counterpart of the AutomationSlice, and the one place a display-only screen names its persona."),
        k(AutomationSlice, "ENTITY_KIND_AUTOMATION_SLICE", "automationSlice", "Automation Slice", "Automation Slices",
          "A moment: a processor watches a read model and issues a command. Replaces a human at the wheel."),
        k(Storyboard, "ENTITY_KIND_STORYBOARD", "storyboard", "Storyboard", "Storyboards",
          "An ordered sequence of slices forming one narrative: entry observation, action sequence, terminal outcome events."),
        k(EventModel, "ENTITY_KIND_EVENT_MODEL", "eventModel", "Event Model", "Event Models",
          "A curated subset of the store rendered as one board: members + storyboards. The unit of viewing."),
        k(Component, "ENTITY_KIND_COMPONENT", "component", "Component", "Components",
          "A piece of runtime infrastructure or code that backs a slice (e.g. a worker, a service). Implementation, not modeling."),
        k(ExternalSystem, "ENTITY_KIND_EXTERNAL_SYSTEM", "externalSystem", "External System", "External Systems",
          "A third-party vendor or upstream system the model integrates with."),
        k(Tracker, "ENTITY_KIND_TRACKER", "tracker", "Tracker", "Trackers",
          "An open question, decision, or follow-up attached to a subject. Doesn't render on the board."),
        k(BoundedContext, "ENTITY_KIND_BOUNDED_CONTEXT", "boundedContext", "Bounded Context", "Bounded Contexts",
          "A context boundary: owns its events/commands/RMs. Decision #27: slug == namespace."),
        k(Domain, "ENTITY_KIND_DOMAIN", "domain", "Domain", "Domains",
          "The container word for the business / problem space. Usually exactly one per project."),
        k(Subdomain, "ENTITY_KIND_SUBDOMAIN", "subdomain", "Subdomain", "Subdomains",
          "One problem the business must be good at. Classified CORE / SUPPORTING / GENERIC."),
        k(Schema, "ENTITY_KIND_SCHEMA", "schema", "Schema", "Schemas",
          "A named field shape that field-bearing entities (events/commands/RMs) reference. Decision #33."),
        k(Project, "ENTITY_KIND_PROJECT", "project", "Project", "Projects",
          "The codebase / repository / customer the model was mapped from. The outermost grouping in a multi-project store."),
        k(Screen, "ENTITY_KIND_SCREEN", "screen", "Screen", "Screens",
          "A URL-addressable surface that composes UI contributions from one or more bounded contexts."),
        k(Term, "ENTITY_KIND_TERM", "term", "Term", "Terms",
          "A word and definition owned by one bounded context, with links to the entities that embody it."),
        k(Ambiguity, "ENTITY_KIND_AMBIGUITY", "ambiguity", "Ambiguity", "Ambiguities",
          "A human ruling about terms that collide across contexts: homonym or synonym, deliberate, rename, or merge."),
        k(ServiceLevelIndicator, "ENTITY_KIND_SERVICE_LEVEL_INDICATOR", "serviceLevelIndicator", "Service Level Indicator", "Service Level Indicators",
          "A measurement bound to a graph Connection (command handling, projection, reaction, display, integration, or journey). Overlay entity; never a vendor query."),
        k(ServiceLevelObjective, "ENTITY_KIND_SERVICE_LEVEL_OBJECTIVE", "serviceLevelObjective", "Service Level Objective", "Service Level Objectives",
          "A target on an indicator over a time window, with the component it holds accountable and the alert policies that watch it."),
        k(AlertPolicy, "ENTITY_KIND_ALERT_POLICY", "alertPolicy", "Alert Policy", "Alert Policies",
          "Conditions that page, ticket, or inform, and the notification targets they route to. Retuning a condition never rewrites a design slice."),
        k(AlertNotificationTarget, "ENTITY_KIND_ALERT_NOTIFICATION_TARGET", "alertNotificationTarget", "Alert Notification Target", "Alert Notification Targets",
          "Where an alert routes (on-call rotation, channel, webhook). Rotating the target never rewrites the policy that points at it."),
        k(TypeLibrary, "ENTITY_KIND_TYPE_LIBRARY", "typeLibrary", "Type Library", "Type Libraries",
          "Tenant-owned protobuf source files under a package prefix the library owns. The server compiles them on demand; the source is the only stored form."),
    ]
}

// Walk a finding list and fill in the inline rule context fields
// (`rule_title`, `rule_category`, `rule_default_severity`) from the
// static catalog. Cheap: O(issues * 1) hash lookup. Callers that want
// raw findings can skip this.
pub fn enrich_issues_with_catalog(issues: &mut [pb::ValidationIssue]) {
    use std::collections::HashMap;
    let catalog = rule_catalog();
    let by_code: HashMap<&str, &pb::ValidationRule> =
        catalog.iter().map(|r| (r.code.as_str(), r)).collect();
    for iss in issues.iter_mut() {
        if let Some(rule) = by_code.get(iss.code.as_str()) {
            iss.rule_title = rule.title.clone();
            iss.rule_category = rule.category.clone();
            iss.rule_default_severity = rule.default_severity;
        }
    }
}

#[must_use]
pub fn rule_catalog() -> Vec<pb::ValidationRule> {
    // Explicit aliases: both enums carry an `Unspecified` variant, so a
    // `use ::*` glob would be ambiguous.
    use pb::{
        validation_issue::Severity::{Error, Info, Warning},
        EntityKind::{
            Ambiguity, AutomationSlice, BoundedContext, Command, CommandSlice, Domain, Event,
            Processor, Project, ReadModel, ReadModelSlice, Screen, ServiceLevelIndicator,
            ServiceLevelObjective, Storyboard, Subdomain, Swimlane, Term, Tracker, Ui, UiSlice,
            Unspecified,
        },
    };
    let r = |code: &str,
             default_severity: pb::validation_issue::Severity,
             title: &str,
             doc: &str,
             subject_kind: pb::EntityKind,
             subject_field: &str,
             category: &str| pb::ValidationRule {
        code: code.into(),
        default_severity: default_severity as i32,
        title: title.into(),
        doc: doc.into(),
        subject_kind: subject_kind as i32,
        subject_field: subject_field.into(),
        category: category.into(),
    };
    vec![
        // ----- model invariants (validate_model) ---------------------------------
        r("MISSING_ID", Error, "Member missing id",
          "A member EntityRef has no id; every member of an EventModel must point at a concrete entity.",
          Unspecified, "", "model"),
        r("INVALID_KIND", Error, "Member has invalid kind",
          "A member EntityRef carries a kind value that does not map to any EntityKind.",
          Unspecified, "", "model"),
        r("MEMBER_NOT_FOUND", Error, "Member not found in store",
          "A member's (kind, id) does not resolve in the store; either the entity was deleted or the model is pointing at nothing.",
          Unspecified, "", "model"),
        r("OPEN_QUESTION_PRESENT", Info, "Entity carries an open question",
          "An entity or scenario carries an OpenQuestionAnnotation: an unanswered design question pinned to it. Informational; humans answer by editing the annotation or removing it once resolved.",
          Unspecified, "metadata", "model"),
        r("ASSUMPTION_PRESENT", Info, "Entity carries an unconfirmed assumption",
          "An entity or scenario carries an AssumptionAnnotation: a belief the design relies on that nobody has confirmed. Unlike an open question, the model already proceeds as if it were true. Informational; humans confirm or correct it by editing the annotation or removing it once settled.",
          Unspecified, "metadata", "model"),
        r("PROCESSOR_MULTIPLE_COMMANDS", Error, "Processor issues more than one command",
          "A processor issues exactly one command. Branching outcomes (approve vs reject) are different scenarios: model each as its own automation slice with its own processor.",
          Processor, "emitted_command", "model"),
        r("READ_MODEL_MULTIPLE_AUTOMATIONS", Error, "Read model consumed by multiple automations",
          "A read model is observed by more than one automation slice; different consumers need different data, so each automation should observe its own consumer-shaped view. Opt in with SharedConsumersAnnotation when the fan-out is intentional.",
          ReadModel, "source_read_models", "model"),
        r("READ_MODEL_SHARED_CONSUMERS", Info, "Read model shared across consumers (declared)",
          "A read model serves multiple automation slices and a SharedConsumersAnnotation declares the arrangement is intentional. Informational; surfaces because the multi-consumer pattern is load-bearing.",
          ReadModel, "source_read_models", "model"),
        r("READ_MODEL_SHARED_CONSUMERS_DOC_EMPTY", Error, "SharedConsumersAnnotation has no doc",
          "A read model carries SharedConsumersAnnotation but its doc is empty; name the consumers and why they share the same data.",
          ReadModel, "metadata", "model"),
        r("EVENT_MULTIPLE_EMITTERS", Error, "Event emitted by more than one command",
          "An event is emitted by more than one distinct command; the board cannot say which decision caused the fact. There is NO annotation escape: when a design fans in, fix it structurally (origin split merged by a read model, or a projection + detector automation firing one dedicated completion command); when an AS-IS mapping fans in, the error is the finding: it reports the mapped system's own causal ambiguity, and the fix belongs in that system's code, not in the model.",
          Event, "", "model"),
        r("CODE_REF_INCOMPLETE", Error, "CodeRefAnnotation missing repo or path",
          "A CodeRefAnnotation must carry a logical `repo` name and a repo-relative `path`; without both the reference cannot be resolved, verified for drift, or queried. Fill both (symbol recommended, line numbers never) or remove the annotation.",
          Unspecified, "metadata", "model"),
        r("EVENT_UNOBSERVED", Warning, "Event sourced by no read model",
          "A fact nobody observes changes nothing: every event the model emits must be sourced by at least one read model (its own views or a downstream context's subscription view). Project it into a view or question why it exists.",
          Event, "source_events", "model"),
        r("COMMAND_NO_EMITTED_EVENTS", Warning, "Command has no CommandSlice that emits events",
          "A command without a CommandSlice listing emitted_events points nowhere and can never cause a state change. Add a CommandSlice with at least one emitted_events entry, or remove the command.",
          Command, "emitted_events", "model"),
        r("COMMAND_MULTI_STREAM_WRITE", Error, "Command writes to more than one stream",
          "One command, one stream. A CommandSlice's emitted_events must all sit on the same swimlane as the command itself; cross-stream effects must be modeled as a saga (the first command writes its single-stream event(s), a processor reads the resulting read model, and an AutomationSlice fires a second command on the second stream). Decision: 1 command → 1 stream.",
          CommandSlice, "emitted_events", "model"),
        r("COMMAND_HAS_NO_TRIGGER", Error, "Command has no trigger source",
          "Every command must be issued by someone: a human via a UI form (CommandSlice carries persona + ui) OR an automation (AutomationSlice's emitted_command references this command). When neither holds, the command is unreachable: nothing in the model can cause it to fire. Add a persona+ui to the CommandSlice (human-triggered) or add an AutomationSlice that fires this command (automation-triggered).",
          Command, "", "model"),
        r("COMMAND_MULTIPLE_ISSUERS", Error, "Command issued by more than one UI or processor",
          "A command's trigger must be unambiguous: at most one UI (via CommandSlice) and at most one processor (via AutomationSlice) may point at a given command, and the two MUST NOT coexist. Two distinct UIs, two distinct processors, or a UI plus a processor all leave the model unable to say who fires the command. Split it into distinct commands per issuer, or consolidate the issuers into one.",
          Command, "", "model"),
        r("EVENT_NO_SWIMLANE", Error, "Event has no swimlane",
          "Every event lives on a stream: the swimlane is the aggregate's lifecycle that owns the event. An event with no `swimlane` field, or whose `swimlane` points at a Swimlane that does not exist in the store, has no stream identity: the runtime cannot write it, no aggregate state can be derived from it, and no read model can project it deterministically. Set `Event.swimlane` to a real Swimlane reference.",
          Event, "swimlane", "model"),
        r("UI_NO_PERSONA", Error, "UI has no persona",
          "Every screen belongs to a human. A Ui carries no owner of its own, so the model must pair it with a Persona in one of the three places the schema allows: a UiSlice carrying BOTH persona and ui (the persona watches there), a CommandSlice carrying both (the persona acts there), or a StoryboardEntry.HumanObserver naming both. A screen no persona owns has no actor: the board cannot say whose lane it belongs to and routes it to the `ui:unassigned` catch-all. A display-only screen names its persona on its UiSlice. Also fires when the attributed persona does not resolve in the store.",
          Ui, "", "model"),
        r("UI_SLICE_NO_READ_MODEL", Error, "UI slice displays nothing",
          "A UiSlice is the moment a persona SEES something, so it must name what is on the screen: at least one `source_read_models` entry pointing at the ReadModel the UI renders. A UiSlice with an empty list claims a screen exists in the flow but says nothing arrives on it, which is the display-side equivalent of an AutomationSlice observing no read model. Add the ReadModel the screen renders, or drop the slice if the screen is a pure form (a CommandSlice already covers the acting moment).",
          UiSlice, "source_read_models", "model"),
        r("EVENT_FIELD_UNSOURCED", Error, "Event field has no source",
          "Every field on a CommandSlice's emitted_event must be sourceable from somewhere the model declares: (1) the firing command's own fields, (2) any command on the same swimlane (prior-command-set aggregate state), (3) the swimlane's `state` field, or (4) a `DerivedFieldAnnotation` marking it system-generated. Two events sharing a field name does NOT count as sourcing; name collision is not data flow.",
          CommandSlice, "emitted_events", "model"),
        r("AUTOMATION_COMMAND_FIELD_UNSOURCED", Error, "Automation command field has no source",
          "When an AutomationSlice's processor fires a command, every field on that command must be sourceable from one of the source_read_models the processor observes (the RM is the data carrier), or carry a `DerivedFieldAnnotation` marking it system-generated. A field that lives nowhere upstream is information-incomplete: the model says the automation produces this command but never says where the value comes from.",
          AutomationSlice, "emitted_command", "model"),
        r("READ_MODEL_FIELD_UNSOURCED", Error, "Read model field has no source event",
          "A ReadModel's `fields` list every value the model claims to project. Every field name must be supplied by at least one of the RM's `source_events` (the event has a FieldSpec with that name) OR by the swimlane state of one of those source events' lanes (`Swimlane.state`). Otherwise the RM is lying: the projection cannot populate the field, and any AutomationSlice that consumes the RM is reading a value that doesn't exist. Add the field to a source event, switch source events, declare the field on the source event's lane via swimlane state, or drop it from the RM.",
          ReadModel, "fields", "model"),
        r("COMMAND_FIELD_NEVER_RECORDED", Warning, "Command field never appears in an emitted event",
          "Every field a command carries is data the runtime accepts. If that field does not appear on any of the slice's emitted_events AND does not appear in the command's swimlane.state, the model is silent about what happened to the value: the command took it in, the events do not record it, and the aggregate does not claim it as state. Either add the field to one of the emitted events, declare it on the lane's swimlane.state, or drop it from the command.",
          CommandSlice, "command", "model"),
        r("SLICE_STALE_REF", Warning, "Slice ref pins an older version than the latest loaded",
          "A slice points at a specific (namespace, slug, version) of a command, ui, persona, processor, event, or read model, but a newer version of that entity exists in the loaded model. Mid-flight migration signal (Decision #24): the slice is still wired to the previous incarnation. Bump the ref to the latest version, or attach a PinnedRefAnnotation with a non-empty doc to the slice to justify staying pinned.",
          Unspecified, "", "model"),
        r("UI_USED_AS_INPUT_AND_OUTPUT", Warning, "UI used as both trigger and render target",
          "A UI appears as both the trigger for a CommandSlice AND the render target for a UiSlice. One surface in both roles is usually a modeling mistake: split the form from the display, or attach a MixedSurfaceAnnotation explaining why.",
          Ui, "", "model"),
        r("TRANSITION_FOREIGN_EVENT", Error, "Swimlane transition references a foreign event",
          "A swimlane's lifecycle transitions reference an event that is not assigned to this lane. A lifecycle only contains its own stream's events.",
          Swimlane, "transitions", "model"),
        r("STREAM_ID_MALFORMED_PLACEHOLDER", Error, "Swimlane stream_id placeholder is an expression",
          "A {placeholder} in the swimlane's stream_id is not a plain name. Register the derived identity in a Uuidv5IdentityAnnotation with a name and reference that name.",
          Swimlane, "stream_id", "model"),
        r("STREAM_ID_UNRESOLVED_PLACEHOLDER", Warning, "Swimlane stream_id placeholder unresolved",
          "A {placeholder} matches no FieldSpec on this lane's events/commands and no named identity annotation. The stream identity must be followable from the data.",
          Swimlane, "stream_id", "model"),
        r("UI_TRANSITION_MISSING_REF", Error, "UI transition missing target",
          "A UI transition entry has no `to` reference.",
          Ui, "transitions", "model"),
        r("UI_TRANSITION_DANGLING", Error, "UI transition target not in store",
          "A UI transition points at a UI that does not exist.",
          Ui, "transitions", "model"),
        r("UI_TRANSITION_DERIVABLE", Warning, "UI transition duplicates a storyboard hop",
          "A UI declares a transition that a storyboard command path already explains. Derived navigation must not be re-declared: drop the transition, or document why the pure-navigation link exists separately.",
          Ui, "transitions", "model"),
        r("DANGLING_REF", Error, "Model carries a dangling reference",
          "An entity reached transitively from this model references something that does not exist in the store.",
          Unspecified, "", "model"),
        r("CROSS_MODEL_BOUNDARY_VIOLATION", Error, "Member belongs to another model",
          "A member of this event model is owned by another event model and no seam is declared between them. Members must respect model ownership.",
          Unspecified, "", "model"),
        r("CROSS_MODEL_REF", Info, "Cross-model reference into declared seam",
          "This model references an entity inside another model's boundary, but the seam between them is declared. Legal, surfaced for situational awareness.",
          Unspecified, "", "model"),
        r("EVENT_NOT_PROJECTED_IN_MODEL", Warning, "Event not projected by any read model in this model",
          "An event included in the model is not sourced by any read model that this model also includes.",
          Event, "source_events", "model"),
        r("RM_EVENT_NOT_PROJECTED", Error, "Read model declares an event nothing projects",
          "A read model lists an event in source_events that no slice in the model's storyboards actually projects.",
          ReadModel, "source_events", "model"),
        r("RM_SOURCE_EVENT_FIELD_NOT_PROJECTED", Error, "Read model is missing a field its wired source event provides",
          "A ReadModelSlice targeting this read model actually consumes a source event, but the read model's `fields` list is missing a field that event carries (via the event's own FieldSpecs or its swimlane's `state` fields). The projection has a wired path from the event's data but the RM never claims the field, so the value is silently dropped. Add the missing field(s) to the read model's `fields` list, or drop the event from `source_events` if the field is not actually needed.",
          ReadModel, "fields", "model"),
        r("ORPHAN_NOT_FLAGGED", Warning, "Entity exists but no slice or storyboard references it",
          "An entity is loaded with the model but is referenced by no slice or storyboard and is not marked as deliberately standalone.",
          Unspecified, "", "model"),
        r("ORPHAN_DOC_EMPTY", Error, "Orphan-flagged entity has no doc",
          "An entity carries an orphan-acknowledgement marker but no explanation. Document why this entity exists outside the slice graph.",
          Unspecified, "", "model"),
        r("ORPHAN_FLAG_OBSOLETE", Info, "Orphan flag no longer applies",
          "An entity is marked as an orphan but is now referenced by a slice or storyboard; the flag can be removed.",
          Unspecified, "", "model"),
        r("TRACKER_DUPLICATE_SUBJECT", Warning, "Tracker subject appears in multiple trackers",
          "Two or more Tracker entities claim the same subject; consolidate so each subject has one tracker.",
          Tracker, "subject", "model"),
        r("TRACKER_MISSING_SUBJECT", Error, "Tracker has no subject",
          "Every Tracker must name the entity it tracks.",
          Tracker, "subject", "model"),
        // ----- storyboard checks (also dispatched by validate_model) -------------
        r("STORYBOARD_MISSING_ENTRY", Warning, "Storyboard declares no entry",
          "A storyboard must say which read model is observed and by whom (human persona+ui or automation processor).",
          Storyboard, "entry", "storyboard"),
        r("STORYBOARD_MISSING_OUTCOME", Warning, "Storyboard declares no outcome events",
          "Name the terminal facts so later storyboards can chain from them.",
          Storyboard, "outcome", "storyboard"),
        r("STORYBOARD_SHARED_SLICE", Warning, "Slice claimed by multiple storyboards",
          "A slice belongs to one storyboard moment; claiming it from several makes ordering ambiguous (tools render the first claim).",
          Storyboard, "slices", "storyboard"),
        r("STORYBOARD_SLICE_NOT_MEMBER", Warning, "Storyboard references a slice not in the model",
          "A storyboard's slice list points at a slice the EventModel does not include as a member.",
          Storyboard, "slices", "storyboard"),
        r("SLICE_NOT_IN_STORYBOARD", Warning, "Slice not anchored in any storyboard",
          "A slice the model includes is not placed into any storyboard moment; it has no narrative position.",
          Unspecified, "", "storyboard"),
        r("PROJECTION_BEFORE_EMITTER", Error, "Read model slice consumes an event before it is emitted",
          "Within a storyboard, a read model slice cannot project an event whose emitting command slice comes later (or never). Move the slice after the emitter or drop the event.",
          ReadModelSlice, "source_events", "storyboard"),
        r("PROJECTION_NOT_ADJACENT", Error, "Projection separated from its emitter by an action slice",
          "A read model slice projects an event, but one or more command/automation slices run between the emitter and the projection. Project each event into its read models before the next command or automation acts.",
          ReadModelSlice, "source_events", "storyboard"),
        r("RM_SLICE_EVENT_NOT_DECLARED", Error, "Read model slice consumes event not declared on the read model",
          "A ReadModelSlice cites an event in source_events that the ReadModel itself does not declare it consumes.",
          ReadModelSlice, "source_events", "storyboard"),
        // ----- strategic (Domain / Subdomain / BoundedContext) -------------------
        r("CONTEXT_SLUG_MISMATCH", Error, "Bounded context slug differs from namespace",
          "Decision #27: a BoundedContext's slug must equal its namespace; this guarantees context ids are globally unique by namespace alone.",
          BoundedContext, "id", "strategic"),
        r("DOMAIN_SLUG_MISMATCH", Error, "Domain slug differs from namespace",
          "A Domain's slug must equal its namespace: the namespace IS the domain identity.",
          Domain, "id", "strategic"),
        r("DOMAIN_HAS_NO_CORE", Info, "Domain has no CORE subdomain",
          "A domain without any CORE subdomain is unusual; CORE is what differentiates the system. Audit whether something here should be classified CORE.",
          Domain, "", "strategic"),
        r("REALIZES_MISSING_REF", Error, "Bounded context `realizes` entry missing id",
          "Every BoundedContext.realizes entry must point at a Subdomain.",
          BoundedContext, "realizes", "strategic"),
        r("REALIZES_DANGLING", Error, "Bounded context realizes a Subdomain that does not exist",
          "The subdomain referenced by BoundedContext.realizes is not in the store.",
          BoundedContext, "realizes", "strategic"),
        r("REALIZES_MULTIPLE", Info, "Bounded context realizes multiple subdomains",
          "A BoundedContext realizing more than one subdomain is allowed but unusual; surfaced for awareness.",
          BoundedContext, "realizes", "strategic"),
        r("SUBDOMAIN_MISSING_DOMAIN", Warning, "Subdomain has no parent domain",
          "Every Subdomain should declare which Domain it belongs to.",
          Subdomain, "domain", "strategic"),
        r("SUBDOMAIN_FOREIGN_DOMAIN", Error, "Subdomain.domain.namespace differs from subdomain namespace",
          "A Subdomain in namespace N must belong to a Domain whose namespace is also N. Cross-namespace ownership is not allowed.",
          Subdomain, "domain", "strategic"),
        r("SUBDOMAIN_DANGLING_DOMAIN", Error, "Subdomain references a Domain that does not exist",
          "The Domain referenced by Subdomain.domain is not in the store.",
          Subdomain, "domain", "strategic"),
        r("SUBDOMAIN_UNCLASSIFIED", Warning, "Subdomain has no classification",
          "Every Subdomain should be classified CORE / SUPPORTING / GENERIC so the strategic intent is explicit.",
          Subdomain, "classification", "strategic"),
        r("SUBDOMAIN_UNREALIZED", Info, "Subdomain is not realized by any bounded context",
          "A subdomain that no BoundedContext realizes is dormant; either delete it or add a context.",
          Subdomain, "realizes", "strategic"),
        r("RELATIONSHIP_MISSING_REF", Error, "Context relationship entry missing upstream id",
          "Every BoundedContext.relationships entry must point at an upstream BoundedContext.",
          BoundedContext, "relationships", "strategic"),
        r("RELATIONSHIP_DANGLING", Error, "Context relationship upstream does not exist",
          "The upstream BoundedContext named in a relationship is not in the store.",
          BoundedContext, "relationships", "strategic"),
        r("RELATIONSHIP_SELF", Error, "Bounded context declares a relationship with itself",
          "A relationship's upstream and downstream cannot be the same BoundedContext.",
          BoundedContext, "relationships", "strategic"),
        r("RELATIONSHIP_UNSPECIFIED_INTENT", Warning, "Context relationship has no intent",
          "Each relationship should declare an intent (CUSTOMER_SUPPLIER / CONFORMIST / PARTNERSHIP / SEPARATE_WAYS / ...). Unspecified intent leaves the strategic choice undocumented.",
          BoundedContext, "relationships", "strategic"),
        r("RELATIONSHIP_CONTRADICTS_SEAM", Error, "Relationship contradicts the declared seam",
          "The relationship's intent contradicts the underlying read-model seam (e.g. CONFORMIST while seam expects CUSTOMER_SUPPLIER). Reconcile the structural seam and the intent.",
          BoundedContext, "relationships", "strategic"),
        r("RELATIONSHIP_WITHOUT_SEAM", Warning, "Relationship declared without a structural seam",
          "A relationship is declared but no subscription read model between the two contexts backs it. Declare the seam (an RM in the downstream context whose source_events live in the upstream).",
          BoundedContext, "relationships", "strategic"),
        r("SEAM_WITHOUT_RELATIONSHIP", Warning, "Structural seam present without relationship intent",
          "A downstream context subscribes to an upstream event but no ContextRelationship records the strategic intent. Declare one (Decision #30).",
          BoundedContext, "seams", "strategic"),
        // ----- project (multi-project hygiene) -----------------------------------
        r("PROJECT_SLUG_MISMATCH", Error, "Project slug differs from namespace",
          "A Project's slug must equal its namespace: the namespace IS the project identity (same rule as Domain).",
          Project, "id", "strategic"),
        r("DOMAIN_MISSING_PROJECT", Warning, "Domain does not declare its project",
          "Decision: in a multi-project store every Domain should point at the Project (codebase / repository / customer) it was mapped from. Set Domain.project.id so cross-project ownership is unambiguous.",
          Domain, "project", "strategic"),
        r("BC_PROJECT_COLLISION", Error, "Bounded contexts from different projects share an id",
          "Two BoundedContext entities live under the same (namespace, slug) but their realized subdomains trace back to different Projects. The newer write silently overwrote the older one. Add a Project to each Domain so the ownership is explicit, and either rename one BC or move it under a project-specific namespace.",
          BoundedContext, "id", "strategic"),
        r("BC_REALIZES_NOTHING", Info, "Bounded context realizes no subdomain",
          "A BoundedContext exists in the knowledge graph but its `realizes` list is empty: it is not mapped to any Subdomain, so the investment classification (core/supporting/generic) has no opinion about it. Map it to at least one subdomain (the BC's purpose in problem-space), or delete the BC if it no longer earns its keep. Silence is not a strategy.",
          BoundedContext, "realizes", "strategic"),
        // ----- composition (Decision #35) ----------------------------------------
        r("SCREEN_NAMESPACE_IS_CONTEXT", Error, "Screen lives inside a bounded-context namespace",
          "A Screen is application composition metadata, not a bounded-context model element. Put Screen entities in a non-context application namespace.",
          Screen, "id", "composition"),
        r("SLOT_UNFILLED", Warning, "Required screen slot has no contributing UI",
          "A required Screen slot has no UI declaring UI.slot for that screen and slot name.",
          Screen, "slots", "composition"),
        r("SCREEN_MANY_CONTRIBUTORS", Info, "Screen composes UI from multiple contexts",
          "A Screen receives UI contributions from more than one namespace. This is legal and intentional composition, surfaced for awareness.",
          Screen, "slots", "composition"),
        // ----- language (Term / Ambiguity) ---------------------------------------
        r("TERM_INVALID_ENTITY_KIND", Error, "Term embodies an invalid entity kind",
          "Term.embodied_by must point at a concrete entity kind.",
          Term, "embodied_by", "language"),
        r("TERM_DANGLING_ENTITY", Error, "Term embodies an entity that does not exist",
          "A Term.embodied_by reference points at an entity that is missing from the store.",
          Term, "embodied_by", "language"),
        r("AMBIGUITY_UNSPECIFIED_KIND", Error, "Ambiguity has no kind",
          "An Ambiguity must declare whether it is a HOMONYM or SYNONYM ruling.",
          Ambiguity, "kind", "language"),
        r("AMBIGUITY_TOO_FEW_TERMS", Error, "Ambiguity has fewer than two terms",
          "An Ambiguity is a ruling across terms, so it must name at least two Term participants.",
          Ambiguity, "terms", "language"),
        r("AMBIGUITY_DANGLING_TERM", Error, "Ambiguity references a missing term",
          "An Ambiguity.terms entry points at a Term that is missing from the store.",
          Ambiguity, "terms", "language"),
        r("AMBIGUITY_UNSPECIFIED_RULING", Error, "Ambiguity has no ruling",
          "An Ambiguity entity is the ruling. It must declare DELIBERATE, RENAME, or MERGE rather than storing unresolved work.",
          Ambiguity, "ruling", "language"),
        // ----- read model key and projection role --------------------------------
        r("RM_KEY_NOT_A_FIELD", Error, "Read model key name is not a declared field",
          "Every name in ReadModel.key must match a FieldSpec name in ReadModel.fields. A key that names a field the read model does not declare cannot identify a row.",
          ReadModel, "key", "model"),
        r("RM_SLICE_KEY_UNSOURCED", Error, "Read model slice source event does not carry a key field",
          "When the target read model declares a key, every event in a ReadModelSlice's source_events must carry FieldSpec entries for ALL key fields. A slice projecting an event that is missing a key field cannot locate the row it writes or deletes.",
          ReadModelSlice, "source_events", "model"),
        r("RM_KEY_MISSING", Info, "Read model has fields but no key declared",
          "A read model with fields but an empty key list has no declared row identity. Surfacing as Info during the migration window. Candidate for Warning when the store is fully migrated.",
          ReadModel, "key", "model"),
        r("RM_SLICE_ROLE_MISSING", Info, "Read model slice has no projection role",
          "A ReadModelSlice targeting a keyed read model has PROJECTION_ROLE_UNSPECIFIED. Declare PROJECTION_ROLE_UPSERT or PROJECTION_ROLE_DELETE so the projection intent is explicit. Surfacing as Info during the migration window.",
          ReadModelSlice, "projection_role", "model"),
        // ----- schema (Decision #33) ---------------------------------------------
        r("SCHEMA_TYPE_URL_EMPTY", Error, "Schema Any has no type_url",
          "An Any-wrapped Schema must carry its type_url so consumers can deserialize it.",
          Unspecified, "schema", "schema"),
        // ----- personal data (PersonalDataAnnotation) -----------------------------
        r("PII_DOC_MISSING", Error, "PersonalDataAnnotation has no doc, or is malformed",
          "A PersonalDataAnnotation's `doc` is empty, or the Any payload cannot be decoded as a PersonalDataAnnotation at all. Either way the annotation says nothing: explain what personal data this is and whose.",
          Unspecified, "schema.fields", "schema"),
        r("PII_EVENT_ERASURE_UNSPECIFIED", Warning, "Personal-data event field has no erasure strategy",
          "A PersonalDataAnnotation on an Event field leaves `erasure` at ERASURE_UNSPECIFIED. Events are immutable, so a personal-data field needs a declared way to make the value unrecoverable when the person asks to be forgotten. Commands are not persisted to an event store, so this rule is Event-only.",
          Event, "schema.fields", "schema"),
        r("PII_ERASURE_NOT_APPLICABLE", Error, "ERASURE_PROJECTION_DELETE on a non-ReadModel field",
          "ERASURE_PROJECTION_DELETE means the row is deleted or overwritten on erasure, which only applies to a ReadModel. An Event or Command field has no row to delete; pick a different Erasure.",
          Unspecified, "schema.fields", "schema"),
        r("PII_SUBJECT_FIELD_UNKNOWN", Error, "subject_field names no sibling field",
          "A PersonalDataAnnotation's `subject_field` must name a field declared in the same field list at the same nesting level. A stale or misspelled name leaves the per-subject key or the row to erase undeclared.",
          Unspecified, "schema.fields", "schema"),
        r("PII_FLOW_UNMARKED", Warning, "Downstream field sourced from personal data has no PersonalDataAnnotation",
          "The data-flow inference layer (InferDataFlow's exact-name FromField match) traces this field to a personal-data field upstream, but the field itself carries no PersonalDataAnnotation. Erasing the upstream field would silently leave an unmarked copy downstream; annotate the field or correct where it actually comes from.",
          Unspecified, "schema.fields", "schema"),
        // ----- reliability (SLI / SLO / AlertPolicy) ------------------------------
        r("SLI_CONNECTION_UNRESOLVED", Error, "Indicator connection does not resolve",
          "A ServiceLevelIndicator's connection points at a slice that is missing or is the wrong slice kind for the Connection variant, names events not on that slice's edges (CommandHandling.events must be a subset of the command slice's emitted_events; Projection.source_events must be a subset of the read model slice's source_events), or names an Integration system the processor does not list in calls. The measurement has nowhere real to attach.",
          ServiceLevelIndicator, "connection", "reliability"),
        r("SLI_JOURNEY_UNREACHABLE", Error, "Journey end is not reachable from its start",
          "A Journey connection's end event cannot be reached from its start event within its scope (one storyboard, or the pooled slices of an EventModel's member storyboards). Reachability is structural, not storyboard order: an edge exists from a source event to a command's emitted events when a read model slice projects the source event and either an automation slice observes that read model and emits the command (a Reaction chain) or a UI slice renders that read model for a persona who then issues the command from the same ui (a human handoff on one screen). This deliberately does not model a persona navigating to a different screen first, so a journey that only a cross-screen handoff can complete is conservatively reported as unreachable even though a human could still complete it by hand; narrow the journey or add the missing ui-to-slice pairing to clear the finding.",
          ServiceLevelIndicator, "connection.journey", "reliability"),
        r("SLI_CORRELATION_UNSOURCED", Error, "Journey correlate_on field missing from an endpoint event",
          "A Journey connection's correlate_on names a field absent from the start event's or end event's `schema` fields. Without the field on both ends the two cannot be matched into one instance.",
          ServiceLevelIndicator, "connection.journey.correlate_on", "reliability"),
        r("SLI_OUTCOME_UNREACHABLE", Error, "OutcomeRatio event cannot terminate the connection",
          "An OutcomeRatio measure's good or bad event is not among the events the connection can end on: for CommandHandling, the command slice's emitted_events; for Journey, the end event or any event reachable from the same start by SLI_JOURNEY_UNREACHABLE's reachability rule. Also fires when good or bad is empty, since a ratio with no numerator or no denominator side measures nothing. Connection kinds with no defined notion of termination (Projection, Reaction, Display, Integration) are not checked.",
          ServiceLevelIndicator, "measure.outcome_ratio", "reliability"),
        r("SLI_SIGNAL_MISMATCH", Error, "Signal cannot measure this connection or indicator",
          "An EventTimestamps signal needs an event marking the end of the span; Display and Integration connections render a screen or call outward with no such event, so EventTimestamps cannot source them. Also fires when the indicator has no signal or no measure at all.",
          ServiceLevelIndicator, "signal", "reliability"),
        r("SLO_THRESHOLD_MISMATCH", Error, "Objective threshold does not match its indicator's measure",
          "A ServiceLevelObjective's Objective must carry a threshold when its indicator measures Latency (the threshold says how slow is too slow) and must not carry one otherwise, since a ratio or completion measure has nothing to threshold.",
          ServiceLevelObjective, "objectives", "reliability"),
        r("SLO_TARGET_RANGE", Error, "Objective target is not a fraction between 0 and 1",
          "An Objective's target or time_slice_target ratio must be strictly between 0 and 1; 0 commits to nothing and 1 (or above) commits to perfection, neither of which is a budget.",
          ServiceLevelObjective, "objectives", "reliability"),
        r("SLO_EXTERNAL_UNALERTED", Warning, "Externally committed objective has no alert policy",
          "A ServiceLevelObjective with an External commitment (what a counterparty can hold the team to) has no alert_policies watching it, so nothing pages before the commitment is breached.",
          ServiceLevelObjective, "alert_policies", "reliability"),
        r("SLO_UNALERTED", Info, "Objective has no alert policy",
          "A ServiceLevelObjective with no commitment or an Internal one has no alert_policies watching it. Informational, and separate from SLO_EXTERNAL_UNALERTED, so an externally committed objective is not double-reported by both rules.",
          ServiceLevelObjective, "alert_policies", "reliability"),
        // ----- scenarios (validate_scenarios) ------------------------------------
        r("SCENARIO_MISSING_REF", Error, "Scenario step missing entity reference",
          "Each Given/When/Then step must point at a concrete entity (event / command / read model).",
          Unspecified, "", "scenarios"),
        r("SCENARIO_DANGLING_REF", Error, "Scenario step references missing entity",
          "A Given/When/Then step points at an entity that is not in the store.",
          Unspecified, "", "scenarios"),
        r("SCENARIO_DUPLICATE_TITLE", Error, "Two scenarios share the same title",
          "Scenario titles must be unique within their owning command/read model.",
          Unspecified, "title", "scenarios"),
        r("SCENARIO_COMMAND_MISMATCH", Error, "Scenario's When command differs from the parent",
          "A CommandScenario's When step must reference the same command the scenario is attached to.",
          Command, "when", "scenarios"),
        r("SCENARIO_READ_MODEL_MISMATCH", Error, "Scenario's Then read model differs from the parent",
          "A ReadModelScenario's Then step must reference the same read model the scenario is attached to.",
          ReadModel, "then", "scenarios"),
        r("SCENARIO_MISSING_THEN", Error, "Scenario has no Then clause",
          "Every scenario must declare its outcome (events for command scenarios, projected state for read-model scenarios).",
          Unspecified, "then", "scenarios"),
        r("SCENARIO_REJECT_NO_REASON", Error, "Rejected scenario has no reason",
          "A scenario whose outcome is `reject` must record the rejection reason.",
          Unspecified, "then", "scenarios"),
        r("SCENARIO_TRANSITION_VIOLATION", Warning, "Scenario Then events break the swimlane lifecycle",
          "The events a scenario's Then claims will result do not match the swimlane's declared lifecycle transitions from the Given state.",
          Unspecified, "then", "scenarios"),
    ]
}

#[cfg(test)]
mod catalog_drift_tests {
    use std::collections::HashSet;

    use super::*;

    // Lifted from a `grep -oE` of this file. If you add an emission code
    // OR remove one, update both the catalog AND this list so the diff
    // test below can guard against drift.
    const EMITTED_CODES: &[&str] = &[
        "AMBIGUITY_DANGLING_TERM",
        "AMBIGUITY_TOO_FEW_TERMS",
        "AMBIGUITY_UNSPECIFIED_KIND",
        "AMBIGUITY_UNSPECIFIED_RULING",
        "ASSUMPTION_PRESENT",
        "AUTOMATION_COMMAND_FIELD_UNSOURCED",
        "BC_PROJECT_COLLISION",
        "BC_REALIZES_NOTHING",
        "CODE_REF_INCOMPLETE",
        "COMMAND_FIELD_NEVER_RECORDED",
        "COMMAND_HAS_NO_TRIGGER",
        "COMMAND_MULTIPLE_ISSUERS",
        "COMMAND_MULTI_STREAM_WRITE",
        "COMMAND_NO_EMITTED_EVENTS",
        "CONTEXT_SLUG_MISMATCH",
        "CROSS_MODEL_BOUNDARY_VIOLATION",
        "CROSS_MODEL_REF",
        "DANGLING_REF",
        "DOMAIN_HAS_NO_CORE",
        "DOMAIN_MISSING_PROJECT",
        "DOMAIN_SLUG_MISMATCH",
        "EVENT_FIELD_UNSOURCED",
        "EVENT_MULTIPLE_EMITTERS",
        "EVENT_NOT_PROJECTED_IN_MODEL",
        "EVENT_NO_SWIMLANE",
        "EVENT_UNOBSERVED",
        "INVALID_KIND",
        "MEMBER_NOT_FOUND",
        "MISSING_ID",
        "OPEN_QUESTION_PRESENT",
        "ORPHAN_DOC_EMPTY",
        "ORPHAN_FLAG_OBSOLETE",
        "ORPHAN_NOT_FLAGGED",
        "PII_DOC_MISSING",
        "PII_ERASURE_NOT_APPLICABLE",
        "PII_EVENT_ERASURE_UNSPECIFIED",
        "PII_FLOW_UNMARKED",
        "PII_SUBJECT_FIELD_UNKNOWN",
        "PROCESSOR_MULTIPLE_COMMANDS",
        "PROJECT_SLUG_MISMATCH",
        "PROJECTION_BEFORE_EMITTER",
        "PROJECTION_NOT_ADJACENT",
        "READ_MODEL_FIELD_UNSOURCED",
        "READ_MODEL_MULTIPLE_AUTOMATIONS",
        "READ_MODEL_SHARED_CONSUMERS",
        "READ_MODEL_SHARED_CONSUMERS_DOC_EMPTY",
        "REALIZES_DANGLING",
        "REALIZES_MISSING_REF",
        "REALIZES_MULTIPLE",
        "RELATIONSHIP_CONTRADICTS_SEAM",
        "RELATIONSHIP_DANGLING",
        "RELATIONSHIP_MISSING_REF",
        "RELATIONSHIP_SELF",
        "RELATIONSHIP_UNSPECIFIED_INTENT",
        "RELATIONSHIP_WITHOUT_SEAM",
        "RM_EVENT_NOT_PROJECTED",
        "RM_KEY_MISSING",
        "RM_KEY_NOT_A_FIELD",
        "RM_SLICE_EVENT_NOT_DECLARED",
        "RM_SLICE_KEY_UNSOURCED",
        "RM_SLICE_ROLE_MISSING",
        "RM_SOURCE_EVENT_FIELD_NOT_PROJECTED",
        "SCENARIO_COMMAND_MISMATCH",
        "SCENARIO_DANGLING_REF",
        "SCENARIO_DUPLICATE_TITLE",
        "SCENARIO_MISSING_REF",
        "SCENARIO_MISSING_THEN",
        "SCENARIO_READ_MODEL_MISMATCH",
        "SCENARIO_REJECT_NO_REASON",
        "SCENARIO_TRANSITION_VIOLATION",
        "SCHEMA_TYPE_URL_EMPTY",
        "SCREEN_MANY_CONTRIBUTORS",
        "SCREEN_NAMESPACE_IS_CONTEXT",
        "SEAM_WITHOUT_RELATIONSHIP",
        "SLICE_NOT_IN_STORYBOARD",
        "SLICE_STALE_REF",
        "SLI_CONNECTION_UNRESOLVED",
        "SLI_CORRELATION_UNSOURCED",
        "SLI_JOURNEY_UNREACHABLE",
        "SLI_OUTCOME_UNREACHABLE",
        "SLI_SIGNAL_MISMATCH",
        "SLOT_UNFILLED",
        "SLO_EXTERNAL_UNALERTED",
        "SLO_TARGET_RANGE",
        "SLO_THRESHOLD_MISMATCH",
        "SLO_UNALERTED",
        "STORYBOARD_MISSING_ENTRY",
        "STORYBOARD_MISSING_OUTCOME",
        "STORYBOARD_SHARED_SLICE",
        "STORYBOARD_SLICE_NOT_MEMBER",
        "STREAM_ID_MALFORMED_PLACEHOLDER",
        "STREAM_ID_UNRESOLVED_PLACEHOLDER",
        "SUBDOMAIN_DANGLING_DOMAIN",
        "SUBDOMAIN_FOREIGN_DOMAIN",
        "SUBDOMAIN_MISSING_DOMAIN",
        "SUBDOMAIN_UNCLASSIFIED",
        "SUBDOMAIN_UNREALIZED",
        "TERM_DANGLING_ENTITY",
        "TERM_INVALID_ENTITY_KIND",
        "TRACKER_DUPLICATE_SUBJECT",
        "TRACKER_MISSING_SUBJECT",
        "TRANSITION_FOREIGN_EVENT",
        "UI_NO_PERSONA",
        "UI_SLICE_NO_READ_MODEL",
        "UI_TRANSITION_DANGLING",
        "UI_TRANSITION_DERIVABLE",
        "UI_TRANSITION_MISSING_REF",
        "UI_USED_AS_INPUT_AND_OUTPUT",
    ];

    #[test]
    fn entity_kind_catalog_covers_every_enum_variant() {
        // Every EntityKind in the proto enum (except UNSPECIFIED, which
        // is the sentinel) must appear in the entity_kind_catalog().
        let want = [
            pb::EntityKind::Event,
            pb::EntityKind::Command,
            pb::EntityKind::ReadModel,
            pb::EntityKind::Processor,
            pb::EntityKind::Ui,
            pb::EntityKind::Persona,
            pb::EntityKind::Swimlane,
            pb::EntityKind::CommandSlice,
            pb::EntityKind::ReadModelSlice,
            pb::EntityKind::AutomationSlice,
            pb::EntityKind::Storyboard,
            pb::EntityKind::EventModel,
            pb::EntityKind::Component,
            pb::EntityKind::ExternalSystem,
            pb::EntityKind::Tracker,
            pb::EntityKind::BoundedContext,
            pb::EntityKind::Domain,
            pb::EntityKind::Subdomain,
            pb::EntityKind::Schema,
            pb::EntityKind::Project,
            pb::EntityKind::Screen,
            pb::EntityKind::Term,
            pb::EntityKind::Ambiguity,
            pb::EntityKind::ServiceLevelIndicator,
            pb::EntityKind::ServiceLevelObjective,
            pb::EntityKind::AlertPolicy,
            pb::EntityKind::AlertNotificationTarget,
            pb::EntityKind::TypeLibrary,
        ];
        let got: HashSet<i32> = entity_kind_catalog().iter().map(|i| i.kind).collect();
        let missing: Vec<i32> = want
            .iter()
            .map(|k| *k as i32)
            .filter(|i| !got.contains(i))
            .collect();
        assert!(
            missing.is_empty(),
            "entity_kind_catalog() is missing {missing:?}"
        );
    }

    #[test]
    fn catalog_covers_every_emitted_code_and_vice_versa() {
        // Bind the catalog so the &str borrows below outlive the
        // expression; an inline `rule_catalog().iter()...` would drop
        // the Vec at the end of the statement and leak refs.
        let rules = rule_catalog();
        let catalog: HashSet<&str> = rules.iter().map(|r| r.code.as_str()).collect();
        let emitted: HashSet<&str> = EMITTED_CODES.iter().copied().collect();
        let missing_in_catalog: Vec<&&str> = emitted.difference(&catalog).collect();
        let extra_in_catalog: Vec<&&str> = catalog.difference(&emitted).collect();
        assert!(
            missing_in_catalog.is_empty(),
            "validation rule catalog is missing codes emitted by validation.rs: {missing_in_catalog:?}"
        );
        assert!(
            extra_in_catalog.is_empty(),
            "validation rule catalog contains codes not emitted anywhere: {extra_in_catalog:?}"
        );
    }
}

fn issue_error(
    code: &str,
    message: String,
    subject: Option<pb::EntityRef>,
    field: &str,
) -> pb::ValidationIssue {
    pb::ValidationIssue {
        severity: pb::validation_issue::Severity::Error as i32,
        code: code.into(),
        message,
        subject,
        subject_field: field.into(),
        ..Default::default()
    }
}

fn slice_ref(entity: &pb::Entity) -> Option<pb::EntityRef> {
    let kind = entity_kind(entity)?;
    let id = entity_id(entity)?.clone();
    Some(pb::EntityRef {
        kind: kind as i32,
        id: Some(id),
    })
}

fn check_ref(
    kind: pb::EntityKind,
    id_opt: Option<&pb::Id>,
    field: &str,
    subject: Option<pb::EntityRef>,
    known: &HashSet<pb::EntityKey>,
    issues: &mut Vec<pb::ValidationIssue>,
) {
    let Some(id) = id_opt else {
        issues.push(issue_error(
            "SCENARIO_MISSING_REF",
            format!("{field} is required"),
            subject,
            field,
        ));
        return;
    };
    let key = pb::EntityKey::new(kind, id);
    if !known.contains(&key) {
        issues.push(issue_error(
            "SCENARIO_DANGLING_REF",
            format!(
                "{} -> {}:{}/{}@{} not found in store",
                field,
                pb::canonical::kind_short(kind),
                id.namespace,
                id.slug,
                id.version
            ),
            subject,
            field,
        ));
    }
}

#[allow(clippy::needless_pass_by_value)]
fn check_unique_titles<I, S>(
    titles: I,
    kind_label: &str,
    subject: Option<pb::EntityRef>,
    issues: &mut Vec<pb::ValidationIssue>,
) where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut seen: HashMap<String, usize> = HashMap::new();
    for t in titles {
        let s = t.as_ref().trim().to_string();
        if s.is_empty() {
            continue;
        }
        *seen.entry(s).or_insert(0) += 1;
    }
    for (title, count) in seen {
        if count > 1 {
            issues.push(issue_error(
                "SCENARIO_DUPLICATE_TITLE",
                format!(
                    "{kind_label} scenario title {title:?} appears {count} times in the same slice"
                ),
                subject.clone(),
                "scenarios[].title",
            ));
        }
    }
}

// Surfaces every OpenQuestionAnnotation and AssumptionAnnotation found in
// `metadata` as an Info finding. Both are purely informational by design:
// an open question names something nobody has answered yet, an assumption
// names something nobody has confirmed yet, and either is a prompt for a
// human to look, never a blocker. Shared by the entity-level walk in
// validate_model and the per-scenario walk in validate_scenarios, since
// CommandScenario/ReadModelScenario/AutomationScenario carry their own
// metadata independent of the entity that owns them.
fn push_annotation_findings(
    metadata: &[prost_types::Any],
    subject: Option<&pb::EntityRef>,
    subject_field: &str,
    label: &str,
    issues: &mut Vec<pb::ValidationIssue>,
) {
    for any in metadata {
        if any
            .type_url
            .ends_with("trogonatlas.annotation.v1alpha1.OpenQuestionAnnotation")
        {
            if let Ok(a) =
                <pb::OpenQuestionAnnotation as prost::Message>::decode(any.value.as_slice())
            {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Info as i32,
                    code: "OPEN_QUESTION_PRESENT".into(),
                    message: format!(
                        "{label} has an open question ({}): {}",
                        a.author, a.question
                    ),
                    subject: subject.cloned(),
                    subject_field: subject_field.into(),
                    ..Default::default()
                });
            }
        } else if any
            .type_url
            .ends_with("trogonatlas.annotation.v1alpha1.AssumptionAnnotation")
        {
            if let Ok(a) =
                <pb::AssumptionAnnotation as prost::Message>::decode(any.value.as_slice())
            {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Info as i32,
                    code: "ASSUMPTION_PRESENT".into(),
                    message: format!(
                        "{label} carries an assumption ({}): {}",
                        a.author, a.statement
                    ),
                    subject: subject.cloned(),
                    subject_field: subject_field.into(),
                    ..Default::default()
                });
            }
        }
    }
}

fn is_derived_field(field: &pb::FieldSpec) -> bool {
    // Doc is mandatory on DerivedFieldAnnotation (same contract as
    // PinnedRefAnnotation / SharedConsumersAnnotation). Presence-only
    // matching would let an empty-doc annotation silently suppress
    // EVENT_FIELD_UNSOURCED / AUTOMATION_COMMAND_FIELD_UNSOURCED.
    for any in &field.metadata {
        if !any
            .type_url
            .ends_with("trogonatlas.eventmodel.v1alpha1.DerivedFieldAnnotation")
        {
            continue;
        }
        if let Ok(ann) =
            <pb::DerivedFieldAnnotation as prost::Message>::decode(any.value.as_slice())
        {
            if !ann.doc.is_empty() {
                return true;
            }
        }
    }
    false
}

type DeclaredFields = HashMap<pb::IdKey, Vec<pb::FieldSpec>>;

fn declared_fields_by_id<T>(
    by_id: &HashMap<pb::IdKey, &T>,
    schema: impl Fn(&T) -> Option<&prost_types::Any>,
) -> DeclaredFields {
    by_id
        .iter()
        .map(|(id, x)| {
            (
                id.clone(),
                trogon_atlas_core::schema::schema_fields(schema(x)),
            )
        })
        .collect()
}

fn declared<'a>(fields: &'a DeclaredFields, id: Option<&pb::Id>) -> &'a [pb::FieldSpec] {
    id.and_then(|id| fields.get(&pb::IdKey::new(id)))
        .map_or(&[], Vec::as_slice)
}

/// A FieldSpec reachable from an Event/Command/ReadModel's `schema` field
/// list, including fields nested under ObjectType. `index_path` locates the
/// FieldSpec for ValidationIssue.subject_field (e.g.
/// "schema.fields[0].type.object.fields[1]"); `name_path` is the dotted
/// field name for messages (e.g. "address.street"); `siblings` are the
/// field names declared in the same list at the same nesting level, for
/// checking PersonalDataAnnotation.subject_field references.
struct PersonalDataField<'a> {
    index_path: String,
    name_path: String,
    field: &'a pb::FieldSpec,
    siblings: Vec<String>,
}

fn walk_personal_data_fields<'a>(
    fields: &'a [pb::FieldSpec],
    index_prefix: &str,
    name_prefix: &str,
    out: &mut Vec<PersonalDataField<'a>>,
) {
    let siblings: Vec<String> = fields.iter().map(|f| f.name.clone()).collect();
    for (i, field) in fields.iter().enumerate() {
        let index_path = format!("{index_prefix}[{i}]");
        let name_path = if name_prefix.is_empty() {
            field.name.clone()
        } else {
            format!("{name_prefix}.{}", field.name)
        };
        if let Some(pb::FieldType {
            kind: Some(pb::field_type::Kind::Object(object)),
        }) = field.r#type.as_ref()
        {
            walk_personal_data_fields(
                &object.fields,
                &format!("{index_path}.type.object.fields"),
                &name_path,
                out,
            );
        }
        out.push(PersonalDataField {
            index_path,
            name_path,
            field,
            siblings: siblings.clone(),
        });
    }
}

/// Decodes a `metadata` entry as PersonalDataAnnotation. Returns `None`
/// when the entry is not a PersonalDataAnnotation at all (not our
/// concern), `Some(Err(()))` when the type_url matches but the payload
/// fails to decode (malformed, reported under PII_DOC_MISSING), or
/// `Some(Ok(ann))` on success.
fn decode_personal_data_annotation(
    any: &prost_types::Any,
) -> Option<Result<pb::PersonalDataAnnotation, ()>> {
    if !any
        .type_url
        .ends_with("trogonatlas.eventmodel.v1alpha1.PersonalDataAnnotation")
    {
        return None;
    }
    Some(
        <pb::PersonalDataAnnotation as prost::Message>::decode(any.value.as_slice())
            .map_err(|_| ()),
    )
}

/// Metadata bag on every top-level entity kind. Exhaustive so a new kind
/// that forgets metadata cannot silently skip CODE_REF / annotation walks.
fn entity_metadata(entity: &pb::Entity) -> &[prost_types::Any] {
    match entity.kind.as_ref() {
        Some(pb::entity::Kind::Event(x)) => &x.metadata,
        Some(pb::entity::Kind::Command(x)) => &x.metadata,
        Some(pb::entity::Kind::ReadModel(x)) => &x.metadata,
        Some(pb::entity::Kind::Processor(x)) => &x.metadata,
        Some(pb::entity::Kind::Ui(x)) => &x.metadata,
        Some(pb::entity::Kind::Persona(x)) => &x.metadata,
        Some(pb::entity::Kind::Swimlane(x)) => &x.metadata,
        Some(pb::entity::Kind::CommandSlice(x)) => &x.metadata,
        Some(pb::entity::Kind::ReadModelSlice(x)) => &x.metadata,
        Some(pb::entity::Kind::AutomationSlice(x)) => &x.metadata,
        Some(pb::entity::Kind::UiSlice(x)) => &x.metadata,
        Some(pb::entity::Kind::Storyboard(x)) => &x.metadata,
        Some(pb::entity::Kind::EventModel(x)) => &x.metadata,
        Some(pb::entity::Kind::Component(x)) => &x.metadata,
        Some(pb::entity::Kind::ExternalSystem(x)) => &x.metadata,
        Some(pb::entity::Kind::Tracker(x)) => &x.metadata,
        Some(pb::entity::Kind::BoundedContext(x)) => &x.metadata,
        Some(pb::entity::Kind::Domain(x)) => &x.metadata,
        Some(pb::entity::Kind::Subdomain(x)) => &x.metadata,
        Some(pb::entity::Kind::Schema(x)) => &x.metadata,
        Some(pb::entity::Kind::Project(x)) => &x.metadata,
        Some(pb::entity::Kind::Screen(x)) => &x.metadata,
        Some(pb::entity::Kind::Term(x)) => &x.metadata,
        Some(pb::entity::Kind::Ambiguity(x)) => &x.metadata,
        Some(pb::entity::Kind::ServiceLevelIndicator(x)) => &x.metadata,
        Some(pb::entity::Kind::ServiceLevelObjective(x)) => &x.metadata,
        Some(pb::entity::Kind::AlertPolicy(x)) => &x.metadata,
        Some(pb::entity::Kind::AlertNotificationTarget(x)) => &x.metadata,
        Some(pb::entity::Kind::TypeLibrary(x)) => &x.metadata,
        None => &[],
    }
}

// Store-level hygiene that doesn't fit inside a single EventModel scope:
// project identity (namespace == slug), Domains declaring their project,
// and cross-project collisions on BoundedContext ids. Called from
// ValidateProject. Per-model ValidateEventModel skips this because the
// concern is global; otherwise the same finding would be emitted once
// per model.
pub fn validate_store(all: &[StoredEntity]) -> Vec<pb::ValidationIssue> {
    let mut issues: Vec<pb::ValidationIssue> = Vec::new();
    let known: HashSet<pb::EntityKey> = all
        .iter()
        .filter_map(|se| {
            let kind = entity_kind(&se.entity)?;
            let id = entity_id(&se.entity)?;
            Some(pb::EntityKey::new(kind, id))
        })
        .collect();

    // PROJECT_SLUG_MISMATCH: same rule as Domain.
    for se in all {
        let Some(pb::entity::Kind::Project(p)) = se.entity.kind.as_ref() else {
            continue;
        };
        let Some(id) = p.id.as_ref() else { continue };
        if id.namespace != id.slug {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "PROJECT_SLUG_MISMATCH".into(),
                message: format!(
                    "project {}/{}@{} has slug {} which differs from namespace {}; the namespace IS the project identity",
                    id.namespace, id.slug, id.version, id.slug, id.namespace,
                ),
                subject: Some(pb::EntityRef {
                    kind: pb::EntityKind::Project as i32,
                    id: Some(id.clone()),
                }),
                subject_field: "id".into(),
                ..pb::ValidationIssue::default()
            });
        }
    }

    // DOMAIN_MISSING_PROJECT: every Domain should declare its Project
    // so cross-project ownership is unambiguous. Build a Domain -> Project
    // map at the same time so the BC_PROJECT_COLLISION pass can use it.
    let mut domain_project: HashMap<(String, String), Option<(String, String)>> = HashMap::new();
    for se in all {
        let Some(pb::entity::Kind::Domain(d)) = se.entity.kind.as_ref() else {
            continue;
        };
        let Some(id) = d.id.as_ref() else { continue };
        let proj = d
            .project
            .as_ref()
            .and_then(|r| r.id.as_ref())
            .map(|pid| (pid.namespace.clone(), pid.slug.clone()));
        domain_project.insert((id.namespace.clone(), id.slug.clone()), proj.clone());
        if proj.is_none() {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Warning as i32,
                code: "DOMAIN_MISSING_PROJECT".into(),
                message: format!(
                    "domain {}/{}@{} does not declare its Project; in a multi-project store every Domain should set Domain.project.id so cross-project ownership is explicit",
                    id.namespace, id.slug, id.version,
                ),
                subject: Some(pb::EntityRef {
                    kind: pb::EntityKind::Domain as i32,
                    id: Some(id.clone()),
                }),
                subject_field: "project".into(),
                ..pb::ValidationIssue::default()
            });
        }
    }

    // BC_PROJECT_COLLISION: group BoundedContexts by (namespace, slug)
    // and resolve each to a project via its realizes -> Subdomain ->
    // Domain -> Project chain. If a single (ns, slug) pair traces to
    // multiple distinct projects, the newer write overwrote the older
    // one, so record the collision so the operator can repair.
    // Subdomain id -> Domain id
    let mut subdomain_domain: HashMap<(String, String), (String, String)> = HashMap::new();
    for se in all {
        let Some(pb::entity::Kind::Subdomain(s)) = se.entity.kind.as_ref() else {
            continue;
        };
        let Some(id) = s.id.as_ref() else { continue };
        if let Some(did) = s.domain.as_ref().and_then(|r| r.id.as_ref()) {
            subdomain_domain.insert(
                (id.namespace.clone(), id.slug.clone()),
                (did.namespace.clone(), did.slug.clone()),
            );
        }
    }
    // (BC ns, BC slug) -> set of projects observed
    let mut bc_projects: HashMap<(String, String), HashSet<(String, String)>> = HashMap::new();
    let mut bc_subjects: HashMap<(String, String), pb::Id> = HashMap::new();
    for se in all {
        let Some(pb::entity::Kind::BoundedContext(bc)) = se.entity.kind.as_ref() else {
            continue;
        };
        let Some(id) = bc.id.as_ref() else { continue };
        bc_subjects.insert((id.namespace.clone(), id.slug.clone()), id.clone());
        for r in &bc.realizes {
            let Some(sid) = r.id.as_ref() else { continue };
            let Some(did) = subdomain_domain.get(&(sid.namespace.clone(), sid.slug.clone())) else {
                continue;
            };
            let Some(Some(proj)) = domain_project.get(did) else {
                continue;
            };
            bc_projects
                .entry((id.namespace.clone(), id.slug.clone()))
                .or_default()
                .insert(proj.clone());
        }
    }
    for ((ns, slug), projects) in &bc_projects {
        if projects.len() <= 1 {
            continue;
        }
        let Some(id) = bc_subjects.get(&(ns.clone(), slug.clone())) else {
            continue;
        };
        let mut list: Vec<String> = projects
            .iter()
            .map(|(pn, ps)| format!("{pn}/{ps}"))
            .collect();
        list.sort_unstable();
        issues.push(pb::ValidationIssue {
            severity: pb::validation_issue::Severity::Error as i32,
            code: "BC_PROJECT_COLLISION".into(),
            message: format!(
                "bounded context {}/{}@{} traces back to {} different projects ({}); the newer write silently overwrote the older one: add a Project to each Domain and either rename the BC or move it under a project-scoped namespace",
                id.namespace, id.slug, id.version, projects.len(), list.join(", "),
            ),
            subject: Some(pb::EntityRef {
                kind: pb::EntityKind::BoundedContext as i32,
                id: Some(id.clone()),
            }),
            subject_field: "id".into(),
            ..pb::ValidationIssue::default()
        });
    }

    // BC_REALIZES_NOTHING: a BoundedContext that maps to no Subdomain has no
    // place on the domain chart's classification bands and silently slips out
    // of the investment story. Surface it as Info so the operator either
    // declares the BC's purpose (realizes ≥ 1 subdomain) or removes the BC.
    for se in all {
        let Some(pb::entity::Kind::BoundedContext(bc)) = se.entity.kind.as_ref() else {
            continue;
        };
        let Some(id) = bc.id.as_ref() else { continue };
        if !bc.realizes.is_empty() {
            continue;
        }
        issues.push(pb::ValidationIssue {
            severity: pb::validation_issue::Severity::Info as i32,
            code: "BC_REALIZES_NOTHING".into(),
            message: format!(
                "bounded context {}/{}@{} realizes no subdomain: it is unmapped on the domain chart; declare its purpose by adding a `realizes` entry, or remove the context",
                id.namespace, id.slug, id.version,
            ),
            subject: Some(pb::EntityRef {
                kind: pb::EntityKind::BoundedContext as i32,
                id: Some(id.clone()),
            }),
            subject_field: "realizes".into(),
            ..pb::ValidationIssue::default()
        });
    }

    // SCREEN_NAMESPACE_IS_CONTEXT / SLOT_UNFILLED / SCREEN_MANY_CONTRIBUTORS.
    let context_namespaces: HashSet<String> = all
        .iter()
        .filter_map(|se| {
            let Some(pb::entity::Kind::BoundedContext(bc)) = se.entity.kind.as_ref() else {
                return None;
            };
            bc.id.as_ref().map(|id| id.namespace.clone())
        })
        .collect();
    let mut screen_slot_fills: HashMap<(pb::IdKey, String), Vec<pb::Id>> = HashMap::new();
    let mut screen_contributors: HashMap<pb::IdKey, HashSet<String>> = HashMap::new();
    for se in all {
        let Some(pb::entity::Kind::Ui(ui)) = se.entity.kind.as_ref() else {
            continue;
        };
        let Some(ui_id) = ui.id.as_ref() else {
            continue;
        };
        let Some(slot) = ui.slot.as_ref() else {
            continue;
        };
        let Some(screen_id) = slot.screen.as_ref().and_then(|r| r.id.as_ref()) else {
            continue;
        };
        let screen_key = pb::IdKey::new(screen_id);
        screen_contributors
            .entry(screen_key.clone())
            .or_default()
            .insert(ui_id.namespace.clone());
        if !slot.slot.trim().is_empty() {
            screen_slot_fills
                .entry((screen_key, slot.slot.clone()))
                .or_default()
                .push(ui_id.clone());
        }
    }
    for se in all {
        let Some(pb::entity::Kind::Screen(screen)) = se.entity.kind.as_ref() else {
            continue;
        };
        let Some(id) = screen.id.as_ref() else {
            continue;
        };
        let subject = Some(pb::EntityRef {
            kind: pb::EntityKind::Screen as i32,
            id: Some(id.clone()),
        });
        if context_namespaces.contains(&id.namespace) {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "SCREEN_NAMESPACE_IS_CONTEXT".into(),
                message: format!(
                    "screen {}/{}@{} lives in bounded-context namespace {}; screens belong in an application namespace",
                    id.namespace, id.slug, id.version, id.namespace,
                ),
                subject: subject.clone(),
                subject_field: "id".into(),
                ..pb::ValidationIssue::default()
            });
        }
        let screen_key = pb::IdKey::new(id);
        for slot in &screen.slots {
            if !slot.required {
                continue;
            }
            let fill_key = (screen_key.clone(), slot.name.clone());
            if !screen_slot_fills.contains_key(&fill_key) {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Warning as i32,
                    code: "SLOT_UNFILLED".into(),
                    message: format!(
                        "screen {}/{}@{} has required slot `{}` but no UI contributes to it",
                        id.namespace, id.slug, id.version, slot.name,
                    ),
                    subject: subject.clone(),
                    subject_field: "slots".into(),
                    ..pb::ValidationIssue::default()
                });
            }
        }
        if let Some(contributors) = screen_contributors.get(&screen_key) {
            if contributors.len() > 1 {
                let mut list: Vec<&str> = contributors.iter().map(String::as_str).collect();
                list.sort_unstable();
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Info as i32,
                    code: "SCREEN_MANY_CONTRIBUTORS".into(),
                    message: format!(
                        "screen {}/{}@{} composes UI from {} contexts ({})",
                        id.namespace,
                        id.slug,
                        id.version,
                        contributors.len(),
                        list.join(", "),
                    ),
                    subject: subject.clone(),
                    subject_field: "slots".into(),
                    ..pb::ValidationIssue::default()
                });
            }
        }
    }

    // TERM_INVALID_ENTITY_KIND / TERM_DANGLING_ENTITY.
    for se in all {
        let Some(pb::entity::Kind::Term(term)) = se.entity.kind.as_ref() else {
            continue;
        };
        let Some(id) = term.id.as_ref() else { continue };
        let subject = Some(pb::EntityRef {
            kind: pb::EntityKind::Term as i32,
            id: Some(id.clone()),
        });
        for (i, embodied) in term.embodied_by.iter().enumerate() {
            let Some(target_id) = embodied.id.as_ref() else {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "TERM_DANGLING_ENTITY".into(),
                    message: format!(
                        "term {}/{}@{} embodied_by[{i}] has no id",
                        id.namespace, id.slug, id.version,
                    ),
                    subject: subject.clone(),
                    subject_field: format!("embodied_by[{i}]"),
                    ..pb::ValidationIssue::default()
                });
                continue;
            };
            let Ok(target_kind) = pb::EntityKind::try_from(embodied.kind) else {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "TERM_INVALID_ENTITY_KIND".into(),
                    message: format!(
                        "term {}/{}@{} embodied_by[{i}] has invalid kind {}",
                        id.namespace, id.slug, id.version, embodied.kind,
                    ),
                    subject: subject.clone(),
                    subject_field: format!("embodied_by[{i}]"),
                    ..pb::ValidationIssue::default()
                });
                continue;
            };
            if matches!(target_kind, pb::EntityKind::Unspecified) {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "TERM_INVALID_ENTITY_KIND".into(),
                    message: format!(
                        "term {}/{}@{} embodied_by[{i}] uses ENTITY_KIND_UNSPECIFIED",
                        id.namespace, id.slug, id.version,
                    ),
                    subject: subject.clone(),
                    subject_field: format!("embodied_by[{i}]"),
                    ..pb::ValidationIssue::default()
                });
                continue;
            }
            if !known.contains(&pb::EntityKey::new(target_kind, target_id)) {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "TERM_DANGLING_ENTITY".into(),
                    message: format!(
                        "term {}/{}@{} embodies missing {}:{}/{}@{}",
                        id.namespace,
                        id.slug,
                        id.version,
                        trogon_atlas_proto::canonical::kind_short(target_kind),
                        target_id.namespace,
                        target_id.slug,
                        target_id.version,
                    ),
                    subject: subject.clone(),
                    subject_field: format!("embodied_by[{i}]"),
                    ..pb::ValidationIssue::default()
                });
            }
        }
    }

    // AMBIGUITY_*.
    for se in all {
        let Some(pb::entity::Kind::Ambiguity(ambiguity)) = se.entity.kind.as_ref() else {
            continue;
        };
        let Some(id) = ambiguity.id.as_ref() else {
            continue;
        };
        let subject = Some(pb::EntityRef {
            kind: pb::EntityKind::Ambiguity as i32,
            id: Some(id.clone()),
        });
        let kind = pb::ambiguity::Kind::try_from(ambiguity.kind)
            .unwrap_or(pb::ambiguity::Kind::Unspecified);
        if matches!(kind, pb::ambiguity::Kind::Unspecified) {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "AMBIGUITY_UNSPECIFIED_KIND".into(),
                message: format!(
                    "ambiguity {}/{}@{} does not declare HOMONYM or SYNONYM",
                    id.namespace, id.slug, id.version,
                ),
                subject: subject.clone(),
                subject_field: "kind".into(),
                ..pb::ValidationIssue::default()
            });
        }
        if ambiguity.terms.len() < 2 {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "AMBIGUITY_TOO_FEW_TERMS".into(),
                message: format!(
                    "ambiguity {}/{}@{} names {} term(s); at least two are required",
                    id.namespace,
                    id.slug,
                    id.version,
                    ambiguity.terms.len(),
                ),
                subject: subject.clone(),
                subject_field: "terms".into(),
                ..pb::ValidationIssue::default()
            });
        }
        for (i, term) in ambiguity.terms.iter().enumerate() {
            let Some(term_id) = term.id.as_ref() else {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "AMBIGUITY_DANGLING_TERM".into(),
                    message: format!(
                        "ambiguity {}/{}@{} terms[{i}] has no id",
                        id.namespace, id.slug, id.version,
                    ),
                    subject: subject.clone(),
                    subject_field: format!("terms[{i}]"),
                    ..pb::ValidationIssue::default()
                });
                continue;
            };
            if !known.contains(&pb::EntityKey::new(pb::EntityKind::Term, term_id)) {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "AMBIGUITY_DANGLING_TERM".into(),
                    message: format!(
                        "ambiguity {}/{}@{} references missing term {}/{}@{}",
                        id.namespace,
                        id.slug,
                        id.version,
                        term_id.namespace,
                        term_id.slug,
                        term_id.version,
                    ),
                    subject: subject.clone(),
                    subject_field: format!("terms[{i}]"),
                    ..pb::ValidationIssue::default()
                });
            }
        }
        let ruling = pb::ambiguity::Ruling::try_from(ambiguity.ruling)
            .unwrap_or(pb::ambiguity::Ruling::Unspecified);
        if matches!(ruling, pb::ambiguity::Ruling::Unspecified) {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "AMBIGUITY_UNSPECIFIED_RULING".into(),
                message: format!(
                    "ambiguity {}/{}@{} does not declare a ruling",
                    id.namespace, id.slug, id.version,
                ),
                subject,
                subject_field: "ruling".into(),
                ..pb::ValidationIssue::default()
            });
        }
    }

    enrich_issues_with_catalog(&mut issues);
    issues
}

#[must_use]
pub fn validate_scenarios(entity: &pb::Entity, all: &[StoredEntity]) -> Vec<pb::ValidationIssue> {
    let mut issues: Vec<pb::ValidationIssue> = Vec::new();
    let known: HashSet<pb::EntityKey> = all
        .iter()
        .filter_map(|se| {
            let k = entity_kind(&se.entity)?;
            let i = entity_id(&se.entity)?;
            Some(pb::EntityKey::new(k, i))
        })
        .collect();
    let subject = slice_ref(entity);
    match entity.kind.as_ref() {
        Some(pb::entity::Kind::CommandSlice(s)) => {
            check_unique_titles(
                s.scenarios.iter().map(|sc| sc.title.as_str()),
                "command",
                subject.clone(),
                &mut issues,
            );
            let slice_command_id = s
                .command
                .as_ref()
                .and_then(|ce| ce.command.as_ref())
                .and_then(|r| r.id.as_ref());
            for sc in &s.scenarios {
                for ev in &sc.given {
                    check_ref(
                        pb::EntityKind::Event,
                        ev.event.as_ref().and_then(|r| r.id.as_ref()),
                        "scenarios[].given[].event",
                        subject.clone(),
                        &known,
                        &mut issues,
                    );
                }
                let cmd_id = sc
                    .when
                    .as_ref()
                    .and_then(|w| w.command.as_ref())
                    .and_then(|r| r.id.as_ref());
                // Catalog SCENARIO_MISSING_REF: every When step must name a
                // concrete command. Absent `when` used to skip this check.
                check_ref(
                    pb::EntityKind::Command,
                    cmd_id,
                    "scenarios[].when.command",
                    subject.clone(),
                    &known,
                    &mut issues,
                );
                if let (Some(slice_id), Some(scenario_id)) = (slice_command_id, cmd_id) {
                    if slice_id != scenario_id {
                        issues.push(issue_error(
                            "SCENARIO_COMMAND_MISMATCH",
                            format!(
                                "scenarios[].when.command {:?}/{}@{} does not match slice.command {:?}/{}@{}",
                                scenario_id.namespace, scenario_id.slug, scenario_id.version,
                                slice_id.namespace, slice_id.slug, slice_id.version,
                            ),
                            subject.clone(),
                            "scenarios[].when.command",
                        ));
                    }
                }
                match &sc.then {
                    Some(pb::command_scenario::Then::Emit(em)) => {
                        for ev in &em.events {
                            check_ref(
                                pb::EntityKind::Event,
                                ev.event.as_ref().and_then(|r| r.id.as_ref()),
                                "scenarios[].then.emit.events[].event",
                                subject.clone(),
                                &known,
                                &mut issues,
                            );
                        }
                    }
                    Some(pb::command_scenario::Then::Reject(rj)) => {
                        if rj.reason_code.trim().is_empty() {
                            issues.push(issue_error(
                                "SCENARIO_REJECT_NO_REASON",
                                "scenarios[].then.reject.reason_code is required".into(),
                                subject.clone(),
                                "scenarios[].then.reject.reason_code",
                            ));
                        }
                    }
                    None => {
                        issues.push(issue_error(
                            "SCENARIO_MISSING_THEN",
                            format!(
                                "command scenario {:?} must set then.emit or then.reject",
                                sc.title
                            ),
                            subject.clone(),
                            "scenarios[].then",
                        ));
                    }
                }
            }
            // Lifecycle conformance: when a stream declares transitions, a
            // scenario may only emit a legal successor of the last given
            // event on that stream. The swimlane decides the fork; GWTs
            // must not invent roads the aggregate would refuse.
            let event_lane: HashMap<pb::IdKey, pb::Id> = all
                .iter()
                .filter_map(|se| match se.entity.kind.as_ref() {
                    Some(pb::entity::Kind::Event(e)) => {
                        let id = e.id.as_ref()?;
                        let lane = e.swimlane.as_ref()?.id.as_ref()?;
                        Some((pb::IdKey::new(id), lane.clone()))
                    }
                    _ => None,
                })
                .collect();
            let lane_transitions: HashMap<pb::IdKey, &Vec<pb::swimlane::Transition>> = all
                .iter()
                .filter_map(|se| match se.entity.kind.as_ref() {
                    Some(pb::entity::Kind::Swimlane(l)) if !l.transitions.is_empty() => {
                        let id = l.id.as_ref()?;
                        Some((pb::IdKey::new(id), &l.transitions))
                    }
                    _ => None,
                })
                .collect();
            let key = |i: &pb::Id| pb::IdKey::new(i);
            for sc in &s.scenarios {
                let mut last_on: HashMap<pb::IdKey, pb::Id> = HashMap::new();
                for ev in &sc.given {
                    let Some(id) = ev.event.as_ref().and_then(|r| r.id.as_ref()) else {
                        continue;
                    };
                    if let Some(lane) = event_lane.get(&key(id)) {
                        // Time only points forward: the given history IS a
                        // stream's past, so consecutive events on one lane
                        // must walk its declared transitions forward.
                        if let (Some(prev), Some(transitions)) =
                            (last_on.get(&key(lane)), lane_transitions.get(&key(lane)))
                        {
                            if let Some(t) = transitions.iter().find(|t| {
                                t.after.as_ref().and_then(|r| r.id.as_ref()) == Some(prev)
                            }) {
                                let legal = t.next.iter().any(|n| {
                                    n.event.as_ref().and_then(|r| r.id.as_ref()) == Some(id)
                                });
                                if !legal {
                                    let allowed: Vec<String> = t
                                        .next
                                        .iter()
                                        .filter_map(|n| {
                                            n.event.as_ref().and_then(|r| r.id.as_ref())
                                        })
                                        .map(|i| i.slug.clone())
                                        .collect();
                                    issues.push(pb::ValidationIssue {
                                        severity: pb::validation_issue::Severity::Warning as i32,
                                        code: "SCENARIO_TRANSITION_VIOLATION".into(),
                                        message: format!(
                                            "scenario {:?} lists given {}/{}@{} after {}/{}@{}, but the {} stream's lifecycle allows only [{}] next; a given history must walk the stream's transitions forward",
                                            sc.title,
                                            id.namespace, id.slug, id.version,
                                            prev.namespace, prev.slug, prev.version,
                                            lane.slug,
                                            allowed.join(", "),
                                        ),
                                        subject: subject.clone(),
                                        subject_field: "scenarios[].given".into(),
                                        ..Default::default()
                                    });
                                }
                            }
                        }
                        last_on.insert(key(lane), id.clone());
                    }
                }
                let Some(pb::command_scenario::Then::Emit(em)) = &sc.then else {
                    continue;
                };
                for ev in &em.events {
                    let Some(id) = ev.event.as_ref().and_then(|r| r.id.as_ref()) else {
                        continue;
                    };
                    let Some(lane) = event_lane.get(&key(id)) else {
                        continue;
                    };
                    let Some(transitions) = lane_transitions.get(&key(lane)) else {
                        continue;
                    };
                    let Some(last) = last_on.get(&key(lane)) else {
                        continue;
                    };
                    let Some(t) = transitions
                        .iter()
                        .find(|t| t.after.as_ref().and_then(|r| r.id.as_ref()) == Some(last))
                    else {
                        continue;
                    };
                    let legal = t
                        .next
                        .iter()
                        .any(|n| n.event.as_ref().and_then(|r| r.id.as_ref()) == Some(id));
                    if !legal {
                        let allowed: Vec<String> = t
                            .next
                            .iter()
                            .filter_map(|n| n.event.as_ref().and_then(|r| r.id.as_ref()))
                            .map(|i| i.slug.clone())
                            .collect();
                        issues.push(pb::ValidationIssue {
                            severity: pb::validation_issue::Severity::Warning as i32,
                            code: "SCENARIO_TRANSITION_VIOLATION".into(),
                            message: format!(
                                "scenario {:?} emits {}/{}@{} after {}/{}@{}, but the {} stream's lifecycle allows only [{}] next; the aggregate would refuse this road",
                                sc.title,
                                id.namespace, id.slug, id.version,
                                last.namespace, last.slug, last.version,
                                lane.slug,
                                allowed.join(", "),
                            ),
                            subject: subject.clone(),
                            subject_field: "scenarios[].then.emit".into(),
                            ..Default::default()
                        });
                    }
                }
            }
            for sc in &s.scenarios {
                push_annotation_findings(
                    &sc.metadata,
                    subject.as_ref(),
                    "scenarios[].metadata",
                    &format!("command scenario {:?}", sc.title),
                    &mut issues,
                );
            }
        }
        Some(pb::entity::Kind::ReadModelSlice(s)) => {
            check_unique_titles(
                s.scenarios.iter().map(|sc| sc.title.as_str()),
                "read-model",
                subject.clone(),
                &mut issues,
            );
            let slice_rm_id = s
                .read_model
                .as_ref()
                .and_then(|re| re.read_model.as_ref())
                .and_then(|r| r.id.as_ref());
            for sc in &s.scenarios {
                for ev in &sc.when {
                    check_ref(
                        pb::EntityKind::Event,
                        ev.event.as_ref().and_then(|r| r.id.as_ref()),
                        "scenarios[].when[].event",
                        subject.clone(),
                        &known,
                        &mut issues,
                    );
                }
                let Some(then) = &sc.then else {
                    issues.push(issue_error(
                        "SCENARIO_MISSING_THEN",
                        format!(
                            "read-model scenario {:?} must set then.read_model",
                            sc.title
                        ),
                        subject.clone(),
                        "scenarios[].then",
                    ));
                    continue;
                };
                let rm_id = then.read_model.as_ref().and_then(|r| r.id.as_ref());
                check_ref(
                    pb::EntityKind::ReadModel,
                    rm_id,
                    "scenarios[].then.read_model",
                    subject.clone(),
                    &known,
                    &mut issues,
                );
                if let (Some(slice_id), Some(scenario_id)) = (slice_rm_id, rm_id) {
                    if slice_id != scenario_id {
                        issues.push(issue_error(
                            "SCENARIO_READ_MODEL_MISMATCH",
                            format!(
                                "scenarios[].then.read_model {:?}/{}@{} does not match slice.read_model {:?}/{}@{}",
                                scenario_id.namespace, scenario_id.slug, scenario_id.version,
                                slice_id.namespace, slice_id.slug, slice_id.version,
                            ),
                            subject.clone(),
                            "scenarios[].then.read_model",
                        ));
                    }
                }
            }
            for sc in &s.scenarios {
                push_annotation_findings(
                    &sc.metadata,
                    subject.as_ref(),
                    "scenarios[].metadata",
                    &format!("read-model scenario {:?}", sc.title),
                    &mut issues,
                );
            }
        }
        Some(pb::entity::Kind::AutomationSlice(s)) => {
            check_unique_titles(
                s.scenarios.iter().map(|sc| sc.title.as_str()),
                "automation",
                subject.clone(),
                &mut issues,
            );
            let slice_command_id = s
                .emitted_command
                .as_ref()
                .and_then(|ce| ce.command.as_ref())
                .and_then(|r| r.id.as_ref());
            for sc in &s.scenarios {
                for rm in &sc.given {
                    check_ref(
                        pb::EntityKind::ReadModel,
                        rm.read_model.as_ref().and_then(|r| r.id.as_ref()),
                        "scenarios[].given[].read_model",
                        subject.clone(),
                        &known,
                        &mut issues,
                    );
                }
                let Some(then) = &sc.then else {
                    // Parity with CommandScenario / ReadModelScenario:
                    // an automation scenario with no Then is meaningless
                    // (the test has no observable side effect to assert)
                    // and was silently accepted by the previous branch.
                    issues.push(issue_error(
                        "SCENARIO_MISSING_THEN",
                        format!("automation scenario {:?} must set then.command", sc.title),
                        subject.clone(),
                        "scenarios[].then",
                    ));
                    continue;
                };
                let cmd_id = then.command.as_ref().and_then(|r| r.id.as_ref());
                check_ref(
                    pb::EntityKind::Command,
                    cmd_id,
                    "scenarios[].then.command",
                    subject.clone(),
                    &known,
                    &mut issues,
                );
                if let (Some(slice_id), Some(scenario_id)) = (slice_command_id, cmd_id) {
                    if slice_id != scenario_id {
                        issues.push(issue_error(
                            "SCENARIO_COMMAND_MISMATCH",
                            format!(
                                "scenarios[].then.command {:?}/{}@{} does not match slice.emitted_command {:?}/{}@{}",
                                scenario_id.namespace, scenario_id.slug, scenario_id.version,
                                slice_id.namespace, slice_id.slug, slice_id.version,
                            ),
                            subject.clone(),
                            "scenarios[].then.command",
                        ));
                    }
                }
            }
            for sc in &s.scenarios {
                push_annotation_findings(
                    &sc.metadata,
                    subject.as_ref(),
                    "scenarios[].metadata",
                    &format!("automation scenario {:?}", sc.title),
                    &mut issues,
                );
            }
        }
        // The context's root document names its boundary: slug == namespace
        // (Decision #27), which also makes it naturally unique per context.
        Some(pb::entity::Kind::BoundedContext(bc)) => {
            if let Some(id) = bc.id.as_ref() {
                if id.slug != id.namespace {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "CONTEXT_SLUG_MISMATCH".into(),
                        message: format!(
                            "bounded context slug {:?} must equal its namespace {:?}; the context IS the namespace, and slug == namespace keeps it one per context",
                            id.slug, id.namespace,
                        ),
                        subject: subject.clone(),
                        subject_field: "id.slug".into(),
                        ..Default::default()
                    });
                }
            }
            for (i, r) in bc.realizes.iter().enumerate() {
                let Some(sid) = r.id.as_ref() else {
                    issues.push(issue_error(
                        "REALIZES_MISSING_REF",
                        format!("realizes[{i}] names no subdomain"),
                        subject.clone(),
                        &format!("realizes[{i}]"),
                    ));
                    continue;
                };
                if !known.contains(&pb::EntityKey::new(pb::EntityKind::Subdomain, sid)) {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "REALIZES_DANGLING".into(),
                        message: format!(
                            "realizes[{i}] -> subdomain {}/{}@{} not found in store",
                            sid.namespace, sid.slug, sid.version,
                        ),
                        subject: subject.clone(),
                        subject_field: format!("realizes[{i}]"),
                        ..Default::default()
                    });
                }
            }
            // Decision #30: intent and data must agree. Derive the seams
            // feeding THIS context (cross-namespace source_events on its
            // read models), then hold the declared relationships to them.
            let own_ns = bc
                .id
                .as_ref()
                .map(|i| i.namespace.clone())
                .unwrap_or_default();
            let upstream_seams: HashSet<String> = all
                .iter()
                .filter_map(|se| match se.entity.kind.as_ref() {
                    Some(pb::entity::Kind::ReadModel(rm)) => Some(rm),
                    _ => None,
                })
                .filter(|rm| rm.id.as_ref().is_some_and(|i| i.namespace == own_ns))
                .flat_map(|rm| rm.source_events.iter())
                .filter_map(|r| r.id.as_ref())
                .filter(|i| i.namespace != own_ns)
                .map(|i| i.namespace.clone())
                .collect();
            let mut declared_upstreams: HashSet<String> = HashSet::new();
            for (i, rel) in bc.relationships.iter().enumerate() {
                let Some(uid) = rel.upstream.as_ref().and_then(|u| u.id.as_ref()) else {
                    issues.push(issue_error(
                        "RELATIONSHIP_MISSING_REF",
                        format!("relationships[{i}] names no upstream context"),
                        subject.clone(),
                        &format!("relationships[{i}].upstream"),
                    ));
                    continue;
                };
                declared_upstreams.insert(uid.namespace.clone());
                if uid.namespace == own_ns {
                    issues.push(issue_error(
                        "RELATIONSHIP_SELF",
                        format!("relationships[{i}] points at this context itself; a context needs no agreement with itself"),
                        subject.clone(),
                        &format!("relationships[{i}].upstream"),
                    ));
                    continue;
                }
                if !known.contains(&pb::EntityKey::new(pb::EntityKind::BoundedContext, uid)) {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "RELATIONSHIP_DANGLING".into(),
                        message: format!(
                            "relationships[{i}] -> bounded_context:{}/{}@{} not found in store",
                            uid.namespace, uid.slug, uid.version,
                        ),
                        subject: subject.clone(),
                        subject_field: format!("relationships[{i}].upstream"),
                        ..Default::default()
                    });
                    continue;
                }
                let intent =
                    pb::bounded_context::context_relationship::Intent::try_from(rel.intent)
                        .unwrap_or(pb::bounded_context::context_relationship::Intent::Unspecified);
                let has_seam = upstream_seams.contains(&uid.namespace);
                match intent {
                    pb::bounded_context::context_relationship::Intent::Unspecified => {
                        issues.push(pb::ValidationIssue {
                            severity: pb::validation_issue::Severity::Warning as i32,
                            code: "RELATIONSHIP_UNSPECIFIED_INTENT".into(),
                            message: format!(
                                "relationships[{i}] declares no intent toward {}; the intent IS the agreement: say who moves when the upstream breaks",
                                uid.namespace,
                            ),
                            subject: subject.clone(),
                            subject_field: format!("relationships[{i}].intent"),
                            ..Default::default()
                        });
                    }
                    pb::bounded_context::context_relationship::Intent::SeparateWays => {
                        if has_seam {
                            issues.push(pb::ValidationIssue {
                                severity: pb::validation_issue::Severity::Error as i32,
                                code: "RELATIONSHIP_CONTRADICTS_SEAM".into(),
                                message: format!(
                                    "relationships[{i}] declares SEPARATE_WAYS with {} but a seam exists (its events feed this context's views); the data says integrated: fix the declaration or remove the seam",
                                    uid.namespace,
                                ),
                                subject: subject.clone(),
                                subject_field: format!("relationships[{i}].intent"),
                                ..Default::default()
                            });
                        }
                    }
                    _ => {
                        if !has_seam {
                            issues.push(pb::ValidationIssue {
                                severity: pb::validation_issue::Severity::Warning as i32,
                                code: "RELATIONSHIP_WITHOUT_SEAM".into(),
                                message: format!(
                                    "relationships[{i}] declares an integration intent toward {} but no seam exists; an agreement no data supports is wishful: add the subscription view or declare SEPARATE_WAYS",
                                    uid.namespace,
                                ),
                                subject: subject.clone(),
                                subject_field: format!("relationships[{i}].intent"),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
            for up in &upstream_seams {
                if !declared_upstreams.contains(up) {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Warning as i32,
                        code: "SEAM_WITHOUT_RELATIONSHIP".into(),
                        message: format!(
                            "events from {up} feed this context's views, but no relationship intent is declared; record the agreement: who moves when {up} wants a breaking change?",
                        ),
                        subject: subject.clone(),
                        subject_field: "relationships".into(),
                        ..Default::default()
                    });
                }
            }
            if bc.realizes.len() > 1 {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Info as i32,
                    code: "REALIZES_MULTIPLE".into(),
                    // ADR #29: one context realizing multiple subdomains is
                    // legal but smells like a monolith confessing.
                    message: format!(
                        "context realizes {} subdomains; legal but worth hearing: one context solving several problems often signals a monolith confessing",
                        bc.realizes.len(),
                    ),
                    subject: subject.clone(),
                    subject_field: "realizes".into(),
                    ..Default::default()
                });
            }
        }
        // The problem space is a knowledge graph (Decision #29): the domain
        // names its own namespace, subdomains live inside it and carry the
        // classification, and contexts map to subdomains via realizes.
        Some(pb::entity::Kind::Domain(d)) => {
            if let Some(id) = d.id.as_ref() {
                if id.slug != id.namespace {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "DOMAIN_SLUG_MISMATCH".into(),
                        message: format!(
                            "domain slug {:?} must equal its namespace {:?}; the domain names the problem-space scope, and slug == namespace keeps it one per scope",
                            id.slug, id.namespace,
                        ),
                        subject: subject.clone(),
                        subject_field: "id.slug".into(),
                        ..Default::default()
                    });
                }
                // A domain with zero core subdomains is suspicious: every
                // business has SOMETHING that differentiates it, and the
                // whole point of subdomain classification is to identify
                // that thing. Zero-core usually means the classification
                // was rushed or a strategic conversation never happened.
                let has_core = all.iter().any(|se| {
                    let Some(kind) = se.entity.kind.as_ref() else {
                        return false;
                    };
                    let pb::entity::Kind::Subdomain(sd) = kind else {
                        return false;
                    };
                    let Some(sd_id) = sd.id.as_ref() else {
                        return false;
                    };
                    sd_id.namespace == id.namespace
                        && sd.classification == pb::subdomain::Classification::Core as i32
                });
                let has_any_subdomain = all.iter().any(|se| {
                    let Some(kind) = se.entity.kind.as_ref() else {
                        return false;
                    };
                    let pb::entity::Kind::Subdomain(sd) = kind else {
                        return false;
                    };
                    sd.id.as_ref().is_some_and(|i| i.namespace == id.namespace)
                });
                if has_any_subdomain && !has_core {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Info as i32,
                        code: "DOMAIN_HAS_NO_CORE".into(),
                        message: "domain has subdomains but none classified core; the whole point of classification is to identify the differentiator: check whether the classification was rushed".into(),
                        subject: subject.clone(),
                        subject_field: "subdomains".into(),
                        ..Default::default()
                    });
                }
            }
        }
        Some(pb::entity::Kind::Subdomain(sd)) => {
            let ns = sd
                .id
                .as_ref()
                .map(|i| i.namespace.clone())
                .unwrap_or_default();
            match sd.domain.as_ref().and_then(|r| r.id.as_ref()) {
                None => {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Warning as i32,
                        code: "SUBDOMAIN_MISSING_DOMAIN".into(),
                        message: "subdomain names no domain; a problem belongs to a business, say which one".into(),
                        subject: subject.clone(),
                        subject_field: "domain".into(),
                        ..Default::default()
                    });
                }
                Some(did) => {
                    if did.namespace != ns {
                        issues.push(pb::ValidationIssue {
                            severity: pb::validation_issue::Severity::Error as i32,
                            code: "SUBDOMAIN_FOREIGN_DOMAIN".into(),
                            message: format!(
                                "subdomain in namespace {:?} points at domain {}/{}; subdomains live in their domain's namespace: the domain partitions ITS problem space, not someone else's",
                                ns, did.namespace, did.slug,
                            ),
                            subject: subject.clone(),
                            subject_field: "domain".into(),
                            ..Default::default()
                        });
                    } else if !known.contains(&pb::EntityKey::new(pb::EntityKind::Domain, did)) {
                        issues.push(pb::ValidationIssue {
                            severity: pb::validation_issue::Severity::Error as i32,
                            code: "SUBDOMAIN_DANGLING_DOMAIN".into(),
                            message: format!(
                                "subdomain points at domain {}/{}@{} which is not in the store",
                                did.namespace, did.slug, did.version,
                            ),
                            subject: subject.clone(),
                            subject_field: "domain".into(),
                            ..Default::default()
                        });
                    }
                }
            }
            if sd.classification == pb::subdomain::Classification::Unspecified as i32 {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Warning as i32,
                    code: "SUBDOMAIN_UNCLASSIFIED".into(),
                    message: "subdomain carries no classification; core/supporting/generic is the entire point: it is a budget document, classify it".into(),
                    subject: subject.clone(),
                    subject_field: "classification".into(),
                    ..Default::default()
                });
            }
            // A subdomain that no bounded context realizes is either
            // future planning ("we know we will need to solve X someday")
            // or dead code ("we used to think X was a thing and forgot
            // to retire it"). Either way it should be deliberate, not
            // accidental.
            if let Some(sd_id) = sd.id.as_ref() {
                let realized = all.iter().any(|se| {
                    let Some(kind) = se.entity.kind.as_ref() else {
                        return false;
                    };
                    let pb::entity::Kind::BoundedContext(bc) = kind else {
                        return false;
                    };
                    bc.realizes.iter().any(|r| {
                        r.id.as_ref().is_some_and(|rid| {
                            rid.namespace == sd_id.namespace && rid.slug == sd_id.slug
                        })
                    })
                });
                if !realized {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Info as i32,
                        code: "SUBDOMAIN_UNREALIZED".into(),
                        message: "subdomain has no bounded context realizing it; either future planning or dead code, make it deliberate".into(),
                        subject: subject.clone(),
                        subject_field: "realizes".into(),
                        ..Default::default()
                    });
                }
            }
        }
        // Project management overlays the model (Decision #25): tracker
        // items must point at real entities, and one tracker holds at most
        // one status per subject.
        Some(pb::entity::Kind::Tracker(t)) => {
            let mut seen: HashMap<String, usize> = HashMap::new();
            for (i, item) in t.items.iter().enumerate() {
                let Some(subject_ref) = item.subject.as_ref() else {
                    issues.push(issue_error(
                        "TRACKER_MISSING_SUBJECT",
                        format!("items[{i}].subject is required"),
                        subject.clone(),
                        &format!("items[{i}].subject"),
                    ));
                    continue;
                };
                let Ok(kind) = pb::EntityKind::try_from(subject_ref.kind) else {
                    continue;
                };
                check_ref(
                    kind,
                    subject_ref.id.as_ref(),
                    &format!("items[{i}].subject"),
                    subject.clone(),
                    &known,
                    &mut issues,
                );
                if let Some(id) = subject_ref.id.as_ref() {
                    let key = format!(
                        "{}:{}/{}@{}",
                        pb::canonical::kind_short(kind),
                        id.namespace,
                        id.slug,
                        id.version
                    );
                    *seen.entry(key).or_insert(0) += 1;
                }
            }
            for (key, count) in seen {
                if count > 1 {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Warning as i32,
                        code: "TRACKER_DUPLICATE_SUBJECT".into(),
                        message: format!(
                            "tracker lists {key} {count} times; one tracker holds one status per subject: use separate trackers for separate workstreams"
                        ),
                        subject: subject.clone(),
                        subject_field: "items[].subject".into(),
                        ..Default::default()
                    });
                }
            }
        }
        _ => {}
    }
    enrich_issues_with_catalog(&mut issues);
    issues
}

// =============================================================================
// validate_model: orchestration
//
// validate_model() walks an EventModel through ~15 independent passes, each
// emitting `ValidationIssue`s for one well-defined invariant. Each pass is
// delimited inline with a `// ===== PASS: <name> =====` banner so the
// boundaries are easy to scan; factoring each pass into a named function
// is a possible refactor, not a correctness debt: the inline form keeps
// the shared inputs cheap and the order of passes obvious.
//
// The order matters only where downstream passes consume the maps/sets
// accumulated by upstream passes, noted on each banner.
//
// The shared inputs are built once at the top:
//   * `known`: every entity in the store, keyed by EntityKey
//   * `member_set`: members claimed by THIS event model
// Both feed every pass; no other state is shared across pass boundaries.
// =============================================================================
pub fn validate_model(model: &pb::EventModel, all: &[StoredEntity]) -> Vec<pb::ValidationIssue> {
    let mut issues: Vec<pb::ValidationIssue> = Vec::new();

    let known: HashSet<pb::EntityKey> = all
        .iter()
        .filter_map(|se| {
            let k = entity_kind(&se.entity)?;
            let i = entity_id(&se.entity)?;
            Some(pb::EntityKey::new(k, i))
        })
        .collect();

    let member_set: HashSet<pb::EntityKey> = model
        .members
        .iter()
        .filter_map(|m| {
            let k = pb::EntityKind::try_from(m.kind).ok()?;
            let i = m.id.as_ref()?;
            Some(pb::EntityKey::new(k, i))
        })
        .collect();

    // ===== PASS: member_exists =====
    // Every claimed member must resolve in the store; missing IDs are an
    // authoring error (the model is pointing at nothing).

    for m in &model.members {
        let Some(id) = m.id.as_ref() else {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "MISSING_ID".into(),
                message: "member EntityRef has no id".into(),
                subject: Some(m.clone()),
                subject_field: String::new(),
                ..Default::default()
            });
            continue;
        };
        let Ok(k) = pb::EntityKind::try_from(m.kind) else {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "INVALID_KIND".into(),
                message: format!("member EntityRef has invalid kind {}", m.kind),
                subject: Some(m.clone()),
                subject_field: String::new(),
                ..Default::default()
            });
            continue;
        };
        let key = pb::EntityKey::new(k, id);
        if !known.contains(&key) {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "MEMBER_NOT_FOUND".into(),
                message: format!(
                    "member {}:{}/{}@{} not found in the store",
                    trogon_atlas_proto::canonical::kind_short(k),
                    id.namespace,
                    id.slug,
                    id.version
                ),
                subject: Some(m.clone()),
                subject_field: String::new(),
                ..Default::default()
            });
        }
    }

    // Invariant: a processor issues exactly ONE command. Branching outcomes
    // (e.g. approve vs reject) are different scenarios and must be modeled as
    // separate automation slices with separate processors.
    //
    // Invariant: a read model is observed by at most ONE automation. Different
    // consumer code paths need different data, so each automation observes its
    // own consumer-shaped view (external webhooks may project into several
    // ExternalSource read models for this purpose).
    let mut processor_commands: HashMap<pb::IdKey, HashSet<String>> = HashMap::new();
    let mut read_model_automations: HashMap<pb::IdKey, HashSet<String>> = HashMap::new();
    for m in &model.members {
        let (Some(id), Ok(pb::EntityKind::AutomationSlice)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(se) =
            all.iter().find(
                |se| match (entity_kind(&se.entity), entity_id(&se.entity)) {
                    (Some(sk), Some(sid)) => {
                        sk == pb::EntityKind::AutomationSlice
                            && sid.namespace == id.namespace
                            && sid.slug == id.slug
                            && sid.version == id.version
                    }
                    _ => false,
                },
            )
        else {
            continue;
        };
        let Some(pb::entity::Kind::AutomationSlice(s)) = se.entity.kind.as_ref() else {
            continue;
        };
        for rm_id in s
            .source_read_models
            .iter()
            .filter_map(|e| e.read_model.as_ref())
            .filter_map(|r| r.id.as_ref())
        {
            read_model_automations
                .entry(pb::IdKey::new(rm_id))
                .or_default()
                .insert(format!("{}/{}@{}", id.namespace, id.slug, id.version));
        }
        let Some(proc_id) = s
            .processor
            .as_ref()
            .and_then(|e| e.processor.as_ref())
            .and_then(|r| r.id.as_ref())
        else {
            continue;
        };
        let Some(cmd_id) = s
            .emitted_command
            .as_ref()
            .and_then(|e| e.command.as_ref())
            .and_then(|r| r.id.as_ref())
        else {
            continue;
        };
        processor_commands
            .entry(pb::IdKey::new(proc_id))
            .or_default()
            .insert(format!(
                "{}/{}@{}",
                cmd_id.namespace, cmd_id.slug, cmd_id.version
            ));
    }
    for (key, cmds) in &processor_commands {
        let (ns, slug, version) = (&key.namespace, &key.slug, key.version);
        if cmds.len() > 1 {
            let mut list: Vec<&str> = cmds.iter().map(String::as_str).collect();
            list.sort_unstable();
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "PROCESSOR_MULTIPLE_COMMANDS".into(),
                message: format!(
                    "processor {}/{}@{} issues {} different commands ({}); a processor issues exactly one command: model each outcome as its own automation slice with its own processor",
                    ns,
                    slug,
                    version,
                    cmds.len(),
                    list.join(", ")
                ),
                subject: Some(pb::EntityRef {
                    kind: pb::EntityKind::Processor as i32,
                    id: Some(key.id()),
                }),
                subject_field: "emitted_command".into(),
                ..Default::default()
            });
        }
    }
    for (key, slices) in &read_model_automations {
        let (ns, slug, version) = (&key.namespace, &key.slug, key.version);
        if slices.len() > 1 {
            let mut list: Vec<&str> = slices.iter().map(String::as_str).collect();
            list.sort_unstable();
            // Decision #34: a SharedConsumersAnnotation on the RM declares
            // that the multi-consumer arrangement is intentional, and the
            // finding downgrades from Error to Info. An empty doc on the
            // annotation triggers the doc-empty companion below.
            let (shared_declared, shared_doc_ok) = all
                .iter()
                .find_map(|se| {
                    let (
                        Some(pb::EntityKind::ReadModel),
                        Some(sid),
                        Some(pb::entity::Kind::ReadModel(rm)),
                    ) = (
                        entity_kind(&se.entity),
                        entity_id(&se.entity),
                        se.entity.kind.as_ref(),
                    )
                    else {
                        return None;
                    };
                    if sid.namespace != *ns || sid.slug != *slug || sid.version != version {
                        return None;
                    }
                    let mut present = false;
                    let mut doc_ok = false;
                    for any in &rm.metadata {
                        if !any
                            .type_url
                            .ends_with("trogonatlas.eventmodel.v1alpha1.SharedConsumersAnnotation")
                        {
                            continue;
                        }
                        present = true;
                        if let Ok(a) = <pb::SharedConsumersAnnotation as prost::Message>::decode(
                            any.value.as_slice(),
                        ) {
                            doc_ok = !a.doc.is_empty();
                        }
                        break;
                    }
                    Some((present, doc_ok))
                })
                .unwrap_or((false, false));

            let severity = if shared_declared {
                pb::validation_issue::Severity::Info as i32
            } else {
                pb::validation_issue::Severity::Error as i32
            };
            let code = if shared_declared {
                "READ_MODEL_SHARED_CONSUMERS"
            } else {
                "READ_MODEL_MULTIPLE_AUTOMATIONS"
            };
            // ADR #34: a SharedConsumersAnnotation marks intentional fan-out
            // from one read model to many automations.
            let message = if shared_declared {
                format!(
                    "read model {}/{}@{} is consumed by {} automation slices ({}); SharedConsumersAnnotation declares this is intentional",
                    ns, slug, version, slices.len(), list.join(", ")
                )
            } else {
                format!(
                    "read model {}/{}@{} is observed by {} automation slices ({}); each automation observes its own consumer-shaped read model: split the view per consumer, or declare a SharedConsumersAnnotation if the consumers genuinely share the same data",
                    ns, slug, version, slices.len(), list.join(", ")
                )
            };
            issues.push(pb::ValidationIssue {
                severity,
                code: code.into(),
                message,
                subject: Some(pb::EntityRef {
                    kind: pb::EntityKind::ReadModel as i32,
                    id: Some(key.id()),
                }),
                subject_field: "source_read_models".into(),
                ..Default::default()
            });
            if shared_declared && !shared_doc_ok {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "READ_MODEL_SHARED_CONSUMERS_DOC_EMPTY".into(),
                    message: format!(
                        "read model {ns}/{slug}@{version} carries SharedConsumersAnnotation but its doc is empty: name the consumers and why they share the same data"
                    ),
                    subject: Some(pb::EntityRef {
                        kind: pb::EntityKind::ReadModel as i32,
                        id: Some(key.id()),
                    }),
                    subject_field: "metadata".into(),
                    ..Default::default()
                });
            }
        }
    }

    // A fact nobody observes changes nothing: every event the model emits
    // must be sourced by at least one read model (its own views or a
    // downstream context's subscription view). An unobserved event is how
    // loops silently fail to close, e.g. a dispatch queue that never
    // clears because the "requested" fact projects nowhere.
    let observed_events: HashSet<(String, String)> = all
        .iter()
        .filter_map(|se| match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::ReadModel(rm)) => Some(rm),
            _ => None,
        })
        .flat_map(|rm| rm.source_events.iter())
        .filter_map(|r| r.id.as_ref())
        .map(|i| (i.namespace.clone(), i.slug.clone()))
        .collect();
    for m in &model.members {
        let (Some(id), Ok(pb::EntityKind::Event)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        if !observed_events.contains(&(id.namespace.clone(), id.slug.clone())) {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Warning as i32,
                code: "EVENT_UNOBSERVED".into(),
                message: format!(
                    "event {}/{}@{} is sourced by no read model anywhere; a fact nobody observes cannot influence anything: project it into a view (e.g. clear the queue that triggered it) or question why it exists",
                    id.namespace, id.slug, id.version,
                ),
                subject: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id.clone()),
                }),
                subject_field: "source_events".into(),
                ..Default::default()
            });
        }
    }

    // A command is intent; events are the facts. Every Command should
    // eventually be wrapped by ≥1 CommandSlice that emits one or more events;
    // without that, the command points nowhere and can never cause a state
    // change. We allow the command to exist temporarily so the model can be
    // drafted iteratively, but flag it as pending work so the gap is
    // tracked, not forgotten. An AutomationSlice issuing the command is not
    // enough on its own: the automation describes who triggers it, the
    // CommandSlice describes what facts result.
    let commands_with_emissions: HashSet<pb::IdKey> = all
        .iter()
        .filter_map(|se| match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::CommandSlice(cs)) => Some(cs),
            _ => None,
        })
        .filter(|cs| !cs.emitted_events.is_empty())
        .filter_map(|cs| cs.command.as_ref())
        .filter_map(|ce| ce.command.as_ref())
        .filter_map(|cr| cr.id.as_ref())
        .map(pb::IdKey::new)
        .collect();
    for m in &model.members {
        let (Some(id), Ok(pb::EntityKind::Command)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        if !commands_with_emissions.contains(&pb::IdKey::new(id)) {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Warning as i32,
                code: "COMMAND_NO_EMITTED_EVENTS".into(),
                message: format!(
                    "command {}/{}@{} has no CommandSlice that emits events; pending work: add a CommandSlice with at least one emitted_events entry, or remove the command",
                    id.namespace, id.slug, id.version,
                ),
                subject: Some(pb::EntityRef {
                    kind: pb::EntityKind::Command as i32,
                    id: Some(id.clone()),
                }),
                subject_field: "emitted_events".into(),
                ..Default::default()
            });
        }
    }

    // ===== PASS: one_command_one_stream =====
    // Invariant: a single command writes to a single stream. A
    // CommandSlice whose emitted_events sit on a swimlane other than
    // the command's own is a saga in disguise: split it into two
    // commands and an AutomationSlice that bridges them.
    //
    // We index events and commands by Id so the swimlane lookup is O(1)
    // and apply the check once per emitted_event in each CommandSlice
    // claimed by this model.
    let event_swimlane: HashMap<pb::IdKey, Option<pb::Id>> = all
        .iter()
        .filter_map(|se| {
            let pb::entity::Kind::Event(e) = se.entity.kind.as_ref()? else {
                return None;
            };
            let id = e.id.as_ref()?;
            Some((
                pb::IdKey::new(id),
                e.swimlane.as_ref().and_then(|s| s.id.as_ref()).cloned(),
            ))
        })
        .collect();
    let command_swimlane: HashMap<pb::IdKey, Option<pb::Id>> = all
        .iter()
        .filter_map(|se| {
            let pb::entity::Kind::Command(c) = se.entity.kind.as_ref()? else {
                return None;
            };
            let id = c.id.as_ref()?;
            Some((
                pb::IdKey::new(id),
                c.swimlane.as_ref().and_then(|s| s.id.as_ref()).cloned(),
            ))
        })
        .collect();
    for m in &model.members {
        let (Some(slice_id), Ok(pb::EntityKind::CommandSlice)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(cs) = all.iter().find_map(|se| match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::CommandSlice(c)) if c.id.as_ref() == Some(slice_id) => Some(c),
            _ => None,
        }) else {
            continue;
        };
        let Some(cmd_id) = cs
            .command
            .as_ref()
            .and_then(|e| e.command.as_ref())
            .and_then(|r| r.id.as_ref())
        else {
            continue;
        };
        let cmd_lane = command_swimlane
            .get(&pb::IdKey::new(cmd_id))
            .cloned()
            .flatten();
        for (i, ee) in cs.emitted_events.iter().enumerate() {
            let Some(ev_id) = ee.event.as_ref().and_then(|r| r.id.as_ref()) else {
                continue;
            };
            let ev_lane = event_swimlane
                .get(&pb::IdKey::new(ev_id))
                .cloned()
                .flatten();
            if let (Some(cmd), Some(ev)) = (cmd_lane.as_ref(), ev_lane.as_ref()) {
                if cmd.namespace != ev.namespace || cmd.slug != ev.slug {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "COMMAND_MULTI_STREAM_WRITE".into(),
                        message: format!(
                            "command slice {}/{}@{} emits event {}/{}@{} on swimlane {}/{} but the command lives on swimlane {}/{}; a command writes to one stream only -- split into a saga: keep this slice's emitted_events on the command's lane, add a new command on the second lane, and bridge them with an AutomationSlice whose processor reads the trigger event's read model",
                            slice_id.namespace, slice_id.slug, slice_id.version,
                            ev_id.namespace, ev_id.slug, ev_id.version,
                            ev.namespace, ev.slug,
                            cmd.namespace, cmd.slug,
                        ),
                        subject: Some(pb::EntityRef {
                            kind: pb::EntityKind::CommandSlice as i32,
                            id: Some(slice_id.clone()),
                        }),
                        subject_field: format!("emitted_events[{i}]"),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // ===== PASS: information_completeness =====
    // Invariants:
    //   (a) COMMAND_HAS_NO_TRIGGER: every Command member must be
    //       reachable from either a human (CommandSlice with persona +
    //       ui) or an automation (some AutomationSlice's
    //       emitted_command references it). Without one, nothing in
    //       the model can cause the command to fire.
    //   (b) EVENT_FIELD_UNSOURCED: each field on a CommandSlice's
    //       emitted_event must come from the command's own fields, the
    //       lane's prior events/commands (the aggregate-state pattern),
    //       or be explicitly annotated as system-derived via
    //       DerivedFieldAnnotation.
    //   (c) AUTOMATION_COMMAND_FIELD_UNSOURCED: each field on an
    //       AutomationSlice's emitted command must come from one of
    //       the source_read_models the processor observes, or be
    //       annotated as system-derived.
    //
    // A field is considered "system-derived" when its FieldSpec.metadata
    // carries a well-formed DerivedFieldAnnotation with a non-empty doc.
    // Empty or malformed payloads do not silence unsourced findings.

    // Lookup tables, all built once.
    let events_by_id: HashMap<pb::IdKey, &pb::Event> = all
        .iter()
        .filter_map(|se| match se.entity.kind.as_ref()? {
            pb::entity::Kind::Event(e) => Some((pb::IdKey::new(e.id.as_ref()?), e)),
            _ => None,
        })
        .collect();
    let commands_by_id: HashMap<pb::IdKey, &pb::Command> = all
        .iter()
        .filter_map(|se| match se.entity.kind.as_ref()? {
            pb::entity::Kind::Command(c) => Some((pb::IdKey::new(c.id.as_ref()?), c)),
            _ => None,
        })
        .collect();
    let rms_by_id: HashMap<pb::IdKey, &pb::ReadModel> = all
        .iter()
        .filter_map(|se| match se.entity.kind.as_ref()? {
            pb::entity::Kind::ReadModel(r) => Some((pb::IdKey::new(r.id.as_ref()?), r)),
            _ => None,
        })
        .collect();
    let event_fields = declared_fields_by_id(&events_by_id, |e| e.schema.as_ref());
    let command_fields = declared_fields_by_id(&commands_by_id, |c| c.schema.as_ref());
    let rm_fields = declared_fields_by_id(&rms_by_id, |r| r.schema.as_ref());
    // For each swimlane id, gather the union of field names declared on
    // any COMMAND assigned to that lane. Commands are state-machine
    // inputs; their fields are always legitimate sources for events on
    // the same lane regardless of which event we're checking. Events
    // are handled separately at check time (the lane's other events,
    // excluding the event being checked) because an event cannot
    // tautologically source its own fields.
    let mut lane_command_fields: HashMap<pb::IdKey, HashSet<String>> = HashMap::new();
    for c in commands_by_id.values() {
        let Some(lane) = c.swimlane.as_ref().and_then(|s| s.id.as_ref()) else {
            continue;
        };
        let entry = lane_command_fields.entry(pb::IdKey::new(lane)).or_default();
        for f in trogon_atlas_core::schema::schema_fields(c.schema.as_ref()) {
            entry.insert(f.name);
        }
    }
    // Per-lane list of (event_id, field_names): diagnostics only; we
    // do NOT use this as a source (peer-event sourcing is name
    // collision, not data flow).
    let mut lane_events: HashMap<pb::IdKey, Vec<(pb::IdKey, HashSet<String>)>> = HashMap::new();
    for e in events_by_id.values() {
        let Some(lane) = e.swimlane.as_ref().and_then(|s| s.id.as_ref()) else {
            continue;
        };
        let Some(eid) = e.id.as_ref() else { continue };
        let names: HashSet<String> = trogon_atlas_core::schema::schema_fields(e.schema.as_ref())
            .into_iter()
            .map(|f| f.name)
            .collect();
        lane_events
            .entry(pb::IdKey::new(lane))
            .or_default()
            .push((pb::IdKey::new(eid), names));
    }
    // Per-lane aggregate-state field names declared on the Swimlane
    // itself. This is the 4th source path for EVENT_FIELD_UNSOURCED:
    // the aggregate owns these fields (an LLM picks them, a runtime
    // generates them, a clock provides them) and any event on the lane
    // can emit them without per-event sourcing.
    let mut lane_state_fields: HashMap<pb::IdKey, HashSet<String>> = HashMap::new();
    for se in all {
        let Some(pb::entity::Kind::Swimlane(lane)) = se.entity.kind.as_ref() else {
            continue;
        };
        let Some(lane_id) = lane.id.as_ref() else {
            continue;
        };
        let names: HashSet<String> = lane.state.iter().map(|f| f.name.clone()).collect();
        lane_state_fields.insert(pb::IdKey::new(lane_id), names);
    }

    // Index command -> set of trigger sources (used by COMMAND_HAS_NO_TRIGGER).
    // A trigger is a CommandSlice that carries BOTH persona AND ui (human),
    // OR an AutomationSlice that emits the command.
    let mut human_triggered: HashSet<pb::IdKey> = HashSet::new();
    let mut automation_triggered: HashSet<pb::IdKey> = HashSet::new();
    for se in all {
        match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::CommandSlice(cs)) => {
                let Some(cmd_id) = cs
                    .command
                    .as_ref()
                    .and_then(|e| e.command.as_ref())
                    .and_then(|r| r.id.as_ref())
                else {
                    continue;
                };
                let has_persona = cs
                    .persona
                    .as_ref()
                    .and_then(|p| p.persona.as_ref())
                    .and_then(|r| r.id.as_ref())
                    .is_some();
                let has_ui = cs
                    .ui
                    .as_ref()
                    .and_then(|u| u.ui.as_ref())
                    .and_then(|r| r.id.as_ref())
                    .is_some();
                if has_persona && has_ui {
                    human_triggered.insert(pb::IdKey::new(cmd_id));
                }
            }
            Some(pb::entity::Kind::AutomationSlice(asl)) => {
                if let Some(cmd_id) = asl
                    .emitted_command
                    .as_ref()
                    .and_then(|e| e.command.as_ref())
                    .and_then(|r| r.id.as_ref())
                {
                    automation_triggered.insert(pb::IdKey::new(cmd_id));
                }
            }
            _ => {}
        }
    }

    for m in &model.members {
        let (Some(cmd_id), Ok(pb::EntityKind::Command)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let key = pb::IdKey::new(cmd_id);
        if !human_triggered.contains(&key) && !automation_triggered.contains(&key) {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "COMMAND_HAS_NO_TRIGGER".into(),
                message: format!(
                    "command {}/{}@{} has no trigger in the model: no CommandSlice carries both persona + ui (human-triggered) and no AutomationSlice emits it (automation-triggered). Add a persona+ui to its CommandSlice, or add an AutomationSlice that fires it",
                    cmd_id.namespace, cmd_id.slug, cmd_id.version,
                ),
                subject: Some(pb::EntityRef {
                    kind: pb::EntityKind::Command as i32,
                    id: Some(cmd_id.clone()),
                }),
                subject_field: String::new(),
                ..Default::default()
            });
        }
    }

    // COMMAND_MULTIPLE_ISSUERS: a command's trigger must be unambiguous.
    // Collect distinct issuer fingerprints (UIs via CommandSlice, processors
    // via AutomationSlice) for every command claimed by this model, and emit
    // an Error whenever a command sees more than one distinct issuer.
    // Repeated references from the same issuer (e.g. the same UI used by two
    // CommandSlices on the same command) collapse to one entry, by design.
    let mut command_issuers: HashMap<pb::IdKey, BTreeSet<CommandIssuer>> = HashMap::new();
    for se in all {
        match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::CommandSlice(cs)) => {
                let Some(cmd_id) = cs
                    .command
                    .as_ref()
                    .and_then(|e| e.command.as_ref())
                    .and_then(|r| r.id.as_ref())
                else {
                    continue;
                };
                if let Some(ui_id) = cs
                    .ui
                    .as_ref()
                    .and_then(|u| u.ui.as_ref())
                    .and_then(|r| r.id.as_ref())
                {
                    command_issuers
                        .entry(pb::IdKey::new(cmd_id))
                        .or_default()
                        .insert(CommandIssuer::ui(ui_id));
                }
            }
            Some(pb::entity::Kind::AutomationSlice(asl)) => {
                let Some(cmd_id) = asl
                    .emitted_command
                    .as_ref()
                    .and_then(|e| e.command.as_ref())
                    .and_then(|r| r.id.as_ref())
                else {
                    continue;
                };
                if let Some(proc_id) = asl
                    .processor
                    .as_ref()
                    .and_then(|p| p.processor.as_ref())
                    .and_then(|r| r.id.as_ref())
                {
                    command_issuers
                        .entry(pb::IdKey::new(cmd_id))
                        .or_default()
                        .insert(CommandIssuer::processor(proc_id));
                }
            }
            _ => {}
        }
    }
    for m in &model.members {
        let (Some(cmd_id), Ok(pb::EntityKind::Command)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(issuers) = command_issuers.get(&pb::IdKey::new(cmd_id)) else {
            continue;
        };
        if issuers.len() <= 1 {
            continue;
        }
        let list = issuers
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        issues.push(pb::ValidationIssue {
            severity: pb::validation_issue::Severity::Error as i32,
            code: "COMMAND_MULTIPLE_ISSUERS".into(),
            message: format!(
                "command {}/{}@{} is issued by {} distinct issuers ({}); a command must have at most one UI and at most one processor, and the two MUST NOT coexist; split into distinct commands per issuer, or consolidate the issuers",
                cmd_id.namespace, cmd_id.slug, cmd_id.version,
                issuers.len(),
                list
            ),
            subject: Some(pb::EntityRef {
                kind: pb::EntityKind::Command as i32,
                id: Some(cmd_id.clone()),
            }),
            subject_field: String::new(),
            ..Default::default()
        });
    }

    // EVENT_MULTIPLE_EMITTERS: the emitting-side mirror of
    // COMMAND_MULTIPLE_ISSUERS, and like it a hard Error with NO annotation
    // escape. Designs fix the fan-in structurally; as-is
    // mappings surface the mapped system's own causal ambiguity as the
    // finding: the fix belongs in that system's code, not the model.
    // Emitters are fingerprinted on the COMMAND (namespace, slug) so several
    // slices of one command collapse to one, including mid-flight Decision
    // #24 migrations where command@1 and command@2 coexist. Version is not
    // part of the fingerprint: evolving one decision is not causal fan-in.
    let mut event_emitters: HashMap<pb::IdKey, BTreeSet<String>> = HashMap::new();
    for se in all {
        let Some(pb::entity::Kind::CommandSlice(cs)) = se.entity.kind.as_ref() else {
            continue;
        };
        let Some(cmd_id) = cs
            .command
            .as_ref()
            .and_then(|e| e.command.as_ref())
            .and_then(|r| r.id.as_ref())
        else {
            continue;
        };
        for ee in &cs.emitted_events {
            let Some(ev_id) = ee.event.as_ref().and_then(|r| r.id.as_ref()) else {
                continue;
            };
            event_emitters
                .entry(pb::IdKey::new(ev_id))
                .or_default()
                .insert(format!("command:{}/{}", cmd_id.namespace, cmd_id.slug));
        }
    }
    for m in &model.members {
        let (Some(ev_id), Ok(pb::EntityKind::Event)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let key = pb::IdKey::new(ev_id);
        let Some(emitters) = event_emitters.get(&key) else {
            continue;
        };
        if emitters.len() <= 1 {
            continue;
        }
        let list = emitters
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        issues.push(pb::ValidationIssue {
            severity: pb::validation_issue::Severity::Error as i32,
            code: "EVENT_MULTIPLE_EMITTERS".into(),
            message: format!(
                "event {}/{}@{} is emitted by {} distinct commands ({}); the board cannot say which decision caused the fact: for designs, fix the fan-in structurally (origin split or detector pattern); for as-is mappings, this error reports the mapped system's own causal ambiguity and the fix belongs in its code",
                ev_id.namespace, ev_id.slug, ev_id.version, emitters.len(), list
            ),
            subject: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(ev_id.clone()),
            }),
            subject_field: String::new(),
            ..Default::default()
        });
    }
    // CODE_REF_INCOMPLETE: a CodeRefAnnotation without repo + path is a
    // bookmark that resolves to nothing: it cannot be verified for drift,
    // queried, or rendered. Walk every member entity's metadata. Malformed
    // payloads are treated as absent: they must not panic.
    for m in &model.members {
        let (Some(m_id), Ok(m_kind)) = (m.id.as_ref(), pb::EntityKind::try_from(m.kind)) else {
            continue;
        };
        let key = pb::IdKey::new(m_id);
        for se in all {
            let (Some(kind), Some(sid)) = (entity_kind(&se.entity), entity_id(&se.entity)) else {
                continue;
            };
            if kind != m_kind || pb::IdKey::new(sid) != key {
                continue;
            }
            let metadata = entity_metadata(&se.entity);
            for any in metadata {
                if !any
                    .type_url
                    .ends_with("trogonatlas.annotation.v1alpha1.CodeRefAnnotation")
                {
                    continue;
                }
                let Ok(cr) =
                    <pb::CodeRefAnnotation as prost::Message>::decode(any.value.as_slice())
                else {
                    continue;
                };
                if !cr.repo.is_empty() && !cr.path.is_empty() {
                    continue;
                }
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "CODE_REF_INCOMPLETE".into(),
                    message: format!(
                        "{}:{}/{}@{} carries a CodeRefAnnotation missing {}: fill the logical repo name and repo-relative path (symbol recommended), or remove the annotation",
                        kind.as_str_name().trim_start_matches("ENTITY_KIND_").to_lowercase(),
                        m_id.namespace, m_id.slug, m_id.version,
                        if cr.repo.is_empty() && cr.path.is_empty() { "repo and path" }
                        else if cr.repo.is_empty() { "repo" } else { "path" }
                    ),
                    subject: Some(pb::EntityRef {
                        kind: m_kind as i32,
                        id: Some(m_id.clone()),
                    }),
                    subject_field: "metadata".into(),
                    ..Default::default()
                });
            }
            break;
        }
    }

    // SLICE_STALE_REF: mid-flight migration signal (Decision #24). For every
    // ref a *member* slice pins (command, ui, persona, processor, emitted/source
    // events, read_model, source_read_models), check the latest version of
    // that (kind, ns, slug) in the loaded set; if the slice pins an older
    // one, surface it so an AI / migration tool can bump the ref or
    // explicitly justify staying back. We emit one issue per stale ref so
    // subject_field pinpoints the exact field (`command`, `ui`,
    // `emitted_events[2]`, `source_read_models[0]`, …). Non-member slices
    // in `all` are out of scope for validate_model (avoids cross-model
    // pollution and validate_project duplication).
    let member_slices: HashSet<pb::EntityKey> = model
        .members
        .iter()
        .filter_map(|m| {
            let id = m.id.as_ref()?;
            let k = pb::EntityKind::try_from(m.kind).ok()?;
            matches!(
                k,
                pb::EntityKind::CommandSlice
                    | pb::EntityKind::ReadModelSlice
                    | pb::EntityKind::AutomationSlice
                    | pb::EntityKind::UiSlice
            )
            .then(|| pb::EntityKey::new(k, id))
        })
        .collect();
    // Decode a LifecycleAnnotation from an entity's metadata, returning the
    // status string (lowercased, trimmed) when the Any is well-formed.
    // Malformed payloads are treated as absent: they must not panic.
    let decode_lifecycle_status = |metadata: &[prost_types::Any]| -> Option<String> {
        for any in metadata {
            if !any
                .type_url
                .ends_with("trogonatlas.annotation.v1alpha1.LifecycleAnnotation")
            {
                continue;
            }
            if let Ok(ann) =
                <pb::LifecycleAnnotation as prost::Message>::decode(any.value.as_slice())
            {
                return Some(ann.status.to_lowercase());
            }
            return None;
        }
        None
    };
    let mut latest_version: HashMap<(pb::EntityKind, String, String), u64> = HashMap::new();
    for se in all {
        let Some(kind_field) = se.entity.kind.as_ref() else {
            continue;
        };
        let (kind, id_opt, metadata) = match kind_field {
            pb::entity::Kind::Event(e) => {
                (pb::EntityKind::Event, e.id.as_ref(), e.metadata.as_slice())
            }
            pb::entity::Kind::Command(c) => (
                pb::EntityKind::Command,
                c.id.as_ref(),
                c.metadata.as_slice(),
            ),
            pb::entity::Kind::ReadModel(r) => (
                pb::EntityKind::ReadModel,
                r.id.as_ref(),
                r.metadata.as_slice(),
            ),
            pb::entity::Kind::Processor(p) => (
                pb::EntityKind::Processor,
                p.id.as_ref(),
                p.metadata.as_slice(),
            ),
            pb::entity::Kind::Ui(u) => (pb::EntityKind::Ui, u.id.as_ref(), u.metadata.as_slice()),
            pb::entity::Kind::Persona(p) => (
                pb::EntityKind::Persona,
                p.id.as_ref(),
                p.metadata.as_slice(),
            ),
            _ => continue,
        };
        // Skip entities whose lifecycle status marks them as not-yet-landed.
        // A draft version must not surface SLICE_STALE_REF across the model
        // before it lands. Use DRAFT_LIFECYCLE_STATUSES to control the set.
        if let Some(status) = decode_lifecycle_status(metadata) {
            if DRAFT_LIFECYCLE_STATUSES.contains(&status.trim()) {
                continue;
            }
        }
        let Some(id) = id_opt else { continue };
        let key = (kind, id.namespace.clone(), id.slug.clone());
        let entry = latest_version.entry(key).or_insert(0);
        if id.version > *entry {
            *entry = id.version;
        }
    }
    let stale_against = |kind: pb::EntityKind, id: &pb::Id| -> Option<u64> {
        let key = (kind, id.namespace.clone(), id.slug.clone());
        latest_version
            .get(&key)
            .copied()
            .filter(|latest| *latest > id.version)
    };
    let stale_issue = |slice_kind: pb::EntityKind,
                       slice_id: &pb::Id,
                       ref_kind: pb::EntityKind,
                       ref_id: &pb::Id,
                       field: String,
                       latest: u64| {
        pb::ValidationIssue {
            severity: pb::validation_issue::Severity::Warning as i32,
            code: "SLICE_STALE_REF".into(),
            message: format!(
                "slice {}/{}@{} pins {} {}/{}@{}, but latest loaded version is {}: bump the ref or justify staying back",
                slice_id.namespace, slice_id.slug, slice_id.version,
                pb::canonical::kind_short(ref_kind),
                ref_id.namespace, ref_id.slug, ref_id.version,
                latest,
            ),
            subject: Some(pb::EntityRef {
                kind: slice_kind as i32,
                id: Some(slice_id.clone()),
            }),
            subject_field: field,
            ..Default::default()
        }
    };
    // Decode a PinnedRefAnnotation from a slice's metadata, returning the
    // decoded annotation when doc is non-empty and the Any is well-formed.
    // Malformed payloads are treated as absent: they must not panic or suppress
    // findings accidentally.
    let decode_pinned_ann = |metadata: &[prost_types::Any]| -> Option<pb::PinnedRefAnnotation> {
        for any in metadata {
            if !any
                .type_url
                .ends_with("trogonatlas.eventmodel.v1alpha1.PinnedRefAnnotation")
            {
                continue;
            }
            if let Ok(ann) =
                <pb::PinnedRefAnnotation as prost::Message>::decode(any.value.as_slice())
            {
                if !ann.doc.is_empty() {
                    return Some(ann);
                }
            }
            // Present but empty doc or malformed: does not cover anything.
            return None;
        }
        None
    };
    // Returns true when the PinnedRefAnnotation on the slice covers the given
    // ref id. The canonical id string format is "<namespace>/<slug>@<version>"
    // matching the OLD pinned version (the version the slice currently pins).
    // When the annotation's pinned list is empty, every stale ref is covered
    // (blanket pin). When the list is non-empty, only listed ids are covered.
    let is_pinned = |ann: &pb::PinnedRefAnnotation, ref_id: &pb::Id| -> bool {
        if ann.pinned.is_empty() {
            return true;
        }
        let canonical = format!("{}/{}@{}", ref_id.namespace, ref_id.slug, ref_id.version);
        ann.pinned.iter().any(|p| p == &canonical)
    };
    for se in all {
        match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::CommandSlice(cs)) => {
                let Some(slice_id) = cs.id.as_ref() else {
                    continue;
                };
                if !member_slices
                    .contains(&pb::EntityKey::new(pb::EntityKind::CommandSlice, slice_id))
                {
                    continue;
                }
                let pin_ann = decode_pinned_ann(&cs.metadata);
                if let Some(r) = cs
                    .persona
                    .as_ref()
                    .and_then(|e| e.persona.as_ref())
                    .and_then(|r| r.id.as_ref())
                {
                    if let Some(latest) = stale_against(pb::EntityKind::Persona, r) {
                        if !pin_ann.as_ref().is_some_and(|a| is_pinned(a, r)) {
                            issues.push(stale_issue(
                                pb::EntityKind::CommandSlice,
                                slice_id,
                                pb::EntityKind::Persona,
                                r,
                                "persona".into(),
                                latest,
                            ));
                        }
                    }
                }
                if let Some(r) = cs
                    .ui
                    .as_ref()
                    .and_then(|e| e.ui.as_ref())
                    .and_then(|r| r.id.as_ref())
                {
                    if let Some(latest) = stale_against(pb::EntityKind::Ui, r) {
                        if !pin_ann.as_ref().is_some_and(|a| is_pinned(a, r)) {
                            issues.push(stale_issue(
                                pb::EntityKind::CommandSlice,
                                slice_id,
                                pb::EntityKind::Ui,
                                r,
                                "ui".into(),
                                latest,
                            ));
                        }
                    }
                }
                if let Some(r) = cs
                    .command
                    .as_ref()
                    .and_then(|e| e.command.as_ref())
                    .and_then(|r| r.id.as_ref())
                {
                    if let Some(latest) = stale_against(pb::EntityKind::Command, r) {
                        if !pin_ann.as_ref().is_some_and(|a| is_pinned(a, r)) {
                            issues.push(stale_issue(
                                pb::EntityKind::CommandSlice,
                                slice_id,
                                pb::EntityKind::Command,
                                r,
                                "command".into(),
                                latest,
                            ));
                        }
                    }
                }
                for (i, ee) in cs.emitted_events.iter().enumerate() {
                    if let Some(r) = ee.event.as_ref().and_then(|r| r.id.as_ref()) {
                        if let Some(latest) = stale_against(pb::EntityKind::Event, r) {
                            if !pin_ann.as_ref().is_some_and(|a| is_pinned(a, r)) {
                                issues.push(stale_issue(
                                    pb::EntityKind::CommandSlice,
                                    slice_id,
                                    pb::EntityKind::Event,
                                    r,
                                    format!("emitted_events[{i}]"),
                                    latest,
                                ));
                            }
                        }
                    }
                }
            }
            Some(pb::entity::Kind::ReadModelSlice(rs)) => {
                let Some(slice_id) = rs.id.as_ref() else {
                    continue;
                };
                if !member_slices.contains(&pb::EntityKey::new(
                    pb::EntityKind::ReadModelSlice,
                    slice_id,
                )) {
                    continue;
                }
                let pin_ann = decode_pinned_ann(&rs.metadata);
                for (i, se_ref) in rs.source_events.iter().enumerate() {
                    if let Some(r) = se_ref.event.as_ref().and_then(|r| r.id.as_ref()) {
                        if let Some(latest) = stale_against(pb::EntityKind::Event, r) {
                            if !pin_ann.as_ref().is_some_and(|a| is_pinned(a, r)) {
                                issues.push(stale_issue(
                                    pb::EntityKind::ReadModelSlice,
                                    slice_id,
                                    pb::EntityKind::Event,
                                    r,
                                    format!("source_events[{i}]"),
                                    latest,
                                ));
                            }
                        }
                    }
                }
                if let Some(r) = rs
                    .read_model
                    .as_ref()
                    .and_then(|e| e.read_model.as_ref())
                    .and_then(|r| r.id.as_ref())
                {
                    if let Some(latest) = stale_against(pb::EntityKind::ReadModel, r) {
                        if !pin_ann.as_ref().is_some_and(|a| is_pinned(a, r)) {
                            issues.push(stale_issue(
                                pb::EntityKind::ReadModelSlice,
                                slice_id,
                                pb::EntityKind::ReadModel,
                                r,
                                "read_model".into(),
                                latest,
                            ));
                        }
                    }
                }
            }
            Some(pb::entity::Kind::UiSlice(us)) => {
                let Some(slice_id) = us.id.as_ref() else {
                    continue;
                };
                if !member_slices.contains(&pb::EntityKey::new(pb::EntityKind::UiSlice, slice_id)) {
                    continue;
                }
                let pin_ann = decode_pinned_ann(&us.metadata);
                if let Some(r) = us
                    .persona
                    .as_ref()
                    .and_then(|e| e.persona.as_ref())
                    .and_then(|r| r.id.as_ref())
                {
                    if let Some(latest) = stale_against(pb::EntityKind::Persona, r) {
                        if !pin_ann.as_ref().is_some_and(|a| is_pinned(a, r)) {
                            issues.push(stale_issue(
                                pb::EntityKind::UiSlice,
                                slice_id,
                                pb::EntityKind::Persona,
                                r,
                                "persona".into(),
                                latest,
                            ));
                        }
                    }
                }
                for (i, rm_ref) in us.source_read_models.iter().enumerate() {
                    if let Some(r) = rm_ref.read_model.as_ref().and_then(|r| r.id.as_ref()) {
                        if let Some(latest) = stale_against(pb::EntityKind::ReadModel, r) {
                            if !pin_ann.as_ref().is_some_and(|a| is_pinned(a, r)) {
                                issues.push(stale_issue(
                                    pb::EntityKind::UiSlice,
                                    slice_id,
                                    pb::EntityKind::ReadModel,
                                    r,
                                    format!("source_read_models[{i}]"),
                                    latest,
                                ));
                            }
                        }
                    }
                }
                if let Some(r) = us
                    .ui
                    .as_ref()
                    .and_then(|e| e.ui.as_ref())
                    .and_then(|r| r.id.as_ref())
                {
                    if let Some(latest) = stale_against(pb::EntityKind::Ui, r) {
                        if !pin_ann.as_ref().is_some_and(|a| is_pinned(a, r)) {
                            issues.push(stale_issue(
                                pb::EntityKind::UiSlice,
                                slice_id,
                                pb::EntityKind::Ui,
                                r,
                                "ui".into(),
                                latest,
                            ));
                        }
                    }
                }
            }
            Some(pb::entity::Kind::AutomationSlice(asl)) => {
                let Some(slice_id) = asl.id.as_ref() else {
                    continue;
                };
                if !member_slices.contains(&pb::EntityKey::new(
                    pb::EntityKind::AutomationSlice,
                    slice_id,
                )) {
                    continue;
                }
                let pin_ann = decode_pinned_ann(&asl.metadata);
                for (i, rm_ref) in asl.source_read_models.iter().enumerate() {
                    if let Some(r) = rm_ref.read_model.as_ref().and_then(|r| r.id.as_ref()) {
                        if let Some(latest) = stale_against(pb::EntityKind::ReadModel, r) {
                            if !pin_ann.as_ref().is_some_and(|a| is_pinned(a, r)) {
                                issues.push(stale_issue(
                                    pb::EntityKind::AutomationSlice,
                                    slice_id,
                                    pb::EntityKind::ReadModel,
                                    r,
                                    format!("source_read_models[{i}]"),
                                    latest,
                                ));
                            }
                        }
                    }
                }
                if let Some(r) = asl
                    .processor
                    .as_ref()
                    .and_then(|e| e.processor.as_ref())
                    .and_then(|r| r.id.as_ref())
                {
                    if let Some(latest) = stale_against(pb::EntityKind::Processor, r) {
                        if !pin_ann.as_ref().is_some_and(|a| is_pinned(a, r)) {
                            issues.push(stale_issue(
                                pb::EntityKind::AutomationSlice,
                                slice_id,
                                pb::EntityKind::Processor,
                                r,
                                "processor".into(),
                                latest,
                            ));
                        }
                    }
                }
                if let Some(r) = asl
                    .emitted_command
                    .as_ref()
                    .and_then(|e| e.command.as_ref())
                    .and_then(|r| r.id.as_ref())
                {
                    if let Some(latest) = stale_against(pb::EntityKind::Command, r) {
                        if !pin_ann.as_ref().is_some_and(|a| is_pinned(a, r)) {
                            issues.push(stale_issue(
                                pb::EntityKind::AutomationSlice,
                                slice_id,
                                pb::EntityKind::Command,
                                r,
                                "emitted_command".into(),
                                latest,
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // UI_SLICE_NO_READ_MODEL: the display-side mirror of an automation that
    // observes nothing. A UiSlice is the moment a persona sees something; with
    // an empty source_read_models it puts a screen in the flow that nothing
    // ever reaches.
    for se in all {
        let Some(pb::entity::Kind::UiSlice(us)) = se.entity.kind.as_ref() else {
            continue;
        };
        let Some(slice_id) = us.id.as_ref() else {
            continue;
        };
        if !member_slices.contains(&pb::EntityKey::new(pb::EntityKind::UiSlice, slice_id)) {
            continue;
        }
        if !us.source_read_models.is_empty() {
            continue;
        }
        issues.push(pb::ValidationIssue {
            severity: pb::validation_issue::Severity::Error as i32,
            code: "UI_SLICE_NO_READ_MODEL".into(),
            message: format!(
                "ui slice {}/{}@{} displays no read model: a screen shows what the model already projected; add a source_read_models entry naming the ReadModel it renders",
                slice_id.namespace, slice_id.slug, slice_id.version,
            ),
            subject: Some(pb::EntityRef {
                kind: pb::EntityKind::UiSlice as i32,
                id: Some(slice_id.clone()),
            }),
            subject_field: "source_read_models".into(),
            ..Default::default()
        });
    }

    // EVENT_NO_SWIMLANE: every event member must declare a swimlane and that
    // swimlane must exist in the store. Without a stream identity, the event
    // has no aggregate and no runtime can write or project it. We fire on
    // (a) missing swimlane field and (b) swimlane reference that doesn't
    // resolve in `all`. The studio silently routes such events to a
    // catch-all "Events" lane; the server should be the authority that says
    // this is broken.
    let swimlane_ids: HashSet<(String, String)> = all
        .iter()
        .filter_map(|se| match se.entity.kind.as_ref()? {
            pb::entity::Kind::Swimlane(s) => {
                let id = s.id.as_ref()?;
                Some((id.namespace.clone(), id.slug.clone()))
            }
            _ => None,
        })
        .collect();
    for m in &model.members {
        let (Some(ev_id), Ok(pb::EntityKind::Event)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(ev) = all.iter().find_map(|se| match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::Event(e)) if e.id.as_ref() == Some(ev_id) => Some(e),
            _ => None,
        }) else {
            continue;
        };
        let lane = ev.swimlane.as_ref().and_then(|r| r.id.as_ref());
        match lane {
            None => {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "EVENT_NO_SWIMLANE".into(),
                    message: format!(
                        "event {}/{}@{} declares no swimlane: it has no stream identity and cannot be written or projected. Set Event.swimlane to a real Swimlane reference",
                        ev_id.namespace, ev_id.slug, ev_id.version,
                    ),
                    subject: Some(pb::EntityRef {
                        kind: pb::EntityKind::Event as i32,
                        id: Some(ev_id.clone()),
                    }),
                    subject_field: "swimlane".into(),
                    ..Default::default()
                });
            }
            Some(lane_id) => {
                if !swimlane_ids.contains(&(lane_id.namespace.clone(), lane_id.slug.clone())) {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "EVENT_NO_SWIMLANE".into(),
                        message: format!(
                            "event {}/{}@{} points at swimlane {}/{} which does not exist in the store; the stream identity is dangling. Add the Swimlane entity, or repoint the event at an existing one",
                            ev_id.namespace, ev_id.slug, ev_id.version,
                            lane_id.namespace, lane_id.slug,
                        ),
                        subject: Some(pb::EntityRef {
                            kind: pb::EntityKind::Event as i32,
                            id: Some(ev_id.clone()),
                        }),
                        subject_field: "swimlane".into(),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // UI_NO_PERSONA: the counterpart of EVENT_NO_SWIMLANE for screens.
    // A Ui has no owner field of its own, so "who operates this screen" is
    // inferred from the three places the schema pairs a persona WITH a ui:
    // a UiSlice carrying both, a CommandSlice carrying both, or a
    // StoryboardEntry.HumanObserver naming both. A display-only screen
    // names its persona on its UiSlice; the storyboard entry is a fallback,
    // no longer the only route.
    //
    // This definition is mirrored EXACTLY by `buildLaneMap` in the studio
    // (webapps/studio/src/lib/layout.ts): the studio routes an unowned screen to a
    // catch-all `ui:unassigned` lane, and the server is the authority that
    // says that lane is a broken state, not a home. The two must agree.
    let persona_ids: HashSet<(String, String)> = all
        .iter()
        .filter_map(|se| match se.entity.kind.as_ref()? {
            pb::entity::Kind::Persona(p) => {
                let id = p.id.as_ref()?;
                Some((id.namespace.clone(), id.slug.clone()))
            }
            _ => None,
        })
        .collect();
    // ui key -> the persona the model attributes it to.
    let mut ui_persona: HashMap<(String, String), pb::Id> = HashMap::new();
    for se in all {
        match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::CommandSlice(cs)) => {
                if let (Some(u), Some(p)) = (
                    cs.ui
                        .as_ref()
                        .and_then(|e| e.ui.as_ref())
                        .and_then(|r| r.id.as_ref()),
                    cs.persona
                        .as_ref()
                        .and_then(|e| e.persona.as_ref())
                        .and_then(|r| r.id.as_ref()),
                ) {
                    ui_persona.insert((u.namespace.clone(), u.slug.clone()), p.clone());
                }
            }
            Some(pb::entity::Kind::UiSlice(us)) => {
                if let (Some(u), Some(p)) = (
                    us.ui
                        .as_ref()
                        .and_then(|e| e.ui.as_ref())
                        .and_then(|r| r.id.as_ref()),
                    us.persona
                        .as_ref()
                        .and_then(|e| e.persona.as_ref())
                        .and_then(|r| r.id.as_ref()),
                ) {
                    ui_persona.insert((u.namespace.clone(), u.slug.clone()), p.clone());
                }
            }
            Some(pb::entity::Kind::Storyboard(sb)) => {
                let Some(pb::storyboard_entry::Observer::Human(h)) =
                    sb.entry.as_ref().and_then(|e| e.observer.as_ref())
                else {
                    continue;
                };
                if let (Some(u), Some(p)) = (
                    h.ui.as_ref()
                        .and_then(|e| e.ui.as_ref())
                        .and_then(|r| r.id.as_ref()),
                    h.persona
                        .as_ref()
                        .and_then(|e| e.persona.as_ref())
                        .and_then(|r| r.id.as_ref()),
                ) {
                    ui_persona.insert((u.namespace.clone(), u.slug.clone()), p.clone());
                }
            }
            _ => {}
        }
    }
    for m in &model.members {
        let (Some(ui_id), Ok(pb::EntityKind::Ui)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        // Mirror EVENT_NO_SWIMLANE: a member that does not resolve in the
        // store is MEMBER_NOT_FOUND's problem, not ours. Reporting both
        // would double-bill one broken reference.
        if !all.iter().any(|se| match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::Ui(u)) => u.id.as_ref() == Some(ui_id),
            _ => false,
        }) {
            continue;
        }
        let key = (ui_id.namespace.clone(), ui_id.slug.clone());
        match ui_persona.get(&key) {
            None => {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "UI_NO_PERSONA".into(),
                    message: format!(
                        "ui {}/{}@{} is owned by no persona: no UiSlice or CommandSlice pairs it with a persona and no storyboard entry observer watches it, so the board cannot say whose lane it belongs to. Add persona + ui to the UiSlice that displays it or the CommandSlice that acts on it, or declare a StoryboardEntry.HumanObserver naming the persona that watches it",
                        ui_id.namespace, ui_id.slug, ui_id.version,
                    ),
                    subject: Some(pb::EntityRef {
                        kind: pb::EntityKind::Ui as i32,
                        id: Some(ui_id.clone()),
                    }),
                    subject_field: "persona".into(),
                    ..Default::default()
                });
            }
            Some(persona_id) => {
                if !persona_ids.contains(&(persona_id.namespace.clone(), persona_id.slug.clone())) {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "UI_NO_PERSONA".into(),
                        message: format!(
                            "ui {}/{}@{} is attributed to persona {}/{} which does not exist in the store; the ownership is dangling. Add the Persona entity, or repoint the attribution at an existing one",
                            ui_id.namespace, ui_id.slug, ui_id.version,
                            persona_id.namespace, persona_id.slug,
                        ),
                        subject: Some(pb::EntityRef {
                            kind: pb::EntityKind::Ui as i32,
                            id: Some(ui_id.clone()),
                        }),
                        subject_field: "persona".into(),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // EVENT_FIELD_UNSOURCED: per emitted event in each CommandSlice.
    for m in &model.members {
        let (Some(slice_id), Ok(pb::EntityKind::CommandSlice)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(cs) = all.iter().find_map(|se| match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::CommandSlice(c)) if c.id.as_ref() == Some(slice_id) => Some(c),
            _ => None,
        }) else {
            continue;
        };
        let Some(cmd_id) = cs
            .command
            .as_ref()
            .and_then(|e| e.command.as_ref())
            .and_then(|r| r.id.as_ref())
        else {
            continue;
        };
        let Some(cmd) = commands_by_id.get(&pb::IdKey::new(cmd_id)) else {
            continue;
        };
        let cmd_field_names: HashSet<&str> = declared(&command_fields, cmd.id.as_ref())
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        let lane_id = cmd.swimlane.as_ref().and_then(|s| s.id.as_ref());
        let lane_cmd_fields = lane_id.and_then(|l| lane_command_fields.get(&pb::IdKey::new(l)));
        let lane_state = lane_id.and_then(|l| lane_state_fields.get(&pb::IdKey::new(l)));
        // lane_events is built for diagnostics only: peer-event
        // sourcing is name collision, not data flow.
        let _ = &lane_events;
        for (ei, ee) in cs.emitted_events.iter().enumerate() {
            let Some(ev_id) = ee.event.as_ref().and_then(|r| r.id.as_ref()) else {
                continue;
            };
            let Some(event) = events_by_id.get(&pb::IdKey::new(ev_id)) else {
                continue;
            };
            for (fi, field) in declared(&event_fields, event.id.as_ref())
                .iter()
                .enumerate()
            {
                if cmd_field_names.contains(field.name.as_str()) {
                    continue;
                }
                if is_derived_field(field) {
                    continue;
                }
                if let Some(set) = lane_cmd_fields {
                    if set.contains(&field.name) {
                        continue;
                    }
                }
                // 4th source: the aggregate's declared state on the
                // swimlane. The lane's runtime owns these fields
                // (LLM picks, generated UUIDs, clocks, derived
                // projections of prior events); any event on the
                // lane may emit them.
                if let Some(state) = lane_state {
                    if state.contains(&field.name) {
                        continue;
                    }
                }
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "EVENT_FIELD_UNSOURCED".into(),
                    message: format!(
                        "event {}/{}@{} field `{}` (emitted by command slice {}/{}@{} via command {}/{}@{}) has no source in the model: not on the command, not on any event/command of swimlane {}, and not marked DerivedFieldAnnotation. Add the field to the command, declare it on another entity on the lane, or annotate the field as system-derived",
                        ev_id.namespace, ev_id.slug, ev_id.version,
                        field.name,
                        slice_id.namespace, slice_id.slug, slice_id.version,
                        cmd_id.namespace, cmd_id.slug, cmd_id.version,
                        lane_id.map_or_else(|| "<unset>".into(), |l| format!("{}/{}", l.namespace, l.slug)),
                    ),
                    subject: Some(pb::EntityRef {
                        kind: pb::EntityKind::CommandSlice as i32,
                        id: Some(slice_id.clone()),
                    }),
                    subject_field: format!("emitted_events[{ei}].declared(&event_fields, event.id.as_ref())[{fi}]"),
                    ..Default::default()
                });
            }
        }
    }

    // AUTOMATION_COMMAND_FIELD_UNSOURCED: per field of the command an
    // AutomationSlice emits, must be present in the union of fields of
    // source_read_models or annotated as derived.
    for m in &model.members {
        let (Some(slice_id), Ok(pb::EntityKind::AutomationSlice)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(asl) = all.iter().find_map(|se| match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::AutomationSlice(a)) if a.id.as_ref() == Some(slice_id) => {
                Some(a)
            }
            _ => None,
        }) else {
            continue;
        };
        let Some(cmd_id) = asl
            .emitted_command
            .as_ref()
            .and_then(|e| e.command.as_ref())
            .and_then(|r| r.id.as_ref())
        else {
            continue;
        };
        let Some(cmd) = commands_by_id.get(&pb::IdKey::new(cmd_id)) else {
            continue;
        };
        let mut rm_field_names: HashSet<&str> = HashSet::new();
        for sr in &asl.source_read_models {
            let Some(rm_id) = sr.read_model.as_ref().and_then(|r| r.id.as_ref()) else {
                continue;
            };
            let Some(rm) = rms_by_id.get(&pb::IdKey::new(rm_id)) else {
                continue;
            };
            for f in declared(&rm_fields, rm.id.as_ref()) {
                rm_field_names.insert(f.name.as_str());
            }
        }
        for (fi, field) in declared(&command_fields, cmd.id.as_ref())
            .iter()
            .enumerate()
        {
            if rm_field_names.contains(field.name.as_str()) {
                continue;
            }
            if is_derived_field(field) {
                continue;
            }
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "AUTOMATION_COMMAND_FIELD_UNSOURCED".into(),
                message: format!(
                    "automation slice {}/{}@{} fires command {}/{}@{} whose field `{}` cannot be sourced from any of its source_read_models. Add the field to one of the source RMs, or route a different RM that carries it",
                    slice_id.namespace, slice_id.slug, slice_id.version,
                    cmd_id.namespace, cmd_id.slug, cmd_id.version,
                    field.name,
                ),
                subject: Some(pb::EntityRef {
                    kind: pb::EntityKind::AutomationSlice as i32,
                    id: Some(slice_id.clone()),
                }),
                subject_field: format!("emitted_command.command.schema.fields[{fi}]"),
                ..Default::default()
            });
        }
    }

    // ===== PASS: read_model_field_provenance =====
    // Every field a ReadModel declares must be supplied by at least
    // one of its source events (the event has a FieldSpec with that
    // name) OR by the swimlane.state of one of the source events'
    // lanes. Otherwise the RM is lying: the projection cannot
    // populate the field, and any AutomationSlice that consumes the
    // RM is reading a value that doesn't actually exist.
    for m in &model.members {
        let (Some(rm_id), Ok(pb::EntityKind::ReadModel)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(rm) = rms_by_id.get(&pb::IdKey::new(rm_id)) else {
            continue;
        };
        // Union of (source event field names) + (each source event's
        // lane's state field names).
        let mut available: HashSet<String> = HashSet::new();
        for sr in &rm.source_events {
            let Some(ev_id) = sr.id.as_ref() else {
                continue;
            };
            if let Some(ev) = events_by_id.get(&pb::IdKey::new(ev_id)) {
                for f in declared(&event_fields, ev.id.as_ref()) {
                    available.insert(f.name.clone());
                }
                if let Some(lane_ref) = ev.swimlane.as_ref().and_then(|s| s.id.as_ref()) {
                    if let Some(state) = lane_state_fields.get(&pb::IdKey::new(lane_ref)) {
                        for n in state {
                            available.insert(n.clone());
                        }
                    }
                }
            }
        }
        for (fi, field) in declared(&rm_fields, rm.id.as_ref()).iter().enumerate() {
            if available.contains(&field.name) {
                continue;
            }
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "READ_MODEL_FIELD_UNSOURCED".into(),
                message: format!(
                    "read model {}/{}@{} declares field `{}` but none of its source events carries that field and no source event's swimlane.state declares it. The RM's projection cannot populate this value: add the field to a source event, switch source events, declare it on the source event's lane via swimlane state, or drop it from the RM",
                    rm_id.namespace, rm_id.slug, rm_id.version, field.name,
                ),
                subject: Some(pb::EntityRef {
                    kind: pb::EntityKind::ReadModel as i32,
                    id: Some(rm_id.clone()),
                }),
                subject_field: format!("schema.fields[{fi}]"),
                ..Default::default()
            });
        }
    }

    // ===== PASS: rm_key_integrity =====
    // (1) RM_KEY_NOT_A_FIELD: every name in ReadModel.key must name a
    //     FieldSpec in ReadModel.fields. A typo or stale name here
    //     means the projection can never locate the row.
    // (2) RM_KEY_MISSING: a ReadModel that has fields but no declared
    //     key cannot have its row identity validated. Surfaced as Info
    //     during the migration window; candidate for Warning once the
    //     store is fully migrated.
    for m in &model.members {
        let (Some(rm_id), Ok(pb::EntityKind::ReadModel)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(rm) = rms_by_id.get(&pb::IdKey::new(rm_id)) else {
            continue;
        };
        let field_names: HashSet<&str> = declared(&rm_fields, rm.id.as_ref())
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        for key_name in &rm.key {
            if !field_names.contains(key_name.as_str()) {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "RM_KEY_NOT_A_FIELD".into(),
                    message: format!(
                        "read model {}/{}@{} lists `{}` in its key but that name is not declared in its fields: every key name must appear in fields",
                        rm_id.namespace, rm_id.slug, rm_id.version, key_name,
                    ),
                    subject: Some(pb::EntityRef {
                        kind: pb::EntityKind::ReadModel as i32,
                        id: Some(rm_id.clone()),
                    }),
                    subject_field: "key".into(),
                    ..Default::default()
                });
            }
        }
        if rm.key.is_empty() && !declared(&rm_fields, rm.id.as_ref()).is_empty() {
            // Migration-window posture: Info now, candidate for Warning
            // once all read models in the store carry a declared key.
            // To promote: confirm no RM_KEY_MISSING issues appear in
            // ValidateProject output, then change the severity below to
            // Warning so warning-gated CI will catch regressions.
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Info as i32,
                code: "RM_KEY_MISSING".into(),
                message: format!(
                    "read model {}/{}@{} declares fields but no key: row identity is undeclared, so key-sourcing checks are suppressed",
                    rm_id.namespace, rm_id.slug, rm_id.version,
                ),
                subject: Some(pb::EntityRef {
                    kind: pb::EntityKind::ReadModel as i32,
                    id: Some(rm_id.clone()),
                }),
                subject_field: "key".into(),
                ..Default::default()
            });
        }
    }

    // ===== PASS: rm_slice_key_sourcing =====
    // When a ReadModel declares a non-empty key, every ReadModelSlice
    // that projects into it must source ALL key fields from its
    // source_events. Key fields come from the event's own FieldSpec
    // names only; lane-state sourcing can be added if a concrete model
    // needs it.
    //
    // Both UPSERT and DELETE roles require the key: a delete that
    // cannot name the row is as broken as an upsert that cannot write
    // one.
    //
    // One issue per (slice, event, missing key field) so the caller
    // can see exactly which (event, field) pair is incomplete.
    for m in &model.members {
        let (Some(slice_id), Ok(pb::EntityKind::ReadModelSlice)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(rs) = all.iter().find_map(|se| match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::ReadModelSlice(s)) if s.id.as_ref() == Some(slice_id) => Some(s),
            _ => None,
        }) else {
            continue;
        };
        let Some(rm_id) = rs
            .read_model
            .as_ref()
            .and_then(|x| x.read_model.as_ref())
            .and_then(|r| r.id.as_ref())
        else {
            continue;
        };
        let Some(rm) = rms_by_id.get(&pb::IdKey::new(rm_id)) else {
            continue;
        };
        if rm.key.is_empty() {
            continue;
        }
        let rm_keys: HashSet<&str> = rm.key.iter().map(std::string::String::as_str).collect();
        let slice_subject = Some(pb::EntityRef {
            kind: pb::EntityKind::ReadModelSlice as i32,
            id: Some(slice_id.clone()),
        });
        for (ei, se_ref) in rs.source_events.iter().enumerate() {
            let Some(ev_id) = se_ref.event.as_ref().and_then(|r| r.id.as_ref()) else {
                continue;
            };
            let ev_fields: HashSet<&str> = events_by_id
                .get(&pb::IdKey::new(ev_id))
                .map(|ev| {
                    declared(&event_fields, ev.id.as_ref())
                        .iter()
                        .map(|f| f.name.as_str())
                        .collect()
                })
                .unwrap_or_default();
            for key_name in &rm_keys {
                if !ev_fields.contains(key_name) {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "RM_SLICE_KEY_UNSOURCED".into(),
                        message: format!(
                            "read model slice {}/{}@{} source_events[{ei}] ({}/{}@{}) does not carry key field `{}` required by read model {}/{}@{}: the projection cannot locate the target row",
                            slice_id.namespace, slice_id.slug, slice_id.version,
                            ev_id.namespace, ev_id.slug, ev_id.version,
                            key_name,
                            rm_id.namespace, rm_id.slug, rm_id.version,
                        ),
                        subject: slice_subject.clone(),
                        subject_field: format!("source_events[{ei}]"),
                        ..Default::default()
                    });
                }
            }
        }
        // RM_SLICE_ROLE_MISSING: the slice targets a keyed RM but has
        // not declared its projection intent.
        if rs.projection_role == pb::ProjectionRole::Unspecified as i32 {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Info as i32,
                code: "RM_SLICE_ROLE_MISSING".into(),
                message: format!(
                    "read model slice {}/{}@{} targets keyed read model {}/{}@{} but has PROJECTION_ROLE_UNSPECIFIED: declare UPSERT or DELETE",
                    slice_id.namespace, slice_id.slug, slice_id.version,
                    rm_id.namespace, rm_id.slug, rm_id.version,
                ),
                subject: slice_subject,
                subject_field: "projection_role".into(),
                ..Default::default()
            });
        }
    }

    // ===== PASS: command_field_destinations =====
    // Every field the command carries must either be recorded by one
    // of the slice's emitted_events OR appear in the command's
    // swimlane.state. Otherwise the runtime takes the value in and
    // the model has no record of what was done with it.
    for m in &model.members {
        let (Some(slice_id), Ok(pb::EntityKind::CommandSlice)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(cs) = all.iter().find_map(|se| match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::CommandSlice(c)) if c.id.as_ref() == Some(slice_id) => Some(c),
            _ => None,
        }) else {
            continue;
        };
        let Some(cmd_id) = cs
            .command
            .as_ref()
            .and_then(|e| e.command.as_ref())
            .and_then(|r| r.id.as_ref())
        else {
            continue;
        };
        let Some(cmd) = commands_by_id.get(&pb::IdKey::new(cmd_id)) else {
            continue;
        };
        // Names the slice's emitted_events declare.
        let mut emitted_field_names: HashSet<&str> = HashSet::new();
        for ee in &cs.emitted_events {
            let Some(ev_id) = ee.event.as_ref().and_then(|r| r.id.as_ref()) else {
                continue;
            };
            if let Some(ev) = events_by_id.get(&pb::IdKey::new(ev_id)) {
                for f in declared(&event_fields, ev.id.as_ref()) {
                    emitted_field_names.insert(f.name.as_str());
                }
            }
        }
        // Names the command's lane state declares.
        let lane_state = cmd
            .swimlane
            .as_ref()
            .and_then(|s| s.id.as_ref())
            .and_then(|l| lane_state_fields.get(&pb::IdKey::new(l)));
        for (fi, field) in declared(&command_fields, cmd.id.as_ref())
            .iter()
            .enumerate()
        {
            if emitted_field_names.contains(field.name.as_str()) {
                continue;
            }
            if let Some(state) = lane_state {
                if state.contains(&field.name) {
                    continue;
                }
            }
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Warning as i32,
                code: "COMMAND_FIELD_NEVER_RECORDED".into(),
                message: format!(
                    "command slice {}/{}@{} fires command {}/{}@{} which carries field `{}`, but no event the slice emits records that field, and the command's swimlane.state does not declare it either. The runtime takes the value in and the model has no record of what happened to it: add the field to one of the emitted events, declare it on the lane's swimlane.state, or drop it from the command",
                    slice_id.namespace, slice_id.slug, slice_id.version,
                    cmd_id.namespace, cmd_id.slug, cmd_id.version,
                    field.name,
                ),
                subject: Some(pb::EntityRef {
                    kind: pb::EntityKind::CommandSlice as i32,
                    id: Some(slice_id.clone()),
                }),
                subject_field: format!("command.command.schema.fields[{fi}]"),
                ..Default::default()
            });
        }
    }

    // A surface has one role per moment: the UI a CommandSlice triggers
    // from (input/control) is a different shape from the UI a UiSlice
    // renders into (output/display). When the SAME UI shows up in both
    // slots, the model is usually leaking a modeling mistake, typically a
    // form being used as a render target when a dedicated history/detail
    // view is needed, or a display screen being shoe-horned into the
    // command-trigger slot because the form wasn't modeled.
    //
    // Legitimate mixed surfaces exist (order tickets, dashboards). Opt out
    // by attaching a MixedSurfaceAnnotation with a non-empty doc to the UI
    // entity, explaining why both roles really do co-locate.
    let mut ui_as_trigger: HashMap<pb::IdKey, Vec<String>> = HashMap::new();
    let mut ui_as_render: HashMap<pb::IdKey, Vec<String>> = HashMap::new();
    for se in all {
        match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::CommandSlice(cs)) => {
                if let (Some(slice_id), Some(ui_id)) = (
                    cs.id.as_ref(),
                    cs.ui
                        .as_ref()
                        .and_then(|u| u.ui.as_ref())
                        .and_then(|r| r.id.as_ref()),
                ) {
                    ui_as_trigger
                        .entry(pb::IdKey::new(ui_id))
                        .or_default()
                        .push(slice_id.slug.clone());
                }
            }
            Some(pb::entity::Kind::UiSlice(us)) => {
                if let (Some(slice_id), Some(ui_id)) = (
                    us.id.as_ref(),
                    us.ui
                        .as_ref()
                        .and_then(|u| u.ui.as_ref())
                        .and_then(|r| r.id.as_ref()),
                ) {
                    ui_as_render
                        .entry(pb::IdKey::new(ui_id))
                        .or_default()
                        .push(slice_id.slug.clone());
                }
            }
            _ => {}
        }
    }
    let mixed_ok: HashSet<pb::IdKey> = all
        .iter()
        .filter_map(|se| match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::Ui(ui)) => Some(ui),
            _ => None,
        })
        .filter(|ui| {
            ui.metadata.iter().any(|m| {
                m.type_url
                    .ends_with("/trogonatlas.eventmodel.v1alpha1.MixedSurfaceAnnotation")
            })
        })
        .filter_map(|ui| ui.id.as_ref())
        .map(pb::IdKey::new)
        .collect();
    for m in &model.members {
        let (Some(id), Ok(pb::EntityKind::Ui)) = (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let key = pb::IdKey::new(id);
        let triggers = ui_as_trigger.get(&key);
        let renders = ui_as_render.get(&key);
        if let (Some(t), Some(r)) = (triggers, renders) {
            if mixed_ok.contains(&key) {
                continue;
            }
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Warning as i32,
                code: "UI_USED_AS_INPUT_AND_OUTPUT".into(),
                message: format!(
                    "ui {}/{}@{} is the trigger surface for {} command slice(s) ({}) AND the render target for {} read-model slice(s) ({}); one UI doing both roles is almost always a modeling mistake: split the form from the display, or attach a MixedSurfaceAnnotation explaining why this surface really is both",
                    id.namespace,
                    id.slug,
                    id.version,
                    t.len(),
                    t.join(", "),
                    r.len(),
                    r.join(", "),
                ),
                subject: Some(pb::EntityRef {
                    kind: pb::EntityKind::Ui as i32,
                    id: Some(id.clone()),
                }),
                subject_field: String::new(),
                ..Default::default()
            });
        }
    }

    // Invariant: a swimlane's stream_id placeholders must be followable.
    // Each `{placeholder}` either names a FieldSpec on an event/command
    // assigned to that swimlane, or names a derived identity registered by a
    // Uuidv5IdentityAnnotation on the swimlane itself. Inline expressions
    // ("{uuidv5(...)}") are malformed: register the identity once under a
    // name and reference the name.
    for m in &model.members {
        let (Some(lane_id), Ok(pb::EntityKind::Swimlane)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(lane) = all.iter().find_map(|se| match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::Swimlane(l)) if l.id.as_ref() == Some(lane_id) => Some(l),
            _ => None,
        }) else {
            continue;
        };
        let lane_subject = Some(pb::EntityRef {
            kind: pb::EntityKind::Swimlane as i32,
            id: Some(lane_id.clone()),
        });
        // Transitions are this stream's state machine; every event in the
        // table must actually live on this stream: a foreign event cannot
        // be a successor on a stream it never lands on.
        for (ti, t) in lane.transitions.iter().enumerate() {
            let refs = t
                .after
                .iter()
                .map(|r| (format!("transitions[{ti}].after"), r.id.as_ref()))
                .chain(t.next.iter().enumerate().map(|(ni, n)| {
                    (
                        format!("transitions[{ti}].next[{ni}]"),
                        n.event.as_ref().and_then(|r| r.id.as_ref()),
                    )
                }));
            for (field, ev_id) in refs {
                let Some(ev_id) = ev_id else { continue };
                let ev = all.iter().find_map(|se| match se.entity.kind.as_ref() {
                    Some(pb::entity::Kind::Event(e)) if e.id.as_ref() == Some(ev_id) => Some(e),
                    _ => None,
                });
                let Some(ev) = ev else { continue };
                if ev.swimlane.as_ref().and_then(|s| s.id.as_ref()) != Some(lane_id) {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "TRANSITION_FOREIGN_EVENT".into(),
                        message: format!(
                            "swimlane {}/{}@{} transition references event {}/{}@{} which is not on this stream; a lifecycle only contains its own stream's events",
                            lane_id.namespace, lane_id.slug, lane_id.version,
                            ev_id.namespace, ev_id.slug, ev_id.version,
                        ),
                        subject: lane_subject.clone(),
                        subject_field: field,
                        ..Default::default()
                    });
                }
            }
        }
        if lane.stream_id.is_empty() {
            continue;
        }
        let mut names: HashSet<String> = HashSet::new();
        for any in &lane.metadata {
            if !any
                .type_url
                .ends_with("trogonatlas.eventmodel.v1alpha1.Uuidv5IdentityAnnotation")
            {
                continue;
            }
            if let Ok(a) =
                <pb::Uuidv5IdentityAnnotation as prost::Message>::decode(any.value.as_slice())
            {
                if !a.name.is_empty() {
                    names.insert(a.name);
                }
            }
        }
        for se in all {
            let (swimlane, schema) = match se.entity.kind.as_ref() {
                Some(pb::entity::Kind::Event(e)) => (e.swimlane.as_ref(), e.schema.as_ref()),
                Some(pb::entity::Kind::Command(c)) => (c.swimlane.as_ref(), c.schema.as_ref()),
                _ => continue,
            };
            if swimlane.and_then(|s| s.id.as_ref()) != Some(lane_id) {
                continue;
            }
            for f in trogon_atlas_core::schema::schema_fields(schema) {
                names.insert(f.name);
            }
        }
        let subject = Some(pb::EntityRef {
            kind: pb::EntityKind::Swimlane as i32,
            id: Some(lane_id.clone()),
        });
        for token in stream_id_placeholders(&lane.stream_id) {
            let is_name =
                !token.is_empty() && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !is_name {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "STREAM_ID_MALFORMED_PLACEHOLDER".into(),
                    message: format!(
                        "swimlane {}/{}@{} stream_id placeholder {{{token}}} is an expression, not a name; register the derived identity in a Uuidv5IdentityAnnotation with a name and reference that name instead",
                        lane_id.namespace, lane_id.slug, lane_id.version,
                    ),
                    subject: subject.clone(),
                    subject_field: "stream_id".into(),
                    ..Default::default()
                });
            } else if !names.contains(&token) {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Warning as i32,
                    code: "STREAM_ID_UNRESOLVED_PLACEHOLDER".into(),
                    message: format!(
                        "swimlane {}/{}@{} stream_id placeholder {{{token}}} matches no FieldSpec on this lane's events/commands and no named identity annotation; the stream identity must be followable from the data",
                        lane_id.namespace, lane_id.slug, lane_id.version,
                    ),
                    subject: subject.clone(),
                    subject_field: "stream_id".into(),
                    ..Default::default()
                });
            }
        }
    }

    // A slice belongs to one storyboard moment; claiming it from several
    // storyboards makes ordering ambiguous (tools render the first claim).
    let mut slice_claims: HashMap<String, Vec<String>> = HashMap::new();

    // Surface-to-surface hops a command path already explains, keyed
    // (from "ns/slug", to "ns/slug"). Derived navigation: a command fires
    // on one surface, the journey lands on another.
    let mut derivable_nav: HashSet<(String, String)> = HashSet::new();

    // Invariant: projections happen in the same moment as the events they
    // consume. Within a storyboard, a read-model slice must come after the
    // slice emitting its source event, and no command or automation slice may
    // run in between, otherwise the board can no longer tell the order.
    for m in &model.members {
        let (Some(sb_ref_id), Ok(pb::EntityKind::Storyboard)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(sb_se) =
            all.iter().find(
                |se| match (entity_kind(&se.entity), entity_id(&se.entity)) {
                    (Some(sk), Some(sid)) => {
                        sk == pb::EntityKind::Storyboard
                            && sid.namespace == sb_ref_id.namespace
                            && sid.slug == sb_ref_id.slug
                            && sid.version == sb_ref_id.version
                    }
                    _ => false,
                },
            )
        else {
            continue;
        };
        let Some(pb::entity::Kind::Storyboard(sb)) = sb_se.entity.kind.as_ref() else {
            continue;
        };
        let sb_subject = Some(pb::EntityRef {
            kind: pb::EntityKind::Storyboard as i32,
            id: Some(sb_ref_id.clone()),
        });
        // Storyboard hygiene: an honest entry (what is observed, by whom) and
        // an outcome (the terminal facts) are what make sub-workflows chain.
        if sb
            .entry
            .as_ref()
            .and_then(|e| e.read_model.as_ref())
            .is_none()
        {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Warning as i32,
                code: "STORYBOARD_MISSING_ENTRY".into(),
                message: format!(
                    "storyboard {}/{}@{} declares no entry; say which read model is observed and by whom (human persona+ui or automation processor)",
                    sb_ref_id.namespace, sb_ref_id.slug, sb_ref_id.version,
                ),
                subject: sb_subject.clone(),
                subject_field: "entry".into(),
                ..Default::default()
            });
        }
        if sb.outcome.as_ref().is_none_or(|o| o.events.is_empty()) {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Warning as i32,
                code: "STORYBOARD_MISSING_OUTCOME".into(),
                message: format!(
                    "storyboard {}/{}@{} declares no outcome events; name the terminal facts so later storyboards can chain from them",
                    sb_ref_id.namespace, sb_ref_id.slug, sb_ref_id.version,
                ),
                subject: sb_subject.clone(),
                subject_field: "outcome".into(),
                ..Default::default()
            });
        }
        let mut ordered: Vec<(pb::EntityKind, &pb::Entity)> = Vec::new();
        for sref in &sb.slices {
            let Some(sid) = sref.id.as_ref() else {
                continue;
            };
            slice_claims
                .entry(format!("{}/{}@{}", sid.namespace, sid.slug, sid.version))
                .or_default()
                .push(format!(
                    "{}/{}@{}",
                    sb_ref_id.namespace, sb_ref_id.slug, sb_ref_id.version
                ));
            let found = all.iter().find_map(|se| {
                let k = entity_kind(&se.entity)?;
                if !matches!(
                    k,
                    pb::EntityKind::CommandSlice
                        | pb::EntityKind::ReadModelSlice
                        | pb::EntityKind::AutomationSlice
                        | pb::EntityKind::UiSlice
                ) {
                    return None;
                }
                let i = entity_id(&se.entity)?;
                (i.namespace == sid.namespace && i.slug == sid.slug && i.version == sid.version)
                    .then_some((k, &se.entity))
            });
            if let Some(f) = found {
                ordered.push(f);
            }
        }
        {
            let mut last_ui: Option<String> = None;
            let mut carrier = false;
            let mut land = |ui_key: String, last_ui: &mut Option<String>, carrier: &mut bool| {
                if let Some(prev) = last_ui.as_ref() {
                    if *carrier && prev != &ui_key {
                        derivable_nav.insert((prev.clone(), ui_key.clone()));
                    }
                }
                *last_ui = Some(ui_key);
                *carrier = false;
            };
            for (_, e) in &ordered {
                match e.kind.as_ref() {
                    Some(pb::entity::Kind::CommandSlice(cs)) => {
                        if let Some(uid) = cs
                            .ui
                            .as_ref()
                            .and_then(|x| x.ui.as_ref())
                            .and_then(|r| r.id.as_ref())
                        {
                            land(
                                format!("{}/{}", uid.namespace, uid.slug),
                                &mut last_ui,
                                &mut carrier,
                            );
                        }
                        carrier = true;
                    }
                    Some(pb::entity::Kind::UiSlice(us)) => {
                        if let Some(uid) = us
                            .ui
                            .as_ref()
                            .and_then(|x| x.ui.as_ref())
                            .and_then(|r| r.id.as_ref())
                        {
                            land(
                                format!("{}/{}", uid.namespace, uid.slug),
                                &mut last_ui,
                                &mut carrier,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }

        let mut emit_idx: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, (_, e)) in ordered.iter().enumerate() {
            if let Some(pb::entity::Kind::CommandSlice(cs)) = e.kind.as_ref() {
                for ev in cs
                    .emitted_events
                    .iter()
                    .filter_map(|x| x.event.as_ref())
                    .filter_map(|r| r.id.as_ref())
                {
                    emit_idx
                        .entry(format!("{}/{}@{}", ev.namespace, ev.slug, ev.version))
                        .or_default()
                        .push(i);
                }
            }
        }
        for (i, (_, e)) in ordered.iter().enumerate() {
            let Some(pb::entity::Kind::ReadModelSlice(rs)) = e.kind.as_ref() else {
                continue;
            };
            let Some(rs_id) = rs.id.as_ref() else {
                continue;
            };
            let subject = Some(pb::EntityRef {
                kind: pb::EntityKind::ReadModelSlice as i32,
                id: Some(rs_id.clone()),
            });
            for ev in rs
                .source_events
                .iter()
                .filter_map(|x| x.event.as_ref())
                .filter_map(|r| r.id.as_ref())
            {
                let key = format!("{}/{}@{}", ev.namespace, ev.slug, ev.version);
                // Nearest preceding emitter (max j where j < i), not last overall.
                let j = emit_idx
                    .get(&key)
                    .and_then(|idxs| idxs.iter().copied().rev().find(|&j| j < i));
                let Some(j) = j else {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "PROJECTION_BEFORE_EMITTER".into(),
                        message: format!(
                            "read model slice {}/{}@{} consumes event {key} before it is emitted in storyboard {}/{}@{}; a slice captures one moment: move it after the emitting slice or drop the event",
                            rs_id.namespace, rs_id.slug, rs_id.version,
                            sb_ref_id.namespace, sb_ref_id.slug, sb_ref_id.version,
                        ),
                        subject: subject.clone(),
                        subject_field: "source_events".into(),
                        ..Default::default()
                    });
                    continue;
                };
                let blockers: Vec<String> = ordered[j + 1..i]
                    .iter()
                    .filter(|(k2, _)| {
                        matches!(
                            k2,
                            pb::EntityKind::CommandSlice | pb::EntityKind::AutomationSlice
                        )
                    })
                    .filter_map(|(_, be)| entity_id(be).map(|bid| bid.slug.clone()))
                    .collect();
                if !blockers.is_empty() {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "PROJECTION_NOT_ADJACENT".into(),
                        message: format!(
                            "read model slice {}/{}@{} consumes event {key}, but {} action slice(s) ({}) run between the emitter and the projection in storyboard {}/{}@{}; project an event into its read models before the next command or automation acts",
                            rs_id.namespace, rs_id.slug, rs_id.version,
                            blockers.len(),
                            blockers.join(", "),
                            sb_ref_id.namespace, sb_ref_id.slug, sb_ref_id.version,
                        ),
                        subject: subject.clone(),
                        subject_field: "source_events".into(),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // Declared navigation must be PURE navigation (tab bars, back
    // affordances, deep links). A hop the storyboards already explain via
    // a command path is derived; declaring it again is managed duplication
    // that drifts the moment the storyboard changes.
    for m in &model.members {
        let (Some(ui_id), Ok(pb::EntityKind::Ui)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(pb::entity::Kind::Ui(ui)) = all
            .iter()
            .find(
                |se| match (entity_kind(&se.entity), entity_id(&se.entity)) {
                    (Some(sk), Some(sid)) => {
                        sk == pb::EntityKind::Ui
                            && sid.namespace == ui_id.namespace
                            && sid.slug == ui_id.slug
                            && sid.version == ui_id.version
                    }
                    _ => false,
                },
            )
            .and_then(|se| se.entity.kind.as_ref())
        else {
            continue;
        };
        let subject = Some(pb::EntityRef {
            kind: pb::EntityKind::Ui as i32,
            id: Some(ui_id.clone()),
        });
        for (ti, t) in ui.transitions.iter().enumerate() {
            let Some(to_id) = t.to.as_ref().and_then(|r| r.id.as_ref()) else {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "UI_TRANSITION_MISSING_REF".into(),
                    message: format!(
                        "ui {}/{}@{} transitions[{ti}] names no target surface",
                        ui_id.namespace, ui_id.slug, ui_id.version,
                    ),
                    subject: subject.clone(),
                    subject_field: format!("transitions[{ti}].to"),
                    ..Default::default()
                });
                continue;
            };
            if !known.contains(&pb::EntityKey::new(pb::EntityKind::Ui, to_id)) {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "UI_TRANSITION_DANGLING".into(),
                    message: format!(
                        "ui {}/{}@{} transitions[{ti}] -> ui:{}/{}@{} not found in store",
                        ui_id.namespace,
                        ui_id.slug,
                        ui_id.version,
                        to_id.namespace,
                        to_id.slug,
                        to_id.version,
                    ),
                    subject: subject.clone(),
                    subject_field: format!("transitions[{ti}].to"),
                    ..Default::default()
                });
                continue;
            }
            let pair = (
                format!("{}/{}", ui_id.namespace, ui_id.slug),
                format!("{}/{}", to_id.namespace, to_id.slug),
            );
            if derivable_nav.contains(&pair) {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Warning as i32,
                    code: "UI_TRANSITION_DERIVABLE".into(),
                    message: format!(
                        "ui {}/{} declares a transition to {}/{} that a storyboard command path already explains; derived navigation must not be re-declared: drop the transition or document why the pure-navigation link exists separately",
                        ui_id.namespace, ui_id.slug, to_id.namespace, to_id.slug,
                    ),
                    subject: subject.clone(),
                    subject_field: format!("transitions[{ti}].to"),
                    ..Default::default()
                });
            }
        }
    }

    for (slice_key, claimers) in &slice_claims {
        if claimers.len() > 1 {
            let mut list: Vec<&str> = claimers.iter().map(String::as_str).collect();
            list.sort_unstable();
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Warning as i32,
                code: "STORYBOARD_SHARED_SLICE".into(),
                message: format!(
                    "slice {slice_key} is claimed by {} storyboards ({}); a slice is one moment in one narrative: duplicate the moment per storyboard or restructure",
                    claimers.len(),
                    list.join(", "),
                ),
                subject: None,
                subject_field: "slices".into(),
                ..Default::default()
            });
        }
    }

    // A slice that is an EventModel member but no storyboard claims it
    // becomes a floating sticky on the board: the curation lists the
    // moment yet no storyboard walks it. Either bind the slice to a
    // storyboard's slice list or drop it from the model.
    let slice_kinds: &[pb::EntityKind] = &[
        pb::EntityKind::CommandSlice,
        pb::EntityKind::ReadModelSlice,
        pb::EntityKind::AutomationSlice,
        pb::EntityKind::UiSlice,
    ];
    for m in &model.members {
        let (Some(id), Ok(k)) = (m.id.as_ref(), pb::EntityKind::try_from(m.kind)) else {
            continue;
        };
        if !slice_kinds.contains(&k) {
            continue;
        }
        let key = format!("{}/{}@{}", id.namespace, id.slug, id.version);
        if slice_claims.contains_key(&key) {
            continue;
        }
        issues.push(pb::ValidationIssue {
            severity: pb::validation_issue::Severity::Warning as i32,
            code: "SLICE_NOT_IN_STORYBOARD".into(),
            message: format!(
                "{}:{}/{}@{} is an event model member but no storyboard claims it; every slice lives in exactly one storyboard: add it to a storyboard's slices list or drop the member",
                pb::canonical::kind_short(k),
                id.namespace, id.slug, id.version,
            ),
            subject: Some(pb::EntityRef { kind: m.kind, id: Some(id.clone()) }),
            subject_field: "members".into(),
            ..Default::default()
        });
    }

    // A storyboard walking a slice the EventModel has not claimed is a
    // curation gap: the storyboard knows the moment, the model does
    // not. Mirror of SLICE_NOT_IN_STORYBOARD.
    for m in &model.members {
        let (Some(sb_id), Ok(pb::EntityKind::Storyboard)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(sb_se) = all.iter().find(|se| {
            matches!(
                (entity_kind(&se.entity), entity_id(&se.entity)),
                (Some(sk), Some(sid))
                    if sk == pb::EntityKind::Storyboard
                        && sid.namespace == sb_id.namespace
                        && sid.slug == sb_id.slug
                        && sid.version == sb_id.version
            )
        }) else {
            continue;
        };
        let Some(pb::entity::Kind::Storyboard(sb)) = sb_se.entity.kind.as_ref() else {
            continue;
        };
        for sref in &sb.slices {
            let Some(sid) = sref.id.as_ref() else {
                continue;
            };
            let is_member = slice_kinds
                .iter()
                .any(|k| member_set.contains(&pb::EntityKey::new(*k, sid)));
            if is_member {
                continue;
            }
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Warning as i32,
                code: "STORYBOARD_SLICE_NOT_MEMBER".into(),
                message: format!(
                    "storyboard {}/{}@{} walks slice {}/{}@{} that the event model does not claim as a member; add the slice to members or drop it from the storyboard",
                    sb_id.namespace, sb_id.slug, sb_id.version,
                    sid.namespace, sid.slug, sid.version,
                ),
                subject: Some(pb::EntityRef {
                    kind: pb::EntityKind::Storyboard as i32,
                    id: Some(sb_id.clone()),
                }),
                subject_field: "slices".into(),
                ..Default::default()
            });
        }
    }

    // RM ↔ rmSlice source-event agreement. The read model declares the
    // events it materializes; each read-model slice declares which of
    // those it actually projects in the storyboard. Drift between the
    // two manifests as the board's "arrow from nowhere": an event
    // touching the RM with no slice attribution, or a slice pulling an
    // event the RM never named.
    //
    // (rm key) → events some in-model rmSlice consumes for it
    let mut rm_consumed_by_slices: HashMap<pb::IdKey, HashSet<pb::IdKey>> = HashMap::new();
    for m in &model.members {
        let (Some(rs_id), Ok(pb::EntityKind::ReadModelSlice)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(rs_se) = all.iter().find(|se| {
            matches!(
                (entity_kind(&se.entity), entity_id(&se.entity)),
                (Some(sk), Some(sid))
                    if sk == pb::EntityKind::ReadModelSlice
                        && sid.namespace == rs_id.namespace
                        && sid.slug == rs_id.slug
                        && sid.version == rs_id.version
            )
        }) else {
            continue;
        };
        let Some(pb::entity::Kind::ReadModelSlice(rs)) = rs_se.entity.kind.as_ref() else {
            continue;
        };
        let Some(rm_id) = rs
            .read_model
            .as_ref()
            .and_then(|x| x.read_model.as_ref())
            .and_then(|r| r.id.as_ref())
        else {
            continue;
        };
        // Resolve the RM so we can compare against its declared sources.
        let declared: HashSet<pb::IdKey> = all
            .iter()
            .find_map(|se| {
                match (
                    entity_kind(&se.entity),
                    entity_id(&se.entity),
                    se.entity.kind.as_ref(),
                ) {
                    (
                        Some(pb::EntityKind::ReadModel),
                        Some(sid),
                        Some(pb::entity::Kind::ReadModel(rm)),
                    ) if sid.namespace == rm_id.namespace
                        && sid.slug == rm_id.slug
                        && sid.version == rm_id.version =>
                    {
                        Some(
                            rm.source_events
                                .iter()
                                .filter_map(|r| r.id.as_ref())
                                .map(pb::IdKey::new)
                                .collect(),
                        )
                    }
                    _ => None,
                }
            })
            .unwrap_or_default();
        let consumed_set = rm_consumed_by_slices
            .entry(pb::IdKey::new(rm_id))
            .or_default();
        let subject = Some(pb::EntityRef {
            kind: pb::EntityKind::ReadModelSlice as i32,
            id: Some(rs_id.clone()),
        });
        for ev in rs
            .source_events
            .iter()
            .filter_map(|x| x.event.as_ref())
            .filter_map(|r| r.id.as_ref())
        {
            let evk = pb::IdKey::new(ev);
            consumed_set.insert(evk.clone());
            if !declared.contains(&evk) {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "RM_SLICE_EVENT_NOT_DECLARED".into(),
                    message: format!(
                        "read model slice {}/{}@{} consumes event {}/{}@{} but its target read model {}/{}@{} does not declare it as a source: add the event to the read model's source_events or drop it from the slice",
                        rs_id.namespace, rs_id.slug, rs_id.version,
                        ev.namespace, ev.slug, ev.version,
                        rm_id.namespace, rm_id.slug, rm_id.version,
                    ),
                    subject: subject.clone(),
                    subject_field: "source_events".into(),
                    ..Default::default()
                });
            }
        }
    }

    // An RM member that names a source event no in-model slice projects
    // is the workflow-state pattern: the entity says "I consume X" but
    // the curated story never walks that projection. Either add a slice
    // or drop the event from the RM.
    for m in &model.members {
        let (Some(rm_id), Ok(pb::EntityKind::ReadModel)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(rm_se) = all.iter().find(|se| {
            matches!(
                (entity_kind(&se.entity), entity_id(&se.entity)),
                (Some(sk), Some(sid))
                    if sk == pb::EntityKind::ReadModel
                        && sid.namespace == rm_id.namespace
                        && sid.slug == rm_id.slug
                        && sid.version == rm_id.version
            )
        }) else {
            continue;
        };
        let Some(pb::entity::Kind::ReadModel(rm)) = rm_se.entity.kind.as_ref() else {
            continue;
        };
        let consumed = rm_consumed_by_slices
            .get(&pb::IdKey::new(rm_id))
            .cloned()
            .unwrap_or_default();
        for ev_ref in rm.source_events.iter().filter_map(|r| r.id.as_ref()) {
            let evk = pb::IdKey::new(ev_ref);
            if !consumed.contains(&evk) {
                // Only complain when the event is curated in THIS model;
                // events from other contexts are projected elsewhere and
                // a complaint here would be cross-boundary noise.
                if !member_set.contains(&pb::EntityKey::new(pb::EntityKind::Event, ev_ref)) {
                    continue;
                }
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "RM_EVENT_NOT_PROJECTED".into(),
                    message: format!(
                        "read model {}/{}@{} declares event {}/{}@{} as a source but no in-model read-model slice projects it; add a rmSlice that consumes it or drop the event from the read model's source_events",
                        rm_id.namespace, rm_id.slug, rm_id.version,
                        ev_ref.namespace, ev_ref.slug, ev_ref.version,
                    ),
                    subject: Some(pb::EntityRef {
                        kind: pb::EntityKind::ReadModel as i32,
                        id: Some(rm_id.clone()),
                    }),
                    subject_field: "source_events".into(),
                    ..Default::default()
                });
                continue;
            }
            // Event IS wired via a slice: check field coverage instead.
            let Some(ev) = events_by_id.get(&evk) else {
                continue;
            };
            let rm_field_names: HashSet<&str> = declared(&rm_fields, rm.id.as_ref())
                .iter()
                .map(|f| f.name.as_str())
                .collect();
            let mut available: HashSet<String> = declared(&event_fields, ev.id.as_ref())
                .iter()
                .map(|f| f.name.clone())
                .collect();
            if let Some(lane_ref) = ev.swimlane.as_ref().and_then(|s| s.id.as_ref()) {
                if let Some(state) = lane_state_fields.get(&pb::IdKey::new(lane_ref)) {
                    available.extend(state.iter().cloned());
                }
            }
            let mut missing: Vec<&str> = available
                .iter()
                .map(std::string::String::as_str)
                .filter(|name| !rm_field_names.contains(name))
                .collect();
            if missing.is_empty() {
                continue;
            }
            missing.sort_unstable();
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "RM_SOURCE_EVENT_FIELD_NOT_PROJECTED".into(),
                message: format!(
                    "read model {}/{}@{} is wired to source event {}/{}@{} via a read-model slice but is missing field(s) `{}` that the event provides (from the event's own fields or its swimlane state); add the missing field(s) to the read model's fields list, or drop the event from source_events if it is not actually needed",
                    rm_id.namespace, rm_id.slug, rm_id.version,
                    ev_ref.namespace, ev_ref.slug, ev_ref.version,
                    missing.join("`, `"),
                ),
                subject: Some(pb::EntityRef {
                    kind: pb::EntityKind::ReadModel as i32,
                    id: Some(rm_id.clone()),
                }),
                subject_field: "schema.fields".into(),
                ..Default::default()
            });
        }
    }

    // EVENT_NOT_PROJECTED_IN_MODEL: an event that is an EM member but
    // no rmSlice in this model consumes it locally. EVENT_UNOBSERVED is
    // the GLOBAL check (no RM anywhere), easy to dodge by leaning on a
    // cross-context subscription view. This rule keeps the curation
    // honest: every event you curate as a member is one this story
    // walks, even when some other context happens to subscribe to it.
    //
    // Cross-context events (member's namespace differs from the EM's
    // namespace) are deliberately upstream: they're consumed via the
    // subscription pattern and validated by their owning model. Skip
    // them here.
    let mut events_consumed_locally: HashSet<pb::IdKey> = HashSet::new();
    for m in &model.members {
        let (Some(id), Ok(pb::EntityKind::ReadModelSlice)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        let Some(se) = all.iter().find(|se| {
            matches!(
                (entity_kind(&se.entity), entity_id(&se.entity)),
                (Some(sk), Some(sid))
                    if sk == pb::EntityKind::ReadModelSlice
                        && sid.namespace == id.namespace
                        && sid.slug == id.slug
                        && sid.version == id.version
            )
        }) else {
            continue;
        };
        let Some(pb::entity::Kind::ReadModelSlice(rs)) = se.entity.kind.as_ref() else {
            continue;
        };
        for ev in rs
            .source_events
            .iter()
            .filter_map(|x| x.event.as_ref())
            .filter_map(|r| r.id.as_ref())
        {
            events_consumed_locally.insert(pb::IdKey::new(ev));
        }
    }
    let model_ns = model.id.as_ref().map_or("", |i| i.namespace.as_str());
    for m in &model.members {
        let (Some(ev_id), Ok(pb::EntityKind::Event)) =
            (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
        else {
            continue;
        };
        // Upstream cross-context events come in through subscription
        // views by design; the local model isn't the place to project
        // them again.
        if ev_id.namespace != model_ns {
            continue;
        }
        let key = pb::IdKey::new(ev_id);
        if events_consumed_locally.contains(&key) {
            continue;
        }
        issues.push(pb::ValidationIssue {
            severity: pb::validation_issue::Severity::Warning as i32,
            code: "EVENT_NOT_PROJECTED_IN_MODEL".into(),
            message: format!(
                "event {}/{}@{} is a member of this event model but no in-model read-model slice projects it; the curation emits the fact yet never shows its consequence: add a rmSlice that consumes it locally, or drop it from members if another model owns the projection",
                ev_id.namespace, ev_id.slug, ev_id.version,
            ),
            subject: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(ev_id.clone()),
            }),
            subject_field: "members".into(),
            ..Default::default()
        });
    }

    for m in &model.members {
        let (Some(id), Ok(k)) = (m.id.as_ref(), pb::EntityKind::try_from(m.kind)) else {
            continue;
        };
        let Some(se) =
            all.iter().find(
                |se| match (entity_kind(&se.entity), entity_id(&se.entity)) {
                    (Some(sk), Some(sid)) => {
                        sk == k
                            && sid.namespace == id.namespace
                            && sid.slug == id.slug
                            && sid.version == id.version
                    }
                    _ => false,
                },
            )
        else {
            continue;
        };
        // A member curated from another context (e.g. the upstream event a
        // subscription view names) is validated by ITS model; walking its
        // internals from here only produces noise about a boundary we
        // deliberately do not own.
        if let Some(model_ns) = model.id.as_ref().map(|i| i.namespace.as_str()) {
            if id.namespace != model_ns {
                continue;
            }
        }
        for o in outbound_refs(&se.entity) {
            // Bounded contexts touch at exactly ONE seam: an upstream EVENT
            // flowing into a downstream subscription READ MODEL (the
            // translation pattern). Any other cross-namespace structural
            // ref couples contexts to each other's internals. EventModel
            // members (curated listing) and external_source.system (shared
            // third-party boundary) are non-structural and stay legal.
            if o.to_id.namespace != id.namespace {
                let seam = o.from_field.starts_with("source_events[")
                    || o.from_field.starts_with("calls[")
                    // Strategic mapping, not behavioral coupling: contexts
                    // point at the problem space they realize (Decision #29)
                    // and record the agreement behind each seam (Decision #30).
                    || o.from_field.starts_with("realizes[")
                    || o.from_field.starts_with("relationships[")
                    || o.from_field == "external_source.system"
                    // Automation slices are the wire that crosses contexts:
                    // a choreographer subscribes to an upstream event (via
                    // source_events on its local read model) and dispatches
                    // a command into the downstream context. The processor
                    // IS the boundary; emitting cross-namespace commands is
                    // the codified pattern (Commanded handlers in the
                    // reference codebase). Flagging it would force a
                    // 2-hop event-relay fiction that doesn't exist at
                    // runtime.
                    || o.from_field == "emitted_command"
                    // Storyboard outcome / entry refs cross contexts too in
                    // choreographies: the outcome event of a settlement
                    // storyboard IS an upstream context's event (the thing
                    // the choreographer reacts to). Allow it for the same
                    // reason source_events[] are allowed.
                    || o.from_field.starts_with("outcome.events[")
                    || o.from_field == "outcome.ui"
                    || o.from_field == "entry.read_model"
                    // Must match outbound_refs paths on StoryboardEntry
                    // (`entry.observer.human.*` / `entry.observer.automation.*`).
                    || o.from_field == "entry.observer.human.persona"
                    || o.from_field == "entry.observer.human.ui"
                    || o.from_field == "entry.observer.automation.processor"
                    // Screen composition: UI lives in a context; Screen lives
                    // in an application namespace by design (SCREEN_NAMESPACE_*).
                    || o.from_field == "slot.screen"
                    // Ambiguity.terms are a legal cross-namespace knowledge
                    // ref class (strategic ubiquitous-language rulings).
                    || o.from_field.starts_with("terms[")
                    || k == pb::EntityKind::EventModel
                    || k == pb::EntityKind::Tracker
                    // Reliability overlays point OUT at the graph the same
                    // way Tracker does (Decision #25); they are never woven
                    // into a slice's own structure, so they never couple two
                    // contexts to each other's internals.
                    || k == pb::EntityKind::ServiceLevelIndicator
                    || k == pb::EntityKind::ServiceLevelObjective
                    || k == pb::EntityKind::AlertPolicy
                    || k == pb::EntityKind::AlertNotificationTarget;
                if !seam {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "CROSS_MODEL_BOUNDARY_VIOLATION".into(),
                        message: format!(
                            "{}:{}/{}@{} reaches into context {:?} via {}; contexts touch only through published events flowing into subscription read models (source_events): translate, don't reference",
                            trogon_atlas_proto::canonical::kind_short(k),
                            id.namespace,
                            id.slug,
                            id.version,
                            o.to_id.namespace,
                            o.from_field,
                        ),
                        subject: Some(pb::EntityRef {
                            kind: k as i32,
                            id: Some(id.clone()),
                        }),
                        subject_field: o.from_field.clone(),
                        ..Default::default()
                    });
                }
            }
            let candidates: Vec<pb::EntityKind> = match o.to_kind {
                None => vec![
                    pb::EntityKind::CommandSlice,
                    pb::EntityKind::ReadModelSlice,
                    pb::EntityKind::AutomationSlice,
                    pb::EntityKind::UiSlice,
                ],
                Some(k) => vec![k],
            };
            let resolved = candidates
                .iter()
                .copied()
                .find(|c| known.contains(&pb::EntityKey::new(*c, &o.to_id)));
            let to_kind_label = match o.to_kind {
                None => "slice",
                Some(k) => trogon_atlas_proto::canonical::kind_short(k),
            };
            match resolved {
                None => {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "DANGLING_REF".into(),
                        message: format!(
                            "{}:{}/{}@{} -> {}:{}/{}@{} is unresolved",
                            trogon_atlas_proto::canonical::kind_short(k),
                            id.namespace,
                            id.slug,
                            id.version,
                            to_kind_label,
                            o.to_id.namespace,
                            o.to_id.slug,
                            o.to_id.version,
                        ),
                        subject: Some(pb::EntityRef {
                            kind: k as i32,
                            id: Some(id.clone()),
                        }),
                        subject_field: o.from_field.clone(),
                        ..Default::default()
                    });
                }
                Some(target_kind) => {
                    let target_key = pb::EntityKey::new(target_kind, &o.to_id);
                    // The declared context seam (subscription source_events,
                    // shared external systems) is legal by design and policed
                    // by CROSS_MODEL_BOUNDARY_VIOLATION, reporting Info for
                    // it would nag every model that integrates correctly.
                    let declared_seam = o.to_id.namespace != id.namespace
                        && (o.from_field.starts_with("source_events[")
                            || o.from_field.starts_with("calls[")
                            || o.from_field == "external_source.system"
                            || o.from_field == "emitted_command"
                            || o.from_field.starts_with("outcome.events[")
                            || o.from_field == "outcome.ui"
                            || o.from_field == "entry.read_model"
                            || o.from_field == "entry.observer.human.persona"
                            || o.from_field == "entry.observer.human.ui"
                            || o.from_field == "entry.observer.automation.processor"
                            || o.from_field == "slot.screen"
                            || o.from_field.starts_with("terms["));
                    // Personas are intentionally shareable across bounded
                    // contexts. Treat a cross-namespace Persona ref as a
                    // legitimate sharing pattern, not a data seam.
                    let shared_persona =
                        target_kind == pb::EntityKind::Persona && o.to_id.namespace != id.namespace;
                    if !member_set.contains(&target_key) && !declared_seam && !shared_persona {
                        issues.push(pb::ValidationIssue {
                            severity: pb::validation_issue::Severity::Info as i32,
                            code: "CROSS_MODEL_REF".into(),
                            message: format!(
                                "{}:{}/{}@{} references {}:{}/{}@{} outside this model",
                                trogon_atlas_proto::canonical::kind_short(k),
                                id.namespace,
                                id.slug,
                                id.version,
                                trogon_atlas_proto::canonical::kind_short(target_kind),
                                o.to_id.namespace,
                                o.to_id.slug,
                                o.to_id.version,
                            ),
                            subject: Some(pb::EntityRef {
                                kind: k as i32,
                                id: Some(id.clone()),
                            }),
                            subject_field: o.from_field.clone(),
                            ..Default::default()
                        });
                    }
                }
            }
        }
    }

    // Decision #31: every member that is not wired into the model's slice
    // graph must declare WHY via OrphanAnnotation. Walk the in-model
    // slices, collect every (kind, ns, slug, version) they reference,
    // then check each behavioral member against that set.
    let mut wired: HashSet<pb::EntityKey> = HashSet::new();
    let record = |kind: pb::EntityKind, id: Option<&pb::Id>, set: &mut HashSet<pb::EntityKey>| {
        if let Some(i) = id {
            set.insert(pb::EntityKey::new(kind, i));
        }
    };
    for m in &model.members {
        let (Some(id), Ok(k)) = (m.id.as_ref(), pb::EntityKind::try_from(m.kind)) else {
            continue;
        };
        let Some(se) = all.iter().find(|se| {
            matches!(
                (entity_kind(&se.entity), entity_id(&se.entity)),
                (Some(sk), Some(sid))
                    if sk == k
                        && sid.namespace == id.namespace
                        && sid.slug == id.slug
                        && sid.version == id.version
            )
        }) else {
            continue;
        };
        match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::CommandSlice(cs)) => {
                record(
                    pb::EntityKind::Persona,
                    cs.persona
                        .as_ref()
                        .and_then(|p| p.persona.as_ref())
                        .and_then(|r| r.id.as_ref()),
                    &mut wired,
                );
                record(
                    pb::EntityKind::Ui,
                    cs.ui
                        .as_ref()
                        .and_then(|p| p.ui.as_ref())
                        .and_then(|r| r.id.as_ref()),
                    &mut wired,
                );
                record(
                    pb::EntityKind::Command,
                    cs.command
                        .as_ref()
                        .and_then(|p| p.command.as_ref())
                        .and_then(|r| r.id.as_ref()),
                    &mut wired,
                );
                for e in &cs.emitted_events {
                    record(
                        pb::EntityKind::Event,
                        e.event.as_ref().and_then(|r| r.id.as_ref()),
                        &mut wired,
                    );
                }
            }
            Some(pb::entity::Kind::ReadModelSlice(rs)) => {
                record(
                    pb::EntityKind::ReadModel,
                    rs.read_model
                        .as_ref()
                        .and_then(|p| p.read_model.as_ref())
                        .and_then(|r| r.id.as_ref()),
                    &mut wired,
                );
                for e in &rs.source_events {
                    record(
                        pb::EntityKind::Event,
                        e.event.as_ref().and_then(|r| r.id.as_ref()),
                        &mut wired,
                    );
                }
            }
            Some(pb::entity::Kind::UiSlice(us)) => {
                record(
                    pb::EntityKind::Persona,
                    us.persona
                        .as_ref()
                        .and_then(|p| p.persona.as_ref())
                        .and_then(|r| r.id.as_ref()),
                    &mut wired,
                );
                record(
                    pb::EntityKind::Ui,
                    us.ui
                        .as_ref()
                        .and_then(|p| p.ui.as_ref())
                        .and_then(|r| r.id.as_ref()),
                    &mut wired,
                );
                for r in &us.source_read_models {
                    record(
                        pb::EntityKind::ReadModel,
                        r.read_model.as_ref().and_then(|x| x.id.as_ref()),
                        &mut wired,
                    );
                }
            }
            Some(pb::entity::Kind::AutomationSlice(as_)) => {
                record(
                    pb::EntityKind::Processor,
                    as_.processor
                        .as_ref()
                        .and_then(|p| p.processor.as_ref())
                        .and_then(|r| r.id.as_ref()),
                    &mut wired,
                );
                for r in &as_.source_read_models {
                    record(
                        pb::EntityKind::ReadModel,
                        r.read_model.as_ref().and_then(|x| x.id.as_ref()),
                        &mut wired,
                    );
                }
                if let Some(cmd) = as_
                    .emitted_command
                    .as_ref()
                    .and_then(|c| c.command.as_ref())
                {
                    record(pb::EntityKind::Command, cmd.id.as_ref(), &mut wired);
                }
            }
            // A storyboard's entry observer is a real wiring point: the
            // persona watching, the UI they observe through, the
            // automation processor that triggers the flow, and the read
            // model that anchors the moment all participate. Counting
            // only slice refs as wiring marked them as orphans.
            Some(pb::entity::Kind::Storyboard(sb)) => {
                if let Some(entry) = sb.entry.as_ref() {
                    record(
                        pb::EntityKind::ReadModel,
                        entry
                            .read_model
                            .as_ref()
                            .and_then(|r| r.read_model.as_ref())
                            .and_then(|r| r.id.as_ref()),
                        &mut wired,
                    );
                    if let Some(observer) = entry.observer.as_ref() {
                        match observer {
                            pb::storyboard_entry::Observer::Human(h) => {
                                record(
                                    pb::EntityKind::Persona,
                                    h.persona
                                        .as_ref()
                                        .and_then(|p| p.persona.as_ref())
                                        .and_then(|r| r.id.as_ref()),
                                    &mut wired,
                                );
                                record(
                                    pb::EntityKind::Ui,
                                    h.ui.as_ref()
                                        .and_then(|p| p.ui.as_ref())
                                        .and_then(|r| r.id.as_ref()),
                                    &mut wired,
                                );
                            }
                            pb::storyboard_entry::Observer::Automation(a) => {
                                record(
                                    pb::EntityKind::Processor,
                                    a.processor
                                        .as_ref()
                                        .and_then(|p| p.processor.as_ref())
                                        .and_then(|r| r.id.as_ref()),
                                    &mut wired,
                                );
                            }
                        }
                    }
                }
                // Outcome events are facts the curation explicitly
                // names, count them as wired too.
                if let Some(outcome) = sb.outcome.as_ref() {
                    for ev in &outcome.events {
                        record(
                            pb::EntityKind::Event,
                            ev.event.as_ref().and_then(|r| r.id.as_ref()),
                            &mut wired,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    let orphan_kinds: &[pb::EntityKind] = &[
        pb::EntityKind::Event,
        pb::EntityKind::Command,
        pb::EntityKind::ReadModel,
        pb::EntityKind::Processor,
        pb::EntityKind::Ui,
        pb::EntityKind::Persona,
    ];
    for m in &model.members {
        let (Some(id), Ok(k)) = (m.id.as_ref(), pb::EntityKind::try_from(m.kind)) else {
            continue;
        };
        if !orphan_kinds.contains(&k) {
            continue;
        }
        let Some(se) = all.iter().find(|se| {
            matches!(
                (entity_kind(&se.entity), entity_id(&se.entity)),
                (Some(sk), Some(sid))
                    if sk == k
                        && sid.namespace == id.namespace
                        && sid.slug == id.slug
                        && sid.version == id.version
            )
        }) else {
            continue;
        };

        let connected = wired.contains(&pb::EntityKey::new(k, id));
        // Walk the entity's metadata looking for OrphanAnnotation.
        let metadata: &[prost_types::Any] = match se.entity.kind.as_ref() {
            Some(pb::entity::Kind::Event(x)) => &x.metadata,
            Some(pb::entity::Kind::Command(x)) => &x.metadata,
            Some(pb::entity::Kind::ReadModel(x)) => &x.metadata,
            Some(pb::entity::Kind::Processor(x)) => &x.metadata,
            Some(pb::entity::Kind::Ui(x)) => &x.metadata,
            Some(pb::entity::Kind::Persona(x)) => &x.metadata,
            _ => &[],
        };
        let mut orphan_doc: Option<String> = None;
        let mut orphan_present = false;
        for any in metadata {
            if !any
                .type_url
                .ends_with("trogonatlas.eventmodel.v1alpha1.OrphanAnnotation")
            {
                continue;
            }
            orphan_present = true;
            if let Ok(a) = <pb::OrphanAnnotation as prost::Message>::decode(any.value.as_slice()) {
                if !a.doc.is_empty() {
                    orphan_doc = Some(a.doc);
                    break;
                }
            }
        }
        let subject = Some(pb::EntityRef {
            kind: m.kind,
            id: Some(id.clone()),
        });
        if !connected && !orphan_present {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Warning as i32,
                code: "ORPHAN_NOT_FLAGGED".into(),
                message: format!(
                    "{}:{}/{}@{} is a member of this event model but no slice produces, consumes, or feeds it: declare an OrphanAnnotation with a non-empty doc explaining why, or wire it into a slice",
                    pb::canonical::kind_short(k),
                    id.namespace, id.slug, id.version,
                ),
                subject: subject.clone(),
                subject_field: "metadata".into(),
                ..Default::default()
            });
        } else if !connected && orphan_present && orphan_doc.is_none() {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "ORPHAN_DOC_EMPTY".into(),
                message: format!(
                    "{}:{}/{}@{} carries OrphanAnnotation but its doc is empty: silence is the noise the annotation is meant to replace",
                    pb::canonical::kind_short(k),
                    id.namespace, id.slug, id.version,
                ),
                subject: subject.clone(),
                subject_field: "metadata".into(),
                ..Default::default()
            });
        } else if connected && orphan_present {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Info as i32,
                code: "ORPHAN_FLAG_OBSOLETE".into(),
                message: format!(
                    "{}:{}/{}@{} is now wired into a slice: remove the OrphanAnnotation",
                    pb::canonical::kind_short(k),
                    id.namespace,
                    id.slug,
                    id.version,
                ),
                subject: subject.clone(),
                subject_field: "metadata".into(),
                ..Default::default()
            });
        }
    }

    // Any member may carry OpenQuestionAnnotation / AssumptionAnnotation
    // metadata: an open design question, or a belief the design proceeds
    // on that nobody has confirmed. Unlike the orphan walk above, this is
    // not restricted to a handful of entity kinds: either annotation can
    // land on any member.
    for m in &model.members {
        let (Some(id), Ok(k)) = (m.id.as_ref(), pb::EntityKind::try_from(m.kind)) else {
            continue;
        };
        let Some(se) = all.iter().find(|se| {
            matches!(
                (entity_kind(&se.entity), entity_id(&se.entity)),
                (Some(sk), Some(sid))
                    if sk == k
                        && sid.namespace == id.namespace
                        && sid.slug == id.slug
                        && sid.version == id.version
            )
        }) else {
            continue;
        };
        let subject = Some(pb::EntityRef {
            kind: m.kind,
            id: Some(id.clone()),
        });
        let label = format!(
            "{}:{}/{}@{}",
            pb::canonical::kind_short(k),
            id.namespace,
            id.slug,
            id.version,
        );
        push_annotation_findings(
            entity_metadata(&se.entity),
            subject.as_ref(),
            "metadata",
            &label,
            &mut issues,
        );
    }

    // Decision #33: shape is declared, not inferred. Two entities
    // pointing at the same schema `type_url` are nominally same-type
    // by declaration, so no twin warning is needed.
    for m in &model.members {
        let (Some(id), Ok(k)) = (m.id.as_ref(), pb::EntityKind::try_from(m.kind)) else {
            continue;
        };
        if !matches!(
            k,
            pb::EntityKind::Event | pb::EntityKind::Command | pb::EntityKind::ReadModel
        ) {
            continue;
        }
        let Some(se) = all.iter().find(|se| {
            matches!(
                (entity_kind(&se.entity), entity_id(&se.entity)),
                (Some(sk), Some(sid))
                    if sk == k
                        && sid.namespace == id.namespace
                        && sid.slug == id.slug
                        && sid.version == id.version
            )
        }) else {
            continue;
        };
        let Some(schema_any) = se
            .entity
            .kind
            .as_ref()
            .and_then(trogon_atlas_core::schema::kind_schema)
        else {
            continue;
        };
        if schema_any.type_url.is_empty() {
            issues.push(pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "SCHEMA_TYPE_URL_EMPTY".into(),
                message: format!(
                    "{}:{}/{}@{} carries a `schema` Any with an empty type_url: every schema must identify its type",
                    pb::canonical::kind_short(k),
                    id.namespace, id.slug, id.version,
                ),
                subject: Some(pb::EntityRef {
                    kind: m.kind,
                    id: Some(id.clone()),
                }),
                subject_field: "schema.type_url".into(),
                ..Default::default()
            });
        }
    }

    for m in &model.members {
        let (Some(id), Ok(k)) = (m.id.as_ref(), pb::EntityKind::try_from(m.kind)) else {
            continue;
        };
        let schema = match k {
            pb::EntityKind::Event => {
                let Some(e) = events_by_id.get(&pb::IdKey::new(id)) else {
                    continue;
                };
                e.schema.as_ref()
            }
            pb::EntityKind::Command => {
                let Some(c) = commands_by_id.get(&pb::IdKey::new(id)) else {
                    continue;
                };
                c.schema.as_ref()
            }
            pb::EntityKind::ReadModel => {
                let Some(r) = rms_by_id.get(&pb::IdKey::new(id)) else {
                    continue;
                };
                r.schema.as_ref()
            }
            _ => continue,
        };
        let fields = trogon_atlas_core::schema::schema_fields(schema);
        let prefix = "schema.fields";
        let mut hits = Vec::new();
        walk_personal_data_fields(&fields, prefix, "", &mut hits);
        let subject = Some(pb::EntityRef {
            kind: m.kind,
            id: Some(id.clone()),
        });
        for hit in &hits {
            for any in &hit.field.metadata {
                let Some(decoded) = decode_personal_data_annotation(any) else {
                    continue;
                };
                let subject_field = format!("{}.metadata", hit.index_path);
                let Ok(ann) = decoded else {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "PII_DOC_MISSING".into(),
                        message: format!(
                            "{}:{}/{}@{} field `{}` carries a malformed PersonalDataAnnotation that failed to decode",
                            pb::canonical::kind_short(k),
                            id.namespace, id.slug, id.version, hit.name_path,
                        ),
                        subject: subject.clone(),
                        subject_field,
                        ..Default::default()
                    });
                    continue;
                };
                if ann.doc.is_empty() {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "PII_DOC_MISSING".into(),
                        message: format!(
                            "{}:{}/{}@{} field `{}` carries a PersonalDataAnnotation with an empty doc - explain what personal data this is and whose",
                            pb::canonical::kind_short(k),
                            id.namespace, id.slug, id.version, hit.name_path,
                        ),
                        subject: subject.clone(),
                        subject_field: subject_field.clone(),
                        ..Default::default()
                    });
                }
                let erasure = pb::personal_data_annotation::Erasure::try_from(ann.erasure)
                    .unwrap_or(pb::personal_data_annotation::Erasure::Unspecified);
                if k == pb::EntityKind::Event
                    && erasure == pb::personal_data_annotation::Erasure::Unspecified
                {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Warning as i32,
                        code: "PII_EVENT_ERASURE_UNSPECIFIED".into(),
                        message: format!(
                            "event {}/{}@{} field `{}` is personal data with no declared erasure strategy - events are immutable, so say how the value becomes unrecoverable (crypto-shredding, external reference, or retention expiry)",
                            id.namespace, id.slug, id.version, hit.name_path,
                        ),
                        subject: subject.clone(),
                        subject_field: subject_field.clone(),
                        ..Default::default()
                    });
                }
                if erasure == pb::personal_data_annotation::Erasure::ProjectionDelete
                    && k != pb::EntityKind::ReadModel
                {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "PII_ERASURE_NOT_APPLICABLE".into(),
                        message: format!(
                            "{}:{}/{}@{} field `{}` declares ERASURE_PROJECTION_DELETE, which only applies to a ReadModel row - an event or command has no row to delete",
                            pb::canonical::kind_short(k),
                            id.namespace, id.slug, id.version, hit.name_path,
                        ),
                        subject: subject.clone(),
                        subject_field: subject_field.clone(),
                        ..Default::default()
                    });
                }
                if !ann.subject_field.is_empty()
                    && !hit.siblings.iter().any(|n| n == &ann.subject_field)
                {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "PII_SUBJECT_FIELD_UNKNOWN".into(),
                        message: format!(
                            "{}:{}/{}@{} field `{}` declares subject_field `{}` which names no field in the same field list",
                            pb::canonical::kind_short(k),
                            id.namespace, id.slug, id.version, hit.name_path, ann.subject_field,
                        ),
                        subject: subject.clone(),
                        subject_field,
                        ..Default::default()
                    });
                }
            }
        }
    }

    // ===== PASS: PII_FLOW_UNMARKED =====
    // Reuses the data-flow inference layer (analysis::flow_links_for_members
    // / analysis::map_field) instead of re-deriving upstream/downstream
    // field matches: when a downstream field is traced (exact-name match,
    // the only deterministic case map_field reports) to an upstream field
    // carrying PersonalDataAnnotation, the downstream field needs its own
    // PersonalDataAnnotation too, or erasing the upstream copy leaves this
    // one behind undeclared.
    let by_key = crate::graph::entity_lookup(all);
    let flow_links = crate::analysis::flow_links_for_members(&model.members, &by_key);
    let mut reported: HashSet<(i32, pb::IdKey, usize)> = HashSet::new();
    for link in &flow_links {
        for down in &link.downstream {
            let Ok(down_kind) = pb::EntityKind::try_from(down.entity_ref.kind) else {
                continue;
            };
            if !matches!(
                down_kind,
                pb::EntityKind::Event | pb::EntityKind::Command | pb::EntityKind::ReadModel
            ) {
                continue;
            }
            let Some(down_id) = down.entity_ref.id.as_ref() else {
                continue;
            };
            let prefix = "schema.fields";
            for (i, f) in down.fields.iter().enumerate() {
                if f.metadata
                    .iter()
                    .any(|a| decode_personal_data_annotation(a).is_some())
                {
                    continue;
                }
                let mapping = crate::analysis::map_field(&down.entity_ref, f, &link.upstream);
                let Some(pb::inferred_field_mapping::Source::FromField(src)) = mapping.source
                else {
                    continue;
                };
                let upstream_is_pii = link
                    .upstream
                    .iter()
                    .find(|u| Some(&u.entity_ref) == src.from_entity.as_ref())
                    .and_then(|u| u.fields.iter().find(|uf| uf.name == src.field_path))
                    .is_some_and(|uf| {
                        uf.metadata
                            .iter()
                            .any(|a| decode_personal_data_annotation(a).is_some())
                    });
                if !upstream_is_pii
                    || !reported.insert((down_kind as i32, pb::IdKey::new(down_id), i))
                {
                    continue;
                }
                let from_label = src
                    .from_entity
                    .as_ref()
                    .and_then(|r| r.id.as_ref())
                    .map_or_else(
                        || "<unknown>".to_string(),
                        |fid| format!("{}/{}@{}", fid.namespace, fid.slug, fid.version),
                    );
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Warning as i32,
                    code: "PII_FLOW_UNMARKED".into(),
                    message: format!(
                        "{}:{}/{}@{} field `{}` is inferred to come from personal-data field `{}` on {} but carries no PersonalDataAnnotation of its own",
                        pb::canonical::kind_short(down_kind),
                        down_id.namespace, down_id.slug, down_id.version, f.name,
                        src.field_path, from_label,
                    ),
                    subject: Some(down.entity_ref.clone()),
                    subject_field: format!("{prefix}[{i}].metadata"),
                    ..Default::default()
                });
            }
        }
    }

    // ===== PASS: reliability (SLI / SLO / AlertPolicy) =====
    // Measurements prove nothing if they point at graph edges, events, or
    // objectives that are not actually there; this block resolves the
    // Connection, Signal, OutcomeRatio and Objective fields against the
    // model the same way earlier passes resolve projection field sourcing.
    {
        let command_slices_by_id: HashMap<pb::IdKey, &pb::CommandSlice> = all
            .iter()
            .filter_map(|se| match se.entity.kind.as_ref()? {
                pb::entity::Kind::CommandSlice(s) => Some((pb::IdKey::new(s.id.as_ref()?), s)),
                _ => None,
            })
            .collect();
        let read_model_slices_by_id: HashMap<pb::IdKey, &pb::ReadModelSlice> = all
            .iter()
            .filter_map(|se| match se.entity.kind.as_ref()? {
                pb::entity::Kind::ReadModelSlice(s) => Some((pb::IdKey::new(s.id.as_ref()?), s)),
                _ => None,
            })
            .collect();
        let automation_slices_by_id: HashMap<pb::IdKey, &pb::AutomationSlice> = all
            .iter()
            .filter_map(|se| match se.entity.kind.as_ref()? {
                pb::entity::Kind::AutomationSlice(s) => Some((pb::IdKey::new(s.id.as_ref()?), s)),
                _ => None,
            })
            .collect();
        let ui_slices_by_id: HashMap<pb::IdKey, &pb::UiSlice> = all
            .iter()
            .filter_map(|se| match se.entity.kind.as_ref()? {
                pb::entity::Kind::UiSlice(s) => Some((pb::IdKey::new(s.id.as_ref()?), s)),
                _ => None,
            })
            .collect();
        let storyboards_by_id: HashMap<pb::IdKey, &pb::Storyboard> = all
            .iter()
            .filter_map(|se| match se.entity.kind.as_ref()? {
                pb::entity::Kind::Storyboard(s) => Some((pb::IdKey::new(s.id.as_ref()?), s)),
                _ => None,
            })
            .collect();
        let event_models_by_id: HashMap<pb::IdKey, &pb::EventModel> = all
            .iter()
            .filter_map(|se| match se.entity.kind.as_ref()? {
                pb::entity::Kind::EventModel(s) => Some((pb::IdKey::new(s.id.as_ref()?), s)),
                _ => None,
            })
            .collect();
        let processors_by_id: HashMap<pb::IdKey, &pb::Processor> = all
            .iter()
            .filter_map(|se| match se.entity.kind.as_ref()? {
                pb::entity::Kind::Processor(s) => Some((pb::IdKey::new(s.id.as_ref()?), s)),
                _ => None,
            })
            .collect();
        let slis_by_id: HashMap<pb::IdKey, &pb::ServiceLevelIndicator> = all
            .iter()
            .filter_map(|se| match se.entity.kind.as_ref()? {
                pb::entity::Kind::ServiceLevelIndicator(s) => {
                    Some((pb::IdKey::new(s.id.as_ref()?), s))
                }
                _ => None,
            })
            .collect();
        let slos_by_id: HashMap<pb::IdKey, &pb::ServiceLevelObjective> = all
            .iter()
            .filter_map(|se| match se.entity.kind.as_ref()? {
                pb::entity::Kind::ServiceLevelObjective(s) => {
                    Some((pb::IdKey::new(s.id.as_ref()?), s))
                }
                _ => None,
            })
            .collect();

        // Every slice ref reachable from a Journey's scope, pooled without
        // regard to storyboard order: reachability below walks edges, not
        // sequence, so pooling keeps an EventModel scope's chained
        // storyboards on one graph.
        let scope_slice_refs = |scope: &pb::connection::journey::Scope| -> Vec<pb::SliceRef> {
            let storyboards: Vec<&pb::Storyboard> = match scope {
                pb::connection::journey::Scope::Storyboard(r) => {
                    r.id.as_ref()
                        .and_then(|id| storyboards_by_id.get(&pb::IdKey::new(id)))
                        .map(|s| vec![*s])
                        .unwrap_or_default()
                }
                pb::connection::journey::Scope::EventModel(r) => r
                    .id
                    .as_ref()
                    .and_then(|id| event_models_by_id.get(&pb::IdKey::new(id)))
                    .map(|em| {
                        em.members
                            .iter()
                            .filter(|mem| {
                                pb::EntityKind::try_from(mem.kind) == Ok(pb::EntityKind::Storyboard)
                            })
                            .filter_map(|mem| mem.id.as_ref())
                            .filter_map(|id| storyboards_by_id.get(&pb::IdKey::new(id)))
                            .copied()
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            storyboards
                .into_iter()
                .flat_map(|sb| sb.slices.clone())
                .collect()
        };

        // Build the event -> event reachability graph for a Journey's
        // scope: an edge src -> dst exists when a read-model slice
        // projects src into a read model that either (a) an automation
        // slice observes, emitting a command whose slice emits dst (a
        // Reaction chain), or (b) a UI slice renders for a persona who
        // then issues a command from the SAME ui, emitting dst (a human
        // handoff on one screen). This is structural only: it does not
        // model a persona navigating to a different screen first, which is
        // why SLI_JOURNEY_UNREACHABLE's doc calls the check conservative.
        let journey_edges =
            |scope: &pb::connection::journey::Scope| -> HashMap<pb::IdKey, HashSet<pb::IdKey>> {
                let slice_refs = scope_slice_refs(scope);
                let mut rm_slices: Vec<&pb::ReadModelSlice> = Vec::new();
                let mut automations: Vec<&pb::AutomationSlice> = Vec::new();
                let mut ui_slices: Vec<&pb::UiSlice> = Vec::new();
                let mut cmd_slices: Vec<&pb::CommandSlice> = Vec::new();
                for sref in &slice_refs {
                    let Some(key_id) = sref.id.as_ref() else {
                        continue;
                    };
                    let key = pb::IdKey::new(key_id);
                    if let Some(s) = read_model_slices_by_id.get(&key) {
                        rm_slices.push(s);
                    }
                    if let Some(s) = automation_slices_by_id.get(&key) {
                        automations.push(s);
                    }
                    if let Some(s) = ui_slices_by_id.get(&key) {
                        ui_slices.push(s);
                    }
                    if let Some(s) = command_slices_by_id.get(&key) {
                        cmd_slices.push(s);
                    }
                }
                let mut edges: HashMap<pb::IdKey, HashSet<pb::IdKey>> = HashMap::new();
                for rs in &rm_slices {
                    let Some(rm_id) = rs
                        .read_model
                        .as_ref()
                        .and_then(|e| e.read_model.as_ref())
                        .and_then(|r| r.id.as_ref())
                    else {
                        continue;
                    };
                    let rm_key = pb::IdKey::new(rm_id);
                    let mut next_commands: HashSet<pb::IdKey> = HashSet::new();
                    for a in &automations {
                        let observes = a.source_read_models.iter().any(|e| {
                            e.read_model
                                .as_ref()
                                .and_then(|r| r.id.as_ref())
                                .is_some_and(|rid| pb::IdKey::new(rid) == rm_key)
                        });
                        if !observes {
                            continue;
                        }
                        if let Some(cmd_id) = a
                            .emitted_command
                            .as_ref()
                            .and_then(|e| e.command.as_ref())
                            .and_then(|r| r.id.as_ref())
                        {
                            next_commands.insert(pb::IdKey::new(cmd_id));
                        }
                    }
                    for u in &ui_slices {
                        let renders = u.source_read_models.iter().any(|e| {
                            e.read_model
                                .as_ref()
                                .and_then(|r| r.id.as_ref())
                                .is_some_and(|rid| pb::IdKey::new(rid) == rm_key)
                        });
                        if !renders {
                            continue;
                        }
                        let Some(ui_id) =
                            u.ui.as_ref()
                                .and_then(|e| e.ui.as_ref())
                                .and_then(|r| r.id.as_ref())
                        else {
                            continue;
                        };
                        let ui_key = pb::IdKey::new(ui_id);
                        for cs in &cmd_slices {
                            let same_ui = cs
                                .ui
                                .as_ref()
                                .and_then(|e| e.ui.as_ref())
                                .and_then(|r| r.id.as_ref())
                                .is_some_and(|cid| pb::IdKey::new(cid) == ui_key);
                            if !same_ui {
                                continue;
                            }
                            if let Some(cmd_id) = cs
                                .command
                                .as_ref()
                                .and_then(|e| e.command.as_ref())
                                .and_then(|r| r.id.as_ref())
                            {
                                next_commands.insert(pb::IdKey::new(cmd_id));
                            }
                        }
                    }
                    if next_commands.is_empty() {
                        continue;
                    }
                    for src_event in rs
                        .source_events
                        .iter()
                        .filter_map(|e| e.event.as_ref())
                        .filter_map(|r| r.id.as_ref())
                    {
                        let src_key = pb::IdKey::new(src_event);
                        for cs in &cmd_slices {
                            let fires = cs
                                .command
                                .as_ref()
                                .and_then(|e| e.command.as_ref())
                                .and_then(|r| r.id.as_ref())
                                .is_some_and(|cid| next_commands.contains(&pb::IdKey::new(cid)));
                            if !fires {
                                continue;
                            }
                            for dst_event in cs
                                .emitted_events
                                .iter()
                                .filter_map(|e| e.event.as_ref())
                                .filter_map(|r| r.id.as_ref())
                            {
                                edges
                                    .entry(src_key.clone())
                                    .or_default()
                                    .insert(pb::IdKey::new(dst_event));
                            }
                        }
                    }
                }
                edges
            };

        let is_reachable =
            |scope: &pb::connection::journey::Scope, start: &pb::Id, end: &pb::Id| -> bool {
                let start_key = pb::IdKey::new(start);
                let end_key = pb::IdKey::new(end);
                if start_key == end_key {
                    return true;
                }
                let edges = journey_edges(scope);
                let mut seen: HashSet<pb::IdKey> = HashSet::new();
                let mut stack = vec![start_key];
                while let Some(cur) = stack.pop() {
                    if cur == end_key {
                        return true;
                    }
                    if !seen.insert(cur.clone()) {
                        continue;
                    }
                    if let Some(nexts) = edges.get(&cur) {
                        stack.extend(nexts.iter().cloned());
                    }
                }
                false
            };

        for m in &model.members {
            let (Some(sli_id), Ok(pb::EntityKind::ServiceLevelIndicator)) =
                (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
            else {
                continue;
            };
            let Some(sli) = slis_by_id.get(&pb::IdKey::new(sli_id)) else {
                continue;
            };
            let subject = Some(pb::EntityRef {
                kind: pb::EntityKind::ServiceLevelIndicator as i32,
                id: Some(sli_id.clone()),
            });
            let label = format!("{}/{}@{}", sli_id.namespace, sli_id.slug, sli_id.version);

            let mut command_handling_emitted: Option<HashSet<pb::IdKey>> = None;
            let mut journey_scope: Option<pb::connection::journey::Scope> = None;
            let mut journey_start: Option<pb::Id> = None;
            let mut journey_end: Option<pb::Id> = None;
            let mut journey_correlate_on: Vec<String> = Vec::new();

            match sli.connection.as_ref().and_then(|c| c.kind.as_ref()) {
                Some(pb::connection::Kind::CommandHandling(ch)) => {
                    let Some(slice_id) = ch.command_slice.as_ref().and_then(|s| s.id.as_ref())
                    else {
                        continue;
                    };
                    let key = pb::IdKey::new(slice_id);
                    if let Some(cs) = command_slices_by_id.get(&key) {
                        let emitted: HashSet<pb::IdKey> = cs
                            .emitted_events
                            .iter()
                            .filter_map(|e| e.event.as_ref())
                            .filter_map(|r| r.id.as_ref())
                            .map(pb::IdKey::new)
                            .collect();
                        for (i, ev) in ch.events.iter().enumerate() {
                            let Some(eid) = ev.id.as_ref() else { continue };
                            if !emitted.contains(&pb::IdKey::new(eid)) {
                                issues.push(pb::ValidationIssue {
                                    severity: pb::validation_issue::Severity::Error as i32,
                                    code: "SLI_CONNECTION_UNRESOLVED".into(),
                                    message: format!(
                                        "service level indicator {label} connection.command_handling.events[{i}] ({}/{}@{}) is not among command slice {}/{}@{}'s emitted_events",
                                        eid.namespace, eid.slug, eid.version,
                                        slice_id.namespace, slice_id.slug, slice_id.version,
                                    ),
                                    subject: subject.clone(),
                                    subject_field: format!("connection.command_handling.events[{i}]"),
                                    ..Default::default()
                                });
                            }
                        }
                        command_handling_emitted = Some(emitted);
                    } else {
                        let wrong_kind = read_model_slices_by_id.contains_key(&key)
                            || automation_slices_by_id.contains_key(&key)
                            || ui_slices_by_id.contains_key(&key);
                        issues.push(pb::ValidationIssue {
                            severity: pb::validation_issue::Severity::Error as i32,
                            code: "SLI_CONNECTION_UNRESOLVED".into(),
                            message: format!(
                                "service level indicator {label} connection.command_handling.command_slice {}/{}@{} {}",
                                slice_id.namespace, slice_id.slug, slice_id.version,
                                if wrong_kind { "is not a CommandSlice" } else { "does not exist" },
                            ),
                            subject: subject.clone(),
                            subject_field: "connection.command_handling.command_slice".into(),
                            ..Default::default()
                        });
                    }
                }
                Some(pb::connection::Kind::Projection(p)) => {
                    let Some(slice_id) = p.read_model_slice.as_ref().and_then(|s| s.id.as_ref())
                    else {
                        continue;
                    };
                    let key = pb::IdKey::new(slice_id);
                    if let Some(rs) = read_model_slices_by_id.get(&key) {
                        let sourced: HashSet<pb::IdKey> = rs
                            .source_events
                            .iter()
                            .filter_map(|e| e.event.as_ref())
                            .filter_map(|r| r.id.as_ref())
                            .map(pb::IdKey::new)
                            .collect();
                        for (i, ev) in p.source_events.iter().enumerate() {
                            let Some(eid) = ev.id.as_ref() else { continue };
                            if !sourced.contains(&pb::IdKey::new(eid)) {
                                issues.push(pb::ValidationIssue {
                                    severity: pb::validation_issue::Severity::Error as i32,
                                    code: "SLI_CONNECTION_UNRESOLVED".into(),
                                    message: format!(
                                        "service level indicator {label} connection.projection.source_events[{i}] ({}/{}@{}) is not among read model slice {}/{}@{}'s source_events",
                                        eid.namespace, eid.slug, eid.version,
                                        slice_id.namespace, slice_id.slug, slice_id.version,
                                    ),
                                    subject: subject.clone(),
                                    subject_field: format!("connection.projection.source_events[{i}]"),
                                    ..Default::default()
                                });
                            }
                        }
                    } else {
                        let wrong_kind = command_slices_by_id.contains_key(&key)
                            || automation_slices_by_id.contains_key(&key)
                            || ui_slices_by_id.contains_key(&key);
                        issues.push(pb::ValidationIssue {
                            severity: pb::validation_issue::Severity::Error as i32,
                            code: "SLI_CONNECTION_UNRESOLVED".into(),
                            message: format!(
                                "service level indicator {label} connection.projection.read_model_slice {}/{}@{} {}",
                                slice_id.namespace, slice_id.slug, slice_id.version,
                                if wrong_kind { "is not a ReadModelSlice" } else { "does not exist" },
                            ),
                            subject: subject.clone(),
                            subject_field: "connection.projection.read_model_slice".into(),
                            ..Default::default()
                        });
                    }
                }
                Some(pb::connection::Kind::Reaction(r)) => {
                    let Some(slice_id) = r.automation_slice.as_ref().and_then(|s| s.id.as_ref())
                    else {
                        continue;
                    };
                    let key = pb::IdKey::new(slice_id);
                    if !automation_slices_by_id.contains_key(&key) {
                        let wrong_kind = command_slices_by_id.contains_key(&key)
                            || read_model_slices_by_id.contains_key(&key)
                            || ui_slices_by_id.contains_key(&key);
                        issues.push(pb::ValidationIssue {
                            severity: pb::validation_issue::Severity::Error as i32,
                            code: "SLI_CONNECTION_UNRESOLVED".into(),
                            message: format!(
                                "service level indicator {label} connection.reaction.automation_slice {}/{}@{} {}",
                                slice_id.namespace, slice_id.slug, slice_id.version,
                                if wrong_kind { "is not an AutomationSlice" } else { "does not exist" },
                            ),
                            subject: subject.clone(),
                            subject_field: "connection.reaction.automation_slice".into(),
                            ..Default::default()
                        });
                    }
                }
                Some(pb::connection::Kind::Display(d)) => {
                    let Some(slice_id) = d.ui_slice.as_ref().and_then(|s| s.id.as_ref()) else {
                        continue;
                    };
                    let key = pb::IdKey::new(slice_id);
                    if !ui_slices_by_id.contains_key(&key) {
                        let wrong_kind = command_slices_by_id.contains_key(&key)
                            || read_model_slices_by_id.contains_key(&key)
                            || automation_slices_by_id.contains_key(&key);
                        issues.push(pb::ValidationIssue {
                            severity: pb::validation_issue::Severity::Error as i32,
                            code: "SLI_CONNECTION_UNRESOLVED".into(),
                            message: format!(
                                "service level indicator {label} connection.display.ui_slice {}/{}@{} {}",
                                slice_id.namespace, slice_id.slug, slice_id.version,
                                if wrong_kind { "is not a UiSlice" } else { "does not exist" },
                            ),
                            subject: subject.clone(),
                            subject_field: "connection.display.ui_slice".into(),
                            ..Default::default()
                        });
                    }
                }
                Some(pb::connection::Kind::Integration(ig)) => {
                    if let (Some(proc_id), Some(sys_id)) = (
                        ig.processor.as_ref().and_then(|r| r.id.as_ref()),
                        ig.system.as_ref().and_then(|r| r.id.as_ref()),
                    ) {
                        if let Some(proc) = processors_by_id.get(&pb::IdKey::new(proc_id)) {
                            let calls = proc
                                .calls
                                .iter()
                                .filter_map(|r| r.id.as_ref())
                                .any(|cid| pb::IdKey::new(cid) == pb::IdKey::new(sys_id));
                            if !calls {
                                issues.push(pb::ValidationIssue {
                                    severity: pb::validation_issue::Severity::Error as i32,
                                    code: "SLI_CONNECTION_UNRESOLVED".into(),
                                    message: format!(
                                        "service level indicator {label} connection.integration.system {}/{}@{} is not in processor {}/{}@{}'s calls",
                                        sys_id.namespace, sys_id.slug, sys_id.version,
                                        proc_id.namespace, proc_id.slug, proc_id.version,
                                    ),
                                    subject: subject.clone(),
                                    subject_field: "connection.integration.system".into(),
                                    ..Default::default()
                                });
                            }
                        }
                    }
                }
                Some(pb::connection::Kind::Journey(j)) => {
                    journey_scope.clone_from(&j.scope);
                    journey_start = j.start.as_ref().and_then(|r| r.id.clone());
                    journey_end = j.end.as_ref().and_then(|r| r.id.clone());
                    journey_correlate_on.clone_from(&j.correlate_on);
                }
                None => {}
            }

            // SLI_JOURNEY_UNREACHABLE
            if let (Some(scope), Some(start), Some(end)) = (
                journey_scope.as_ref(),
                journey_start.as_ref(),
                journey_end.as_ref(),
            ) {
                if !is_reachable(scope, start, end) {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "SLI_JOURNEY_UNREACHABLE".into(),
                        message: format!(
                            "service level indicator {label} connection.journey end {}/{}@{} is not reachable from start {}/{}@{} within its scope",
                            end.namespace, end.slug, end.version,
                            start.namespace, start.slug, start.version,
                        ),
                        subject: subject.clone(),
                        subject_field: "connection.journey".into(),
                        ..Default::default()
                    });
                }
            }

            // SLI_CORRELATION_UNSOURCED
            if let (Some(start), Some(end)) = (journey_start.as_ref(), journey_end.as_ref()) {
                let field_names = |eid: &pb::Id| -> HashSet<String> {
                    events_by_id
                        .get(&pb::IdKey::new(eid))
                        .map(|e| {
                            trogon_atlas_core::schema::schema_fields(e.schema.as_ref())
                                .into_iter()
                                .map(|f| f.name)
                                .collect()
                        })
                        .unwrap_or_default()
                };
                let start_fields = field_names(start);
                let end_fields = field_names(end);
                for (i, name) in journey_correlate_on.iter().enumerate() {
                    if !start_fields.contains(name) {
                        issues.push(pb::ValidationIssue {
                            severity: pb::validation_issue::Severity::Error as i32,
                            code: "SLI_CORRELATION_UNSOURCED".into(),
                            message: format!(
                                "service level indicator {label} connection.journey.correlate_on[{i}] `{name}` is not a field of start event {}/{}@{}",
                                start.namespace, start.slug, start.version,
                            ),
                            subject: subject.clone(),
                            subject_field: format!("connection.journey.correlate_on[{i}]"),
                            ..Default::default()
                        });
                    }
                    if !end_fields.contains(name) {
                        issues.push(pb::ValidationIssue {
                            severity: pb::validation_issue::Severity::Error as i32,
                            code: "SLI_CORRELATION_UNSOURCED".into(),
                            message: format!(
                                "service level indicator {label} connection.journey.correlate_on[{i}] `{name}` is not a field of end event {}/{}@{}",
                                end.namespace, end.slug, end.version,
                            ),
                            subject: subject.clone(),
                            subject_field: format!("connection.journey.correlate_on[{i}]"),
                            ..Default::default()
                        });
                    }
                }
            }

            // SLI_OUTCOME_UNREACHABLE
            if let Some(pb::service_level_indicator::Measure::OutcomeRatio(or)) =
                sli.measure.as_ref()
            {
                if or.good.is_empty() {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "SLI_OUTCOME_UNREACHABLE".into(),
                        message: format!(
                            "service level indicator {label} measure.outcome_ratio.good is empty; a ratio needs a success side"
                        ),
                        subject: subject.clone(),
                        subject_field: "measure.outcome_ratio.good".into(),
                        ..Default::default()
                    });
                }
                if or.bad.is_empty() {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "SLI_OUTCOME_UNREACHABLE".into(),
                        message: format!(
                            "service level indicator {label} measure.outcome_ratio.bad is empty; a ratio needs a failure side"
                        ),
                        subject: subject.clone(),
                        subject_field: "measure.outcome_ratio.bad".into(),
                        ..Default::default()
                    });
                }
                let mut check_terminal = |events: &[pb::EventRef], field: &str| {
                    for (i, ev) in events.iter().enumerate() {
                        let Some(eid) = ev.id.as_ref() else { continue };
                        let key = pb::IdKey::new(eid);
                        let ok = if let Some(emitted) = &command_handling_emitted {
                            emitted.contains(&key)
                        } else if let (Some(scope), Some(start), Some(end)) = (
                            journey_scope.as_ref(),
                            journey_start.as_ref(),
                            journey_end.as_ref(),
                        ) {
                            key == pb::IdKey::new(end) || is_reachable(scope, start, eid)
                        } else {
                            true
                        };
                        if !ok {
                            issues.push(pb::ValidationIssue {
                                severity: pb::validation_issue::Severity::Error as i32,
                                code: "SLI_OUTCOME_UNREACHABLE".into(),
                                message: format!(
                                    "service level indicator {label} {field}[{i}] ({}/{}@{}) cannot terminate the connection",
                                    eid.namespace, eid.slug, eid.version,
                                ),
                                subject: subject.clone(),
                                subject_field: format!("{field}[{i}]"),
                                ..Default::default()
                            });
                        }
                    }
                };
                check_terminal(&or.good, "measure.outcome_ratio.good");
                check_terminal(&or.bad, "measure.outcome_ratio.bad");
            }

            // SLI_SIGNAL_MISMATCH
            match sli.signal.as_ref().and_then(|s| s.source.as_ref()) {
                None => {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "SLI_SIGNAL_MISMATCH".into(),
                        message: format!(
                            "service level indicator {label} has no signal; nothing says where the measurement comes from"
                        ),
                        subject: subject.clone(),
                        subject_field: "signal".into(),
                        ..Default::default()
                    });
                }
                Some(pb::signal::Source::EventTimestamps(_)) => {
                    let mismatched = matches!(
                        sli.connection.as_ref().and_then(|c| c.kind.as_ref()),
                        Some(
                            pb::connection::Kind::Display(_) | pb::connection::Kind::Integration(_)
                        )
                    );
                    if mismatched {
                        issues.push(pb::ValidationIssue {
                            severity: pb::validation_issue::Severity::Error as i32,
                            code: "SLI_SIGNAL_MISMATCH".into(),
                            message: format!(
                                "service level indicator {label} uses an EventTimestamps signal but its connection has no event marking the end"
                            ),
                            subject: subject.clone(),
                            subject_field: "signal".into(),
                            ..Default::default()
                        });
                    }
                }
                Some(pb::signal::Source::Telemetry(_)) => {}
            }
            if sli.measure.is_none() {
                issues.push(pb::ValidationIssue {
                    severity: pb::validation_issue::Severity::Error as i32,
                    code: "SLI_SIGNAL_MISMATCH".into(),
                    message: format!("service level indicator {label} has no measure declared"),
                    subject: subject.clone(),
                    subject_field: "measure".into(),
                    ..Default::default()
                });
            }
        }

        for m in &model.members {
            let (Some(slo_id), Ok(pb::EntityKind::ServiceLevelObjective)) =
                (m.id.as_ref(), pb::EntityKind::try_from(m.kind))
            else {
                continue;
            };
            let Some(slo) = slos_by_id.get(&pb::IdKey::new(slo_id)) else {
                continue;
            };
            let subject = Some(pb::EntityRef {
                kind: pb::EntityKind::ServiceLevelObjective as i32,
                id: Some(slo_id.clone()),
            });
            let label = format!("{}/{}@{}", slo_id.namespace, slo_id.slug, slo_id.version);

            let is_latency = slo
                .indicator
                .as_ref()
                .and_then(|r| r.id.as_ref())
                .and_then(|id| slis_by_id.get(&pb::IdKey::new(id)))
                .is_some_and(|sli| {
                    matches!(
                        sli.measure,
                        Some(pb::service_level_indicator::Measure::Latency(_))
                    )
                });

            for (i, obj) in slo.objectives.iter().enumerate() {
                if is_latency && obj.threshold.is_none() {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "SLO_THRESHOLD_MISMATCH".into(),
                        message: format!(
                            "service level objective {label} objectives[{i}] has no threshold but its indicator measures latency"
                        ),
                        subject: subject.clone(),
                        subject_field: format!("objectives[{i}].threshold"),
                        ..Default::default()
                    });
                }
                if !is_latency && obj.threshold.is_some() {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Error as i32,
                        code: "SLO_THRESHOLD_MISMATCH".into(),
                        message: format!(
                            "service level objective {label} objectives[{i}] has a threshold but its indicator does not measure latency"
                        ),
                        subject: subject.clone(),
                        subject_field: format!("objectives[{i}].threshold"),
                        ..Default::default()
                    });
                }
                for (field_name, target) in [
                    ("target", obj.target.as_ref()),
                    ("time_slice_target", obj.time_slice_target.as_ref()),
                ] {
                    if let Some(t) = target {
                        if !(t.ratio > 0.0 && t.ratio < 1.0) {
                            issues.push(pb::ValidationIssue {
                                severity: pb::validation_issue::Severity::Error as i32,
                                code: "SLO_TARGET_RANGE".into(),
                                message: format!(
                                    "service level objective {label} objectives[{i}].{field_name} ratio {} is not strictly between 0 and 1",
                                    t.ratio,
                                ),
                                subject: subject.clone(),
                                subject_field: format!("objectives[{i}].{field_name}"),
                                ..Default::default()
                            });
                        }
                    }
                }
            }

            if slo.alert_policies.is_empty() {
                let is_external = matches!(
                    slo.commitment.as_ref().and_then(|c| c.kind.as_ref()),
                    Some(pb::commitment::Kind::External(_))
                );
                if is_external {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Warning as i32,
                        code: "SLO_EXTERNAL_UNALERTED".into(),
                        message: format!(
                            "service level objective {label} has an External commitment but no alert_policies"
                        ),
                        subject: subject.clone(),
                        subject_field: "alert_policies".into(),
                        ..Default::default()
                    });
                } else {
                    issues.push(pb::ValidationIssue {
                        severity: pb::validation_issue::Severity::Info as i32,
                        code: "SLO_UNALERTED".into(),
                        message: format!("service level objective {label} has no alert_policies"),
                        subject: subject.clone(),
                        subject_field: "alert_policies".into(),
                        ..Default::default()
                    });
                }
            }
        }
    }

    enrich_issues_with_catalog(&mut issues);
    issues
}

/// Extracts the top-level `{...}` placeholders from a `stream_id` template,
/// treating nested braces as part of the enclosing placeholder so inline
/// expressions like `{uuidv5(a/{b})}` surface as one malformed token.
fn stream_id_placeholders(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '{' {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut depth = 1usize;
        let mut j = start;
        while j < chars.len() && depth > 0 {
            match chars[j] {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            j += 1;
        }
        let end = if depth == 0 { j - 1 } else { chars.len() };
        out.push(chars[start..end].iter().collect());
        i = j;
    }
    out
}

#[cfg(test)]
mod stream_id_tests {
    use prost::Message;

    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn lane(stream_id: &str, metadata: Vec<prost_types::Any>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Swimlane(pb::Swimlane {
                id: Some(id("lane")),
                title: "Lane".into(),
                stream_id: stream_id.into(),
                metadata,
                ..Default::default()
            })),
        }
    }

    fn event_on_lane(field: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id("ev")),
                title: "ev".into(),
                swimlane: Some(pb::SwimlaneRef {
                    id: Some(id("lane")),
                }),
                schema: Some(trogon_atlas_core::schema::pack_fields(vec![
                    pb::FieldSpec {
                        name: field.into(),
                        r#type: Some(pb::FieldType {
                            kind: Some(pb::field_type::Kind::String(pb::field_type::StringType {})),
                        }),
                        ..Default::default()
                    },
                ])),
                ..Default::default()
            })),
        }
    }

    fn model() -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::Swimlane as i32,
                id: Some(id("lane")),
            }],
            ..Default::default()
        }
    }

    fn codes(issues: &[pb::ValidationIssue]) -> Vec<&str> {
        issues
            .iter()
            .filter(|i| i.code.starts_with("STREAM_ID"))
            .map(|i| i.code.as_str())
            .collect()
    }

    #[test]
    fn placeholders_handle_nesting() {
        assert_eq!(
            stream_id_placeholders("a-{x}-{uuidv5(p/{q}/{r})}"),
            vec!["x".to_string(), "uuidv5(p/{q}/{r})".to_string()]
        );
    }

    #[test]
    fn field_backed_placeholder_is_clean() {
        let all = vec![
            stored(lane("user-{username}", vec![])),
            stored(event_on_lane("username")),
        ];
        assert_eq!(codes(&validate_model(&model(), &all)), [] as [&str; 0]);
    }

    #[test]
    fn named_identity_placeholder_is_clean() {
        let ann = pb::Uuidv5IdentityAnnotation {
            name: "eligibility_id".into(),
            template: "x/{a}/{b}".into(),
            version: 1,
            ..Default::default()
        };
        let any = prost_types::Any {
            type_url:
                "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Uuidv5IdentityAnnotation"
                    .into(),
            value: ann.encode_to_vec(),
        };
        let all = vec![stored(lane("re-{eligibility_id}", vec![any]))];
        assert_eq!(codes(&validate_model(&model(), &all)), [] as [&str; 0]);
    }

    #[test]
    fn inline_expression_is_malformed_error() {
        let all = vec![stored(lane("re-{uuidv5(x/{a}/{b})}", vec![]))];
        let issues = validate_model(&model(), &all);
        assert_eq!(codes(&issues), vec!["STREAM_ID_MALFORMED_PLACEHOLDER"]);
        assert!(issues
            .iter()
            .any(|i| i.code == "STREAM_ID_MALFORMED_PLACEHOLDER"
                && i.severity == pb::validation_issue::Severity::Error as i32));
    }

    #[test]
    fn unknown_name_is_unresolved_warning() {
        let all = vec![
            stored(lane("user-{nope}", vec![])),
            stored(event_on_lane("username")),
        ];
        let issues = validate_model(&model(), &all);
        assert_eq!(codes(&issues), vec!["STREAM_ID_UNRESOLVED_PLACEHOLDER"]);
        assert!(issues
            .iter()
            .any(|i| i.code == "STREAM_ID_UNRESOLVED_PLACEHOLDER"
                && i.severity == pb::validation_issue::Severity::Warning as i32));
    }
}

#[cfg(test)]
mod transition_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn event(slug: &str, lane: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(slug)),
                title: slug.into(),
                swimlane: Some(pb::SwimlaneRef { id: Some(id(lane)) }),
                ..Default::default()
            })),
        }
    }

    fn lane_with_transitions(slug: &str, after: &str, next: &[&str]) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Swimlane(pb::Swimlane {
                id: Some(id(slug)),
                title: slug.into(),
                transitions: vec![pb::swimlane::Transition {
                    after: Some(pb::EventRef {
                        id: Some(id(after)),
                    }),
                    next: next
                        .iter()
                        .map(|n| pb::swimlane::transition::Next {
                            event: Some(pb::EventRef { id: Some(id(n)) }),
                            doc: "guard".into(),
                            metadata: Vec::new(),
                        })
                        .collect(),
                }],
                ..Default::default()
            })),
        }
    }

    fn slice_with_scenario(given: &[&str], emit: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id("slice")),
                title: "slice".into(),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef {
                        id: Some(id("cmd")),
                    }),
                    ..Default::default()
                }),
                scenarios: vec![pb::CommandScenario {
                    id: "s1".into(),
                    title: "s1".into(),
                    given: given
                        .iter()
                        .map(|g| pb::EventExample {
                            event: Some(pb::EventRef { id: Some(id(g)) }),
                            payload: None,
                        })
                        .collect(),
                    when: Some(pb::CommandExample {
                        command: Some(pb::CommandRef {
                            id: Some(id("cmd")),
                        }),
                        payload: None,
                    }),
                    then: Some(pb::command_scenario::Then::Emit(pb::EmittedEvents {
                        events: vec![pb::EventExample {
                            event: Some(pb::EventRef { id: Some(id(emit)) }),
                            payload: None,
                        }],
                    })),
                    ..Default::default()
                }],
                ..Default::default()
            })),
        }
    }

    fn base() -> Vec<StoredEntity> {
        vec![
            stored(event("reserved", "lane")),
            stored(event("claimed", "lane")),
            stored(event("expired", "lane")),
            stored(lane_with_transitions(
                "lane",
                "reserved",
                &["claimed", "expired"],
            )),
            stored(pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::Command(pb::Command {
                    id: Some(id("cmd")),
                    title: "cmd".into(),
                    ..Default::default()
                })),
            }),
        ]
    }

    fn codes(issues: &[pb::ValidationIssue], code: &str) -> usize {
        issues.iter().filter(|i| i.code == code).count()
    }

    #[test]
    fn legal_successor_is_clean() {
        let slice = slice_with_scenario(&["reserved"], "claimed");
        assert_eq!(
            codes(
                &validate_scenarios(&slice, &base()),
                "SCENARIO_TRANSITION_VIOLATION"
            ),
            0
        );
    }

    #[test]
    fn illegal_successor_warns() {
        // claimed -> expired is not in the table (only reserved -> {claimed, expired})
        let mut all = base();
        all.push(stored(lane_with_transitions("other", "x", &["y"])));
        let slice = slice_with_scenario(&["reserved", "claimed"], "expired");
        let issues = validate_scenarios(&slice, &all);
        // last given on lane is `claimed`; no transition declared for it -> skip.
        assert_eq!(codes(&issues, "SCENARIO_TRANSITION_VIOLATION"), 0);
        // but emitting `expired` straight after `expired` given violates reserved's table? craft direct case:
        let slice2 = slice_with_scenario(&["reserved"], "reserved");
        let issues2 = validate_scenarios(&slice2, &base());
        assert_eq!(codes(&issues2, "SCENARIO_TRANSITION_VIOLATION"), 1);
    }

    #[test]
    fn given_history_must_walk_transitions_forward() {
        // reserved -> {claimed, expired}; given [reserved, reserved] walks backward in time.
        let slice = slice_with_scenario(&["reserved", "reserved"], "claimed");
        let issues = validate_scenarios(&slice, &base());
        assert_eq!(
            codes(&issues, "SCENARIO_TRANSITION_VIOLATION"),
            1,
            "{issues:?}"
        );
        // a legal forward walk is clean: reserved -> claimed
        let ok = slice_with_scenario(&["reserved", "claimed"], "claimed");
        let issues_ok = validate_scenarios(&ok, &base());
        // emission claimed-after-claimed has no declared transition -> only given check applies, which passed
        assert_eq!(
            codes(&issues_ok, "SCENARIO_TRANSITION_VIOLATION"),
            0,
            "{issues_ok:?}"
        );
    }

    #[test]
    fn foreign_event_in_table_errors() {
        let mut all = base();
        all.push(stored(event("alien", "other-lane")));
        all.push(stored(lane_with_transitions(
            "lane2",
            "alien",
            &["claimed"],
        )));
        let model = pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::Swimlane as i32,
                id: Some(id("lane2")),
            }],
            ..Default::default()
        };
        let issues = validate_model(&model, &all);
        assert_eq!(codes(&issues, "TRANSITION_FOREIGN_EVENT"), 2, "{issues:?}");
    }
}

#[cfg(test)]
mod scenario_gap_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    // Catalog SCENARIO_MISSING_REF: each When step must point at a concrete
    // entity. A CommandScenario with when=None used to skip the check entirely.
    #[test]
    fn command_scenario_missing_when_errors() {
        let slice = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id("slice")),
                title: "slice".into(),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef {
                        id: Some(id("cmd")),
                    }),
                    ..Default::default()
                }),
                scenarios: vec![pb::CommandScenario {
                    id: "s1".into(),
                    title: "no when".into(),
                    doc: String::new(),
                    given: Vec::new(),
                    when: None,
                    then: Some(pb::command_scenario::Then::Emit(pb::EmittedEvents {
                        events: vec![pb::EventExample {
                            event: Some(pb::EventRef {
                                id: Some(id("happened")),
                            }),
                            payload: None,
                        }],
                    })),
                    metadata: Vec::new(),
                }],
                ..Default::default()
            })),
        };
        let issues = validate_scenarios(&slice, &[]);
        assert!(
            issues
                .iter()
                .any(|i| { i.code == "SCENARIO_MISSING_REF" && i.subject_field.contains("when") }),
            "command scenario without when must emit SCENARIO_MISSING_REF; got {issues:?}"
        );
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;

    fn id(ns: &str, slug: &str) -> pb::Id {
        pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn member(kind: pb::EntityKind, ns: &str, slug: &str) -> pb::EntityRef {
        pb::EntityRef {
            kind: kind as i32,
            id: Some(id(ns, slug)),
        }
    }

    fn event(ns: &str, slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(ns, slug)),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn count(issues: &[pb::ValidationIssue], code: &str) -> usize {
        issues.iter().filter(|i| i.code == code).count()
    }

    #[test]
    fn subscription_view_seam_is_legal() {
        // downstream read model sources an upstream event: the ONE seam.
        let view = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id("downstream", "orders.delivered")),
                title: "orders.delivered".into(),
                source_events: vec![pb::EventRef {
                    id: Some(id("upstream", "order.fulfilled")),
                }],
                ..Default::default()
            })),
        };
        let model = pb::EventModel {
            id: Some(id("downstream", "m")),
            title: "m".into(),
            members: vec![member(
                pb::EntityKind::ReadModel,
                "downstream",
                "orders.delivered",
            )],
            ..Default::default()
        };
        let all = vec![stored(view), stored(event("upstream", "order.fulfilled"))];
        let issues = validate_model(&model, &all);
        assert_eq!(
            count(&issues, "CROSS_MODEL_BOUNDARY_VIOLATION"),
            0,
            "{issues:?}"
        );
        assert_eq!(count(&issues, "CROSS_MODEL_REF"), 0); // declared seam: legal, silent
    }

    #[test]
    fn cross_context_command_emission_is_violation() {
        // a slice emitting another context's event reaches into its stream.
        let slice = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id("downstream", "bad-slice")),
                title: "bad".into(),
                emitted_events: vec![pb::EventEdge {
                    event: Some(pb::EventRef {
                        id: Some(id("upstream", "order.fulfilled")),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
        };
        let model = pb::EventModel {
            id: Some(id("downstream", "m")),
            title: "m".into(),
            members: vec![member(
                pb::EntityKind::CommandSlice,
                "downstream",
                "bad-slice",
            )],
            ..Default::default()
        };
        let all = vec![stored(slice), stored(event("upstream", "order.fulfilled"))];
        let issues = validate_model(&model, &all);
        assert_eq!(
            count(&issues, "CROSS_MODEL_BOUNDARY_VIOLATION"),
            1,
            "{issues:?}"
        );
    }

    /// Screens live in an application namespace by design; UI.slot.screen is
    /// composition metadata, not a behavioral cross-context seam. Flagging it
    /// as CROSS_MODEL_BOUNDARY_VIOLATION rejects every valid Screen fill.
    #[test]
    fn bug_ui_slot_screen_cross_namespace_is_legal() {
        let ui_ent = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Ui(pb::Ui {
                id: Some(id("checkout", "order-panel")),
                title: "order-panel".into(),
                slot: Some(pb::ScreenSlotRef {
                    screen: Some(pb::ScreenRef {
                        id: Some(id("app", "checkout")),
                    }),
                    slot: "main".into(),
                }),
                ..Default::default()
            })),
        };
        let screen_ent = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Screen(pb::Screen {
                id: Some(id("app", "checkout")),
                title: "checkout".into(),
                ..Default::default()
            })),
        };
        let model = pb::EventModel {
            id: Some(id("checkout", "m")),
            title: "m".into(),
            members: vec![member(pb::EntityKind::Ui, "checkout", "order-panel")],
            ..Default::default()
        };
        let issues = validate_model(&model, &[stored(ui_ent), stored(screen_ent)]);
        assert_eq!(
            count(&issues, "CROSS_MODEL_BOUNDARY_VIOLATION"),
            0,
            "UI.slot.screen into application namespace must be legal; got {issues:?}"
        );
        assert_eq!(
            count(&issues, "CROSS_MODEL_REF"),
            0,
            "slot.screen is a declared composition seam; got {issues:?}"
        );
    }

    /// Ambiguity.terms are documented as a legal cross-namespace knowledge
    /// ref class (strategic backlog). The allowlist must not treat them as
    /// behavioral boundary violations.
    #[test]
    fn bug_ambiguity_terms_cross_namespace_is_legal() {
        let amb = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Ambiguity(pb::Ambiguity {
                id: Some(id("marketplace", "review-collision")),
                kind: pb::ambiguity::Kind::Homonym as i32,
                ruling: pb::ambiguity::Ruling::Deliberate as i32,
                terms: vec![
                    pb::TermRef {
                        id: Some(id("checkout", "review")),
                    },
                    pb::TermRef {
                        id: Some(id("payments", "review")),
                    },
                ],
                ..Default::default()
            })),
        };
        let t1 = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Term(pb::Term {
                id: Some(id("checkout", "review")),
                title: "review".into(),
                ..Default::default()
            })),
        };
        let t2 = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Term(pb::Term {
                id: Some(id("payments", "review")),
                title: "review".into(),
                ..Default::default()
            })),
        };
        let model = pb::EventModel {
            id: Some(id("marketplace", "m")),
            title: "m".into(),
            members: vec![member(
                pb::EntityKind::Ambiguity,
                "marketplace",
                "review-collision",
            )],
            ..Default::default()
        };
        let issues = validate_model(&model, &[stored(amb), stored(t1), stored(t2)]);
        assert_eq!(
            count(&issues, "CROSS_MODEL_BOUNDARY_VIOLATION"),
            0,
            "Ambiguity.terms[] cross-namespace must be legal; got {issues:?}"
        );
        assert_eq!(
            count(&issues, "CROSS_MODEL_REF"),
            0,
            "Ambiguity.terms[] is a declared knowledge seam; got {issues:?}"
        );
    }

    /// outbound_refs emits `entry.observer.human.ui`, but the seam allowlist
    /// still checks the stale `entry.human.ui` path, so legal choreography
    /// storyboards falsely trip CROSS_MODEL_BOUNDARY_VIOLATION.
    #[test]
    fn bug_storyboard_entry_observer_ui_cross_namespace_is_legal() {
        let board = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Storyboard(pb::Storyboard {
                id: Some(id("downstream", "flow")),
                title: "flow".into(),
                entry: Some(pb::StoryboardEntry {
                    observer: Some(pb::storyboard_entry::Observer::Human(
                        pb::storyboard_entry::HumanObserver {
                            ui: Some(pb::UiEdge {
                                ui: Some(pb::UiRef {
                                    id: Some(id("upstream", "shared-ui")),
                                }),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        };
        let ui_ent = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Ui(pb::Ui {
                id: Some(id("upstream", "shared-ui")),
                title: "shared-ui".into(),
                ..Default::default()
            })),
        };
        let model = pb::EventModel {
            id: Some(id("downstream", "m")),
            title: "m".into(),
            members: vec![member(pb::EntityKind::Storyboard, "downstream", "flow")],
            ..Default::default()
        };
        let issues = validate_model(&model, &[stored(board), stored(ui_ent)]);
        assert_eq!(
            count(&issues, "CROSS_MODEL_BOUNDARY_VIOLATION"),
            0,
            "entry.observer.human.ui must match outbound_refs path; got {issues:?}"
        );
        assert_eq!(
            count(&issues, "CROSS_MODEL_REF"),
            0,
            "entry.observer.human.ui is a declared choreography seam; got {issues:?}"
        );
    }
}

#[cfg(test)]
mod context_root_tests {
    use super::*;

    fn bc(ns: &str, slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::BoundedContext(pb::BoundedContext {
                id: Some(pb::Id {
                    namespace: ns.into(),
                    slug: slug.into(),
                    version: 1,
                }),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    #[test]
    fn slug_equal_to_namespace_is_clean() {
        let issues = validate_scenarios(&bc("accounts", "accounts"), &[]);
        assert!(
            !issues.iter().any(|i| i.code == "CONTEXT_SLUG_MISMATCH"),
            "{issues:?}"
        );
    }

    #[test]
    fn slug_diverging_from_namespace_errors() {
        let issues = validate_scenarios(&bc("accounts", "identity"), &[]);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "CONTEXT_SLUG_MISMATCH")
                .count(),
            1
        );
    }
}

#[cfg(test)]
mod observed_event_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn event(slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn model_with_event(slug: &str) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id(slug)),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn unobserved_event_warns() {
        let all = vec![stored(event("ghost"))];
        let issues = validate_model(&model_with_event("ghost"), &all);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "EVENT_UNOBSERVED")
                .count(),
            1,
            "{issues:?}"
        );
    }

    #[test]
    fn observed_event_is_clean() {
        let view = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id("view")),
                title: "view".into(),
                source_events: vec![pb::EventRef {
                    id: Some(id("seen")),
                }],
                ..Default::default()
            })),
        };
        let all = vec![stored(event("seen")), stored(view)];
        let issues = validate_model(&model_with_event("seen"), &all);
        assert!(
            !issues.iter().any(|i| i.code == "EVENT_UNOBSERVED"),
            "{issues:?}"
        );
    }
}

#[cfg(test)]
mod command_emission_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn command(slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Command(pb::Command {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn model_with_command(slug: &str) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::Command as i32,
                id: Some(id(slug)),
            }],
            ..Default::default()
        }
    }

    fn command_slice(slug: &str, cmd_slug: &str, events: Vec<&str>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id(slug)),
                title: slug.into(),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef {
                        id: Some(id(cmd_slug)),
                    }),
                    ..Default::default()
                }),
                emitted_events: events
                    .into_iter()
                    .map(|ev| pb::EventEdge {
                        event: Some(pb::EventRef { id: Some(id(ev)) }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    #[test]
    fn command_with_no_slice_warns() {
        let all = vec![stored(command("ghost"))];
        let issues = validate_model(&model_with_command("ghost"), &all);
        let hits: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "COMMAND_NO_EMITTED_EVENTS")
            .collect();
        assert_eq!(hits.len(), 1, "{issues:?}");
        assert_eq!(
            hits[0].severity,
            pb::validation_issue::Severity::Warning as i32
        );
    }

    #[test]
    fn command_with_slice_but_no_events_warns() {
        let all = vec![
            stored(command("dispatch")),
            stored(command_slice("dispatch-slice", "dispatch", vec![])),
        ];
        let issues = validate_model(&model_with_command("dispatch"), &all);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "COMMAND_NO_EMITTED_EVENTS")
                .count(),
            1,
            "{issues:?}",
        );
    }

    #[test]
    fn command_with_slice_and_events_is_clean() {
        let all = vec![
            stored(command("fire")),
            stored(command_slice("fire-slice", "fire", vec!["fired"])),
        ];
        let issues = validate_model(&model_with_command("fire"), &all);
        assert!(
            !issues.iter().any(|i| i.code == "COMMAND_NO_EMITTED_EVENTS"),
            "{issues:?}",
        );
    }
}

#[cfg(test)]
mod command_issuer_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn command(slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Command(pb::Command {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn ui(slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Ui(pb::Ui {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn processor(slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Processor(pb::Processor {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn command_slice_with_ui(slug: &str, ui_slug: &str, cmd_slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id(slug)),
                title: slug.into(),
                ui: Some(pb::UiEdge {
                    ui: Some(pb::UiRef {
                        id: Some(id(ui_slug)),
                    }),
                    ..Default::default()
                }),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef {
                        id: Some(id(cmd_slug)),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        }
    }

    fn command_handler_slice(slug: &str, cmd_slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id(slug)),
                title: slug.into(),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef {
                        id: Some(id(cmd_slug)),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        }
    }

    fn automation_slice(slug: &str, proc_slug: &str, cmd_slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
                id: Some(id(slug)),
                title: slug.into(),
                processor: Some(pb::ProcessorEdge {
                    processor: Some(pb::ProcessorRef {
                        id: Some(id(proc_slug)),
                    }),
                    ..Default::default()
                }),
                emitted_command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef {
                        id: Some(id(cmd_slug)),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        }
    }

    fn model_with_command(slug: &str) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::Command as i32,
                id: Some(id(slug)),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn single_ui_issuer_is_clean() {
        let all = vec![
            stored(command("post")),
            stored(ui("composer")),
            stored(command_slice_with_ui("post-slice", "composer", "post")),
        ];
        let issues = validate_model(&model_with_command("post"), &all);
        assert!(
            !issues.iter().any(|i| i.code == "COMMAND_MULTIPLE_ISSUERS"),
            "{issues:?}",
        );
    }

    #[test]
    fn same_ui_referenced_twice_is_clean() {
        let all = vec![
            stored(command("post")),
            stored(ui("composer")),
            stored(command_slice_with_ui("post-slice-a", "composer", "post")),
            stored(command_slice_with_ui("post-slice-b", "composer", "post")),
        ];
        let issues = validate_model(&model_with_command("post"), &all);
        assert!(
            !issues.iter().any(|i| i.code == "COMMAND_MULTIPLE_ISSUERS"),
            "{issues:?}",
        );
    }

    #[test]
    fn processor_issuer_with_command_handler_slice_is_clean() {
        let all = vec![
            stored(command("post")),
            stored(processor("auto-poster")),
            stored(automation_slice("auto-slice", "auto-poster", "post")),
            stored(command_handler_slice("post-handler", "post")),
        ];
        let issues = validate_model(&model_with_command("post"), &all);
        assert!(
            !issues.iter().any(|i| i.code == "COMMAND_MULTIPLE_ISSUERS"),
            "{issues:?}",
        );
        assert!(
            !issues.iter().any(|i| i.code == "COMMAND_HAS_NO_TRIGGER"),
            "{issues:?}",
        );
    }

    #[test]
    fn two_distinct_uis_error() {
        let all = vec![
            stored(command("post")),
            stored(ui("composer")),
            stored(ui("moderation-panel")),
            stored(command_slice_with_ui("post-slice", "composer", "post")),
            stored(command_slice_with_ui(
                "moderate-slice",
                "moderation-panel",
                "post",
            )),
        ];
        let issues = validate_model(&model_with_command("post"), &all);
        let hits: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "COMMAND_MULTIPLE_ISSUERS")
            .collect();
        assert_eq!(hits.len(), 1, "{issues:?}");
        assert_eq!(
            hits[0].severity,
            pb::validation_issue::Severity::Error as i32
        );
    }

    #[test]
    fn ui_plus_processor_errors() {
        let all = vec![
            stored(command("post")),
            stored(ui("composer")),
            stored(processor("auto-poster")),
            stored(command_slice_with_ui("post-slice", "composer", "post")),
            stored(automation_slice("auto-slice", "auto-poster", "post")),
        ];
        let issues = validate_model(&model_with_command("post"), &all);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "COMMAND_MULTIPLE_ISSUERS")
                .count(),
            1,
            "{issues:?}",
        );
    }

    #[test]
    fn two_distinct_processors_error() {
        let all = vec![
            stored(command("retry")),
            stored(processor("a")),
            stored(processor("b")),
            stored(automation_slice("a-slice", "a", "retry")),
            stored(automation_slice("b-slice", "b", "retry")),
        ];
        let issues = validate_model(&model_with_command("retry"), &all);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "COMMAND_MULTIPLE_ISSUERS")
                .count(),
            1,
            "{issues:?}",
        );
    }
}

#[cfg(test)]
mod information_completeness_tests {
    use prost::Message;

    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn field(name: &str, kind: pb::field_type::Kind) -> pb::FieldSpec {
        pb::FieldSpec {
            name: name.into(),
            r#type: Some(pb::FieldType { kind: Some(kind) }),
            ..Default::default()
        }
    }

    fn derived_field(name: &str, kind: pb::field_type::Kind) -> pb::FieldSpec {
        let annotation = pb::DerivedFieldAnnotation {
            doc: "runtime supplies this value".into(),
            kind: 0,
        };
        pb::FieldSpec {
            name: name.into(),
            r#type: Some(pb::FieldType { kind: Some(kind) }),
            metadata: vec![prost_types::Any {
                type_url:
                    "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.DerivedFieldAnnotation"
                        .into(),
                value: annotation.encode_to_vec(),
            }],
            ..Default::default()
        }
    }

    fn string_kind() -> pb::field_type::Kind {
        pb::field_type::Kind::String(pb::field_type::StringType {})
    }

    fn timestamp_kind() -> pb::field_type::Kind {
        pb::field_type::Kind::Timestamp(pb::field_type::TimestampType {})
    }

    fn lane(slug: &str, state: Vec<pb::FieldSpec>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Swimlane(pb::Swimlane {
                id: Some(id(slug)),
                state,
                ..Default::default()
            })),
        }
    }

    fn command(slug: &str, lane: &str, fields: Vec<pb::FieldSpec>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Command(pb::Command {
                id: Some(id(slug)),
                swimlane: Some(pb::SwimlaneRef { id: Some(id(lane)) }),
                schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
                ..Default::default()
            })),
        }
    }

    fn event(slug: &str, lane: &str, fields: Vec<pb::FieldSpec>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(slug)),
                swimlane: Some(pb::SwimlaneRef { id: Some(id(lane)) }),
                schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
                ..Default::default()
            })),
        }
    }

    fn read_model(slug: &str, fields: Vec<pb::FieldSpec>, source_events: Vec<&str>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(slug)),
                schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
                source_events: source_events
                    .into_iter()
                    .map(|slug| pb::EventRef { id: Some(id(slug)) })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn command_slice(slug: &str, command: &str, emitted_events: Vec<&str>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id(slug)),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef {
                        id: Some(id(command)),
                    }),
                    ..Default::default()
                }),
                emitted_events: emitted_events
                    .into_iter()
                    .map(|slug| pb::EventEdge {
                        event: Some(pb::EventRef { id: Some(id(slug)) }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn automation_slice(
        slug: &str,
        source_read_models: Vec<&str>,
        processor: &str,
        emitted_command: &str,
    ) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
                id: Some(id(slug)),
                source_read_models: source_read_models
                    .into_iter()
                    .map(|slug| pb::ReadModelEdge {
                        read_model: Some(pb::ReadModelRef { id: Some(id(slug)) }),
                        ..Default::default()
                    })
                    .collect(),
                processor: Some(pb::ProcessorEdge {
                    processor: Some(pb::ProcessorRef {
                        id: Some(id(processor)),
                    }),
                    ..Default::default()
                }),
                emitted_command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef {
                        id: Some(id(emitted_command)),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        }
    }

    fn processor(slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Processor(pb::Processor {
                id: Some(id(slug)),
                ..Default::default()
            })),
        }
    }

    fn model_member(kind: pb::EntityKind, slug: &str) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: kind as i32,
                id: Some(id(slug)),
            }],
            ..Default::default()
        }
    }

    fn count(issues: &[pb::ValidationIssue], code: &str) -> usize {
        issues.iter().filter(|issue| issue.code == code).count()
    }

    #[test]
    fn event_fields_can_come_from_command_lane_state_or_derived_annotation() {
        let all = vec![
            stored(lane("order", vec![field("status", string_kind())])),
            stored(command(
                "place-order",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(event(
                "order-placed",
                "order",
                vec![
                    field("order_id", string_kind()),
                    field("status", string_kind()),
                    derived_field("placed_at", timestamp_kind()),
                ],
            )),
            stored(command_slice(
                "place-order-slice",
                "place-order",
                vec!["order-placed"],
            )),
        ];
        let issues = validate_model(
            &model_member(pb::EntityKind::CommandSlice, "place-order-slice"),
            &all,
        );
        assert_eq!(count(&issues, "EVENT_FIELD_UNSOURCED"), 0, "{issues:?}");
    }

    #[test]
    fn event_field_without_declared_source_errors() {
        let all = vec![
            stored(lane("order", Vec::new())),
            stored(command(
                "place-order",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(event(
                "order-placed",
                "order",
                vec![field("customer_id", string_kind())],
            )),
            stored(command_slice(
                "place-order-slice",
                "place-order",
                vec!["order-placed"],
            )),
        ];
        let issues = validate_model(
            &model_member(pb::EntityKind::CommandSlice, "place-order-slice"),
            &all,
        );
        assert_eq!(count(&issues, "EVENT_FIELD_UNSOURCED"), 1, "{issues:?}");
    }

    // DerivedFieldAnnotation.doc is required (annotation contract + comment
    // claiming "checked elsewhere"). Presence-only matching lets an empty-doc
    // annotation silently suppress EVENT_FIELD_UNSOURCED.
    #[test]
    fn empty_doc_derived_field_annotation_does_not_silence_unsourced() {
        let empty_derived = pb::FieldSpec {
            name: "placed_at".into(),
            r#type: Some(pb::FieldType {
                kind: Some(timestamp_kind()),
            }),
            metadata: vec![prost_types::Any {
                type_url:
                    "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.DerivedFieldAnnotation"
                        .into(),
                value: pb::DerivedFieldAnnotation {
                    doc: String::new(),
                    kind: 0,
                }
                .encode_to_vec(),
            }],
            ..Default::default()
        };
        let all = vec![
            stored(lane("order", Vec::new())),
            stored(command(
                "place-order",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(event(
                "order-placed",
                "order",
                vec![field("order_id", string_kind()), empty_derived],
            )),
            stored(command_slice(
                "place-order-slice",
                "place-order",
                vec!["order-placed"],
            )),
        ];
        let issues = validate_model(
            &model_member(pb::EntityKind::CommandSlice, "place-order-slice"),
            &all,
        );
        assert_eq!(
            count(&issues, "EVENT_FIELD_UNSOURCED"),
            1,
            "empty-doc DerivedFieldAnnotation must not silence EVENT_FIELD_UNSOURCED: {issues:?}"
        );
    }

    #[test]
    fn automation_command_fields_can_come_from_source_read_model_or_derived_annotation() {
        let all = vec![
            stored(command(
                "ship-order",
                "order",
                vec![
                    field("order_id", string_kind()),
                    derived_field("queued_at", timestamp_kind()),
                ],
            )),
            stored(read_model(
                "orders-ready",
                vec![field("order_id", string_kind())],
                Vec::new(),
            )),
            stored(processor("shipper")),
            stored(automation_slice(
                "ship-order-auto",
                vec!["orders-ready"],
                "shipper",
                "ship-order",
            )),
        ];
        let issues = validate_model(
            &model_member(pb::EntityKind::AutomationSlice, "ship-order-auto"),
            &all,
        );
        assert_eq!(
            count(&issues, "AUTOMATION_COMMAND_FIELD_UNSOURCED"),
            0,
            "{issues:?}"
        );
    }

    #[test]
    fn automation_command_field_without_source_read_model_errors() {
        let all = vec![
            stored(command(
                "ship-order",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(read_model(
                "orders-ready",
                vec![field("customer_id", string_kind())],
                Vec::new(),
            )),
            stored(processor("shipper")),
            stored(automation_slice(
                "ship-order-auto",
                vec!["orders-ready"],
                "shipper",
                "ship-order",
            )),
        ];
        let issues = validate_model(
            &model_member(pb::EntityKind::AutomationSlice, "ship-order-auto"),
            &all,
        );
        assert_eq!(
            count(&issues, "AUTOMATION_COMMAND_FIELD_UNSOURCED"),
            1,
            "{issues:?}"
        );
    }

    #[test]
    fn read_model_fields_can_come_from_source_events_or_lane_state() {
        let all = vec![
            stored(lane("order", vec![field("status", string_kind())])),
            stored(event(
                "order-placed",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(read_model(
                "orders",
                vec![
                    field("order_id", string_kind()),
                    field("status", string_kind()),
                ],
                vec!["order-placed"],
            )),
        ];
        let issues = validate_model(&model_member(pb::EntityKind::ReadModel, "orders"), &all);
        assert_eq!(
            count(&issues, "READ_MODEL_FIELD_UNSOURCED"),
            0,
            "{issues:?}"
        );
    }

    #[test]
    fn read_model_field_without_source_event_or_lane_state_errors() {
        let all = vec![
            stored(lane("order", Vec::new())),
            stored(event(
                "order-placed",
                "order",
                vec![field("order_id", string_kind())],
            )),
            stored(read_model(
                "orders",
                vec![field("customer_id", string_kind())],
                vec!["order-placed"],
            )),
        ];
        let issues = validate_model(&model_member(pb::EntityKind::ReadModel, "orders"), &all);
        assert_eq!(
            count(&issues, "READ_MODEL_FIELD_UNSOURCED"),
            1,
            "{issues:?}"
        );
    }
}

#[cfg(test)]
mod personal_data_tests {
    use prost::Message;

    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn string_kind() -> pb::field_type::Kind {
        pb::field_type::Kind::String(pb::field_type::StringType {})
    }

    fn object_kind(fields: Vec<pb::FieldSpec>) -> pb::field_type::Kind {
        pb::field_type::Kind::Object(pb::field_type::ObjectType { fields })
    }

    fn field(name: &str, kind: pb::field_type::Kind) -> pb::FieldSpec {
        pb::FieldSpec {
            name: name.into(),
            r#type: Some(pb::FieldType { kind: Some(kind) }),
            ..Default::default()
        }
    }

    fn pii_annotation(
        doc: &str,
        erasure: pb::personal_data_annotation::Erasure,
        subject_field: &str,
    ) -> prost_types::Any {
        prost_types::Any {
            type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.PersonalDataAnnotation"
                .into(),
            value: pb::PersonalDataAnnotation {
                doc: doc.into(),
                erasure: erasure as i32,
                subject_field: subject_field.into(),
            }
            .encode_to_vec(),
        }
    }

    fn pii_field(
        name: &str,
        kind: pb::field_type::Kind,
        annotation: prost_types::Any,
    ) -> pb::FieldSpec {
        pb::FieldSpec {
            name: name.into(),
            r#type: Some(pb::FieldType { kind: Some(kind) }),
            metadata: vec![annotation],
            ..Default::default()
        }
    }

    fn lane(slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Swimlane(pb::Swimlane {
                id: Some(id(slug)),
                ..Default::default()
            })),
        }
    }

    fn event(slug: &str, lane: &str, fields: Vec<pb::FieldSpec>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(slug)),
                swimlane: Some(pb::SwimlaneRef { id: Some(id(lane)) }),
                schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
                ..Default::default()
            })),
        }
    }

    fn command(slug: &str, lane: &str, fields: Vec<pb::FieldSpec>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Command(pb::Command {
                id: Some(id(slug)),
                swimlane: Some(pb::SwimlaneRef { id: Some(id(lane)) }),
                schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
                ..Default::default()
            })),
        }
    }

    fn read_model(slug: &str, fields: Vec<pb::FieldSpec>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(slug)),
                schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
                ..Default::default()
            })),
        }
    }

    fn model_member(kind: pb::EntityKind, slug: &str) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: kind as i32,
                id: Some(id(slug)),
            }],
            ..Default::default()
        }
    }

    fn count(issues: &[pb::ValidationIssue], code: &str) -> usize {
        issues.iter().filter(|issue| issue.code == code).count()
    }

    use pb::personal_data_annotation::Erasure;

    #[test]
    fn personal_data_field_with_doc_emits_no_issue() {
        let all = vec![
            stored(lane("customer")),
            stored(event(
                "customer-registered",
                "customer",
                vec![pii_field(
                    "email",
                    string_kind(),
                    pii_annotation("customer's email address", Erasure::CryptoShredding, ""),
                )],
            )),
        ];
        let issues = validate_model(
            &model_member(pb::EntityKind::Event, "customer-registered"),
            &all,
        );
        assert_eq!(count(&issues, "PII_DOC_MISSING"), 0, "{issues:?}");
    }

    #[test]
    fn empty_doc_personal_data_annotation_errors() {
        let all = vec![
            stored(lane("customer")),
            stored(event(
                "customer-registered",
                "customer",
                vec![pii_field(
                    "email",
                    string_kind(),
                    pii_annotation("", Erasure::CryptoShredding, ""),
                )],
            )),
        ];
        let issues = validate_model(
            &model_member(pb::EntityKind::Event, "customer-registered"),
            &all,
        );
        assert_eq!(count(&issues, "PII_DOC_MISSING"), 1, "{issues:?}");
    }

    #[test]
    fn malformed_personal_data_annotation_reports_doc_missing() {
        let malformed = pb::FieldSpec {
            name: "email".into(),
            r#type: Some(pb::FieldType {
                kind: Some(string_kind()),
            }),
            metadata: vec![prost_types::Any {
                type_url:
                    "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.PersonalDataAnnotation"
                        .into(),
                value: vec![0xFF, 0xFF, 0xFF],
            }],
            ..Default::default()
        };
        let all = vec![
            stored(lane("customer")),
            stored(event("customer-registered", "customer", vec![malformed])),
        ];
        let issues = validate_model(
            &model_member(pb::EntityKind::Event, "customer-registered"),
            &all,
        );
        assert_eq!(count(&issues, "PII_DOC_MISSING"), 1, "{issues:?}");
    }

    #[test]
    fn event_personal_data_field_with_declared_erasure_emits_no_warning() {
        let all = vec![
            stored(lane("customer")),
            stored(event(
                "customer-registered",
                "customer",
                vec![pii_field(
                    "email",
                    string_kind(),
                    pii_annotation("customer's email address", Erasure::CryptoShredding, ""),
                )],
            )),
        ];
        let issues = validate_model(
            &model_member(pb::EntityKind::Event, "customer-registered"),
            &all,
        );
        assert_eq!(
            count(&issues, "PII_EVENT_ERASURE_UNSPECIFIED"),
            0,
            "{issues:?}"
        );
    }

    #[test]
    fn event_personal_data_field_with_unspecified_erasure_warns() {
        let all = vec![
            stored(lane("customer")),
            stored(event(
                "customer-registered",
                "customer",
                vec![pii_field(
                    "email",
                    string_kind(),
                    pii_annotation("customer's email address", Erasure::Unspecified, ""),
                )],
            )),
        ];
        let issues = validate_model(
            &model_member(pb::EntityKind::Event, "customer-registered"),
            &all,
        );
        assert_eq!(
            count(&issues, "PII_EVENT_ERASURE_UNSPECIFIED"),
            1,
            "{issues:?}"
        );
    }

    #[test]
    fn projection_delete_on_read_model_field_emits_no_issue() {
        let all = vec![stored(read_model(
            "customer-profile",
            vec![pii_field(
                "email",
                string_kind(),
                pii_annotation("customer's email address", Erasure::ProjectionDelete, ""),
            )],
        ))];
        let issues = validate_model(
            &model_member(pb::EntityKind::ReadModel, "customer-profile"),
            &all,
        );
        assert_eq!(
            count(&issues, "PII_ERASURE_NOT_APPLICABLE"),
            0,
            "{issues:?}"
        );
    }

    #[test]
    fn projection_delete_on_event_field_errors() {
        let all = vec![
            stored(lane("customer")),
            stored(event(
                "customer-registered",
                "customer",
                vec![pii_field(
                    "email",
                    string_kind(),
                    pii_annotation("customer's email address", Erasure::ProjectionDelete, ""),
                )],
            )),
        ];
        let issues = validate_model(
            &model_member(pb::EntityKind::Event, "customer-registered"),
            &all,
        );
        assert_eq!(
            count(&issues, "PII_ERASURE_NOT_APPLICABLE"),
            1,
            "{issues:?}"
        );
    }

    #[test]
    fn projection_delete_on_command_field_errors() {
        let all = vec![
            stored(lane("customer")),
            stored(command(
                "update-email",
                "customer",
                vec![pii_field(
                    "email",
                    string_kind(),
                    pii_annotation("customer's email address", Erasure::ProjectionDelete, ""),
                )],
            )),
        ];
        let issues = validate_model(&model_member(pb::EntityKind::Command, "update-email"), &all);
        assert_eq!(
            count(&issues, "PII_ERASURE_NOT_APPLICABLE"),
            1,
            "{issues:?}"
        );
    }

    #[test]
    fn subject_field_naming_a_sibling_emits_no_issue() {
        let all = vec![stored(read_model(
            "customer-profile",
            vec![
                field("customer_id", string_kind()),
                pii_field(
                    "email",
                    string_kind(),
                    pii_annotation(
                        "customer's email address",
                        Erasure::ProjectionDelete,
                        "customer_id",
                    ),
                ),
            ],
        ))];
        let issues = validate_model(
            &model_member(pb::EntityKind::ReadModel, "customer-profile"),
            &all,
        );
        assert_eq!(count(&issues, "PII_SUBJECT_FIELD_UNKNOWN"), 0, "{issues:?}");
    }

    #[test]
    fn subject_field_naming_no_sibling_errors() {
        let all = vec![stored(read_model(
            "customer-profile",
            vec![pii_field(
                "email",
                string_kind(),
                pii_annotation(
                    "customer's email address",
                    Erasure::ProjectionDelete,
                    "does_not_exist",
                ),
            )],
        ))];
        let issues = validate_model(
            &model_member(pb::EntityKind::ReadModel, "customer-profile"),
            &all,
        );
        assert_eq!(count(&issues, "PII_SUBJECT_FIELD_UNKNOWN"), 1, "{issues:?}");
    }

    #[test]
    fn nested_object_field_personal_data_annotation_is_walked() {
        let all = vec![stored(read_model(
            "customer-profile",
            vec![field(
                "address",
                object_kind(vec![pii_field(
                    "street",
                    string_kind(),
                    pii_annotation("", Erasure::ProjectionDelete, ""),
                )]),
            )],
        ))];
        let issues = validate_model(
            &model_member(pb::EntityKind::ReadModel, "customer-profile"),
            &all,
        );
        assert_eq!(count(&issues, "PII_DOC_MISSING"), 1, "{issues:?}");
        assert!(
            issues
                .iter()
                .any(|i| i.code == "PII_DOC_MISSING"
                    && i.subject_field.contains("type.object.fields")),
            "{issues:?}"
        );
    }

    #[test]
    fn nested_object_field_subject_field_checks_its_own_sibling_list() {
        // `city` lives in the SAME ObjectType field list as `zip`, so the
        // sibling check must look at that nested list, not the
        // top-level ReadModel fields.
        let all = vec![stored(read_model(
            "customer-profile",
            vec![field(
                "address",
                object_kind(vec![
                    field("zip", string_kind()),
                    pii_field(
                        "city",
                        string_kind(),
                        pii_annotation("customer's city", Erasure::ProjectionDelete, "zip"),
                    ),
                ]),
            )],
        ))];
        let issues = validate_model(
            &model_member(pb::EntityKind::ReadModel, "customer-profile"),
            &all,
        );
        assert_eq!(count(&issues, "PII_SUBJECT_FIELD_UNKNOWN"), 0, "{issues:?}");
    }
}

#[cfg(test)]
mod pii_flow_unmarked_tests {
    use prost::Message;

    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn string_kind() -> pb::field_type::Kind {
        pb::field_type::Kind::String(pb::field_type::StringType {})
    }

    fn object_kind(fields: Vec<pb::FieldSpec>) -> pb::field_type::Kind {
        pb::field_type::Kind::Object(pb::field_type::ObjectType { fields })
    }

    fn field(name: &str, kind: pb::field_type::Kind) -> pb::FieldSpec {
        pb::FieldSpec {
            name: name.into(),
            r#type: Some(pb::FieldType { kind: Some(kind) }),
            ..Default::default()
        }
    }

    fn pii_annotation(
        doc: &str,
        erasure: pb::personal_data_annotation::Erasure,
    ) -> prost_types::Any {
        prost_types::Any {
            type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.PersonalDataAnnotation"
                .into(),
            value: pb::PersonalDataAnnotation {
                doc: doc.into(),
                erasure: erasure as i32,
                subject_field: String::new(),
            }
            .encode_to_vec(),
        }
    }

    fn pii_field(
        name: &str,
        kind: pb::field_type::Kind,
        annotation: prost_types::Any,
    ) -> pb::FieldSpec {
        pb::FieldSpec {
            name: name.into(),
            r#type: Some(pb::FieldType { kind: Some(kind) }),
            metadata: vec![annotation],
            ..Default::default()
        }
    }

    fn lane(slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Swimlane(pb::Swimlane {
                id: Some(id(slug)),
                ..Default::default()
            })),
        }
    }

    fn event(slug: &str, lane: &str, fields: Vec<pb::FieldSpec>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(slug)),
                swimlane: Some(pb::SwimlaneRef { id: Some(id(lane)) }),
                schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
                ..Default::default()
            })),
        }
    }

    fn read_model(slug: &str, source_event: &str, fields: Vec<pb::FieldSpec>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(slug)),
                schema: Some(trogon_atlas_core::schema::pack_fields(fields)),
                source_events: vec![pb::EventRef {
                    id: Some(id(source_event)),
                }],
                ..Default::default()
            })),
        }
    }

    fn model_member(kind: pb::EntityKind, slug: &str) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: kind as i32,
                id: Some(id(slug)),
            }],
            ..Default::default()
        }
    }

    fn count(issues: &[pb::ValidationIssue], code: &str) -> usize {
        issues.iter().filter(|issue| issue.code == code).count()
    }

    use pb::personal_data_annotation::Erasure;

    #[test]
    fn downstream_field_sourced_from_pii_field_without_own_annotation_warns() {
        let all = vec![
            stored(lane("customer")),
            stored(event(
                "customer-registered",
                "customer",
                vec![pii_field(
                    "email",
                    string_kind(),
                    pii_annotation("customer's email address", Erasure::CryptoShredding),
                )],
            )),
            stored(read_model(
                "customer-profile",
                "customer-registered",
                vec![field("email", string_kind())],
            )),
        ];
        let issues = validate_model(
            &model_member(pb::EntityKind::ReadModel, "customer-profile"),
            &all,
        );
        let matching: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "PII_FLOW_UNMARKED")
            .collect();
        assert_eq!(matching.len(), 1, "{issues:?}");
        assert!(matching[0].message.contains("email"), "{matching:?}");
        assert_eq!(matching[0].subject_field, "schema.fields[0].metadata");
        assert_eq!(
            matching[0].subject,
            Some(pb::EntityRef {
                kind: pb::EntityKind::ReadModel as i32,
                id: Some(id("customer-profile")),
            })
        );
    }

    #[test]
    fn downstream_field_already_annotated_emits_no_warning() {
        let all = vec![
            stored(lane("customer")),
            stored(event(
                "customer-registered",
                "customer",
                vec![pii_field(
                    "email",
                    string_kind(),
                    pii_annotation("customer's email address", Erasure::CryptoShredding),
                )],
            )),
            stored(read_model(
                "customer-profile",
                "customer-registered",
                vec![pii_field(
                    "email",
                    string_kind(),
                    pii_annotation("customer's email address", Erasure::ProjectionDelete),
                )],
            )),
        ];
        let issues = validate_model(
            &model_member(pb::EntityKind::ReadModel, "customer-profile"),
            &all,
        );
        assert_eq!(count(&issues, "PII_FLOW_UNMARKED"), 0, "{issues:?}");
    }

    #[test]
    fn downstream_field_sourced_from_non_pii_field_emits_no_warning() {
        let all = vec![
            stored(lane("customer")),
            stored(event(
                "customer-registered",
                "customer",
                vec![field("email", string_kind())],
            )),
            stored(read_model(
                "customer-profile",
                "customer-registered",
                vec![field("email", string_kind())],
            )),
        ];
        let issues = validate_model(
            &model_member(pb::EntityKind::ReadModel, "customer-profile"),
            &all,
        );
        assert_eq!(count(&issues, "PII_FLOW_UNMARKED"), 0, "{issues:?}");
    }

    // Known gap: the data-flow inference layer only matches top-level field
    // names (FlowEndpoint.fields comes from schema_fields, which does not
    // flatten nested ObjectType fields). A PersonalDataAnnotation on a
    // nested field is invisible to this rule even when the enclosing
    // top-level object field is copied downstream unchanged; it can only
    // see PersonalDataAnnotation declared on the matched top-level field
    // itself.
    #[test]
    fn nested_personal_data_field_is_invisible_to_flow_inference() {
        let all = vec![
            stored(lane("customer")),
            stored(event(
                "customer-registered",
                "customer",
                vec![field(
                    "address",
                    object_kind(vec![pii_field(
                        "street",
                        string_kind(),
                        pii_annotation("customer's street", Erasure::CryptoShredding),
                    )]),
                )],
            )),
            stored(read_model(
                "customer-profile",
                "customer-registered",
                vec![field(
                    "address",
                    object_kind(vec![field("street", string_kind())]),
                )],
            )),
        ];
        let issues = validate_model(
            &model_member(pb::EntityKind::ReadModel, "customer-profile"),
            &all,
        );
        assert_eq!(count(&issues, "PII_FLOW_UNMARKED"), 0, "{issues:?}");
    }
}

#[cfg(test)]
mod slice_stale_ref_tests {
    use super::*;

    fn id_v(slug: &str, version: u64) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn command_v(slug: &str, version: u64) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Command(pb::Command {
                id: Some(id_v(slug, version)),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn ui_v(slug: &str, version: u64) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Ui(pb::Ui {
                id: Some(id_v(slug, version)),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn event_v(slug: &str, version: u64) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id_v(slug, version)),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn command_slice_pinning(
        slug: &str,
        ui_ref: pb::Id,
        cmd_ref: pb::Id,
        evt_refs: Vec<pb::Id>,
    ) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id_v(slug, 1)),
                title: slug.into(),
                ui: Some(pb::UiEdge {
                    ui: Some(pb::UiRef { id: Some(ui_ref) }),
                    ..Default::default()
                }),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef { id: Some(cmd_ref) }),
                    ..Default::default()
                }),
                emitted_events: evt_refs
                    .into_iter()
                    .map(|id| pb::EventEdge {
                        event: Some(pb::EventRef { id: Some(id) }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn model_with_slice(slug: &str) -> pb::EventModel {
        pb::EventModel {
            id: Some(id_v("m", 1)),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::CommandSlice as i32,
                id: Some(id_v(slug, 1)),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn ref_pinning_latest_is_clean() {
        let all = vec![
            stored(ui_v("composer", 2)),
            stored(command_v("post", 1)),
            stored(event_v("posted", 1)),
            stored(command_slice_pinning(
                "post-slice",
                id_v("composer", 2),
                id_v("post", 1),
                vec![id_v("posted", 1)],
            )),
        ];
        let issues = validate_model(&model_with_slice("post-slice"), &all);
        assert!(
            !issues.iter().any(|i| i.code == "SLICE_STALE_REF"),
            "{issues:?}",
        );
    }

    #[test]
    fn ui_pinned_to_older_version_flags() {
        let all = vec![
            stored(ui_v("composer", 1)),
            stored(ui_v("composer", 2)),
            stored(command_v("post", 1)),
            stored(command_slice_pinning(
                "post-slice",
                id_v("composer", 1),
                id_v("post", 1),
                vec![],
            )),
        ];
        let issues = validate_model(&model_with_slice("post-slice"), &all);
        let hits: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "SLICE_STALE_REF")
            .collect();
        assert_eq!(hits.len(), 1, "{issues:?}");
        assert_eq!(hits[0].subject_field, "ui");
        assert_eq!(
            hits[0].severity,
            pb::validation_issue::Severity::Warning as i32
        );
    }

    #[test]
    fn multiple_stale_refs_emit_one_issue_each() {
        let all = vec![
            stored(ui_v("composer", 1)),
            stored(ui_v("composer", 3)),
            stored(command_v("post", 1)),
            stored(command_v("post", 4)),
            stored(event_v("posted", 1)),
            stored(event_v("posted", 2)),
            stored(command_slice_pinning(
                "post-slice",
                id_v("composer", 1),
                id_v("post", 1),
                vec![id_v("posted", 1)],
            )),
        ];
        let issues = validate_model(&model_with_slice("post-slice"), &all);
        let hits: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "SLICE_STALE_REF")
            .collect();
        assert_eq!(hits.len(), 3, "{issues:?}");
        let fields: HashSet<&str> = hits.iter().map(|i| i.subject_field.as_str()).collect();
        assert!(fields.contains("ui"), "{fields:?}");
        assert!(fields.contains("command"), "{fields:?}");
        assert!(fields.contains("emitted_events[0]"), "{fields:?}");
    }

    // Build a PinnedRefAnnotation Any payload for use in slice metadata.
    fn pinned_ann_any(doc: &str, pinned: Vec<&str>) -> prost_types::Any {
        use prost::Message;
        let ann = pb::PinnedRefAnnotation {
            doc: doc.into(),
            pinned: pinned.into_iter().map(String::from).collect(),
        };
        prost_types::Any {
            type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.PinnedRefAnnotation"
                .into(),
            value: ann.encode_to_vec(),
        }
    }

    // Build a CommandSlice with a metadata payload attached.
    fn command_slice_with_metadata(
        slug: &str,
        ui_ref: pb::Id,
        cmd_ref: pb::Id,
        evt_refs: Vec<pb::Id>,
        metadata: Vec<prost_types::Any>,
    ) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id_v(slug, 1)),
                title: slug.into(),
                ui: Some(pb::UiEdge {
                    ui: Some(pb::UiRef { id: Some(ui_ref) }),
                    ..Default::default()
                }),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef { id: Some(cmd_ref) }),
                    ..Default::default()
                }),
                emitted_events: evt_refs
                    .into_iter()
                    .map(|id| pb::EventEdge {
                        event: Some(pb::EventRef { id: Some(id) }),
                        ..Default::default()
                    })
                    .collect(),
                metadata,
                ..Default::default()
            })),
        }
    }

    // A blanket PinnedRefAnnotation (empty pinned list, non-empty doc) silences
    // all stale refs on the slice.
    #[test]
    fn blanket_pin_silences_all_stale_refs() {
        let all = vec![
            stored(ui_v("composer", 1)),
            stored(ui_v("composer", 2)),
            stored(command_v("post", 1)),
            stored(command_v("post", 3)),
            stored(event_v("posted", 1)),
            stored(event_v("posted", 2)),
            stored(command_slice_with_metadata(
                "post-slice",
                id_v("composer", 1),
                id_v("post", 1),
                vec![id_v("posted", 1)],
                vec![pinned_ann_any(
                    "v3 command stabilization window not yet closed",
                    vec![],
                )],
            )),
        ];
        let issues = validate_model(&model_with_slice("post-slice"), &all);
        assert!(
            !issues.iter().any(|i| i.code == "SLICE_STALE_REF"),
            "blanket pin should silence all stale refs: {issues:?}",
        );
    }

    // A targeted PinnedRefAnnotation listing one canonical id silences only
    // that ref; a second stale ref on the same slice still fires.
    #[test]
    fn targeted_pin_silences_only_listed_ref() {
        let all = vec![
            stored(ui_v("composer", 1)),
            stored(ui_v("composer", 2)),
            stored(command_v("post", 1)),
            stored(command_v("post", 3)),
            stored(event_v("posted", 1)),
            stored(command_slice_with_metadata(
                "post-slice",
                id_v("composer", 1),
                id_v("post", 1),
                vec![id_v("posted", 1)],
                // Pin only the ui ref (composer@1); the command ref (post@1)
                // still fires because it is not listed.
                vec![pinned_ann_any(
                    "composer v2 API is unstable; holding on v1 deliberately",
                    vec!["t/composer@1"],
                )],
            )),
        ];
        let issues = validate_model(&model_with_slice("post-slice"), &all);
        let hits: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "SLICE_STALE_REF")
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "only the unlisted stale ref should fire: {issues:?}"
        );
        assert_eq!(hits[0].subject_field, "command", "{issues:?}");
    }

    // An empty doc does NOT silence anything even when pinned entries are listed.
    #[test]
    fn empty_doc_does_not_silence() {
        let all = vec![
            stored(ui_v("composer", 1)),
            stored(ui_v("composer", 2)),
            stored(command_v("post", 1)),
            stored(command_slice_with_metadata(
                "post-slice",
                id_v("composer", 1),
                id_v("post", 1),
                vec![],
                vec![pinned_ann_any("", vec!["t/composer@1"])],
            )),
        ];
        let issues = validate_model(&model_with_slice("post-slice"), &all);
        assert!(
            issues.iter().any(|i| i.code == "SLICE_STALE_REF"),
            "empty doc must not silence: {issues:?}",
        );
    }

    // An annotation listing a different canonical id (wrong version or wrong slug)
    // does NOT silence the actual stale ref.
    #[test]
    fn annotation_naming_different_id_does_not_silence() {
        let all = vec![
            stored(ui_v("composer", 1)),
            stored(ui_v("composer", 2)),
            stored(command_v("post", 1)),
            stored(command_slice_with_metadata(
                "post-slice",
                id_v("composer", 1),
                id_v("post", 1),
                vec![],
                // Lists v2 (the latest), not v1 (the pinned version), so
                // the actual stale ref at t/composer@1 is not covered.
                vec![pinned_ann_any(
                    "intentional hold for another reason",
                    vec!["t/composer@2"],
                )],
            )),
        ];
        let issues = validate_model(&model_with_slice("post-slice"), &all);
        assert!(
            issues.iter().any(|i| i.code == "SLICE_STALE_REF"),
            "annotation naming a different id must not silence: {issues:?}",
        );
    }

    // Build a LifecycleAnnotation Any payload for use in entity metadata.
    fn lifecycle_ann_any(status: &str) -> prost_types::Any {
        use prost::Message;
        let ann = pb::LifecycleAnnotation {
            status: status.into(),
            doc: String::new(),
        };
        prost_types::Any {
            type_url: "type.googleapis.com/trogonatlas.annotation.v1alpha1.LifecycleAnnotation"
                .into(),
            value: ann.encode_to_vec(),
        }
    }

    // Build an Event entity carrying a LifecycleAnnotation in its metadata.
    fn event_v_with_lifecycle(slug: &str, version: u64, status: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: "t".into(),
                    slug: slug.into(),
                    version,
                }),
                title: slug.into(),
                metadata: vec![lifecycle_ann_any(status)],
                ..Default::default()
            })),
        }
    }

    // A draft-annotated v2 present in the loaded set does NOT cause a
    // SLICE_STALE_REF for a slice that references v1. Draft versions must not
    // nag the whole model into migrating before they land.
    #[test]
    fn draft_annotated_v2_does_not_trigger_stale_ref() {
        let all = vec![
            stored(ui_v("composer", 1)),
            stored(command_v("post", 1)),
            stored(event_v("posted", 1)),
            // A draft v2 of the event is present but must be excluded from the
            // latest_version map, so the v1 ref in the slice is not stale.
            stored(event_v_with_lifecycle("posted", 2, "draft")),
            stored(command_slice_pinning(
                "post-slice",
                id_v("composer", 1),
                id_v("post", 1),
                vec![id_v("posted", 1)],
            )),
        ];
        let issues = validate_model(&model_with_slice("post-slice"), &all);
        assert!(
            !issues.iter().any(|i| i.code == "SLICE_STALE_REF"),
            "draft v2 must not cause SLICE_STALE_REF: {issues:?}",
        );
    }

    // Same exemption applies for "proposed" status.
    #[test]
    fn proposed_annotated_v2_does_not_trigger_stale_ref() {
        let all = vec![
            stored(ui_v("composer", 1)),
            stored(command_v("post", 1)),
            stored(event_v("posted", 1)),
            stored(event_v_with_lifecycle("posted", 2, "proposed")),
            stored(command_slice_pinning(
                "post-slice",
                id_v("composer", 1),
                id_v("post", 1),
                vec![id_v("posted", 1)],
            )),
        ];
        let issues = validate_model(&model_with_slice("post-slice"), &all);
        assert!(
            !issues.iter().any(|i| i.code == "SLICE_STALE_REF"),
            "proposed v2 must not cause SLICE_STALE_REF: {issues:?}",
        );
    }

    // Flipping the annotation to "accepted" (not in the draft set) makes the
    // SLICE_STALE_REF finding surface, because v2 is now a landed version.
    #[test]
    fn accepted_v2_triggers_stale_ref() {
        let all = vec![
            stored(ui_v("composer", 1)),
            stored(command_v("post", 1)),
            stored(event_v("posted", 1)),
            stored(event_v_with_lifecycle("posted", 2, "accepted")),
            stored(command_slice_pinning(
                "post-slice",
                id_v("composer", 1),
                id_v("post", 1),
                vec![id_v("posted", 1)],
            )),
        ];
        let issues = validate_model(&model_with_slice("post-slice"), &all);
        let hits: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "SLICE_STALE_REF")
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "accepted v2 must surface SLICE_STALE_REF: {issues:?}",
        );
        assert_eq!(hits[0].subject_field, "emitted_events[0]", "{issues:?}");
    }

    // Removing the LifecycleAnnotation (no annotation at all) also causes the
    // finding to surface: the version counts as latest.
    #[test]
    fn no_annotation_v2_triggers_stale_ref() {
        let all = vec![
            stored(ui_v("composer", 1)),
            stored(command_v("post", 1)),
            stored(event_v("posted", 1)),
            stored(event_v("posted", 2)),
            stored(command_slice_pinning(
                "post-slice",
                id_v("composer", 1),
                id_v("post", 1),
                vec![id_v("posted", 1)],
            )),
        ];
        let issues = validate_model(&model_with_slice("post-slice"), &all);
        assert!(
            issues.iter().any(|i| i.code == "SLICE_STALE_REF"),
            "unannotated v2 must cause SLICE_STALE_REF: {issues:?}",
        );
    }

    // Decision #24 / validate_model scope: SLICE_STALE_REF must only fire for
    // slices that are members of the model under validation. Walking every
    // slice in `all` pollutes unrelated models and duplicates findings under
    // validate_project (once per EventModel).
    #[test]
    fn stale_ref_on_non_member_slice_is_not_reported() {
        let all = vec![
            stored(ui_v("composer", 1)),
            stored(ui_v("composer", 2)),
            stored(command_v("post", 1)),
            stored(command_slice_pinning(
                "foreign-slice",
                id_v("composer", 1),
                id_v("post", 1),
                vec![],
            )),
            stored(event_v("unrelated", 1)),
        ];
        // Model claims only an unrelated event: foreign-slice is in the store
        // but not a member.
        let model = pb::EventModel {
            id: Some(id_v("m", 1)),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id_v("unrelated", 1)),
            }],
            ..Default::default()
        };
        let issues = validate_model(&model, &all);
        assert!(
            !issues.iter().any(|i| i.code == "SLICE_STALE_REF"),
            "non-member slice must not emit SLICE_STALE_REF for this model: {issues:?}",
        );
    }
}

#[cfg(test)]
mod ui_no_persona_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn ui(slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Ui(pb::Ui {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn persona(slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Persona(pb::Persona {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn ui_edge(slug: &str) -> pb::UiEdge {
        pb::UiEdge {
            ui: Some(pb::UiRef { id: Some(id(slug)) }),
            ..Default::default()
        }
    }

    fn persona_edge(slug: &str) -> pb::PersonaEdge {
        pb::PersonaEdge {
            persona: Some(pb::PersonaRef { id: Some(id(slug)) }),
            ..Default::default()
        }
    }

    fn command_slice(slug: &str, p: Option<&str>, u: Option<&str>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id(slug)),
                title: slug.into(),
                persona: p.map(persona_edge),
                ui: u.map(ui_edge),
                ..Default::default()
            })),
        }
    }

    // A display-only screen. With a persona the slice attributes the screen on
    // its own; without one it is the storyboard entry's job.
    fn ui_slice(slug: &str, p: Option<&str>, u: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::UiSlice(pb::UiSlice {
                id: Some(id(slug)),
                title: slug.into(),
                persona: p.map(persona_edge),
                ui: Some(ui_edge(u)),
                ..Default::default()
            })),
        }
    }

    fn storyboard_watching(slug: &str, p: &str, u: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Storyboard(pb::Storyboard {
                id: Some(id(slug)),
                title: slug.into(),
                entry: Some(pb::StoryboardEntry {
                    read_model: None,
                    observer: Some(pb::storyboard_entry::Observer::Human(
                        pb::storyboard_entry::HumanObserver {
                            persona: Some(persona_edge(p)),
                            ui: Some(ui_edge(u)),
                        },
                    )),
                }),
                ..Default::default()
            })),
        }
    }

    fn model_with_ui(slug: &str) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::Ui as i32,
                id: Some(id(slug)),
            }],
            ..Default::default()
        }
    }

    fn hits(issues: &[pb::ValidationIssue]) -> Vec<&pb::ValidationIssue> {
        issues
            .iter()
            .filter(|i| i.code == "UI_NO_PERSONA")
            .collect()
    }

    #[test]
    fn ui_attributed_by_command_slice_is_clean() {
        let all = vec![
            stored(persona("buyer")),
            stored(ui("cart")),
            stored(command_slice(
                "place-order-slice",
                Some("buyer"),
                Some("cart"),
            )),
        ];
        let issues = validate_model(&model_with_ui("cart"), &all);
        assert!(hits(&issues).is_empty(), "{issues:?}");
    }

    // The load-bearing case: a display-only screen names its own persona on
    // the UiSlice that renders it, with no storyboard needed to vouch for it.
    #[test]
    fn display_only_ui_attributed_by_ui_slice_is_clean() {
        let all = vec![
            stored(persona("manager")),
            stored(ui("ops-console")),
            stored(ui_slice("orders-list-ui", Some("manager"), "ops-console")),
        ];
        let issues = validate_model(&model_with_ui("ops-console"), &all);
        assert!(hits(&issues).is_empty(), "{issues:?}");
    }

    // The storyboard entry remains a fallback for a UiSlice that names no
    // persona of its own.
    #[test]
    fn display_only_ui_attributed_by_storyboard_entry_is_clean() {
        let all = vec![
            stored(persona("manager")),
            stored(ui("ops-console")),
            stored(ui_slice("orders-list-ui", None, "ops-console")),
            stored(storyboard_watching("sb-ops", "manager", "ops-console")),
        ];
        let issues = validate_model(&model_with_ui("ops-console"), &all);
        assert!(hits(&issues).is_empty(), "{issues:?}");
    }

    #[test]
    fn display_only_ui_with_no_observer_errors() {
        let all = vec![
            stored(ui("ops-console")),
            stored(ui_slice("orders-list-ui", None, "ops-console")),
        ];
        let issues = validate_model(&model_with_ui("ops-console"), &all);
        let h = hits(&issues);
        assert_eq!(h.len(), 1, "{issues:?}");
        assert_eq!(h[0].subject_field, "persona");
        assert_eq!(h[0].severity, pb::validation_issue::Severity::Error as i32);
    }

    #[test]
    fn ui_referenced_by_nothing_errors() {
        let all = vec![stored(ui("orphan-screen"))];
        let issues = validate_model(&model_with_ui("orphan-screen"), &all);
        assert_eq!(hits(&issues).len(), 1, "{issues:?}");
    }

    // A CommandSlice with a ui but no persona attributes nothing: the
    // pairing is what carries the ownership.
    #[test]
    fn command_slice_without_persona_does_not_attribute() {
        let all = vec![
            stored(ui("cart")),
            stored(command_slice("handler-slice", None, Some("cart"))),
        ];
        let issues = validate_model(&model_with_ui("cart"), &all);
        assert_eq!(hits(&issues).len(), 1, "{issues:?}");
    }

    #[test]
    fn ui_attributed_to_missing_persona_errors() {
        let all = vec![
            stored(ui("cart")),
            stored(command_slice(
                "place-order-slice",
                Some("ghost"),
                Some("cart"),
            )),
        ];
        let issues = validate_model(&model_with_ui("cart"), &all);
        let h = hits(&issues);
        assert_eq!(h.len(), 1, "{issues:?}");
        assert!(h[0].message.contains("ghost"), "{}", h[0].message);
    }

    // Scoped to members, exactly like EVENT_NO_SWIMLANE: an unowned screen
    // outside the model under validation is not this model's problem.
    #[test]
    fn non_member_ui_is_ignored() {
        let all = vec![stored(ui("elsewhere"))];
        let model = pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: Vec::new(),
            ..Default::default()
        };
        let issues = validate_model(&model, &all);
        assert!(hits(&issues).is_empty(), "{issues:?}");
    }

    // A member that does not resolve in the store is MEMBER_NOT_FOUND's
    // problem; UI_NO_PERSONA must not pile a second error onto the same
    // broken reference (EVENT_NO_SWIMLANE behaves the same way).
    #[test]
    fn member_ui_missing_from_store_reports_only_member_not_found() {
        let issues = validate_model(&model_with_ui("cart"), &[]);
        assert!(hits(&issues).is_empty(), "{issues:?}");
        assert!(
            issues.iter().any(|i| i.code == "MEMBER_NOT_FOUND"),
            "{issues:?}",
        );
    }
}

#[cfg(test)]
mod event_no_swimlane_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn swimlane(slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Swimlane(pb::Swimlane {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn event(slug: &str, lane: Option<&str>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(slug)),
                title: slug.into(),
                swimlane: lane.map(|l| pb::SwimlaneRef { id: Some(id(l)) }),
                ..Default::default()
            })),
        }
    }

    fn model_with_event(slug: &str) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id(slug)),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn event_with_resolved_swimlane_is_clean() {
        let all = vec![
            stored(swimlane("order")),
            stored(event("order-placed", Some("order"))),
        ];
        let issues = validate_model(&model_with_event("order-placed"), &all);
        assert!(
            !issues.iter().any(|i| i.code == "EVENT_NO_SWIMLANE"),
            "{issues:?}",
        );
    }

    #[test]
    fn event_without_swimlane_errors() {
        let all = vec![stored(event("ghost", None))];
        let issues = validate_model(&model_with_event("ghost"), &all);
        let hits: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "EVENT_NO_SWIMLANE")
            .collect();
        assert_eq!(hits.len(), 1, "{issues:?}");
        assert_eq!(hits[0].subject_field, "swimlane");
        assert_eq!(
            hits[0].severity,
            pb::validation_issue::Severity::Error as i32
        );
    }

    #[test]
    fn event_with_dangling_swimlane_errors() {
        let all = vec![stored(event("lost", Some("missing-lane")))];
        let issues = validate_model(&model_with_event("lost"), &all);
        let hits: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "EVENT_NO_SWIMLANE")
            .collect();
        assert_eq!(hits.len(), 1, "{issues:?}");
        assert!(
            hits[0].message.contains("missing-lane"),
            "{}",
            hits[0].message,
        );
    }
}

#[cfg(test)]
mod bc_realizes_nothing_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: slug.into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn subdomain(ns: &str, slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Subdomain(pb::Subdomain {
                id: Some(pb::Id {
                    namespace: ns.into(),
                    slug: slug.into(),
                    version: 1,
                }),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn bc(ns: &str, realizes: &[(&str, &str)]) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::BoundedContext(pb::BoundedContext {
                id: Some(id(ns)),
                title: ns.into(),
                realizes: realizes
                    .iter()
                    .map(|(n, s)| pb::SubdomainRef {
                        id: Some(pb::Id {
                            namespace: (*n).into(),
                            slug: (*s).into(),
                            version: 1,
                        }),
                    })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    #[test]
    fn bc_with_realizes_is_clean() {
        let all = vec![
            stored(subdomain("market", "identity")),
            stored(bc("checkout", &[("market", "identity")])),
        ];
        let issues = validate_store(&all);
        assert!(
            !issues.iter().any(|i| i.code == "BC_REALIZES_NOTHING"),
            "{issues:?}",
        );
    }

    #[test]
    fn bc_realizing_nothing_emits_info() {
        let all = vec![stored(bc("checkout", &[]))];
        let issues = validate_store(&all);
        let hits: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "BC_REALIZES_NOTHING")
            .collect();
        assert_eq!(hits.len(), 1, "{issues:?}");
        assert_eq!(hits[0].subject_field, "realizes");
        assert_eq!(
            hits[0].severity,
            pb::validation_issue::Severity::Info as i32
        );
        assert_eq!(
            hits[0]
                .subject
                .as_ref()
                .and_then(|s| s.id.as_ref())
                .map(|i| i.slug.as_str()),
            Some("checkout"),
        );
    }
}

#[cfg(test)]
mod ui_transition_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn ui(slug: &str, transitions: &[&str]) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Ui(pb::Ui {
                id: Some(id(slug)),
                title: slug.into(),
                transitions: transitions
                    .iter()
                    .map(|to| pb::ui::Transition {
                        to: Some(pb::UiRef { id: Some(id(to)) }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn command_slice(slug: &str, ui_slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id(slug)),
                title: slug.into(),
                ui: Some(pb::UiEdge {
                    ui: Some(pb::UiRef {
                        id: Some(id(ui_slug)),
                    }),
                    ..Default::default()
                }),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef {
                        id: Some(id("cmd")),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        }
    }

    fn ui_slice(slug: &str, ui_slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::UiSlice(pb::UiSlice {
                id: Some(id(slug)),
                title: slug.into(),
                ui: Some(pb::UiEdge {
                    ui: Some(pb::UiRef {
                        id: Some(id(ui_slug)),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        }
    }

    fn storyboard(slug: &str, slices: &[&str]) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Storyboard(pb::Storyboard {
                id: Some(id(slug)),
                title: slug.into(),
                slices: slices
                    .iter()
                    .map(|s| pb::SliceRef { id: Some(id(s)) })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn model(members: &[(pb::EntityKind, &str)]) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: members
                .iter()
                .map(|(k, s)| pb::EntityRef {
                    kind: *k as i32,
                    id: Some(id(s)),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn derivable_transition_warns() {
        let all = vec![
            stored(ui("a", &["b"])),
            stored(ui("b", &[])),
            stored(command_slice("s1", "a")),
            stored(ui_slice("s2", "b")),
            stored(storyboard("sb", &["s1", "s2"])),
        ];
        let m = model(&[
            (pb::EntityKind::Ui, "a"),
            (pb::EntityKind::Ui, "b"),
            (pb::EntityKind::Storyboard, "sb"),
        ]);
        let issues = validate_model(&m, &all);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "UI_TRANSITION_DERIVABLE")
                .count(),
            1,
            "{issues:?}"
        );
    }

    #[test]
    fn pure_navigation_is_clean() {
        let all = vec![
            stored(ui("a", &["c"])),
            stored(ui("b", &[])),
            stored(ui("c", &[])),
            stored(command_slice("s1", "a")),
            stored(ui_slice("s2", "b")),
            stored(storyboard("sb", &["s1", "s2"])),
        ];
        let m = model(&[
            (pb::EntityKind::Ui, "a"),
            (pb::EntityKind::Ui, "c"),
            (pb::EntityKind::Storyboard, "sb"),
        ]);
        let issues = validate_model(&m, &all);
        assert!(
            !issues.iter().any(|i| i.code.starts_with("UI_TRANSITION")),
            "{issues:?}"
        );
    }

    #[test]
    fn dangling_transition_errors() {
        let all = vec![stored(ui("a", &["ghost"]))];
        let m = model(&[(pb::EntityKind::Ui, "a")]);
        let issues = validate_model(&m, &all);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "UI_TRANSITION_DANGLING")
                .count(),
            1,
            "{issues:?}"
        );
    }
}

#[cfg(test)]
mod knowledge_graph_tests {
    use super::*;

    fn id(ns: &str, slug: &str) -> pb::Id {
        pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn domain(ns: &str, slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Domain(pb::Domain {
                id: Some(id(ns, slug)),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn subdomain(ns: &str, slug: &str, domain_ns: Option<&str>, classified: bool) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Subdomain(pb::Subdomain {
                id: Some(id(ns, slug)),
                title: slug.into(),
                domain: domain_ns.map(|d| pb::DomainRef { id: Some(id(d, d)) }),
                classification: if classified {
                    pb::subdomain::Classification::Core as i32
                } else {
                    pb::subdomain::Classification::Unspecified as i32
                },
                ..Default::default()
            })),
        }
    }

    fn context(ns: &str, realizes: &[(&str, &str)]) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::BoundedContext(pb::BoundedContext {
                id: Some(id(ns, ns)),
                title: ns.into(),
                realizes: realizes
                    .iter()
                    .map(|(n, s)| pb::SubdomainRef { id: Some(id(n, s)) })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    #[test]
    fn domain_slug_must_equal_namespace() {
        let bad = domain("marketplace", "other");
        let issues = validate_scenarios(&bad, &[]);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "DOMAIN_SLUG_MISMATCH")
                .count(),
            1,
            "{issues:?}"
        );
        let good = domain("marketplace", "marketplace");
        let issues = validate_scenarios(&good, &[]);
        assert!(
            !issues.iter().any(|i| i.code == "DOMAIN_SLUG_MISMATCH"),
            "{issues:?}"
        );
    }

    #[test]
    fn subdomain_domain_rules() {
        let all = vec![stored(domain("marketplace", "marketplace"))];
        let foreign = subdomain("marketplace", "identity", Some("otherbiz"), true);
        let issues = validate_scenarios(&foreign, &all);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "SUBDOMAIN_FOREIGN_DOMAIN")
                .count(),
            1,
            "{issues:?}"
        );

        let dangling = subdomain("nowhere", "identity", Some("nowhere"), true);
        let issues = validate_scenarios(&dangling, &all);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "SUBDOMAIN_DANGLING_DOMAIN")
                .count(),
            1,
            "{issues:?}"
        );

        let orphan = subdomain("marketplace", "identity", None, false);
        let issues = validate_scenarios(&orphan, &all);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "SUBDOMAIN_MISSING_DOMAIN")
                .count(),
            1,
            "{issues:?}"
        );
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "SUBDOMAIN_UNCLASSIFIED")
                .count(),
            1,
            "{issues:?}"
        );

        // "good" with no realizing context fires the new SUBDOMAIN_UNREALIZED
        // Info nudge; structurally it is otherwise clean.
        let good = subdomain("marketplace", "identity", Some("marketplace"), true);
        let issues = validate_scenarios(&good, &all);
        let codes: Vec<&str> = issues.iter().map(|i| i.code.as_str()).collect();
        assert_eq!(codes, vec!["SUBDOMAIN_UNREALIZED"], "{issues:?}");

        // Once a context realizes the subdomain, the Info nudge disappears.
        let with_realizer = vec![
            stored(domain("marketplace", "marketplace")),
            stored(context("accounts", &[("marketplace", "identity")])),
        ];
        let issues = validate_scenarios(&good, &with_realizer);
        assert!(
            !issues.iter().any(|i| i.code == "SUBDOMAIN_UNREALIZED"),
            "{issues:?}"
        );
    }

    #[test]
    fn domain_with_no_core_warns_info() {
        // A domain with subdomains but none classified Core fires the
        // new Info nudge. A domain with at least one Core subdomain is
        // clean; a domain with NO subdomains at all is also clean (the
        // classification question only applies if there is something
        // to classify).
        let core_sub = subdomain("marketplace", "identity", Some("marketplace"), true);
        let supporting = subdomain_supporting("marketplace", "billing", Some("marketplace"));

        let dom = domain("marketplace", "marketplace");

        // No subdomains at all: silent.
        let issues = validate_scenarios(&dom, &[stored(dom.clone())]);
        assert!(
            !issues.iter().any(|i| i.code == "DOMAIN_HAS_NO_CORE"),
            "{issues:?}"
        );

        // Subdomains but no core: Info nudge.
        let no_core = vec![stored(dom.clone()), stored(supporting.clone())];
        let issues = validate_scenarios(&dom, &no_core);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "DOMAIN_HAS_NO_CORE")
                .count(),
            1,
            "{issues:?}"
        );

        // At least one core: silent.
        let has_core = vec![stored(dom.clone()), stored(core_sub), stored(supporting)];
        let issues = validate_scenarios(&dom, &has_core);
        assert!(
            !issues.iter().any(|i| i.code == "DOMAIN_HAS_NO_CORE"),
            "{issues:?}"
        );
    }

    fn subdomain_supporting(ns: &str, slug: &str, domain_ns: Option<&str>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Subdomain(pb::Subdomain {
                id: Some(id(ns, slug)),
                title: slug.into(),
                domain: domain_ns.map(|d| pb::DomainRef { id: Some(id(d, d)) }),
                classification: pb::subdomain::Classification::Supporting as i32,
                ..Default::default()
            })),
        }
    }

    #[test]
    fn realizes_rules() {
        let all = vec![
            stored(domain("marketplace", "marketplace")),
            stored(subdomain(
                "marketplace",
                "identity",
                Some("marketplace"),
                true,
            )),
            stored(subdomain(
                "marketplace",
                "billing",
                Some("marketplace"),
                true,
            )),
        ];
        let dangling = context("accounts", &[("marketplace", "ghost")]);
        let issues = validate_scenarios(&dangling, &all);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "REALIZES_DANGLING")
                .count(),
            1,
            "{issues:?}"
        );

        let confessing = context(
            "accounts",
            &[("marketplace", "identity"), ("marketplace", "billing")],
        );
        let issues = validate_scenarios(&confessing, &all);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "REALIZES_MULTIPLE")
                .count(),
            1,
            "{issues:?}"
        );

        let good = context("accounts", &[("marketplace", "identity")]);
        let issues = validate_scenarios(&good, &all);
        assert!(
            !issues.iter().any(|i| i.code.starts_with("REALIZES")),
            "{issues:?}"
        );
    }
}

#[cfg(test)]
mod store_composition_language_tests {
    use super::*;

    fn id(ns: &str, slug: &str) -> pb::Id {
        pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn context(ns: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::BoundedContext(pb::BoundedContext {
                id: Some(id(ns, ns)),
                title: ns.into(),
                ..Default::default()
            })),
        }
    }

    fn screen(ns: &str, slug: &str, slots: Vec<pb::Slot>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Screen(pb::Screen {
                id: Some(id(ns, slug)),
                title: slug.into(),
                slots,
                ..Default::default()
            })),
        }
    }

    fn ui(ns: &str, slug: &str, screen_ns: &str, screen_slug: &str, slot: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Ui(pb::Ui {
                id: Some(id(ns, slug)),
                title: slug.into(),
                slot: Some(pb::ScreenSlotRef {
                    screen: Some(pb::ScreenRef {
                        id: Some(id(screen_ns, screen_slug)),
                    }),
                    slot: slot.into(),
                }),
                ..Default::default()
            })),
        }
    }

    fn term(ns: &str, slug: &str, embodied_by: Vec<pb::EntityRef>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Term(pb::Term {
                id: Some(id(ns, slug)),
                title: slug.into(),
                embodied_by,
                ..Default::default()
            })),
        }
    }

    fn ambiguity(
        terms: Vec<pb::TermRef>,
        kind: pb::ambiguity::Kind,
        ruling: pb::ambiguity::Ruling,
    ) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Ambiguity(pb::Ambiguity {
                id: Some(id("marketplace", "review-collision")),
                kind: kind as i32,
                terms,
                ruling: ruling as i32,
                ..Default::default()
            })),
        }
    }

    fn codes(issues: &[pb::ValidationIssue]) -> HashSet<&str> {
        issues.iter().map(|i| i.code.as_str()).collect()
    }

    #[test]
    fn screen_rules_validate_namespace_slots_and_contributors() {
        let issues = validate_store(&[
            stored(context("checkout")),
            stored(screen(
                "checkout",
                "summary",
                vec![pb::Slot {
                    name: "main".into(),
                    required: true,
                    ..Default::default()
                }],
            )),
        ]);
        let code_set = codes(&issues);
        assert!(
            code_set.contains("SCREEN_NAMESPACE_IS_CONTEXT"),
            "{issues:?}"
        );
        assert!(code_set.contains("SLOT_UNFILLED"), "{issues:?}");

        let issues = validate_store(&[
            stored(context("checkout")),
            stored(context("payments")),
            stored(screen(
                "app",
                "checkout",
                vec![pb::Slot {
                    name: "main".into(),
                    required: true,
                    ..Default::default()
                }],
            )),
            stored(ui("checkout", "order-panel", "app", "checkout", "main")),
            stored(ui("payments", "payment-panel", "app", "checkout", "main")),
        ]);
        let code_set = codes(&issues);
        assert!(
            !code_set.contains("SCREEN_NAMESPACE_IS_CONTEXT"),
            "{issues:?}"
        );
        assert!(!code_set.contains("SLOT_UNFILLED"), "{issues:?}");
        assert!(code_set.contains("SCREEN_MANY_CONTRIBUTORS"), "{issues:?}");
    }

    #[test]
    fn term_and_ambiguity_rules_validate_real_references_and_rulings() {
        let target = pb::EntityRef {
            kind: pb::EntityKind::Ui as i32,
            id: Some(id("reviews", "review-form")),
        };
        let review_term = term("reviews", "review", vec![target]);
        let moderation_term = term("moderation", "review", Vec::new());
        let issues = validate_store(&[
            stored(review_term.clone()),
            stored(moderation_term.clone()),
            stored(ambiguity(
                vec![pb::TermRef {
                    id: Some(id("reviews", "review")),
                }],
                pb::ambiguity::Kind::Unspecified,
                pb::ambiguity::Ruling::Unspecified,
            )),
        ]);
        let code_set = codes(&issues);
        assert!(code_set.contains("TERM_DANGLING_ENTITY"), "{issues:?}");
        assert!(
            code_set.contains("AMBIGUITY_UNSPECIFIED_KIND"),
            "{issues:?}"
        );
        assert!(code_set.contains("AMBIGUITY_TOO_FEW_TERMS"), "{issues:?}");
        assert!(
            code_set.contains("AMBIGUITY_UNSPECIFIED_RULING"),
            "{issues:?}"
        );

        let issues = validate_store(&[
            stored(pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::Ui(pb::Ui {
                    id: Some(id("reviews", "review-form")),
                    title: "Review form".into(),
                    ..Default::default()
                })),
            }),
            stored(review_term),
            stored(moderation_term),
            stored(ambiguity(
                vec![
                    pb::TermRef {
                        id: Some(id("reviews", "review")),
                    },
                    pb::TermRef {
                        id: Some(id("moderation", "review")),
                    },
                ],
                pb::ambiguity::Kind::Homonym,
                pb::ambiguity::Ruling::Deliberate,
            )),
        ]);
        let code_set = codes(&issues);
        assert!(!code_set.contains("TERM_DANGLING_ENTITY"), "{issues:?}");
        assert!(
            !code_set.contains("AMBIGUITY_UNSPECIFIED_KIND"),
            "{issues:?}"
        );
        assert!(!code_set.contains("AMBIGUITY_TOO_FEW_TERMS"), "{issues:?}");
        assert!(
            !code_set.contains("AMBIGUITY_UNSPECIFIED_RULING"),
            "{issues:?}"
        );
    }
}

#[cfg(test)]
mod relationship_tests {
    use super::*;

    fn id(ns: &str, slug: &str) -> pb::Id {
        pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn context(ns: &str, rels: &[(&str, i32)]) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::BoundedContext(pb::BoundedContext {
                id: Some(id(ns, ns)),
                title: ns.into(),
                relationships: rels
                    .iter()
                    .map(|(up, intent)| pb::bounded_context::ContextRelationship {
                        upstream: Some(pb::BoundedContextRef {
                            id: Some(id(up, up)),
                        }),
                        intent: *intent,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn seam_view(down_ns: &str, up_ns: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(down_ns, "subscription-view")),
                title: "subscription-view".into(),
                source_events: vec![pb::EventRef {
                    id: Some(id(up_ns, "thing.done")),
                }],
                ..Default::default()
            })),
        }
    }

    use pb::bounded_context::context_relationship::Intent;

    #[test]
    fn seam_without_relationship_nudges() {
        let all = vec![stored(context("up", &[])), stored(seam_view("down", "up"))];
        let issues = validate_scenarios(&context("down", &[]), &all);
        assert_eq!(
            issues
                .iter()
                .filter(|i| {
                    i.code == "SEAM_WITHOUT_RELATIONSHIP"
                        && i.severity == pb::validation_issue::Severity::Warning as i32
                })
                .count(),
            1,
            "{issues:?}"
        );
    }

    #[test]
    fn declared_intent_with_seam_is_clean() {
        let all = vec![stored(context("up", &[])), stored(seam_view("down", "up"))];
        let down = context("down", &[("up", Intent::CustomerSupplier as i32)]);
        let issues = validate_scenarios(&down, &all);
        assert!(
            !issues.iter().any(
                |i| i.code.starts_with("RELATIONSHIP") || i.code == "SEAM_WITHOUT_RELATIONSHIP"
            ),
            "{issues:?}"
        );
    }

    #[test]
    fn separate_ways_with_seam_errors() {
        let all = vec![stored(context("up", &[])), stored(seam_view("down", "up"))];
        let down = context("down", &[("up", Intent::SeparateWays as i32)]);
        let issues = validate_scenarios(&down, &all);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "RELATIONSHIP_CONTRADICTS_SEAM")
                .count(),
            1,
            "{issues:?}"
        );
    }

    #[test]
    fn intent_without_seam_warns() {
        let all = vec![stored(context("up", &[]))];
        let down = context("down", &[("up", Intent::Conformist as i32)]);
        let issues = validate_scenarios(&down, &all);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "RELATIONSHIP_WITHOUT_SEAM")
                .count(),
            1,
            "{issues:?}"
        );
    }

    #[test]
    fn dangling_and_self_error() {
        let issues = validate_scenarios(
            &context("down", &[("ghost", Intent::Conformist as i32)]),
            &[],
        );
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "RELATIONSHIP_DANGLING")
                .count(),
            1,
            "{issues:?}"
        );
        let all = vec![stored(context("down", &[]))];
        let issues = validate_scenarios(
            &context("down", &[("down", Intent::Conformist as i32)]),
            &all,
        );
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "RELATIONSHIP_SELF")
                .count(),
            1,
            "{issues:?}"
        );
    }
}

/// Tests for validation rules that previously had no coverage. Each test
/// constructs the minimum entity graph required to trigger one specific
/// rule code, asserts that code fires once, and asserts that adjacent
/// rules do NOT fire, guarding against silent regression when nearby
/// passes change.
#[cfg(test)]
mod previously_untested_rules {
    use super::*;

    fn id(ns: &str, slug: &str, v: u64) -> pb::Id {
        pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: v,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn entity_ref(kind: pb::EntityKind, id: pb::Id) -> pb::EntityRef {
        pb::EntityRef {
            kind: kind as i32,
            id: Some(id),
        }
    }

    // ---- TRACKER_MISSING_SUBJECT / TRACKER_DUPLICATE_SUBJECT ----

    fn tracker_item(subject: Option<pb::EntityRef>, doc: &str) -> pb::tracker::Item {
        pb::tracker::Item {
            subject,
            status: 0,
            doc: doc.into(),
            metadata: Vec::new(),
        }
    }

    fn tracker(slug: &str, items: Vec<pb::tracker::Item>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Tracker(pb::Tracker {
                id: Some(id("t", slug, 1)),
                title: slug.into(),
                doc: String::new(),
                items,
                metadata: Vec::new(),
                supersedes: None,
            })),
        }
    }

    #[test]
    fn tracker_missing_subject_errors() {
        let issues = validate_scenarios(&tracker("milestone", vec![tracker_item(None, "")]), &[]);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "TRACKER_MISSING_SUBJECT")
                .count(),
            1,
            "{issues:?}",
        );
    }

    #[test]
    fn tracker_duplicate_subject_warns() {
        // The tracker has two items pointing at the same Event subject.
        // We seed the Event in `all` so the dangling-ref check doesn't fire.
        let event = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id("t", "ev", 1)),
                title: "ev".into(),
                doc: String::new(),
                swimlane: None,
                metadata: Vec::new(),
                supersedes: None,
                schema: None,
            })),
        };
        let dup_subject = entity_ref(pb::EntityKind::Event, id("t", "ev", 1));
        let t = tracker(
            "milestone",
            vec![
                tracker_item(Some(dup_subject.clone()), "first"),
                tracker_item(Some(dup_subject), "second"),
            ],
        );
        let issues = validate_scenarios(&t, &[stored(event)]);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "TRACKER_DUPLICATE_SUBJECT")
                .count(),
            1,
            "{issues:?}",
        );
    }

    // ---- STORYBOARD_MISSING_ENTRY / STORYBOARD_MISSING_OUTCOME ----

    fn sb_missing_entry_and_outcome(ns: &str, slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Storyboard(pb::Storyboard {
                id: Some(id(ns, slug, 1)),
                title: slug.into(),
                doc: String::new(),
                slices: Vec::new(),
                entry: None,
                outcome: None,
                traces: Vec::new(),
                metadata: Vec::new(),
                supersedes: None,
            })),
        }
    }

    fn model_with_storyboard_member(ns: &str, slug: &str) -> pb::EventModel {
        pb::EventModel {
            id: Some(id(ns, "m", 1)),
            title: "m".into(),
            doc: String::new(),
            members: vec![entity_ref(pb::EntityKind::Storyboard, id(ns, slug, 1))],
            metadata: Vec::new(),
            supersedes: None,
        }
    }

    #[test]
    fn storyboard_missing_entry_warns() {
        let ns = "sb";
        let sb = sb_missing_entry_and_outcome(ns, "story");
        let issues = validate_model(&model_with_storyboard_member(ns, "story"), &[stored(sb)]);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "STORYBOARD_MISSING_ENTRY")
                .count(),
            1,
            "{issues:?}",
        );
    }

    #[test]
    fn storyboard_missing_outcome_warns() {
        let ns = "sb";
        let sb = sb_missing_entry_and_outcome(ns, "story");
        let issues = validate_model(&model_with_storyboard_member(ns, "story"), &[stored(sb)]);
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.code == "STORYBOARD_MISSING_OUTCOME")
                .count(),
            1,
            "{issues:?}",
        );
    }

    // ---- UI_TRANSITION_DANGLING ----

    #[test]
    fn ui_transition_dangling_errors_on_unknown_target() {
        let ns = "u";
        let ui = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Ui(pb::Ui {
                id: Some(id(ns, "home", 1)),
                title: "home".into(),
                doc: String::new(),
                design_ref: String::new(),
                transitions: vec![pb::ui::Transition {
                    to: Some(pb::UiRef {
                        id: Some(id(ns, "ghost", 1)),
                    }),
                    doc: String::new(),
                    metadata: Vec::new(),
                }],
                metadata: Vec::new(),
                supersedes: None,
                slot: None,
            })),
        };
        let model = pb::EventModel {
            id: Some(id(ns, "m", 1)),
            title: "m".into(),
            doc: String::new(),
            members: vec![entity_ref(pb::EntityKind::Ui, id(ns, "home", 1))],
            metadata: Vec::new(),
            supersedes: None,
        };
        let issues = validate_model(&model, &[stored(ui)]);
        let count = issues
            .iter()
            .filter(|i| i.code == "UI_TRANSITION_DANGLING")
            .count();
        assert_eq!(count, 1, "{issues:?}");
    }
}

#[cfg(test)]
mod rm_slice_event_not_declared_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn read_model(slug: &str, source_events: Vec<&str>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(slug)),
                source_events: source_events
                    .into_iter()
                    .map(|s| pb::EventRef { id: Some(id(s)) })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn read_model_slice(slug: &str, rm_slug: &str, source_events: Vec<&str>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
                id: Some(id(slug)),
                read_model: Some(pb::ReadModelEdge {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(id(rm_slug)),
                    }),
                    ..Default::default()
                }),
                source_events: source_events
                    .into_iter()
                    .map(|s| pb::EventEdge {
                        event: Some(pb::EventRef { id: Some(id(s)) }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn model(members: &[(pb::EntityKind, &str)]) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: members
                .iter()
                .map(|(k, s)| pb::EntityRef {
                    kind: *k as i32,
                    id: Some(id(s)),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn slice_consuming_undeclared_event_errors() {
        let all = vec![
            stored(read_model("account-profile", vec!["account.registered"])),
            stored(read_model_slice(
                "project-username",
                "account-profile",
                vec!["username.claimed"],
            )),
        ];
        let m = model(&[
            (pb::EntityKind::ReadModel, "account-profile"),
            (pb::EntityKind::ReadModelSlice, "project-username"),
        ]);
        let issues = validate_model(&m, &all);
        let matching: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "RM_SLICE_EVENT_NOT_DECLARED")
            .collect();
        assert_eq!(matching.len(), 1, "{issues:?}");
        assert_eq!(
            matching[0].severity,
            pb::validation_issue::Severity::Error as i32,
            "{issues:?}"
        );
        assert_eq!(
            matching[0]
                .subject
                .as_ref()
                .and_then(|s| pb::EntityKind::try_from(s.kind).ok()),
            Some(pb::EntityKind::ReadModelSlice),
            "{issues:?}"
        );
    }

    #[test]
    fn slice_consuming_declared_event_emits_no_issue() {
        let all = vec![
            stored(read_model(
                "account-profile",
                vec!["account.registered", "username.claimed"],
            )),
            stored(read_model_slice(
                "project-username",
                "account-profile",
                vec!["username.claimed"],
            )),
        ];
        let m = model(&[
            (pb::EntityKind::ReadModel, "account-profile"),
            (pb::EntityKind::ReadModelSlice, "project-username"),
        ]);
        let issues = validate_model(&m, &all);
        let count = issues
            .iter()
            .filter(|i| i.code == "RM_SLICE_EVENT_NOT_DECLARED")
            .count();
        assert_eq!(count, 0, "{issues:?}");
    }
}

#[cfg(test)]
mod rm_key_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "k".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn field(name: &str) -> pb::FieldSpec {
        pb::FieldSpec {
            name: name.into(),
            ..Default::default()
        }
    }

    fn read_model(
        slug: &str,
        fields: Vec<&str>,
        key: Vec<&str>,
        source_events: Vec<&str>,
    ) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(slug)),
                schema: Some(trogon_atlas_core::schema::pack_fields(
                    fields.into_iter().map(field).collect(),
                )),
                key: key.into_iter().map(String::from).collect(),
                source_events: source_events
                    .into_iter()
                    .map(|s| pb::EventRef { id: Some(id(s)) })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn event(slug: &str, fields: Vec<&str>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(slug)),
                schema: Some(trogon_atlas_core::schema::pack_fields(
                    fields.into_iter().map(field).collect(),
                )),
                ..Default::default()
            })),
        }
    }

    fn read_model_slice(
        slug: &str,
        rm_slug: &str,
        source_events: Vec<&str>,
        role: pb::ProjectionRole,
    ) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
                id: Some(id(slug)),
                read_model: Some(pb::ReadModelEdge {
                    read_model: Some(pb::ReadModelRef {
                        id: Some(id(rm_slug)),
                    }),
                    ..Default::default()
                }),
                source_events: source_events
                    .into_iter()
                    .map(|s| pb::EventEdge {
                        event: Some(pb::EventRef { id: Some(id(s)) }),
                        ..Default::default()
                    })
                    .collect(),
                projection_role: role as i32,
                ..Default::default()
            })),
        }
    }

    fn model(members: &[(pb::EntityKind, &str)]) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: members
                .iter()
                .map(|(k, s)| pb::EntityRef {
                    kind: *k as i32,
                    id: Some(id(s)),
                })
                .collect(),
            ..Default::default()
        }
    }

    // RM_KEY_NOT_A_FIELD: key names a field the RM does not declare.
    #[test]
    fn key_name_not_in_fields_errors() {
        let all = vec![stored(read_model(
            "orders",
            vec!["total"],
            vec!["order_id"],
            vec![],
        ))];
        let m = model(&[(pb::EntityKind::ReadModel, "orders")]);
        let issues = validate_model(&m, &all);
        let matching: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "RM_KEY_NOT_A_FIELD")
            .collect();
        assert_eq!(matching.len(), 1, "{issues:?}");
        assert_eq!(
            matching[0].severity,
            pb::validation_issue::Severity::Error as i32,
        );
        assert_eq!(matching[0].subject_field, "key");
    }

    // RM_KEY_MISSING: RM has fields but no key emits exactly one Info.
    #[test]
    fn rm_with_fields_and_no_key_emits_one_info() {
        let all = vec![stored(read_model(
            "orders",
            vec!["order_id", "total"],
            vec![],
            vec![],
        ))];
        let m = model(&[(pb::EntityKind::ReadModel, "orders")]);
        let issues = validate_model(&m, &all);
        let matching: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "RM_KEY_MISSING")
            .collect();
        assert_eq!(matching.len(), 1, "{issues:?}");
        assert_eq!(
            matching[0].severity,
            pb::validation_issue::Severity::Info as i32,
        );
    }

    // RM_SLICE_KEY_UNSOURCED: keyed RM with a delete-role slice whose event
    // lacks the key field errors on the right subject_field.
    #[test]
    fn keyed_rm_slice_event_missing_key_field_errors() {
        let rm_slug = "review-eligibility";
        let ev_slug = "eligibility.consumed";
        let slice_slug = "s-consumed";
        // RM declares fields [order_id, user_id] and key [order_id].
        // The source event only has user_id, so order_id is missing.
        let all = vec![
            stored(read_model(
                rm_slug,
                vec!["order_id", "user_id"],
                vec!["order_id"],
                vec![ev_slug],
            )),
            stored(event(ev_slug, vec!["user_id"])),
            stored(read_model_slice(
                slice_slug,
                rm_slug,
                vec![ev_slug],
                pb::ProjectionRole::Delete,
            )),
        ];
        let m = model(&[
            (pb::EntityKind::ReadModel, rm_slug),
            (pb::EntityKind::ReadModelSlice, slice_slug),
        ]);
        let issues = validate_model(&m, &all);
        let matching: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "RM_SLICE_KEY_UNSOURCED")
            .collect();
        assert_eq!(matching.len(), 1, "{issues:?}");
        assert_eq!(
            matching[0].severity,
            pb::validation_issue::Severity::Error as i32,
        );
        // subject is the slice, subject_field pinpoints the source_events index.
        assert_eq!(
            matching[0]
                .subject
                .as_ref()
                .and_then(|s| pb::EntityKind::try_from(s.kind).ok()),
            Some(pb::EntityKind::ReadModelSlice),
        );
        assert_eq!(matching[0].subject_field, "source_events[0]");
    }

    // Clean: keyed RM, slice event carries the key field, no RM_SLICE_KEY_UNSOURCED.
    #[test]
    fn keyed_rm_slice_event_carrying_key_is_clean() {
        let rm_slug = "review-eligibility";
        let ev_slug = "eligibility.consumed";
        let slice_slug = "s-consumed";
        let all = vec![
            stored(read_model(
                rm_slug,
                vec!["order_id", "user_id"],
                vec!["order_id"],
                vec![ev_slug],
            )),
            stored(event(ev_slug, vec!["order_id", "user_id"])),
            stored(read_model_slice(
                slice_slug,
                rm_slug,
                vec![ev_slug],
                pb::ProjectionRole::Delete,
            )),
        ];
        let m = model(&[
            (pb::EntityKind::ReadModel, rm_slug),
            (pb::EntityKind::ReadModelSlice, slice_slug),
        ]);
        let issues = validate_model(&m, &all);
        let count = issues
            .iter()
            .filter(|i| i.code == "RM_SLICE_KEY_UNSOURCED")
            .count();
        assert_eq!(count, 0, "{issues:?}");
    }

    // RM_SLICE_ROLE_MISSING: unspecified role with a keyed RM emits Info.
    #[test]
    fn unspecified_role_on_keyed_rm_emits_info() {
        let rm_slug = "review-eligibility";
        let ev_slug = "eligibility.granted";
        let slice_slug = "s-granted";
        let all = vec![
            stored(read_model(
                rm_slug,
                vec!["order_id"],
                vec!["order_id"],
                vec![ev_slug],
            )),
            stored(event(ev_slug, vec!["order_id"])),
            stored(read_model_slice(
                slice_slug,
                rm_slug,
                vec![ev_slug],
                pb::ProjectionRole::Unspecified,
            )),
        ];
        let m = model(&[
            (pb::EntityKind::ReadModel, rm_slug),
            (pb::EntityKind::ReadModelSlice, slice_slug),
        ]);
        let issues = validate_model(&m, &all);
        let matching: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "RM_SLICE_ROLE_MISSING")
            .collect();
        assert_eq!(matching.len(), 1, "{issues:?}");
        assert_eq!(
            matching[0].severity,
            pb::validation_issue::Severity::Info as i32,
        );
    }

    // No RM_SLICE_KEY_UNSOURCED when the RM has no key, regardless of slices.
    #[test]
    fn no_key_on_rm_produces_no_slice_key_unsourced() {
        let rm_slug = "orders";
        let ev_slug = "order.placed";
        let slice_slug = "s-placed";
        // RM has fields but no key.
        let all = vec![
            stored(read_model(rm_slug, vec!["total"], vec![], vec![ev_slug])),
            stored(event(ev_slug, vec!["amount"])), // event does NOT carry "total"
            stored(read_model_slice(
                slice_slug,
                rm_slug,
                vec![ev_slug],
                pb::ProjectionRole::Upsert,
            )),
        ];
        let m = model(&[
            (pb::EntityKind::ReadModel, rm_slug),
            (pb::EntityKind::ReadModelSlice, slice_slug),
        ]);
        let issues = validate_model(&m, &all);
        let count = issues
            .iter()
            .filter(|i| i.code == "RM_SLICE_KEY_UNSOURCED")
            .count();
        assert_eq!(count, 0, "{issues:?}");
    }

    // RM_SOURCE_EVENT_FIELD_NOT_PROJECTED: RM is wired to a source event via
    // a read-model slice, but the RM's fields list is missing fields the
    // event carries.
    #[test]
    fn rm_source_event_field_not_projected_lists_missing_fields() {
        let rm_slug = "account-profile";
        let ev_slug = "username.claimed";
        let slice_slug = "s-claimed";
        let all = vec![
            stored(read_model(
                rm_slug,
                vec!["account_id"],
                vec![],
                vec![ev_slug],
            )),
            stored(event(ev_slug, vec!["username", "account_id", "claimed_at"])),
            stored(read_model_slice(
                slice_slug,
                rm_slug,
                vec![ev_slug],
                pb::ProjectionRole::Upsert,
            )),
        ];
        let m = model(&[
            (pb::EntityKind::ReadModel, rm_slug),
            (pb::EntityKind::ReadModelSlice, slice_slug),
        ]);
        let issues = validate_model(&m, &all);
        let matching: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "RM_SOURCE_EVENT_FIELD_NOT_PROJECTED")
            .collect();
        assert_eq!(matching.len(), 1, "{issues:?}");
        assert_eq!(
            matching[0].severity,
            pb::validation_issue::Severity::Error as i32,
        );
        assert_eq!(
            matching[0]
                .subject
                .as_ref()
                .and_then(|s| pb::EntityKind::try_from(s.kind).ok()),
            Some(pb::EntityKind::ReadModel),
        );
        assert!(matching[0].message.contains("username"));
        assert!(matching[0].message.contains("claimed_at"));
        let rm_event_not_projected_count = issues
            .iter()
            .filter(|i| i.code == "RM_EVENT_NOT_PROJECTED")
            .count();
        assert_eq!(rm_event_not_projected_count, 0, "{issues:?}");
    }

    // Clean: RM declares every field the wired source event provides.
    #[test]
    fn rm_source_event_field_not_projected_when_all_fields_declared_emits_no_issue() {
        let rm_slug = "account-profile";
        let ev_slug = "username.claimed";
        let slice_slug = "s-claimed";
        let all = vec![
            stored(read_model(
                rm_slug,
                vec!["username", "account_id", "claimed_at"],
                vec![],
                vec![ev_slug],
            )),
            stored(event(ev_slug, vec!["username", "account_id", "claimed_at"])),
            stored(read_model_slice(
                slice_slug,
                rm_slug,
                vec![ev_slug],
                pb::ProjectionRole::Upsert,
            )),
        ];
        let m = model(&[
            (pb::EntityKind::ReadModel, rm_slug),
            (pb::EntityKind::ReadModelSlice, slice_slug),
        ]);
        let issues = validate_model(&m, &all);
        let count = issues
            .iter()
            .filter(|i| i.code == "RM_SOURCE_EVENT_FIELD_NOT_PROJECTED")
            .count();
        assert_eq!(count, 0, "{issues:?}");
    }
}

#[cfg(test)]
mod event_emitter_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn event(slug: &str, metadata: Vec<prost_types::Any>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(slug)),
                title: slug.into(),
                metadata,
                ..Default::default()
            })),
        }
    }

    fn command(slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Command(pb::Command {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn command_slice(slug: &str, cmd_slug: &str, event_slugs: &[&str]) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id(slug)),
                title: slug.into(),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef {
                        id: Some(id(cmd_slug)),
                    }),
                    ..Default::default()
                }),
                emitted_events: event_slugs
                    .iter()
                    .map(|e| pb::EventEdge {
                        event: Some(pb::EventRef { id: Some(id(e)) }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn model_with_event(slug: &str) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id(slug)),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn single_emitter_is_clean() {
        let all = vec![
            stored(event("paid", vec![])),
            stored(command("pay")),
            stored(command_slice("pay-slice", "pay", &["paid"])),
        ];
        let issues = validate_model(&model_with_event("paid"), &all);
        assert!(
            !issues
                .iter()
                .any(|i| i.code.starts_with("EVENT_MULTIPLE_EMITTERS")),
            "{issues:?}",
        );
    }

    #[test]
    fn two_slices_of_one_command_collapse_to_one_emitter() {
        let all = vec![
            stored(event("paid", vec![])),
            stored(command("pay")),
            stored(command_slice("pay-slice", "pay", &["paid"])),
            stored(command_slice("pay-slice-alt", "pay", &["paid"])),
        ];
        let issues = validate_model(&model_with_event("paid"), &all);
        assert!(
            !issues.iter().any(|i| i.code == "EVENT_MULTIPLE_EMITTERS"),
            "{issues:?}",
        );
    }

    // Decision #24 mid-flight migration: command@1 and command@2 are the same
    // logical decision evolving. Fingerprinting emitters with @version treats
    // them as distinct commands and falsely raises EVENT_MULTIPLE_EMITTERS.
    #[test]
    fn two_versions_of_one_command_collapse_to_one_emitter() {
        let cmd_v = |slug: &str, version: u64| pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Command(pb::Command {
                id: Some(pb::Id {
                    namespace: "t".into(),
                    slug: slug.into(),
                    version,
                }),
                title: slug.into(),
                ..Default::default()
            })),
        };
        let slice_v = |slug: &str, cmd_slug: &str, cmd_ver: u64, event_slug: &str| pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id(slug)),
                title: slug.into(),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef {
                        id: Some(pb::Id {
                            namespace: "t".into(),
                            slug: cmd_slug.into(),
                            version: cmd_ver,
                        }),
                    }),
                    ..Default::default()
                }),
                emitted_events: vec![pb::EventEdge {
                    event: Some(pb::EventRef {
                        id: Some(id(event_slug)),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
        };
        let all = vec![
            stored(event("paid", vec![])),
            stored(cmd_v("pay", 1)),
            stored(cmd_v("pay", 2)),
            stored(slice_v("pay-slice-v1", "pay", 1, "paid")),
            stored(slice_v("pay-slice-v2", "pay", 2, "paid")),
        ];
        let issues = validate_model(&model_with_event("paid"), &all);
        assert!(
            !issues.iter().any(|i| i.code == "EVENT_MULTIPLE_EMITTERS"),
            "command version upgrade must not invent EVENT_MULTIPLE_EMITTERS: {issues:?}",
        );
    }

    #[test]
    fn two_commands_error() {
        let all = vec![
            stored(event("phase.completed", vec![])),
            stored(command("complete-a")),
            stored(command("complete-b")),
            stored(command_slice("a-slice", "complete-a", &["phase.completed"])),
            stored(command_slice("b-slice", "complete-b", &["phase.completed"])),
        ];
        let issues = validate_model(&model_with_event("phase.completed"), &all);
        let found: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "EVENT_MULTIPLE_EMITTERS")
            .collect();
        assert_eq!(found.len(), 1, "{issues:?}");
        assert_eq!(
            found[0].severity,
            pb::validation_issue::Severity::Error as i32
        );
        assert!(found[0].message.contains("command:t/complete-a"));
        assert!(found[0].message.contains("command:t/complete-b"));
        assert!(
            !found[0].message.contains("command:t/complete-a@")
                && !found[0].message.contains("command:t/complete-b@"),
            "emitter fingerprint must not include command version: {}",
            found[0].message
        );
    }

    #[test]
    fn non_member_event_is_not_checked() {
        let all = vec![
            stored(event("phase.completed", vec![])),
            stored(command("complete-a")),
            stored(command("complete-b")),
            stored(command_slice("a-slice", "complete-a", &["phase.completed"])),
            stored(command_slice("b-slice", "complete-b", &["phase.completed"])),
        ];
        // Model lists a different event; the multi-emitter one is out of scope.
        let all2 = {
            let mut v = all;
            v.push(stored(event("other", vec![])));
            v
        };
        let issues = validate_model(&model_with_event("other"), &all2);
        assert!(
            !issues.iter().any(|i| i.code == "EVENT_MULTIPLE_EMITTERS"),
            "{issues:?}",
        );
    }
}

#[cfg(test)]
mod code_ref_tests {
    use prost::Message as _;

    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn code_ref(repo: &str, path: &str) -> prost_types::Any {
        prost_types::Any {
            type_url: "type.googleapis.com/trogonatlas.annotation.v1alpha1.CodeRefAnnotation"
                .into(),
            value: pb::CodeRefAnnotation {
                repo: repo.into(),
                path: path.into(),
                symbol: "M.F".into(),
                r#ref: String::new(),
            }
            .encode_to_vec(),
        }
    }

    fn processor(slug: &str, metadata: Vec<prost_types::Any>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Processor(pb::Processor {
                id: Some(id(slug)),
                title: slug.into(),
                metadata,
                ..Default::default()
            })),
        }
    }

    fn model_with_processor(slug: &str) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::Processor as i32,
                id: Some(id(slug)),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn complete_code_ref_is_clean() {
        let all = vec![stored(processor(
            "worker",
            vec![code_ref("umbrella", "apps/x/worker.ex")],
        ))];
        let issues = validate_model(&model_with_processor("worker"), &all);
        assert!(
            !issues.iter().any(|i| i.code == "CODE_REF_INCOMPLETE"),
            "{issues:?}",
        );
    }

    #[test]
    fn missing_repo_errors() {
        let all = vec![stored(processor(
            "worker",
            vec![code_ref("", "apps/x/worker.ex")],
        ))];
        let issues = validate_model(&model_with_processor("worker"), &all);
        let found: Vec<_> = issues
            .iter()
            .filter(|i| i.code == "CODE_REF_INCOMPLETE")
            .collect();
        assert_eq!(found.len(), 1, "{issues:?}");
        assert!(
            found[0].message.contains("missing repo"),
            "{}",
            found[0].message
        );
    }

    #[test]
    fn missing_path_errors() {
        let all = vec![stored(processor("worker", vec![code_ref("umbrella", "")]))];
        let issues = validate_model(&model_with_processor("worker"), &all);
        assert!(
            issues
                .iter()
                .any(|i| i.code == "CODE_REF_INCOMPLETE" && i.message.contains("missing path")),
            "{issues:?}",
        );
    }

    #[test]
    fn absent_annotation_is_clean() {
        let all = vec![stored(processor("worker", vec![]))];
        let issues = validate_model(&model_with_processor("worker"), &all);
        assert!(
            !issues.iter().any(|i| i.code == "CODE_REF_INCOMPLETE"),
            "{issues:?}",
        );
    }

    // Catalog: walk every member entity's metadata. The matcher used to
    // fall through Component (and other non-slice kinds) to `&[]`, so an
    // incomplete CodeRef on those members was silently ignored.
    #[test]
    fn incomplete_code_ref_on_component_errors() {
        let component = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Component(pb::Component {
                id: Some(id("billing-svc")),
                title: "billing-svc".into(),
                metadata: vec![code_ref("", "apps/billing/worker.ex")],
                ..Default::default()
            })),
        };
        let model = pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::Component as i32,
                id: Some(id("billing-svc")),
            }],
            ..Default::default()
        };
        let issues = validate_model(&model, &[stored(component)]);
        assert!(
            issues
                .iter()
                .any(|i| i.code == "CODE_REF_INCOMPLETE" && i.message.contains("missing repo")),
            "CODE_REF_INCOMPLETE must cover Component metadata; got {issues:?}"
        );
    }
}
#[cfg(test)]
mod projection_ordering_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn command_slice(slug: &str, cmd: &str, events: &[&str]) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id(slug)),
                title: slug.into(),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef { id: Some(id(cmd)) }),
                    ..Default::default()
                }),
                emitted_events: events
                    .iter()
                    .map(|e| pb::EventEdge {
                        event: Some(pb::EventRef { id: Some(id(e)) }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn rm_slice(slug: &str, events: &[&str]) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
                id: Some(id(slug)),
                title: slug.into(),
                source_events: events
                    .iter()
                    .map(|e| pb::EventEdge {
                        event: Some(pb::EventRef { id: Some(id(e)) }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn storyboard(slug: &str, slices: &[&str]) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Storyboard(pb::Storyboard {
                id: Some(id(slug)),
                title: slug.into(),
                slices: slices
                    .iter()
                    .map(|s| pb::SliceRef { id: Some(id(s)) })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn model(members: &[(pb::EntityKind, &str)]) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: members
                .iter()
                .map(|(k, s)| pb::EntityRef {
                    kind: *k as i32,
                    id: Some(id(s)),
                })
                .collect(),
            ..Default::default()
        }
    }

    // Catalog: "emitting command slice comes later (or never)".
    // Current emit_idx lookup silently `continue`s when the emitter is absent
    // from the storyboard, so PROJECTION_BEFORE_EMITTER never fires.
    #[test]
    fn projection_before_emitter_when_emitter_absent_from_storyboard() {
        let all = vec![
            stored(command_slice("emit-slice", "place", &["order.placed"])),
            stored(rm_slice("proj-slice", &["order.placed"])),
            // Emitter exists in the model but is NOT sequenced in this storyboard.
            stored(storyboard("sb", &["proj-slice"])),
        ];
        let m = model(&[
            (pb::EntityKind::CommandSlice, "emit-slice"),
            (pb::EntityKind::ReadModelSlice, "proj-slice"),
            (pb::EntityKind::Storyboard, "sb"),
        ]);
        let issues = validate_model(&m, &all);
        assert!(
            issues
                .iter()
                .any(|i| i.code == "PROJECTION_BEFORE_EMITTER"),
            "catalog promises PROJECTION_BEFORE_EMITTER when the emitter is never in the storyboard; got {issues:?}"
        );
    }

    // Adjacency is measured from the nearest PRECEDING emitter. Storing only
    // the last emit index makes a legal mid-sequence projection look like it
    // runs before its (later) emitter.
    #[test]
    fn projection_after_earlier_emitter_is_not_before_emitter() {
        let all = vec![
            stored(command_slice("first-emit", "place", &["order.placed"])),
            stored(rm_slice("proj-slice", &["order.placed"])),
            stored(command_slice(
                "second-emit",
                "place-again",
                &["order.placed"],
            )),
            stored(storyboard(
                "sb",
                &["first-emit", "proj-slice", "second-emit"],
            )),
        ];
        let m = model(&[
            (pb::EntityKind::CommandSlice, "first-emit"),
            (pb::EntityKind::ReadModelSlice, "proj-slice"),
            (pb::EntityKind::CommandSlice, "second-emit"),
            (pb::EntityKind::Storyboard, "sb"),
        ]);
        let issues = validate_model(&m, &all);
        assert!(
            !issues
                .iter()
                .any(|i| i.code == "PROJECTION_BEFORE_EMITTER"),
            "projection follows the nearest preceding emitter; last-wins emit_idx must not invent PROJECTION_BEFORE_EMITTER: {issues:?}"
        );
    }

    #[test]
    fn action_between_emitter_and_projection_is_not_adjacent() {
        let all = vec![
            stored(command_slice("emit-slice", "place", &["order.placed"])),
            stored(command_slice("other-action", "cancel", &[])),
            stored(rm_slice("proj-slice", &["order.placed"])),
            stored(storyboard(
                "sb",
                &["emit-slice", "other-action", "proj-slice"],
            )),
        ];
        let m = model(&[
            (pb::EntityKind::CommandSlice, "emit-slice"),
            (pb::EntityKind::CommandSlice, "other-action"),
            (pb::EntityKind::ReadModelSlice, "proj-slice"),
            (pb::EntityKind::Storyboard, "sb"),
        ]);
        let issues = validate_model(&m, &all);
        assert!(
            issues.iter().any(|i| i.code == "PROJECTION_NOT_ADJACENT"),
            "expected PROJECTION_NOT_ADJACENT when an action slice sits between emitter and projection; got {issues:?}"
        );
    }

    fn automation_slice(slug: &str, cmd: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
                id: Some(id(slug)),
                title: slug.into(),
                emitted_command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef { id: Some(id(cmd)) }),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        }
    }

    // AutomationSlices do not emit events (they issue commands), but they
    // ARE action slices for adjacency. Confirm they block projection the
    // same way a CommandSlice does, and that emit_idx still only indexes
    // CommandSlice emitters (no false "emitter" from the automation).
    #[test]
    fn automation_between_emitter_and_projection_is_not_adjacent() {
        let all = vec![
            stored(command_slice("emit-slice", "place", &["order.placed"])),
            stored(automation_slice("auto-slice", "follow-up")),
            stored(rm_slice("proj-slice", &["order.placed"])),
            stored(storyboard(
                "sb",
                &["emit-slice", "auto-slice", "proj-slice"],
            )),
        ];
        let m = model(&[
            (pb::EntityKind::CommandSlice, "emit-slice"),
            (pb::EntityKind::AutomationSlice, "auto-slice"),
            (pb::EntityKind::ReadModelSlice, "proj-slice"),
            (pb::EntityKind::Storyboard, "sb"),
        ]);
        let issues = validate_model(&m, &all);
        assert!(
            issues.iter().any(|i| i.code == "PROJECTION_NOT_ADJACENT"),
            "AutomationSlice between emitter and projection must emit PROJECTION_NOT_ADJACENT; got {issues:?}"
        );
        assert!(
            !issues
                .iter()
                .any(|i| i.code == "PROJECTION_BEFORE_EMITTER"),
            "CommandSlice still owns emission; automation must not erase the preceding emitter: {issues:?}"
        );
    }
}

#[cfg(test)]
mod annotation_finding_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "t".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn open_question_any(question: &str, author: &str) -> prost_types::Any {
        use prost::Message;
        let ann = pb::OpenQuestionAnnotation {
            question: question.into(),
            author: author.into(),
            created_at: String::new(),
        };
        prost_types::Any {
            type_url: "type.googleapis.com/trogonatlas.annotation.v1alpha1.OpenQuestionAnnotation"
                .into(),
            value: ann.encode_to_vec(),
        }
    }

    fn assumption_any(statement: &str, author: &str) -> prost_types::Any {
        use prost::Message;
        let ann = pb::AssumptionAnnotation {
            statement: statement.into(),
            basis: String::new(),
            author: author.into(),
            created_at: String::new(),
        };
        prost_types::Any {
            type_url: "type.googleapis.com/trogonatlas.annotation.v1alpha1.AssumptionAnnotation"
                .into(),
            value: ann.encode_to_vec(),
        }
    }

    fn event_with_metadata(slug: &str, metadata: Vec<prost_types::Any>) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(slug)),
                title: slug.into(),
                metadata,
                ..Default::default()
            })),
        }
    }

    fn model(members: &[(pb::EntityKind, &str)]) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: members
                .iter()
                .map(|(k, s)| pb::EntityRef {
                    kind: *k as i32,
                    id: Some(id(s)),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn open_question_on_entity_surfaces_info_finding() {
        let all = vec![stored(event_with_metadata(
            "order.placed",
            vec![open_question_any(
                "does cancellation need its own event?",
                "alex",
            )],
        ))];
        let m = model(&[(pb::EntityKind::Event, "order.placed")]);
        let issues = validate_model(&m, &all);
        let hit = issues
            .iter()
            .find(|i| i.code == "OPEN_QUESTION_PRESENT")
            .unwrap_or_else(|| panic!("expected OPEN_QUESTION_PRESENT; got {issues:?}"));
        assert_eq!(hit.severity, pb::validation_issue::Severity::Info as i32);
        assert!(hit
            .message
            .contains("does cancellation need its own event?"));
        assert!(hit.message.contains("alex"));
        assert_eq!(
            hit.subject
                .as_ref()
                .and_then(|s| s.id.as_ref())
                .map(|i| i.slug.as_str()),
            Some("order.placed")
        );
    }

    #[test]
    fn assumption_on_entity_surfaces_info_finding() {
        let all = vec![stored(event_with_metadata(
            "order.placed",
            vec![assumption_any(
                "the buyer has exactly one shipping address",
                "alex",
            )],
        ))];
        let m = model(&[(pb::EntityKind::Event, "order.placed")]);
        let issues = validate_model(&m, &all);
        let hit = issues
            .iter()
            .find(|i| i.code == "ASSUMPTION_PRESENT")
            .unwrap_or_else(|| panic!("expected ASSUMPTION_PRESENT; got {issues:?}"));
        assert_eq!(hit.severity, pb::validation_issue::Severity::Info as i32);
        assert!(hit
            .message
            .contains("the buyer has exactly one shipping address"));
        assert!(hit.message.contains("alex"));
    }

    #[test]
    fn entity_without_either_annotation_surfaces_neither() {
        let all = vec![stored(event_with_metadata("order.placed", vec![]))];
        let m = model(&[(pb::EntityKind::Event, "order.placed")]);
        let issues = validate_model(&m, &all);
        assert!(!issues
            .iter()
            .any(|i| i.code == "OPEN_QUESTION_PRESENT" || i.code == "ASSUMPTION_PRESENT"));
    }

    #[test]
    fn both_annotations_on_same_entity_surface_both_findings() {
        let all = vec![stored(event_with_metadata(
            "order.placed",
            vec![open_question_any("q", "a1"), assumption_any("s", "a2")],
        ))];
        let m = model(&[(pb::EntityKind::Event, "order.placed")]);
        let issues = validate_model(&m, &all);
        assert!(issues.iter().any(|i| i.code == "OPEN_QUESTION_PRESENT"));
        assert!(issues.iter().any(|i| i.code == "ASSUMPTION_PRESENT"));
    }

    // Per-scenario metadata (CommandScenario.metadata) is independent of
    // the entity's own metadata: a scenario can carry its own open
    // question or assumption distinct from the slice that owns it.
    fn command_slice_with_scenario_metadata(
        slug: &str,
        cmd: &str,
        metadata: Vec<prost_types::Any>,
    ) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id(slug)),
                title: slug.into(),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef { id: Some(id(cmd)) }),
                    ..Default::default()
                }),
                scenarios: vec![pb::CommandScenario {
                    id: "s1".into(),
                    title: "happy path".into(),
                    when: Some(pb::CommandExample {
                        command: Some(pb::CommandRef { id: Some(id(cmd)) }),
                        payload: None,
                    }),
                    then: Some(pb::command_scenario::Then::Emit(pb::EmittedEvents {
                        events: vec![],
                    })),
                    metadata,
                    ..Default::default()
                }],
                ..Default::default()
            })),
        }
    }

    #[test]
    fn open_question_on_command_scenario_surfaces_info_finding() {
        let slice = command_slice_with_scenario_metadata(
            "place-slice",
            "place",
            vec![open_question_any("what happens on a tie?", "alex")],
        );
        let issues = validate_scenarios(&slice, &[]);
        let hit = issues
            .iter()
            .find(|i| i.code == "OPEN_QUESTION_PRESENT")
            .unwrap_or_else(|| panic!("expected OPEN_QUESTION_PRESENT; got {issues:?}"));
        assert_eq!(hit.severity, pb::validation_issue::Severity::Info as i32);
        assert!(hit.message.contains("what happens on a tie?"));
        assert_eq!(hit.subject_field, "scenarios[].metadata");
    }

    #[test]
    fn assumption_on_command_scenario_surfaces_info_finding() {
        let slice = command_slice_with_scenario_metadata(
            "place-slice",
            "place",
            vec![assumption_any(
                "the payload always carries a currency",
                "alex",
            )],
        );
        let issues = validate_scenarios(&slice, &[]);
        let hit = issues
            .iter()
            .find(|i| i.code == "ASSUMPTION_PRESENT")
            .unwrap_or_else(|| panic!("expected ASSUMPTION_PRESENT; got {issues:?}"));
        assert_eq!(hit.severity, pb::validation_issue::Severity::Info as i32);
        assert!(hit
            .message
            .contains("the payload always carries a currency"));
    }
}

#[cfg(test)]
mod reliability_entity_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "ns".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn slo_with_indicator(slo_slug: &str, indicator_slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ServiceLevelObjective(
                pb::ServiceLevelObjective {
                    id: Some(id(slo_slug)),
                    title: slo_slug.into(),
                    indicator: Some(pb::ServiceLevelIndicatorRef {
                        id: Some(id(indicator_slug)),
                    }),
                    ..Default::default()
                },
            )),
        }
    }

    fn indicator(slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ServiceLevelIndicator(
                pb::ServiceLevelIndicator {
                    id: Some(id(slug)),
                    title: slug.into(),
                    ..Default::default()
                },
            )),
        }
    }

    fn model_with_member(kind: pb::EntityKind, slug: &str) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members: vec![pb::EntityRef {
                kind: kind as i32,
                id: Some(id(slug)),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn slo_with_unresolved_indicator_is_dangling() {
        let all = vec![stored(slo_with_indicator(
            "checkout-availability",
            "missing-sli",
        ))];
        let issues = validate_model(
            &model_with_member(
                pb::EntityKind::ServiceLevelObjective,
                "checkout-availability",
            ),
            &all,
        );
        let hits: Vec<_> = issues.iter().filter(|i| i.code == "DANGLING_REF").collect();
        assert_eq!(hits.len(), 1, "{issues:?}");
        assert_eq!(hits[0].subject_field, "indicator");
        assert!(
            hits[0].message.contains("missing-sli"),
            "{}",
            hits[0].message
        );
    }

    #[test]
    fn slo_with_resolved_indicator_has_no_dangling_ref() {
        let all = vec![
            stored(slo_with_indicator(
                "checkout-availability",
                "checkout-latency",
            )),
            stored(indicator("checkout-latency")),
        ];
        let issues = validate_model(
            &model_with_member(
                pb::EntityKind::ServiceLevelObjective,
                "checkout-availability",
            ),
            &all,
        );
        assert!(
            issues.iter().all(|i| i.code != "DANGLING_REF"),
            "{issues:?}"
        );
    }
}

#[cfg(test)]
mod reliability_rule_tests {
    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "ns".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn stored(entity: pb::Entity) -> StoredEntity {
        StoredEntity {
            entity,
            etag: "1".into(),
        }
    }

    fn count(issues: &[pb::ValidationIssue], code: &str) -> usize {
        issues.iter().filter(|i| i.code == code).count()
    }

    fn entity_ref(kind: pb::EntityKind, slug: &str) -> pb::EntityRef {
        pb::EntityRef {
            kind: kind as i32,
            id: Some(id(slug)),
        }
    }

    fn model_with_members(members: Vec<pb::EntityRef>) -> pb::EventModel {
        pb::EventModel {
            id: Some(id("m")),
            title: "m".into(),
            members,
            ..Default::default()
        }
    }

    fn event_ref(slug: &str) -> pb::EventRef {
        pb::EventRef { id: Some(id(slug)) }
    }

    fn slice_ref(slug: &str) -> pb::SliceRef {
        pb::SliceRef { id: Some(id(slug)) }
    }

    fn event_edge(slug: &str) -> pb::EventEdge {
        pb::EventEdge {
            event: Some(event_ref(slug)),
            ..Default::default()
        }
    }

    fn rm_edge(slug: &str) -> pb::ReadModelEdge {
        pb::ReadModelEdge {
            read_model: Some(pb::ReadModelRef { id: Some(id(slug)) }),
            ..Default::default()
        }
    }

    fn cmd_edge(slug: &str) -> pb::CommandEdge {
        pb::CommandEdge {
            command: Some(pb::CommandRef { id: Some(id(slug)) }),
            ..Default::default()
        }
    }

    fn ui_edge(slug: &str) -> pb::UiEdge {
        pb::UiEdge {
            ui: Some(pb::UiRef { id: Some(id(slug)) }),
            ..Default::default()
        }
    }

    fn event(slug: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            })),
        }
    }

    fn event_with_fields(slug: &str, fields: &[&str]) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(slug)),
                title: slug.into(),
                schema: Some(trogon_atlas_core::schema::pack_fields(
                    fields
                        .iter()
                        .map(|n| pb::FieldSpec {
                            name: (*n).into(),
                            ..Default::default()
                        })
                        .collect(),
                )),
                ..Default::default()
            })),
        }
    }

    fn command_slice(
        slug: &str,
        emitted: &[&str],
        command: Option<&str>,
        ui: Option<&str>,
    ) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(id(slug)),
                title: slug.into(),
                emitted_events: emitted.iter().map(|e| event_edge(e)).collect(),
                command: command.map(cmd_edge),
                ui: ui.map(ui_edge),
                ..Default::default()
            })),
        }
    }

    fn read_model_slice(slug: &str, source_events: &[&str], read_model: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ReadModelSlice(pb::ReadModelSlice {
                id: Some(id(slug)),
                title: slug.into(),
                source_events: source_events.iter().map(|e| event_edge(e)).collect(),
                read_model: Some(rm_edge(read_model)),
                ..Default::default()
            })),
        }
    }

    fn automation_slice(slug: &str, source_read_model: &str, emitted_command: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
                id: Some(id(slug)),
                title: slug.into(),
                source_read_models: vec![rm_edge(source_read_model)],
                emitted_command: Some(cmd_edge(emitted_command)),
                ..Default::default()
            })),
        }
    }

    fn ui_slice(slug: &str, source_read_model: &str, ui: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::UiSlice(pb::UiSlice {
                id: Some(id(slug)),
                title: slug.into(),
                source_read_models: vec![rm_edge(source_read_model)],
                ui: Some(ui_edge(ui)),
                ..Default::default()
            })),
        }
    }

    fn storyboard(slug: &str, slices: &[&str]) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Storyboard(pb::Storyboard {
                id: Some(id(slug)),
                title: slug.into(),
                slices: slices.iter().map(|s| slice_ref(s)).collect(),
                ..Default::default()
            })),
        }
    }

    fn processor(slug: &str, calls: &[&str]) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Processor(pb::Processor {
                id: Some(id(slug)),
                title: slug.into(),
                calls: calls
                    .iter()
                    .map(|s| pb::ExternalSystemRef { id: Some(id(s)) })
                    .collect(),
                ..Default::default()
            })),
        }
    }

    fn sli(
        slug: &str,
        connection: pb::Connection,
        measure: Option<pb::service_level_indicator::Measure>,
        signal: Option<pb::Signal>,
    ) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ServiceLevelIndicator(
                pb::ServiceLevelIndicator {
                    id: Some(id(slug)),
                    title: slug.into(),
                    connection: Some(connection),
                    measure,
                    signal,
                    ..Default::default()
                },
            )),
        }
    }

    fn event_timestamps() -> pb::Signal {
        pb::Signal {
            source: Some(pb::signal::Source::EventTimestamps(
                pb::signal::EventTimestamps {},
            )),
        }
    }

    fn latency_measure() -> pb::service_level_indicator::Measure {
        pb::service_level_indicator::Measure::Latency(pb::service_level_indicator::Latency {})
    }

    fn conn_command_handling(slice: &str, events: &[&str]) -> pb::Connection {
        pb::Connection {
            kind: Some(pb::connection::Kind::CommandHandling(
                pb::connection::CommandHandling {
                    command_slice: Some(slice_ref(slice)),
                    events: events.iter().map(|e| event_ref(e)).collect(),
                },
            )),
        }
    }

    fn conn_projection(slice: &str, events: &[&str]) -> pb::Connection {
        pb::Connection {
            kind: Some(pb::connection::Kind::Projection(
                pb::connection::Projection {
                    read_model_slice: Some(slice_ref(slice)),
                    source_events: events.iter().map(|e| event_ref(e)).collect(),
                },
            )),
        }
    }

    fn conn_display(slice: &str) -> pb::Connection {
        pb::Connection {
            kind: Some(pb::connection::Kind::Display(pb::connection::Display {
                ui_slice: Some(slice_ref(slice)),
            })),
        }
    }

    fn conn_integration(proc_slug: &str, system_slug: &str) -> pb::Connection {
        pb::Connection {
            kind: Some(pb::connection::Kind::Integration(
                pb::connection::Integration {
                    processor: Some(pb::ProcessorRef {
                        id: Some(id(proc_slug)),
                    }),
                    system: Some(pb::ExternalSystemRef {
                        id: Some(id(system_slug)),
                    }),
                },
            )),
        }
    }

    fn conn_journey_storyboard(
        storyboard_slug: &str,
        start: &str,
        end: &str,
        correlate_on: &[&str],
    ) -> pb::Connection {
        pb::Connection {
            kind: Some(pb::connection::Kind::Journey(pb::connection::Journey {
                scope: Some(pb::connection::journey::Scope::Storyboard(
                    pb::StoryboardRef {
                        id: Some(id(storyboard_slug)),
                    },
                )),
                start: Some(event_ref(start)),
                end: Some(event_ref(end)),
                correlate_on: correlate_on.iter().map(|s| (*s).to_string()).collect(),
            })),
        }
    }

    #[test]
    fn connection_unresolved_wrong_slice_kind() {
        let all = vec![
            stored(read_model_slice("rms", &[], "rm")),
            stored(sli(
                "availability",
                conn_command_handling("rms", &[]),
                Some(latency_measure()),
                Some(event_timestamps()),
            )),
        ];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "availability",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLI_CONNECTION_UNRESOLVED"), 1, "{issues:?}");
    }

    #[test]
    fn connection_unresolved_event_not_emitted() {
        let all = vec![
            stored(event("checkout.failed")),
            stored(command_slice(
                "checkout-slice",
                &["checkout.started"],
                None,
                None,
            )),
            stored(sli(
                "checkout-latency",
                conn_command_handling("checkout-slice", &["checkout.failed"]),
                Some(latency_measure()),
                Some(event_timestamps()),
            )),
        ];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "checkout-latency",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLI_CONNECTION_UNRESOLVED"), 1, "{issues:?}");
    }

    #[test]
    fn connection_unresolved_command_handling_clean() {
        let all = vec![
            stored(command_slice(
                "checkout-slice",
                &["checkout.completed"],
                None,
                None,
            )),
            stored(sli(
                "checkout-latency",
                conn_command_handling("checkout-slice", &["checkout.completed"]),
                Some(latency_measure()),
                Some(event_timestamps()),
            )),
        ];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "checkout-latency",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLI_CONNECTION_UNRESOLVED"), 0, "{issues:?}");
    }

    #[test]
    fn connection_unresolved_projection_source_event_not_declared() {
        let all = vec![
            stored(read_model_slice("rms", &["order.placed"], "order-summary")),
            stored(sli(
                "projection-latency",
                conn_projection("rms", &["order.cancelled"]),
                Some(latency_measure()),
                Some(event_timestamps()),
            )),
        ];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "projection-latency",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLI_CONNECTION_UNRESOLVED"), 1, "{issues:?}");
    }

    #[test]
    fn connection_unresolved_integration_system_not_called() {
        let all = vec![
            stored(processor("billing-processor", &["payments-gateway"])),
            stored(sli(
                "shipping-call",
                conn_integration("billing-processor", "shipping-carrier"),
                Some(latency_measure()),
                Some(event_timestamps()),
            )),
        ];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "shipping-call",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLI_CONNECTION_UNRESOLVED"), 1, "{issues:?}");
    }

    fn journey_fixture_slices() -> Vec<StoredEntity> {
        vec![
            stored(read_model_slice(
                "rm-slice",
                &["order.placed"],
                "order-summary",
            )),
            stored(automation_slice(
                "auto-slice",
                "order-summary",
                "ship-order",
            )),
            stored(command_slice(
                "ship-slice",
                &["order.shipped"],
                Some("ship-order"),
                None,
            )),
        ]
    }

    #[test]
    fn journey_unreachable_with_no_bridging_slice() {
        let mut all = vec![stored(storyboard("checkout-sb", &["rm-slice"]))];
        all.push(stored(read_model_slice(
            "rm-slice",
            &["order.placed"],
            "order-summary",
        )));
        all.push(stored(sli(
            "checkout-journey",
            conn_journey_storyboard("checkout-sb", "order.placed", "order.shipped", &[]),
            Some(latency_measure()),
            Some(event_timestamps()),
        )));
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "checkout-journey",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLI_JOURNEY_UNREACHABLE"), 1, "{issues:?}");
    }

    #[test]
    fn journey_reachable_via_automation_chain() {
        let mut all = journey_fixture_slices();
        all.push(stored(storyboard(
            "checkout-sb",
            &["rm-slice", "auto-slice", "ship-slice"],
        )));
        all.push(stored(sli(
            "checkout-journey",
            conn_journey_storyboard("checkout-sb", "order.placed", "order.shipped", &[]),
            Some(latency_measure()),
            Some(event_timestamps()),
        )));
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "checkout-journey",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLI_JOURNEY_UNREACHABLE"), 0, "{issues:?}");
    }

    #[test]
    fn correlation_unsourced_missing_on_end_event() {
        let mut all = journey_fixture_slices();
        all.push(stored(event_with_fields("order.placed", &["order_id"])));
        all.push(stored(event_with_fields("order.shipped", &["shipment_id"])));
        all.push(stored(storyboard(
            "checkout-sb",
            &["rm-slice", "auto-slice", "ship-slice"],
        )));
        all.push(stored(sli(
            "checkout-journey",
            conn_journey_storyboard(
                "checkout-sb",
                "order.placed",
                "order.shipped",
                &["order_id"],
            ),
            Some(latency_measure()),
            Some(event_timestamps()),
        )));
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "checkout-journey",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLI_CORRELATION_UNSOURCED"), 1, "{issues:?}");
    }

    #[test]
    fn correlation_sourced_on_both_ends_is_clean() {
        let mut all = journey_fixture_slices();
        all.push(stored(event_with_fields("order.placed", &["order_id"])));
        all.push(stored(event_with_fields("order.shipped", &["order_id"])));
        all.push(stored(storyboard(
            "checkout-sb",
            &["rm-slice", "auto-slice", "ship-slice"],
        )));
        all.push(stored(sli(
            "checkout-journey",
            conn_journey_storyboard(
                "checkout-sb",
                "order.placed",
                "order.shipped",
                &["order_id"],
            ),
            Some(latency_measure()),
            Some(event_timestamps()),
        )));
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "checkout-journey",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLI_CORRELATION_UNSOURCED"), 0, "{issues:?}");
    }

    fn outcome_ratio_measure(good: &[&str], bad: &[&str]) -> pb::service_level_indicator::Measure {
        pb::service_level_indicator::Measure::OutcomeRatio(
            pb::service_level_indicator::OutcomeRatio {
                good: good.iter().map(|e| event_ref(e)).collect(),
                bad: bad.iter().map(|e| event_ref(e)).collect(),
            },
        )
    }

    #[test]
    fn outcome_unreachable_event_not_emitted_by_command_slice() {
        let all = vec![
            stored(command_slice(
                "checkout-slice",
                &["checkout.completed", "checkout.failed"],
                None,
                None,
            )),
            stored(sli(
                "checkout-outcome",
                conn_command_handling("checkout-slice", &[]),
                Some(outcome_ratio_measure(
                    &["checkout.completed"],
                    &["checkout.timed-out"],
                )),
                Some(event_timestamps()),
            )),
        ];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "checkout-outcome",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLI_OUTCOME_UNREACHABLE"), 1, "{issues:?}");
    }

    #[test]
    fn outcome_unreachable_empty_bad_side() {
        let all = vec![
            stored(command_slice(
                "checkout-slice",
                &["checkout.completed"],
                None,
                None,
            )),
            stored(sli(
                "checkout-outcome",
                conn_command_handling("checkout-slice", &[]),
                Some(outcome_ratio_measure(&["checkout.completed"], &[])),
                Some(event_timestamps()),
            )),
        ];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "checkout-outcome",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLI_OUTCOME_UNREACHABLE"), 1, "{issues:?}");
    }

    #[test]
    fn outcome_reachable_command_handling_is_clean() {
        let all = vec![
            stored(command_slice(
                "checkout-slice",
                &["checkout.completed", "checkout.failed"],
                None,
                None,
            )),
            stored(sli(
                "checkout-outcome",
                conn_command_handling("checkout-slice", &[]),
                Some(outcome_ratio_measure(
                    &["checkout.completed"],
                    &["checkout.failed"],
                )),
                Some(event_timestamps()),
            )),
        ];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "checkout-outcome",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLI_OUTCOME_UNREACHABLE"), 0, "{issues:?}");
    }

    #[test]
    fn signal_mismatch_event_timestamps_on_display() {
        let all = vec![
            stored(ui_slice("dash-slice", "order-summary", "dashboard-ui")),
            stored(sli(
                "dashboard-availability",
                conn_display("dash-slice"),
                Some(latency_measure()),
                Some(event_timestamps()),
            )),
        ];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "dashboard-availability",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLI_SIGNAL_MISMATCH"), 1, "{issues:?}");
    }

    #[test]
    fn signal_mismatch_missing_signal() {
        let all = vec![
            stored(command_slice(
                "checkout-slice",
                &["checkout.completed"],
                None,
                None,
            )),
            stored(sli(
                "checkout-latency",
                conn_command_handling("checkout-slice", &[]),
                Some(latency_measure()),
                None,
            )),
        ];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "checkout-latency",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLI_SIGNAL_MISMATCH"), 1, "{issues:?}");
    }

    #[test]
    fn signal_mismatch_missing_measure() {
        let all = vec![
            stored(command_slice(
                "checkout-slice",
                &["checkout.completed"],
                None,
                None,
            )),
            stored(sli(
                "checkout-latency",
                conn_command_handling("checkout-slice", &[]),
                None,
                Some(event_timestamps()),
            )),
        ];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "checkout-latency",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLI_SIGNAL_MISMATCH"), 1, "{issues:?}");
    }

    fn slo(
        slug: &str,
        indicator: &str,
        objectives: Vec<pb::service_level_objective::Objective>,
        alert_policies: &[&str],
        commitment: Option<pb::Commitment>,
    ) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::ServiceLevelObjective(
                pb::ServiceLevelObjective {
                    id: Some(id(slug)),
                    title: slug.into(),
                    indicator: Some(pb::ServiceLevelIndicatorRef {
                        id: Some(id(indicator)),
                    }),
                    objectives,
                    alert_policies: alert_policies
                        .iter()
                        .map(|s| pb::AlertPolicyRef { id: Some(id(s)) })
                        .collect(),
                    commitment,
                    ..Default::default()
                },
            )),
        }
    }

    fn target(ratio: f64) -> pb::Target {
        pb::Target { ratio }
    }

    fn external_commitment() -> pb::Commitment {
        pb::Commitment {
            kind: Some(pb::commitment::Kind::External(pb::commitment::External {
                counterparty: None,
                agreement: String::new(),
                consequence: String::new(),
            })),
        }
    }

    #[test]
    fn threshold_mismatch_latency_missing_threshold() {
        let all = vec![
            stored(sli(
                "checkout-latency",
                conn_command_handling("checkout-slice", &[]),
                Some(latency_measure()),
                Some(event_timestamps()),
            )),
            stored(slo(
                "checkout-latency-slo",
                "checkout-latency",
                vec![pb::service_level_objective::Objective {
                    target: Some(target(0.99)),
                    ..Default::default()
                }],
                &["ap"],
                None,
            )),
        ];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelObjective,
            "checkout-latency-slo",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLO_THRESHOLD_MISMATCH"), 1, "{issues:?}");
    }

    #[test]
    fn threshold_mismatch_non_latency_has_threshold() {
        let all = vec![
            stored(sli(
                "checkout-outcome",
                conn_command_handling("checkout-slice", &[]),
                Some(outcome_ratio_measure(
                    &["checkout.completed"],
                    &["checkout.failed"],
                )),
                Some(event_timestamps()),
            )),
            stored(slo(
                "checkout-outcome-slo",
                "checkout-outcome",
                vec![pb::service_level_objective::Objective {
                    threshold: Some(pb::LatencyThreshold::default()),
                    target: Some(target(0.99)),
                    ..Default::default()
                }],
                &["ap"],
                None,
            )),
        ];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelObjective,
            "checkout-outcome-slo",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLO_THRESHOLD_MISMATCH"), 1, "{issues:?}");
    }

    #[test]
    fn target_range_rejects_boundary_and_above() {
        let all = vec![stored(slo(
            "checkout-slo",
            "checkout-latency",
            vec![
                pb::service_level_objective::Objective {
                    target: Some(target(0.0)),
                    ..Default::default()
                },
                pb::service_level_objective::Objective {
                    target: Some(target(1.0)),
                    ..Default::default()
                },
            ],
            &["ap"],
            None,
        ))];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelObjective,
            "checkout-slo",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLO_TARGET_RANGE"), 2, "{issues:?}");
    }

    #[test]
    fn target_range_accepts_open_interval() {
        let all = vec![stored(slo(
            "checkout-slo",
            "checkout-latency",
            vec![pb::service_level_objective::Objective {
                target: Some(target(0.995)),
                ..Default::default()
            }],
            &["ap"],
            None,
        ))];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelObjective,
            "checkout-slo",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLO_TARGET_RANGE"), 0, "{issues:?}");
    }

    #[test]
    fn external_commitment_with_no_alerts_is_warning() {
        let all = vec![stored(slo(
            "checkout-sla",
            "checkout-latency",
            vec![],
            &[],
            Some(external_commitment()),
        ))];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelObjective,
            "checkout-sla",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLO_EXTERNAL_UNALERTED"), 1, "{issues:?}");
        assert_eq!(count(&issues, "SLO_UNALERTED"), 0, "{issues:?}");
    }

    #[test]
    fn internal_objective_with_no_alerts_is_info() {
        let all = vec![stored(slo(
            "checkout-slo",
            "checkout-latency",
            vec![],
            &[],
            None,
        ))];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelObjective,
            "checkout-slo",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLO_UNALERTED"), 1, "{issues:?}");
        assert_eq!(count(&issues, "SLO_EXTERNAL_UNALERTED"), 0, "{issues:?}");
    }

    #[test]
    fn objective_with_alerts_reports_neither_unalerted_code() {
        let all = vec![stored(slo(
            "checkout-slo",
            "checkout-latency",
            vec![],
            &["ap"],
            Some(external_commitment()),
        ))];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelObjective,
            "checkout-slo",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(count(&issues, "SLO_EXTERNAL_UNALERTED"), 0, "{issues:?}");
        assert_eq!(count(&issues, "SLO_UNALERTED"), 0, "{issues:?}");
    }

    #[test]
    fn reliability_overlay_members_are_exempt_from_cross_model_boundary_violation() {
        let other_ns_event = pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: "other".into(),
                    slug: "payment.settled".into(),
                    version: 1,
                }),
                title: "payment.settled".into(),
                ..Default::default()
            })),
        };
        let cross_ns_connection = pb::Connection {
            kind: Some(pb::connection::Kind::Journey(pb::connection::Journey {
                scope: Some(pb::connection::journey::Scope::Storyboard(
                    pb::StoryboardRef {
                        id: Some(id("checkout-sb")),
                    },
                )),
                start: Some(pb::EventRef {
                    id: Some(pb::Id {
                        namespace: "other".into(),
                        slug: "payment.settled".into(),
                        version: 1,
                    }),
                }),
                end: Some(pb::EventRef {
                    id: Some(pb::Id {
                        namespace: "other".into(),
                        slug: "payment.settled".into(),
                        version: 1,
                    }),
                }),
                correlate_on: vec![],
            })),
        };
        let all = vec![
            stored(other_ns_event),
            stored(storyboard("checkout-sb", &[])),
            stored(sli(
                "cross-ns-journey",
                cross_ns_connection,
                Some(latency_measure()),
                Some(event_timestamps()),
            )),
        ];
        let model = model_with_members(vec![entity_ref(
            pb::EntityKind::ServiceLevelIndicator,
            "cross-ns-journey",
        )]);
        let issues = validate_model(&model, &all);
        assert_eq!(
            count(&issues, "CROSS_MODEL_BOUNDARY_VIOLATION"),
            0,
            "{issues:?}"
        );
    }
}
