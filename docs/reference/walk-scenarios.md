# walk_scenarios reference

`walk_scenarios` plays example data through an event model. Validation
checks the model's structure; the walk runs concrete cases against it and
reports where the model fails to carry the data a real timeline needs. It
is available as the `trogon-atlas walk-scenarios` command and the
`walk_scenarios` MCP tool. Both only read.

See [walk scenarios in CI](../how-to/walk-scenarios.md) for usage. This page
describes the case format, how each step is walked, and every finding code.

## Case format

A case is a `Specification` document. Cases live next to the model as
files (for example `examples/specifications/`); they are not stored
entities yet.

```yaml
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Specification
metadata:
  namespace: walk-reviews        # default namespace for bare refs
  name: approve-review-42
spec:
  title: Submit review 42 with score 4, then approve it
  given:                         # initial values
    events:                      # events already on their streams
    - event: review.submitted
      payload: {review_id: r-42, rating: 4, status: submitted}
    readModels:                  # rows of externally fed read models
    - readModel: review-queue
      rows:
      - {review_id: r-42, status: submitted}
  steps:                         # walked in order
  - command: approve-review
    via: auto-approve            # optional: the AutomationSlice or UiSlice issuing it
    answer: {status: approved}   # external values the command receives
    expect:
      emit: [review.approved]    # or `reject: REASON_CODE` ("" for any reason)
      readModels:                # rows after this step
      - readModel: review-queue
        rows:
        - {review_id: r-42, status: approved}
  then:
    readModels:                  # rows at the end of the case
    - readModel: review-queue
      rows:
      - {review_id: r-42, status: approved, score: 4}
```

Refs are `slug`, `namespace/slug` or `namespace/slug@version`. A bare slug
resolves in the case's namespace, and an unpinned ref resolves to the
latest version. Each part must be a valid id and a version a positive
integer; a malformed ref fails the case file instead of falling back to the
latest version. References inside the model resolve to the exact version
they pin. A `given` event may name an event in a namespace that is not
walked, so upstream events can feed local projections.

Expected `rows` list every row of the read model: the count must match, and
the expected rows must pair one-to-one with actual rows, each matching on the
fields it names, in any order.
`rows: []` asserts the read model is empty.

Unknown fields are refused, so a typo such as `stepz` fails the run
instead of being ignored.

## How a step is walked

1. **Issuers.** The issuers of the command are the AutomationSlices that
   emit it and the UiSlices showing the command slice's `ui`. `via` narrows
   them to one slice, which must be one of them.
2. **Command data.** Each command field gets a value from the rows of the
   read models those issuers observe, or from the step's `answer` (a `null`
   answer supplies nothing). When the answer names a read model's `key`
   fields, only rows with those key values count. Fields with a
   `DerivedFieldAnnotation` get a placeholder; optional fields may stay
   empty. A conflicting answer still wins, and is reported. A required field
   whose rows disagree, with nothing else to supply it, is an error.
3. **Outcome.** The command slice's scenarios are matched against the
   stream's history: the scenario's `given` events must have occurred, in
   order, among the events of the command's timeline (its own stream, plus
   events elsewhere that share a field with the command and agree on every
   shared field; derived and state placeholders are left out of the
   comparison), and the stream's current state (its last lifecycle event,
   or its last event when the swimlane has no lifecycle) must be one of
   them. An empty `given` matches only an empty stream. The step's `expect`
   is compared against the matching scenarios: an expected reject the model
   does not make is reported and the walk continues with what the model
   does; an expected emit that contradicts a matching scenario is reported
   and the scenario's events are walked. With no `expect`, a matching reject
   refuses the command, a matching emit emits its events, and a slice that
   emits exactly one event emits it.
4. **Event data.** Each emitted event field gets a value from the command,
   from the step's `answer` when the field is swimlane `state`, from the
   latest earlier event on the event's stream that carries the field (a
   `null` there supplies nothing), or as a placeholder for
   swimlane `state` or a `DerivedFieldAnnotation`. A required event field is
   reported unless the command already reported it missing.
