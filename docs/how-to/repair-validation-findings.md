# Repair a validation finding and retain the workflow

Use this procedure when Studio's **Copy fix prompt** hands a finding to an
agent. The pasted snapshot is a starting point; the live store in the
specified branch is authoritative. Apply model changes through Atlas
CLI/MCP and use Studio to verify the resulting view.

This is a maintained procedure, not an automatic repair command. Check the
[repair patterns](../explanation/validation-repair-patterns.md) for applicable
precedents before choosing a domain change.

## Establish the current scope

Record the endpoint, branch, EventModel identity including version, Studio
scope path, finding code, subject identity, and subject field. Keep tokens in
the process environment, never in the record or command arguments.

Point the MCP host at the server's `/mcp` endpoint, for example
`http://127.0.0.1:50069/mcp` on the development stack, and have it send your
key as `Authorization: Bearer <token>`.

The endpoint scopes each request's tool calls through the
`x-trogon-atlas-branch` header. For baseline, omit the header. For a named
branch, have the MCP host send that name on every request. Do not change
branches halfway through a repair.

Fetch the EventModel with `get_entity_json`, then run `validate_event_model`.
Both use explicit `namespace`, `slug`, and numeric `version` arguments;
`get_entity_json` also takes `kind: "event_model"`. Request `json: true` for
validation. Save the full validation response before editing, not just the
error count. Match the current finding by code, exact subject, field, and
storyboard context. If it is gone, report that instead of replaying an old
repair.

Use `list_validation_rules` with `json: true` when the rule's meaning needs
clarification. A new rule or unclear result can require inspection of its
validator implementation; the catalog and tool contracts are the starting
point.

To survey a whole namespace instead, run `validate_project` with `namespace`
set to it and `json: true`: every EventModel in the namespace is validated in
one call. Filter its findings on a code to list what needs work, for example
`SLICE_NO_SCENARIOS` (Info) for the slices that still lack a given/when/then
scenario.

## Discover the full dependency context

Start from the finding's `evidence` when it has any: each item names an
exact entity, field, pinned target and, for storyboard-order rules, the
storyboard position. See [finding evidence](../reference/finding-evidence.md)
for the roles and which rules carry them. Evidence names the direct causes,
not the full dependency context.

Use existing model tools before searching exports manually:

| Tool | Use |
| --- | --- |
| `get_entity_json` | Fetch an exact definition and retain its opaque `etag`. |
| `get_outgoing_references` | Find definitions referenced by the subject or a related slice. |
| `get_incoming_references` | Find consumers, emitting slices, containing storyboards, and model memberships. |
| `get_impact` | Find related entities around a proposed change by traversing incoming and outgoing references. |

These tools take flat identity arguments, for example:

```json
{
  "kind": "event",
  "namespace": "ecommerce",
  "slug": "order.fulfilled",
  "version": 1
}
```

For `get_impact`, optionally add `max_depth`. Zero uses the server default,
currently capped at 16 hops; it does not prove an unlimited closure. Avoid
kind filters until the necessary dependency paths are understood. Impact
results are inspection candidates, not exclusively downstream dependents.
Inspect reference directions to distinguish an emitter from a consumer, then fetch each
needed definition with its exact referenced version. Do not substitute the
latest version for a pinned version.

For temporal findings, fetch every containing storyboard and its complete
ordered slice list. Preserve repeated occurrences. Fetch those slices,
source and emitted events, entry views and observers, relevant swimlanes,
commands, scenarios, and upstream emitters. Inspect the upstream storyboard
when the source crosses a context boundary. A missing item in a copied
snapshot does not establish that the item is missing from the model.

The reference and impact MCP tools currently return only the first gRPC
page: they do not expose page-token arguments. Decode the response and check
`nextPageToken` (wire field `next_page_token`). If nonempty, obtain the
remaining pages through the corresponding gRPC operation before treating
the result as complete. Exact entity reads and paginated namespace exports
can also supply missing definitions. Record any unresolved depth or paging
limit; incomplete context is not proof that no dependency exists.

## Choose a behavior-preserving repair

Write down the intended behavior, the current causal chain, and the evidence
supporting the proposed change. State which contracts, scenarios, and
entities must remain unchanged. If the domain intent is ambiguous, record
the specific missing fact instead of inventing behavior.

