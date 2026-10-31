# Finding evidence

A [`ValidationIssue`](../../proto/trogonatlas/api/eventmodel/v1alpha1/validation.proto)
can carry `evidence`: typed causes that name the exact entities, fields and
storyboard positions behind the finding. Clients read the causes from
`evidence` instead of parsing `message`. The message stays as the human
explanation.

## Evidence item

| Field | Meaning |
| --- | --- |
| `role` | What the cause contributes to the finding. See [roles](#roles). |
| `entity` | Entity holding the cause, at the exact version the validator read. |
| `field` | Structural path inside `entity`, with list indexes (`emitted_events[2]`). Empty when the entity itself is the cause. |
| `target` | What `entity.field` references, at the exact pinned version. |
| `occurrence` | Storyboard id and zero-based index into `Storyboard.slices`. Set by storyboard-order rules. |
| `detail` | Human note. Presentation only. |

## Roles

| Role | Cause |
| --- | --- |
| `ROLE_ISSUER` | `entity.field` wires `target`, a UI or processor, as an issuer of the subject command. |
| `ROLE_EMITTER` | `entity.field` names `target` as a command emitting the subject event. |
| `ROLE_STALE_PIN` | `entity.field` pins `target`, an older version than the latest loaded. |
| `ROLE_LATEST_VERSION` | `entity` is the latest loaded version of a stale pin's target. |
| `ROLE_CONSUMER` | `entity.field` consumes event `target` at `occurrence`. |
| `ROLE_LATER_EMITTER` | `entity.field` emits `target`, but only after the consuming occurrence. |
| `ROLE_EARLIER_EMITTER` | `entity.field` emits `target` at the nearest occurrence before the consumer. |
| `ROLE_INTERVENING_ACTION` | `entity` is an action slice placed between the emitter and the projection. |

## Rules that carry evidence

`ListValidationRules` reports the roles each rule's findings carry in
`ValidationRule.evidence_roles`. An empty list means the rule explains its
causes only in `message`; every rule not listed below is in that state.

| Rule | Roles |
| --- | --- |
| `COMMAND_MULTIPLE_ISSUERS` | One `ROLE_ISSUER` per wiring slice, so a slice repeated across versions and an issuer wired by several slices each appear. |
| `EVENT_MULTIPLE_EMITTERS` | One `ROLE_EMITTER` per emitting command slice, with the exact command version it names. |
| `SLICE_STALE_REF` | `ROLE_STALE_PIN` for the pinned reference and `ROLE_LATEST_VERSION` for the newer version. |
| `PROJECTION_BEFORE_EMITTER` | `ROLE_CONSUMER` and one `ROLE_LATER_EMITTER` per emitting occurrence after it. No emitter evidence when the storyboard never emits the event. |
| `PROJECTION_NOT_ADJACENT` | `ROLE_CONSUMER`, `ROLE_EARLIER_EMITTER` and one `ROLE_INTERVENING_ACTION` per action slice in between. |

A slice placed more than once in a storyboard yields one finding per
occurrence. Those findings share code, subject and message and differ only
in the `occurrence` of their evidence.

## Ordering and identity

The server sorts evidence by role, then entity, field, target and
occurrence, comparing references by kind, namespace, slug and version and
occurrences by storyboard then position. Exact duplicates are removed. The
same causes therefore always produce the same list, whatever order the
store returns entities in.

A finding's identity is its code, subject (with version), `subject_field`
and evidence without `detail`. Server-side finding comparison, such as
attributing strategic findings to a change, already uses that identity.

## Visibility

Validation reads the caller's visible snapshot. Evidence names only
entities in that snapshot or references those entities carry, which the
caller can already read through the subject and message.

## Transports

| Transport | Shape |
| --- | --- |
| gRPC | `ValidationIssue.evidence` |
| MCP `validate_event_model` / `validate_project` with `json: true` | Proto JSON: `evidence[].role` is the enum name, references are `EntityRef` objects. |
| MCP mutation responses and `trogon-atlas --format json` | `evidence[]` of `{role, entity, field, target, occurrence: {storyboard, position}, detail}`; `role` is the lowercase name without `ROLE_`, references are canonical id strings, absent parts are `null`. |
| `trogon-atlas` text output | One indented line per cause under the finding. |
