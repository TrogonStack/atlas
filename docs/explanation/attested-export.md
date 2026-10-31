# Attested export: one state, one digest

`ExportAttestedModel` returns an event model's manifests, its validation
findings and coverage, and the identity of the state both came from, as one
bundle bound by one digest. `trogon-atlas verify` checks that digest without
contacting a server. A server configured with a signing key also signs the
digest, so a verifier holding the matching public key can check who sealed
the bundle. The bundle format is pinned in the
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

## One snapshot per bundle

The server reads everything in a bundle from one snapshot and never holds
writes off to do it. Writers keep committing while an export runs. What
moved underneath, or what the snapshot could not resolve, is written into
the bundle as a marker the digest covers, so a reader can act on it. This
follows [layered information](principles/layered-information.md): a
partial answer says so in a field, not in silence.

- **One read, one state.** The manifests, the findings and the snapshot id
  all come from the same cached snapshot. The bundle names that snapshot by
  its id and by the newest changeset recorded before it was read.
- **Every exported entity is confirmed against the store.** After reading
  the snapshot, the export fetches each entity it carries and compares its
  etag. An entity that changed or was deleted since is listed in
  `unresolved` as changed or deleted, and the bundle still carries the
  snapshot's copy, never the live one. A complete bundle's entries were
  all unchanged from the moment the snapshot was read until they were
  confirmed, so together they are the store's state at that moment.
- **What the snapshot lacks is listed, not skipped.** A reference to an
  entity the snapshot does not hold is listed as missing, or as hidden when
  the caller's namespace visibility excludes it. A model the snapshot does
  not hold is read directly and listed as outside the snapshot.
- **The boundary is compared, not locked.** The export reads the newest
  baseline changeset, and on a branch the branch's identity, before and
  after. A newer changeset that touched anything the export read or looked
  for, a moved branch, a deleted branch or changed namespace visibility
  marks the bundle with `REASON_CONCURRENT_WRITE`. A baseline write to a
  key the branch holds its own copy of does not change the branch's view,
  so it does not mark a branch export. Default chosen for
  review: a concurrent write marks the bundle instead of failing the export
  with a retry, so an export never loses to a busy writer.
- **A replica says what it cannot confirm.** A reader or standby replica's
  snapshot cache is not refreshed by the writer's commits, so its snapshot
  may be older than the boundary it names. Default chosen for review: a
  replica exports, and marks the bundle with
  `REASON_BOUNDARY_UNVERIFIED`, rather than refusing with `NOT_WRITER`.

A concurrent write therefore yields either a bundle bound to the snapshot
read before the write, or a bundle that says which entities moved. It never
yields a bundle that mixes the two without saying so.

## Current and historical bundles

A bundle states whether it describes the current state or a past one:

- **Current** names the newest baseline changeset, and on a branch also the
  newest branch changeset, recorded before its snapshot was read.
- **Historical** reads baseline as it stood right after
  `as_of_changeset` landed, rewound the same way `GetEventModelProjection`
  rewinds it, and names that changeset as its boundary. It reads baseline
  history only, so a branch header is refused. The rewind already undoes
  every newer changeset, so a historical export does not confirm entities
  against the live store, and live writes after its changeset do not mark
  it concurrent.

Manifests render through the type libraries as they are now. Default
chosen for review: a historical export whose type libraries changed after
the requested changeset is marked with `REASON_TYPE_LIBRARIES_CHANGED`,
because old entities rendered through new types may describe a state that
never existed.

## Incomplete is never clean

The server caps how many entities a read loads, and the board walk and the
rewind have their own bounds. An export that hit any of them did not see its
whole scope, and an empty finding list over part of a model is not a pass.
The same holds for a bundle that lists unresolved entities or a concurrent
write.

Default chosen for review: such an export is refused with
`EXPORT_INCOMPLETE`. A caller who still wants the record passes
`allow_incomplete` and gets a bundle whose completeness is `INCOMPLETE`,
with every reason listed. Verification checks that a bundle claiming to be
complete agrees with its own validation coverage, records no truncated
closure and lists no skipped or unresolved entities, so relabelling a
truncated, partially validated or unresolved bundle as complete fails even
when the digest is recomputed. A reason with no other trace in the bundle,
such as a concurrent write that touched only the changeset log, is carried
by `completeness` alone, so only a signature could stop it being dropped. The CLI exits 11 for an intact
incomplete bundle, the same code an incomplete `validate` run gets, and
prints its reasons and unresolved entities.

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
sealed it under its format. It does not prove who sealed it. Anyone can
build a bundle, compute its digest and hand it over, and `verify` accepts it.
An unsigned bundle is reported as integrity only, issuer unauthenticated.

Treat an unsigned bundle as trustworthy only as far as the channel it
arrived through. Storing the digest somewhere the producer cannot rewrite,
such as a release record, a signed commit or a CI log, turns a later
`verify` into a check that the bundle is the one that was recorded.

## Signed bundles move the trust boundary to a key

A signed bundle carries an Ed25519 signature over its digest and the id of
the key that made it. The signature moves trust from the channel to the
key: a verifier who holds the server's public key, obtained through a
channel it already trusts, can check that whoever sealed the bundle held
the private key, however the bundle reached it.

What a signature proves stops there:

- **Who held the key, not that the server was honest.** A signature says
  the bundle was sealed with that key. It does not make the findings
  correct or the state complete, and anyone who obtains the private key
  can sign anything.
- **Only against keys the verifier chose.** The key id inside a bundle
  names a key; it does not vouch for it. `verify` checks a signature only
  against public keys passed with `--trusted-key`. A signed bundle checked
  without them is reported as signature present, not checked, and must not
  be described as authenticated.
- **Never a better completeness.** The signature covers the digest, and the
  digest covers the completeness state and reasons. A signed incomplete
  bundle is still incomplete, and relabelling it complete breaks the
  digest and therefore the signature.

The format records whether a bundle is signed. The unsigned format refuses
a signature and the signed format requires one, and each format digests
behind its own domain tag. Stripping a signature therefore means
resealing the bundle under the unsigned format, which a verifier holding
trusted keys refuses, because it requires a signature once it is given
any key.

- Default chosen for review: signing is off unless the operator
  configures a key, and an unsigned server's bundles are byte-for-byte
  what they were before signing existed.
- Default chosen for review: the server never generates or stores a
  signing key. The operator names a key file or an environment variable,
  and the server reads it on every export, so a replaced key file takes
  effect without a restart.
- Default chosen for review: a server configured to sign that cannot load
  its key refuses the export with `SIGNING_KEY_UNAVAILABLE` instead of
  returning an unsigned bundle, so a verifier expecting a signature never
  receives a quiet downgrade.
- Default chosen for review: `verify` given trusted keys refuses an
  unsigned bundle, a signature by any other key and a signature that does
  not verify.

The formats, the signed statement and the server settings are in the
[attested bundle format reference](../reference/attested-bundle-format.md).
Key setup is in [sign attested bundles](../how-to/sign-attested-bundles.md).

## What this is not

- **Not a backup.** A bundle holds rendered manifests for one model's
  board, not the store.
- **Not proof the model is correct.** It proves which findings a given
  catalogue produced for a given state. Findings with Error severity are
  still in the bundle and still errors.
- **Not available through MCP yet.** The RPC and the CLI are the supported
  surfaces.
