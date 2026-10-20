# How to start an event model from scratch

Build the smallest event model that validates with no errors or warnings:
one human-issued command, one event, and one read model, tied together by
one storyboard. The
[`acme-wishlist`](../../examples/manifests/acme-wishlist/) namespace is
the finished result; each step below names the file it adds.

Create the entities in the order below. Each entity refers only to
entities from earlier steps, so every check runs against a complete set.

## Check each step

Keep the files for one namespace in one directory. After each step, check
everything written so far in one batch:

```sh
trogon-atlas apply -f acme-wishlist/ --dry-run
```

`--dry-run` runs request-time validation only. It checks each manifest,
and it rejects a slice whose scenario refers to an entity that is not in
the store or in the same batch (`SCENARIO_DANGLING_REF`). It does not run
model validation, so the last step checks the whole model.

## 1. Describe the problem space

| File | Kind | Fields the rules check |
| --- | --- | --- |
| `project.yaml` | `Project` | slug equal to the namespace |
| `domain.yaml` | `Domain` | `project` |
| `subdomain.yaml` | `Subdomain` | `domain`, `classification` |
| `bounded-context.yaml` | `BoundedContext` | slug equal to the namespace, `realizes` |

- A Project's slug must equal its namespace (`PROJECT_SLUG_MISMATCH`,
  Error).
- A Domain without `project` warns (`DOMAIN_MISSING_PROJECT`). The server
  rejects a Domain whose `project` is in another namespace.
- The rule catalog requires a Subdomain to name its Domain in the same
  namespace (`SUBDOMAIN_MISSING_DOMAIN`, `SUBDOMAIN_FOREIGN_DOMAIN`) and to
  carry a `classification` (`SUBDOMAIN_UNCLASSIFIED`).
- A namespace is a bounded context, and the BoundedContext slug equals the
  namespace (`CONTEXT_SLUG_MISMATCH`).
- A BoundedContext that realizes no Subdomain is reported
  (`BC_REALIZES_NOTHING`, Info).

`validate_project` with a `project_id` scope finds an EventModel only
through this chain: the model's namespace, then the BoundedContext in that
namespace, its first `realizes` Subdomain, that Subdomain's Domain, and the
Domain's `project`. Without the chain, the project report is empty.

## 2. Add the stream

| File | Kind | Fields the rules check |
| --- | --- | --- |
| `swimlane.yaml` | `Swimlane` | `streamId` |

Each `{placeholder}` in `streamId` must be a plain name
(`STREAM_ID_MALFORMED_PLACEHOLDER`, Error) that matches a field on this
lane's events or commands (`STREAM_ID_UNRESOLVED_PLACEHOLDER`, Warning).
`wishlist-{shopper_id}` uses the `shopper_id` field that the next step
adds.

## 3. Add the event and the command

| File | Kind | Fields the rules check |
| --- | --- | --- |
| `event.yaml` | `Event` | `swimlane`, `schema` |
| `command.yaml` | `Command` | `swimlane`, `schema` |

- Every event needs a `swimlane` that exists (`EVENT_NO_SWIMLANE`, Error).
- Declare fields in `schema`, an Any with `'@type':
  type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Schema`
  (`SCHEMA_TYPE_URL_EMPTY`, Error).
- Every command field must appear on an emitted event or in the lane's
  `state` (`COMMAND_FIELD_NEVER_RECORDED`, Warning).
- A command slice's emitted events must sit on the command's swimlane
  (`COMMAND_MULTI_STREAM_WRITE`, Error).

## 4. Add who issues the command

| File | Kind | Fields the rules check |
| --- | --- | --- |
| `persona.yaml` | `Persona` | none |
| `ui.yaml` | `Ui` | none |

Every command needs a trigger: a CommandSlice with both `persona` and `ui`,
or an AutomationSlice that fires it (`COMMAND_HAS_NO_TRIGGER`, Error). A
human trigger is smaller, because an automation also needs a Processor and
an AutomationSlice. Every UI needs a persona beside it on a slice or a
storyboard entry (`UI_NO_PERSONA`, Error).

## 5. Add the command slice

| File | Kind | Fields the rules check |
| --- | --- | --- |
| `command-slice.yaml` | `CommandSlice` | `command`, `emittedEvents`, `persona`, `ui`, `scenarios` |

