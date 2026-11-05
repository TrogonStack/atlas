# Author a storyboard

Follow these storyboard authoring rules. Validation enforces them: a violation
is rejected or flagged. Fetch an entity with get_entity_json, edit its proto3
JSON, and write it back with put_entity_json.

- A storyboard is a TEMPORAL sequence: each slice is one moment. A read-model
  slice lists only the events that just happened at that moment, never the
  type's full source list (the ReadModel entity carries the type-level list).
- Projection adjacency: every event's read-model projections come directly
  after the emitting slice, before the next command or automation slice acts.
  Updates from divergent branches (e.g. approve vs reject) are separate slices,
  never merged.
- One processor issues exactly one command; branching outcomes are separate
  automation slices with separate processors. Record 'these ship as one worker'
  with a Component entity listing them as members, not by sharing entities.
- Split read models by shape, not by consumer: add a read model only when a
  consumer needs different fields, key or source events. Consumers that read
  the same data share one read model; when several automations observe it,
  declare a SharedConsumersAnnotation naming them. A command's decision state
  comes from its stream (the given events) and is not a read model. Two read
  models with the same source events and fields are a duplicate. External
  integrations are declared on BOTH seams: the processor that performs the call
  lists the ExternalSystem in Processor.calls (outbound), and the results land
  in ExternalSource read models pointing at the same system (inbound); the
  round trip is structural, never prose-only.
- Sub-workflows are SEPARATE storyboards chained implicitly: storyboard A's
  outcome events project into read models that storyboard B's entry observes.
  Never build one mega-storyboard; never invent nesting. The EventModel is the
  workflow umbrella, and its ordered members list sequences the storyboards.
- Every storyboard declares an honest entry (the read model observed plus the
  human persona+ui or automation processor observing it) and an outcome (the
  terminal events). Clock/schedule triggers are read models observed by
  automations.
- Single-use or entitlement constraints (e.g. review-once) get their own
  aggregate swimlane; that stream is the consistency boundary.
- Swimlane stream_id placeholders must be followable names: each {placeholder}
  references a FieldSpec on that lane's events/commands, or the name of a
  Uuidv5IdentityAnnotation registered on the swimlane (composed identities).
  Never inline expressions like {uuidv5(...)}; register the identity once under
  a name and write e.g. 'review-eligibility-{eligibility_id}'.
- Forks in the road live on the SWIMLANE: declare the stream's lifecycle as
  Swimlane.transitions (after `<event>`, next = `[<event> + guard doc, ...]`).
  Exclusivity comes from the consistency boundary: competing commands append to
  the same stream and the aggregate accepts one successor, so alternatives
  without a shared stream are parallel fan-out, not a fork. Cycles are legal
  (lifecycles loop). Tools derive the storyboard fork graph from the table, and
  validation checks every GWT against it: a scenario may only emit a legal
  successor of the last given event on that stream.
- TIME ONLY POINTS FORWARD. Every edge goes from an earlier moment to a later
  one; recurrence of a type (a screen shown again, a view updated again, a
  lifecycle re-entered) is a NEW occurrence, never an arrow into the past; a
  backward arrow destroys the ability to read time off the graph. Order slices
  so events precede their projections and projections precede their consumers;
  write scenario given histories as forward walks of each stream's transitions
  (validated both in given order and in emission).
- Project management layers ON TOP of the model, never inside it. Delivery
  status (todo/in_progress/review/blocked/done) lives in Tracker entities:
  denormalized lists of {subject EntityRef, TrackStatus, doc} pointing at any
  entity kind. Design entities carry NO status fields; flipping a status never
  mutates the model. Several trackers may overlay one model (per team, per
  milestone); within one tracker each subject appears once. Trackers are NOT
  event model members. 'What needs work?' = list_entities kind=tracker.
- UI.transitions declares PURE navigation only: surface-to-surface links that
  carry no command and no event (tab bars, back affordances, menu links, deep
  links). Any hop a storyboard command path already explains is DERIVED from
  the slices; re-declaring it is managed duplication and warns
  (UI_TRANSITION_DERIVABLE). Cycles are legal in the data (a navigation map is
  a graph, like the swimlane lifecycle); renderers keep time forward by
  unrolling occurrences.
- Screen is composition, UI is the moment. Use UI for every slice and
  storyboard touchpoint. Use Screen only for URL-addressable surfaces that
  compose UI contributions through UI.slot. Put Screen entities in a
  non-context application namespace. Required Screen slots need at least one
  contributing UI. A Screen with UI contributions from multiple contexts is
  expected shared-surface composition, not a replacement for context seams.
- Disconnects are data when declared. Any model member that no slice produces,
  consumes, or feeds is DISCONNECTED. Disconnects are not always errors: a
  subscription view may wait for its consumer, an audit log may have no UI yet,
  a niche flow may be deferred. Declare the disconnect with OrphanAnnotation (a
  canonical Any in the entity's metadata) carrying a non-empty `doc` explaining
  WHY. Unflagged disconnects warn (ORPHAN_NOT_FLAGGED); flag-with-empty-doc
  errors (ORPHAN_DOC_EMPTY); flag-on-a-wired-entity nudges
  (ORPHAN_FLAG_OBSOLETE); remove the annotation once the entity is connected.
- Open questions and assumptions are first-class, not comments. An
  OpenQuestionAnnotation names something the model does not yet decide: put it
  on the entity or scenario where the gap lives, with `question` and `author`.
  An AssumptionAnnotation names something the design already proceeds as if
  true but nobody confirmed, with fields `statement`, optional `basis`, and
  `author`. Both attach via put_entity_json as a canonical Any ("@type":
  "type.googleapis.com/trogonatlas.annotation.v1alpha1.OpenQuestionAnnotation"
  or "...AssumptionAnnotation") in the entity's metadata, or a scenario's own
  metadata. Both surface as Info findings (OPEN_QUESTION_PRESENT,
  ASSUMPTION_PRESENT), never an error or a warning, so recording one never
  blocks a mutation.

Mutation responses from put_entity_json return non-blocking validation
findings, strategic ones included, in an `issues` array: read it and fix them.
Run validate_event_model after every mutation batch and fix all Errors and
Warnings.