For ordering findings, distinguish a local projection that must follow a
local emitter from an upstream subscription that already populated the
storyboard's declared entry view. The
[subscription entry example](../explanation/validation-repair-patterns.md#upstream-subscription-at-a-local-storyboard-entry)
shows the evidence required for that distinction. Removing a member is not
a general solution to a validation error.

For strategic findings such as `DOMAIN_IN_CONTEXT_NAMESPACE` and
`REALIZES_MIXED_CLASSIFICATIONS`, repair the shape of the problem space,
not the individual entity. The
[problem space pattern](../explanation/validation-repair-patterns.md#one-problem-space-contexts-realize-across-namespaces)
records the evidence and the decision. Never silence `DOMAIN_MISSING_PROJECT`
with a Project and a Domain in each context namespace.

To move a problem space that was declared inside context namespaces:

1. Put the Project, the Domain and every Subdomain in one problem-space
   namespace, with the Project and Domain slugs equal to that namespace.
   Copy each Subdomain's classification and doc unchanged.
2. Run `retarget_references` from each old Subdomain to its replacement,
   with `filter_kinds` set to `bounded_context`, so every `realizes`
   points across namespaces. Preview with `dry_run: true` first.
3. Delete the old Subdomains, Domains and Projects with `delete_entity` in
   its default `fail_if_referenced` mode. A refusal names a referrer that
   still needs retargeting.
4. If a context still realizes Subdomains of different classifications,
   split it along the classification line. Reclassifying a Subdomain to
   match the other half misstates where the business invests.
5. Run `validate_project` with the problem-space `namespace`, then with the
   Project scope. Strategic findings appear once, on the store-level report.

Sometimes the model is right and the rule is wrong. Check the rule's
description against the definitions you fetched. If the finding
contradicts a fact that the model states on purpose, such as an external
source that supplies a read model's fields, change the validator, not the
model. Do not invent a source event, drop a field, or remove the fact only
to pass the rule. Record the case in the
[repair patterns](../explanation/validation-repair-patterns.md) so the next
reviewer can tell a modeling gap from a rule gap.

Limit the proposed edits to the affected definitions. Preserve their
unrelated fields and use the current `etag` for every existing entity put.
If a contract needs a new version, follow the
[versioning procedure](draft-then-land.md) and inspect affected referrers.

## Preview and apply the guarded change

For a related set of edits, use `batch_mutate_json`. The outer MCP argument
is `request_json`, a proto3 JSON `BatchMutateRequest` object (a JSON-encoded
string is also accepted). Inside that request, use `validateOnly: true` for the
preview and `ifMatch` on each existing-entity put. Each `entity` is an
`Entity` wrapper, such as `{ "storyboard": <complete revised definition> }`.
Use `createOnly: true` for an intentional new entity.

Save the exact request and decoded response. Require `STATUS_VALIDATED`
and inspect per-operation findings before applying. This preview checks
batch admissibility; it does **not** replace full EventModel validation.
When full candidate validation is needed before touching the target branch,
use an appropriate isolated branch and validate that scope.

Re-read the definitions used to justify the repair before applying. If a
write target or a relevant dependency changed, refresh the evidence and
preview again. `ifMatch` protects written entities from stale replacement;
it does not lock the whole dependency graph.

Apply the same proposed operations with `validateOnly: false`. Require
`STATUS_APPLIED` and save its returned etags. On a stale-etag failure,
refresh and reconsider instead of removing the guard. On a timeout or an
uncertain mutation outcome, reread live state and changesets before
retrying. Batch operations use best-effort rollback on storage failures;
do not infer that an unsuccessful response always means nothing changed.

### Decode the actual tool response

The MCP result contains text blocks holding an envelope. Check transport
errors and `isError` before reading that envelope.

| Operation | Envelope |
| --- | --- |
| `get_entity_json` | `{ "etag": "...", "json": { ... } }` with an object-valued entity. |
| `validate_event_model` with `json: true` | `{ "json": "..." }` with a JSON-encoded response string. |
| Reference tools, `get_impact` | `{ "binpb_base64": "..." }` with the protobuf response. |
| `put_entity`, `put_entity_json`, `delete_entity`, `batch_mutate`, `batch_mutate_json`, `merge_branch`, `apply_manifests` | The `scope`, `branch` and `binpb_base64` envelope plus an `issues` array. Use `scope` and `branch` to confirm which store took the write. Each issue has `severity`, `code`, `subject`, `field` and `message`; batch and apply issues add `op_index` and `op_entity`, for delete ops as well as puts. Strategic findings arrive here without blocking the write, and a `delete_entity` `dry_run` previews them. |

The JSON response helper can fall back to `binpb_base64` if reflection
rendering fails. Decode binary responses using the matching RPC response
descriptor from the
[canonical schema](../../proto/trogonatlas/api/eventmodel/v1alpha1/service.proto), rather
than guessing success from the presence of an envelope. For example, with
`@grpc/proto-loader`, the service method's `responseDeserialize` decodes
`BatchMutateResponse`; do not assume a message descriptor has a
`deserialize` method.

## Verify the result and preserve source fixtures

Rerun `validate_event_model` for the affected model and relevant upstream or
dependent models. Compare full findings by code, severity, exact subject,
field, and message. Retain added and removed findings, not only totals.
Check that the requested finding is resolved, inspect every new finding,
and verify the definitions designated as unchanged. Re-fetch written
entities to confirm the intended content was stored. Existing unrelated
findings do not become resolved merely because the target was repaired.

If model membership changed, explain which findings left the validation
scope. A source schema warning disappearing from a downstream curation is
not an upstream schema fix. Validation success alone does not demonstrate
that behavior and contracts were preserved; retain those checks separately.

Verify Studio when the repair changes the displayed model. This supplements
the CLI/MCP evidence and does not replace it.

For a fixture-backed model, update only the corresponding seed definitions
under `examples/manifests/`. Compare live definitions first: a full
manifest replay can overwrite unrelated live changes. Preview the selected
seed manifests with `trogon-atlas apply --dry-run` in the same branch and endpoint.
That check verifies manifest admissibility, not full scoped validation.

When the seed itself is the thing to repair, edit the YAML, run
`trogon-atlas fmt -w` on the touched files, and run
`cargo test -p trogon-atlas-server --test example_manifests_validate`. That
test applies the `acme` problem space and every example namespace to an
ephemeral store and lists each warning or error by code and subject, so it
doubles as the before and after record. Finish with
`mise run manifests:verify -- bootstrap`.

## Keep the result available to the next session

Before cleanup or handoff, save a descriptive run record in a directory
Git ignores, at the git root returned by `git rev-parse --show-toplevel`.
Name it with a date and the repair name, and add a numeric suffix if that
path already exists. Reuse a record only for an explicit resume. Run
records stay private; the procedure and generalizable lessons belong in the
checked-in documents.

Include the following in the record:

- Scope, branch, exact identities, original finding, and execution status.
- Fetched definitions and etags, full ordered storyboards, reference paths,
  and any missing or truncated context.
- Repair rationale, rejected alternatives, preserved contracts, and the
  exact proposed operations.
- Commands and any temporary script source in fenced blocks, with input and
  output captures. Preserve the source itself, not just its temporary path;
  include checksums to verify an archived copy.
- Dry-run and apply results, before/after findings, unchanged-definition
  checks, remaining findings, Studio evidence, and any seed commit.
- How to resume safely, known tool limitations, and which steps could become
  deterministic automation. Mark a case-specific script as historical when
  its prerequisites or captured etags no longer hold.

Update the [pattern document](../explanation/validation-repair-patterns.md)
when a repair establishes a reusable decision rule. Add or amend the
procedure here when tool use improves. Keep private payloads and credentials
out of checked-in examples. Memory and AFTERTASK entries may point to these
artifacts; they are not substitutes for them.

## Grow the workflow into tools

Start with an executable procedure in this guide. When a step repeats,
extract that deterministic operation into a maintained tool and link it
here. Context collection, exact-version fetching, decoding, guarded request
construction, evidence capture, and validation comparison are candidates.
Keep domain interpretation explicit until a repair's applicability can be
checked mechanically.

An automated step must declare its inputs and outputs, validate them with
Zod for TypeScript commands, retain branch and version scope, handle partial
context and stale reads, and have meaningful regression checks. Preserve a
worked example that exercises it. Replace the corresponding manual step
with its command once implemented; do not add a competing undocumented
script or claim automation exists while only this procedure exists.