- A command without a slice that emits events warns
  (`COMMAND_NO_EMITTED_EVENTS`).
- Every field on an emitted event must come from the command, the lane, or
  a `DerivedFieldAnnotation` (`EVENT_FIELD_UNSOURCED`, Error). The command
  marks `added_at` as derived, since the system sets it.
- A slice with no scenario is reported (`SLICE_NO_SCENARIOS`, Info). Each
  scenario needs an outcome (`SCENARIO_MISSING_THEN`, Error).

## 6. Add the read model and its slice

| File | Kind | Fields the rules check |
| --- | --- | --- |
| `read-model.yaml` | `ReadModel` | `sourceEvents`, `schema`, `key` |
| `read-model-slice.yaml` | `ReadModelSlice` | `readModel`, `sourceEvents`, `projectionRole` |

- Every event must be sourced by a read model (`EVENT_UNOBSERVED`,
  Warning).
- Every read model field must come from a source event or its lane's
  `state` (`READ_MODEL_FIELD_UNSOURCED`, Error). The read model must also
  carry every field of a projected event
  (`RM_SOURCE_EVENT_FIELD_NOT_PROJECTED`, Error).
- Every `key` name must be a field (`RM_KEY_NOT_A_FIELD`, Error), and
  every projected event must carry all key fields
  (`RM_SLICE_KEY_UNSOURCED`, Error).
- A slice may only project events its read model declares
  (`RM_SLICE_EVENT_NOT_DECLARED`, Error). Each declared event needs a slice
  in a storyboard that projects it (`RM_EVENT_NOT_PROJECTED`, Error).
- Add a read model only for a different shape: other fields, key or
  source events. Screens and automations that read the same data share
  one read model.
- Read models in one namespace with the same key, fields and source
  events are a warning (`READ_MODEL_DUPLICATE_SHAPE`): merge them.
- When several automation slices observe one read model, name them in a
  `SharedConsumersAnnotation` (`READ_MODEL_MULTIPLE_AUTOMATIONS` is Info).
  See `examples/manifests/acme-customer-data-deletion/read-model.yaml`.
- A command's decision state comes from its stream (the given events).
  Do not add a read model for it.

## 7. Add the storyboard

| File | Kind | Fields the rules check |
| --- | --- | --- |
| `storyboard.yaml` | `Storyboard` | `entry`, `outcome`, `slices` |

- Declare the `entry`: the read model observed, and who observes it
  (`human` with persona and UI, or `automation` with a processor)
  (`STORYBOARD_MISSING_ENTRY`, Warning).
- Declare the terminal events in `outcome` (`STORYBOARD_MISSING_OUTCOME`,
  Warning).
- Put every slice in a storyboard (`SLICE_NOT_IN_STORYBOARD`, Warning).
  Place each projection after the slice that emits its event and before
  the next command or automation slice (`PROJECTION_BEFORE_EMITTER`,
  `PROJECTION_NOT_ADJACENT`, Errors).

## 8. Add the event model

| File | Kind | Fields the rules check |
| --- | --- | --- |
| `event-model.yaml` | `EventModel` | `members` |

The validator only checks members of the EventModel. List every entity
from steps 1 to 7 except the Project, with its `kind`. Every member must
exist (`MEMBER_NOT_FOUND`, Error), and every slice in a storyboard must be
a member (`STORYBOARD_SLICE_NOT_MEMBER`, Warning). Every event, command,
read model, persona, and UI must be wired to a slice or the storyboard
(`ORPHAN_NOT_FLAGGED`, Warning).

## Apply and validate

Apply the namespace:

```sh
trogon-atlas apply -f acme-wishlist/
```

Run model validation through the `validate_event_model` MCP tool:

```json
{ "namespace": "acme-wishlist", "slug": "acme-wishlist-model", "version": 1 }
```

Then run `validate_project` with `project_id` set to the Project. The
finished model reports no errors or warnings, and two Info findings:
`CROSS_MODEL_REF` for the Domain's reference to the Project, which is not a
member, and `SLICE_NO_SCENARIOS` for the read model slice.

As the model grows, run `validate_event_model` after every batch and fix
all errors. To find what a code means, call `list_validation_rules`; to
repair one, follow
[repair a validation finding](repair-validation-findings.md).
