# Generated glossary contracts

Status: implemented native profile, with agent procedures for extraction and
maintenance. Contract revision and mutation compatibility floors are 25.
[Delivery coverage](generated-glossary-gaps.md) maps the original gaps to the
implementation; [acceptance](generated-glossary-acceptance.md) identifies the
executable checks. Semantic definition quality remains subject to expert review.

## Entries and source evidence

A native [Term](../../proto/trogonatlas/eventmodel/v1alpha1/strategic.proto)
keeps its namespace/slug/version identity, definition in doc, modeled embodied_by
references and optional owning Domain. Its optional
[GlossaryEntry](../../proto/trogonatlas/eventmodel/v1alpha1/glossary.proto)
adds generation revision, audience, evidence, next review date, terminology
policy and a server-owned review. Terms without this field remain legal;
they are not implicitly generated or reviewed.

Generated entries require nonblank title and definition, explicit generation
revision and audience, and bounded evidence. Each evidence item selects an exact
retained Document, its incarnation UID, SHA-256 of the original retained bytes,
an authored locator and an exact supporting quotation. The backend verifies
key, UID, bytes and quote containment. The locator is descriptive; it is not
independently resolved as a source parser location.

Evidence Documents must share the Term namespace. A Term retains quoted source
text, so this profile keeps that text under the same namespace visibility
boundary as its source. Obtain and retain authorized material in that context
before authoring the candidate. A link grants no permission to fetch content.
Missing, changed and inaccessible sources stay explicit.

Source text must be retained UTF-8 and is read inertly. Code repository/revision
annotations remain provenance assertions. Retained bytes and a matching code
pointer do not verify an external repository checkout, ancestry or the truth of
a definition. The controlled
[source corpus](../../examples/manifests/generated-glossary/sources.yaml)
includes organizational and code snapshots for exercising the procedure.

Source-control ownership cannot supply the native glossary field on create or
own a glossary-bearing Term as a whole. Supported field-specific updates retain
local glossary state under the existing [source isolation](source-integration-contracts.md)
boundary.

## Candidate meaning and review authority

The candidate digest uses shared canonical content with glossary domain
separation. It includes exact Term identity, definition, modeled references,
metadata, owning Domain, source evidence, generation revision, audience and
policy. Review and next-review date are excluded. Equivalent supported metadata
encodings preserve the digest. Unrenderable metadata has no canonical review
identity.

ReviewGlossaryTerm requires an authenticated, nonanonymous User principal with
ordinary write authority over the Term namespace. Agent credentials, directory
assignments and lifecycle labels cannot assert review. The caller supplies the
exact Term, current etag, disposition, reason and optional preview flag. The
server records the authenticated actor, time, candidate digest and owner UID.
This authenticates the explicit operation; it does not verify expertise or that
a physical human operated the credential.

ACCEPTED revalidates source evidence and preferred replacement targets.
REJECTED and NEEDS_CHANGES can bind the unchanged stored candidate when those
targets have changed or become unavailable. These negative decisions preserve
the exact meaning and evidence; they do not attest current source validity.
This permits explicit retirement without restoring a deleted source incarnation.

Ordinary writes cannot mint, replace or clear reviews. They may preserve the
same receipt. A correction after NEEDS_CHANGES can change the candidate; the old
receipt then becomes stale until an explicit review binds the new meaning.
A review preview performs checks without persisting authority.

While its current review is ACCEPTED, meaning is immutable at that exact version.
Propose a correction as a new Term version without copying the previous review.
Scheduling changes may
preserve the meaning review. Retiring an accepted version requires an explicit
review disposition before deletion; incoming-reference protection still applies.

Reviews use existing guarded native writes, history and writer authority.
On a stale etag, refresh and reconsider. After a lost or unavailable response,
read the exact entry and history to reconcile before retrying. A failed response
alone does not prove that no mutation landed. Response-size checks run before
review mutation, leaving headroom for the returned inspection record.

## Publication and retrieval

A glossary selection names explicit namespaces and an optional audience.
Without include_unreviewed, the catalog returns the highest version with a
current accepted review for each namespace/slug/audience family. A newer draft
does not displace it. Omitting audience preserves distinct audience families.
With include_unreviewed, historical versions and unreviewed entries remain
inspectable; highest stored version is not an approval policy.

On exact or historical reads, `published` means that the entry has a current
accepted review. The default catalog still selects only the highest such version
in each family; the flag alone does not establish that an older version is the
family's default selection.

Compact indexes carry exact reference, UID, etag, title, audience, publication
selection, state, explanation and next-review date. Fetch full definitions,
policies, review records and evidence by exact reference. Pagination binds
caller visibility, branch, selection and the read revision; a changed query or
read set invalidates its token.