5. **Lifecycle.** Nothing may land on a stream after a terminal lifecycle
   event (one with no transition). An event that is part of its swimlane's
   `transitions` must be a `next` of the stream's previous lifecycle event.
6. **Projection.** Every latest-version ReadModelSlice that sources the event
   writes the row addressed by the read model's `key` (UPSERT copies the
   event's fields that the read model declares; DELETE removes the row).
   Numeric keys are normalized, so `4` and `4.0` address one row, and a
   string key never shares a row with a number. For each
   slice scenario whose `when` events have now occurred for this row (events
   that carry every key value of the row; every event for a keyless read
   model),
   every field its `then` payload names must be set on the row.

A `given` event outside the walked model and named without a version
matches whichever version a slice or scenario pins.

Stream instances come from the swimlane's `stream_id` template. A
command's stream is filled from the command's values. An event on the
command's swimlane lands on that stream. An event on another swimlane lands
on the stream its own template names, filled from the command's values. An
entity without a swimlane is on no stream and shares no history. After all cases, storyboards and command, read-model and
automation slice scenarios in the cases' namespaces that no case reached
are reported as coverage findings. A storyboard counts as walked when a
case exercised any of its slices.

## Output

Findings use the `ValidationIssue` shape in proto JSON, plus where the case
produced them:

| Field | Meaning |
| --- | --- |
| `severity`, `ruleDefaultSeverity` | `SEVERITY_ERROR`, `SEVERITY_WARNING` or `SEVERITY_INFO`. |
| `code`, `ruleTitle` | A code from the table below. |
| `ruleCategory` | Always `walk`. |
| `message` | What happened, naming exact `namespace/slug@version` identities. |
| `subject` | `{kind, id}` of the model entity to change; `id.version` is a decimal string, as in proto JSON. |
| `subjectField` | The field inside the subject, for example `schema.fields[score]`. |
| `case` | `namespace/name` of the case; absent on coverage findings. |
| `step` | 1-based step index; absent for `given` and end-of-case findings. |

The report also carries `cases` (per case: steps, error and warning counts,
`passed`) and `coverage` (storyboards and scenarios walked out of the total).
The MCP tool adds `skipped`, the entities the export path could not render.
`trogon-atlas walk-scenarios` exits `1` when any error is reported, and
`5` for an invalid case file
(see [exit codes](trogon-atlas-exit-codes.md)).

## Findings

