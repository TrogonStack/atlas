# Repair a validation finding

The live store on the branch you were given is authoritative; a pasted
snapshot is only a starting point. Keep one branch for the whole repair.

Fetch the EventModel with get_entity_json and run validate_event_model with
`json: true`. Save the full response before editing. Match the finding by
code, exact subject, field and storyboard context; if it is gone, report that
instead of replaying an old repair. Call list_validation_rules with
`json: true` when the rule's meaning is unclear. To survey a namespace, run
validate_project with that `namespace`.

Discover the dependency context before choosing a change: get_outgoing_references
for what the subject points at, get_incoming_references for consumers,
emitters, containing storyboards and memberships, and get_impact for the
neighborhood of a proposed change. These return the first page only: decode
the response and follow a nonempty `nextPageToken` before treating it as
complete. Fetch each definition at its exact pinned version. For temporal
findings, fetch every containing storyboard with its complete ordered slice
list and keep repeated occurrences.

Choose a behavior-preserving repair. Write down the intended behavior, the
causal chain and the evidence, and which contracts and scenarios must stay
unchanged. If the domain intent is ambiguous, record the missing fact instead
of inventing behavior. Removing a member is not a general fix. When the model
states a fact on purpose and the rule contradicts it, the validator is wrong,
not the model.

For strategic findings, repair the shape of the problem space: put the
Project, the Domain and every Subdomain in one problem-space namespace, move
each `realizes` with retarget_references (preview with `dry_run: true`), then
remove the old entities with delete_entity in its default
`fail_if_referenced` mode. Never silence DOMAIN_MISSING_PROJECT with a Project
and a Domain in each context namespace.

Preview related edits with batch_mutate_json and `validateOnly: true`, with
`ifMatch` set to the current etag on every existing-entity put. Require
`STATUS_VALIDATED`, re-read the definitions that justify the repair, then apply
the same operations with `validateOnly: false` and require `STATUS_APPLIED`.
On a stale etag, refresh and reconsider; never drop the guard. On an uncertain
outcome, re-read live state before retrying.

Verify by rerunning validate_event_model for the affected and dependent
models. Compare findings by code, severity, subject, field and message, and
keep both the added and the removed ones. Re-fetch written entities to confirm
what was stored.