Publication and freshness are separate. An accepted definition remains selected
when its evidence changes or disappears, with SOURCE_CHANGED or
SOURCE_UNAVAILABLE and explicit incomplete status where appropriate. REVIEW_DUE
records the declared calendar deadline; it does not fabricate a review.
Review-due evaluation uses the current UTC calendar date. Usage checks also
default to that date unless the caller supplies an explicit `on` date.
Evidence resolution is CURRENT, CHANGED or UNAVAILABLE, independently of expert
review state. Consumers inspect both publication and freshness.

General MCP search also accepts namespace filters, and listing exposes opt-in
latest_versions_only while preserving its all-versions default. These tools do
not replace glossary publication selection.

## Generation and maintenance

The [generation guide](../how-to/generate-and-maintain-a-glossary.md) and
discoverable generate-glossary MCP prompt give an agent the source, authoring,
review, publication and lookup procedure. The agent extracts words and phrases,
including familiar words used in unusual ways, proposes concise contextual
definitions and preserves uncertainty. Backend source checks establish
traceability; expert review determines whether the meaning is useful and correct.

InspectGlossaryRefresh compares evidence, selected source versions and generation
revision without writing. Selecting Document version 2 marks a version-1
citation for reconciliation even while the original snapshot is retained.
Absence from a partial input selection does not authorize deletion.
Failed or partial refresh preserves earlier publication and expert changes.

Record attempts, input coverage, source inventory, candidates, review and
publication results independently in guarded native Documents using the guide.
Preserve an unsuccessful attempt locally when the backend is unavailable, then
reconcile before recording it. A run Document is an attributed record, not
review authority or an apply receipt.

Periodic review uses an externally scheduled agent and the declared review date.
This profile installs no scheduler or remote provider connector. Acquisition is
separate from interpreting supplied, authorized text. Agent context loading
starts with a compact index and retrieves only needed exact definitions.

## Comparison and terminology policy

CompareGlossaries returns advisory candidates from normalized title and
definition text: different definitions for equal titles, equal definitions
under different titles, and missing counterparts. These lexical comparisons are
not semantic equivalence proofs or Ambiguity rulings. The agent and expert
interpret them with evidence and preserve intentional contextual differences.

A Term policy declares ALLOWED, DEPRECATED or PROHIBITED usage, an optional exact
preferred Term and rationale. Optional calendar boundaries use the half-open
interval [effective_from, effective_until). An absent boundary is open.
Invalid intervals, unknown enums, self-replacements and replacement cycles refuse.
References join discovery, impact, guarded writes, branches, manifests and safe
deletion.

Policy applies to the selected published version; an inactive policy does not
fall back to a historical version. The generic descriptive deprecated_on
annotation retains its meaning. A date or label never renames a model entity or
changes implemented application behavior.

CheckGlossaryUsage requires explicit published audience selection and an exact
text revision. It returns scoped word/phrase matches with exact Term references,
UTF-8 byte ranges, rationale and optional preferred reference. Case folding
preserves original byte spans. Explicit exclusions and quoted-text skipping are
bounded; an unmatched quote refuses rather than hiding unchecked text.
The text revision is a caller-supplied label echoed in the result. Retain the
checked bytes and their digest in the run record to bind that label to input;
the checker does not resolve or verify an external repository revision.
Stale or unavailable evidence produces an incomplete check instead of a clean
verdict. Suggestions never apply a rename.

## Studio and authority

Studio provides read-only glossary discovery and exact entry, source and modeled
usage inspection, comparison, freshness and correction handoff. The bridge
forwards caller and branch, requires exact positive versions, refuses historical
glossary requests and exposes no review or authoring operation. Modeled usage
inspection rejects a substituted kind or version.

The copy action communicates exact context and etag to an agent; it is not a
mutation. People inspect and communicate, and agents use governed backend tools.
Ownership, directory membership and cached views grant no additional permission.

## Bounds and failure behavior

| Boundary | Limit |
| --- | --- |
| Context selection | Explicit nonempty selection, at most 32 namespaces |
| Catalog scan and page | 5,000 rows; page size at most 500, default 100 |
| Evidence per entry | 32 items; exact quotes at most 16,384 UTF-8 bytes each |
| Generated title / definition | 256 / 16,384 UTF-8 bytes |
| Generation revision / audience | 256 / 128 UTF-8 bytes on generated entries |
| Encoded Term | 1 MiB |
| Evidence traversal | 10,000 examinations and 32 MiB examined encoded content |
| Source inspection | At most 32 exact Documents and 1 MiB total retained UTF-8 bytes |
| Derived response | At most 3 MiB encoded protobuf and JSON |
| Review preflight | At most 2 MiB entity JSON before mutation |
| Core comparison or usage input | At most 1,000 Terms per input and 8 MiB combined candidate bytes |
| Checked text and match work | 256 KiB text and 16 MiB term/text match work |
| Usage exclusions / findings | At most 1,024 each; comparison output is also bounded |

Native entity, Document, Store and transport limits still apply. Quotas refuse
explicitly rather than silently truncating or returning an unchecked clean result.
Operational storage/authorization errors remain errors. Missing evidence, known
source changes, incomplete discovery and unavailable infrastructure have distinct
outcomes.