| Code | Severity | Title | Trigger |
| --- | --- | --- | --- |
| `WALK_REF_UNRESOLVED` | Error | Case names an entity the model lacks | A step's command, a `via` slice, or an expected read model does not resolve in the walked model. Refs are `slug`, `namespace/slug` or `namespace/slug@version`; a bare slug resolves in the case's namespace and an unpinned ref resolves to the latest version. |
| `WALK_CASE_INVALID` | Error | Case is malformed | A step expects both an emit and a reject; state one of them, or neither to let the model decide. |
| `WALK_VIA_NOT_ISSUER` | Error | `via` does not issue the command | A step's `via` names an AutomationSlice or UiSlice that does not issue the step's command (an automation must emit it; a UI slice must show the command slice's `ui`). |
| `WALK_SCHEMA_OPAQUE` | Info | Schema not inspected | The entity's schema is a type-library message the walker cannot see into, so its fields are not checked; observed and answered values pass through unchecked. |
| `WALK_COMMAND_UNHANDLED` | Error | No command slice handles the command | A step issues a command that no CommandSlice names, so the model says nothing about what the command does. |
| `WALK_COMMAND_FIELD_UNSUPPLIED` | Error | Command field gets no value | At this step no read model observed by the UI or automation that issues the command holds a value for the field, and the step's external answer does not supply it. Project the field into an observed read model, observe a read model that carries it, mark it with a DerivedFieldAnnotation, or document it as an external answer. |
| `WALK_COMMAND_FIELD_CONFLICT` | Error | Command field sources disagree | Two sources (observed read-model rows or the external answer) offer different values for one command field. The model does not say which one wins. |
| `WALK_COMMAND_FIELD_AMBIGUOUS` | Warning | Observed rows disagree on a command field | Several rows of an observed read model match the step and hold different values for the field. Put the read model's key fields in the step's answer to pick the row. Reported as an error when the field is required and nothing else supplies it. |
| `WALK_EVENT_FIELD_UNSOURCED` | Error | Event field gets no value | At this step an emitted event field gets no value from the command, from an earlier event on the same stream, from the swimlane's `state`, or from a DerivedFieldAnnotation. |
| `WALK_EVENT_NOT_EMITTED_BY_SLICE` | Error | Expected event is not one the slice emits | The case expects an event the command's CommandSlice does not list in `emitted_events`. Unless a scenario decides the step, the walk emits nothing, so later steps do not see the event. |
| `WALK_LIFECYCLE_VIOLATION` | Error | Event is not a legal successor | The emitted event is not a `next` of the stream's previous lifecycle event in `Swimlane.transitions`. The step may be legal on its own but not after the steps before it. |
| `WALK_ACCEPTED_AFTER_TERMINAL` | Error | Command accepted after a terminal event | The stream's previous lifecycle event has no successor, yet the command emitted an event because no reject scenario refuses it. Add a reject scenario for this history. |
| `WALK_LIFECYCLE_START` | Warning | Stream starts mid-lifecycle | The first lifecycle event on the stream is only ever reached as a successor of another event. Give the earlier event in the case's `given`, or model the event as a lifecycle start. |
| `WALK_REJECT_UNMODELED` | Error | Expected reject is not modeled | The case expects the command to be refused, but no reject scenario on the command slice matches the stream's history, so the model accepts the command. |
| `WALK_REJECT_REASON_MISMATCH` | Warning | Reject reason differs | A reject scenario refuses the command, but with a different reason code than the case expects. |
| `WALK_UNEXPECTED_REJECT` | Error | Model refuses an expected command | The case expects events, but a reject scenario on the command slice matches the stream's history and refuses the command. |
| `WALK_EMIT_MISMATCH` | Error | Model emits something else | A scenario on the command slice matches the stream's history and decides the step's events, but they differ from the events the case expects. The walk follows the model. |
| `WALK_SCENARIO_MISSING` | Warning | No scenario covers the step | No scenario on the command slice has this command with a `given` matching the stream's history and the step's outcome. The walk continued, but the model does not document this timeline. |
| `WALK_OUTCOME_UNDETERMINED` | Error | Step outcome cannot be decided | The step states no expected outcome, no scenario matches, and the command slice emits more than one event. State `expect.emit` or `expect.reject` on the step. |
| `WALK_STREAM_UNRESOLVED` | Warning | Stream id has an unfilled placeholder | The swimlane's `stream_id` names a placeholder the payload does not carry, so the walk cannot tell stream instances apart. |
| `WALK_PROJECTION_KEY_UNSOURCED` | Error | Projected event lacks a row key | A read-model slice projects an event whose payload does not carry every `key` field of the target read model, so the projection cannot address a row. |
| `WALK_PROJECTION_FIELD_MISSING` | Error | Projected row lacks a scenario field | A read-model slice scenario says the row holds this field after these events, but the walked projection left it unset. |
| `WALK_READ_MODEL_MISMATCH` | Error | Read model differs from the case | After the step (or at the end of the case) the read model's rows differ from the rows the case expects. |
| `WALK_STORYBOARD_UNWALKED` | Info | Storyboard not walked | No case exercised any slice of this storyboard. |
| `WALK_SCENARIO_UNWALKED` | Info | Scenario not walked | No case step matched this slice scenario. |

## Limits

- Payload values are compared as JSON; placeholders such as
  `<derived:timestamp>` and `<state:status>` only equal themselves.
- A schema held as a type library message is opaque: its fields are not
  checked, and the command's values pass through unchanged.
- The walk does not fire automations on its own. Each command is a step.
- An expected `rows` list holds at most 256 rows per read model.
- Slice scenario payloads are example data with their own identifiers, so
  the projection check compares which fields are set, not their values.
  Put the values to check in the case's expected rows.
