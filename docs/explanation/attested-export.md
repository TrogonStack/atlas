# Attested export: one state, one digest

`ExportAttestedModel` returns an event model's manifests, its validation
findings and coverage, and the identity of the state both came from, as one
bundle bound by one digest. `trogon-atlas verify` checks that digest without
contacting a server. The bundle format is pinned in the
[attested bundle format reference](../reference/attested-bundle-format.md),
and the commands are in
[export and verify an attested model](../how-to/export-and-verify-an-attested-model.md).

## The problem it solves

A review or a release gate wants to say "this model, checked by these rules,
had these findings". Before this, that statement took several reads:
`ExportEventModelManifests` for the manifests, `ValidateEventModel` for the
findings, `GetSnapshotId` for the state. Each read saw whatever the store
held at its own moment. A write that landed between two of them produced a
record whose findings described a model its manifests did not contain, and
nothing in the record could show it.

A saved validation report was also only prose. Nothing tied the findings to
the manifests, so editing either after the fact left no trace.

## One state per bundle

The server captures everything in a bundle from one state, and refuses
rather than mixing two:

- **Mutations are held off for the whole capture.** Every write this server
  performs runs under its mutation lock, and the export holds that lock from
  the first read to the last. The manifests, the findings, the snapshot id
  and the changeset boundary are therefore all read with no write between
  them.
- **Only the writer exports.** A reader or standby replica's snapshot cache
  is not invalidated by the writer's commits, so holding its own lock pins
  nothing. Default chosen for review: a replica refuses with `NOT_WRITER`
  rather than attest a state it cannot pin. The writer lease is checked
  before and after the capture.
- **What the lock does not cover is compared instead.** Deleting a branch
  and moving a namespace do not take the mutation lock. A branch export
  reads the branch's identity before and after the capture, and every
  export reads the caller's namespace visibility twice. A difference
  refuses the export with `UNAVAILABLE`, which means retry.

## Current and historical bundles

A bundle states whether it describes the current state or a past one:

- **Current** names the newest baseline changeset, and on a branch also the
  newest branch changeset, that the captured state includes.
- **Historical** reads baseline as it stood right after
  `as_of_changeset` landed, rewound the same way `GetEventModelProjection`
  rewinds it, and names that changeset as its boundary. It reads baseline
  history only, so a branch header is refused.

Manifests render through the type libraries as they are now. Default
chosen for review: a historical export refuses when a type library the
caller can see changed after the requested changeset, because old entities
rendered through new types would describe a state that never existed.

## Incomplete is never clean

The server caps how many entities a read loads, and the board walk and the
rewind have their own bounds. An export that hit any of them did not see its
whole scope, and an empty finding list over part of a model is not a pass.

Default chosen for review: such an export is refused with
`EXPORT_INCOMPLETE`. A caller who still wants the record passes
`allow_incomplete` and gets a bundle whose completeness is `INCOMPLETE`,
with every reason listed. Verification checks that a bundle claiming to be
complete agrees with its own validation coverage and lists no skipped
entities, so relabelling an incomplete bundle as complete fails even when
the digest is recomputed. The CLI exits 11 for an intact incomplete bundle,
the same code an incomplete `validate` run gets.

## What the digest covers

The digest covers every field of the bundle except the digest itself. It
reuses the content hash rules from
[snapshot identity](snapshot-identity.md): proto3 JSON, re-serialized with
sorted keys and pinned number formatting, then SHA-256 behind a domain tag
of its own. The tag carries the format version, so a later format cannot
produce a digest an older verifier accepts by accident.

The entries inside the bundle are the same per-entity content hashes
`GetSnapshotId` returns, and the bundle's closure id is the snapshot id over
exactly those entries. A verifier recomputes it, so an entry cannot be
replaced and the digest resealed without the closure id disagreeing.

The rule catalogue is identified by a digest over the
`ListValidationRules` response, plus the server version that produced the
bundle. Two bundles validated by different catalogues therefore differ even
when every finding matches.

## Integrity, not authenticity

A digest that verifies proves that the bundle is unchanged since something
sealed it under this format. It does not prove who sealed it. Anyone can
build a bundle, compute its digest and hand it over, and `verify` accepts it.

Treat a bundle as trustworthy only as far as the channel it arrived
through. Storing the digest somewhere the producer cannot rewrite, such as a
release record, a signed commit or a CI log, turns a later `verify` into a
check that the bundle is the one that was recorded.

Signing is future work: a signature over the digest would let a verifier
check which server produced a bundle without trusting the channel. The
format leaves room for it in a later version, and an unknown field is
refused today rather than ignored, so a signature cannot be added to a
bundle without the verifier noticing.

## What this is not

- **Not a backup.** A bundle holds rendered manifests for one model's
  board, not the store.
- **Not proof the model is correct.** It proves which findings a given
  catalogue produced for a given state. Findings with Error severity are
  still in the bundle and still errors.
- **Not available through MCP yet.** The RPC and the CLI are the supported
  surfaces.
