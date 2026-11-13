# Generated glossary delivery coverage

Status: the native glossary profile and connected agent procedure are
implemented. The identifiers below retain the original gap inventory's scope
and map it to maintained contracts, tools and acceptance checks. See
[contracts](generated-glossary-contracts.md), the
[generation guide](../how-to/generate-and-maintain-a-glossary.md) and
[acceptance evidence](generated-glossary-acceptance.md).

[Martin Fowler's Generated Glossary](https://martinfowler.com/bliki/GeneratedGlossary.html)
describes extracting specialized vocabulary from organizational material,
reviewing it with domain experts, letting agents consult definitions economically
and refreshing the glossary. Atlas supplies this workflow through native Terms,
retained Documents, authenticated review and connected agents.

Semantic extraction and interpretation are agent tasks. Publication requires an
explicit User review; the backend checks identity, evidence and content binding,
but cannot certify domain expertise. Connected acceptance uses synthetic users.
Remote acquisition and an installed scheduler are outside this supplied-input
profile. Periodic maintenance uses an externally scheduled agent.

## GG-01 Generation procedure

**Implemented:** the generation guide and discoverable `generate-glossary` MCP
prompt cover authorized source selection, contextual extraction, correction,
guarded publication, exact lookup and refresh. The
[supplied corpus](../../examples/manifests/generated-glossary/sources.yaml)
contains organizational, customer and code material without preapproved Terms.

**Evidence:** connected CLI/MCP tests exercise the procedure. An authoring agent
also extracted candidates from the supplied corpus, and a separate Reader agent
consumed the published selection. Real domain-expert evaluation remains a
deployment activity, not a property established by the test identity.

## GG-02 Source evidence

**Implemented:** generated Terms retain exact same-namespace Document references,
incarnation UIDs, byte digests, authored locators and supporting quotations.
`inspect_glossary_sources` returns bounded inert UTF-8 text and source pins.
Run Documents record source coverage and generation revision. Missing, changed
and unavailable material remain distinct from current evidence.

**Evidence:** core evidence tests and connected backend tests exercise changed,
deleted, recreated, unauthorized and oversized material. A locator is descriptive;
quote containment does not prove that a definition follows from the passage.
Code pointers do not verify an external checkout or authorize acquisition.

## GG-03 Extraction and definition quality

**Implemented:** the prompt and guide require words, phrases, specialized meanings,
contextual alternatives, uncertainty and concise evidence-bearing definitions.
Generated-entry admission rejects blank definitions, unsupported quotes and
invalid evidence. Generic Terms retain their existing admission contract.

**Evidence:** the authoring run extracted Parcel, Ready to ship, Booking,
Consignment and Delivery with internal, customer and code audiences. Blank
definitions and fabricated supporting text refused. Semantic correctness,
conflicting interpretations and extraction recall require expert judgment.

## GG-04 Expert review

**Implemented:** `review_glossary_term` records ACCEPTED, NEEDS_CHANGES or REJECTED
against the exact candidate digest, owner UID and current etag. Authenticated User
write authority is required. Ordinary writes and Agent credentials cannot mint
review. Corrections after NEEDS_CHANGES invalidate the earlier content binding;
accepted meaning requires a new version. Unresolved candidates stay unpublished.

**Evidence:** backend and MCP tests cover correction, rejection, preview, forged
review refusal, stale review/etag and immutable accepted meaning. These protocol
checks do not verify a credential holder's expertise.

## GG-05 Scoped compact retrieval

**Implemented:** backend, CLI and MCP expose explicit namespace/audience selection,
bounded compact indexes, exact definition retrieval and revision-bound paging.
Default publication selects the highest currently accepted version per
namespace/slug/audience family. A newer draft does not replace it. General MCP
search exposes namespace filters and listing has opt-in latest-version selection.

**Evidence:** connected tests cover audience families, restricted visibility,
page-token scope and source/output budgets. The independent Reader run measured
compact-index and selective-definition bytes and latency. Model-token cost is
model dependent and is not inferred from byte counts.

## GG-06 Agent context loading

**Implemented:** agent bootstrap guidance starts with a scoped compact index and
loads only relevant exact definitions, retaining ambiguity, publication and
freshness warnings. It keeps organizational, code and customer meanings separate.

**Evidence:** a separate Reader agent discovered the published index and fetched
only the Parcel and readiness entries needed to explain the contextual meanings.
It used actual backend results rather than the extraction plan or source corpus.

## GG-07 Publication and history

**Implemented:** glossary review uses native etag guards, validation, branches,
history and manifests. Accepted review binds exact content and source identity.
The guide requires read/history reconciliation after uncertain responses before
retrying; a failed response alone does not establish that no write landed.

**Evidence:** connected branch review and landing preserve source and Term
incarnations. Tests cover stale writes, preview, review in manifest export and
reapply, safe deletion and refusal before oversized review mutation.

## GG-08 Refresh and freshness

**Implemented:** `inspect_glossary_refresh` compares selected exact sources and
generation revision without writing. Source changes, availability and review-due
state remain separate from publication. Run Documents retain attempt and input
coverage independently. Agents apply corrections through guarded writes and
schedule periodic review externally.

**Evidence:** connected tests detect a newly selected source version, preserve
old publication, refuse review against changed evidence and prevent rebinding to
a recreated source. Partial input does not authorize removal. Accepted retirement
requires an explicit review disposition before deletion.

## GG-09 Glossary comparison

**Implemented:** `compare_glossaries` returns bounded candidates for equal labels
with different text, equal definitions with different labels and missing
counterparts. Exact entries and evidence remain available for interpretation;
the guide covers organizational/code and customer comparisons.

**Evidence:** core and connected tests plus the supplied-corpus run preserve
contextual meanings. Lexical candidates do not establish semantic equivalence,
conflict or an Ambiguity ruling; agents and experts decide those explicitly.

## GG-10 Terminology policy

**Implemented:** native policy supports ALLOWED, DEPRECATED and PROHIBITED usage,
exact preferred replacement, rationale, audience selection and optional half-open
calendar intervals. References participate in authorization, impact, lifecycle,
branches and manifests. Descriptive `deprecated_on` retains its existing meaning.

**Evidence:** tests cover intervals, unknown enums, self/cyclic replacements,
reference traversal and safe deletion. The supplied corpus proposes Consignment
as unsuitable operational copy with Parcel as its exact replacement.

## GG-11 Code and UI checks

**Implemented:** `check_glossary_usage` checks an explicit text revision against
published audience-scoped policies, returning exact Terms, UTF-8 byte spans,
reasons and preferred replacements. Explicit exclusions and quoted-text skipping
are bounded. The generation guide explains applying this to supplied code,
documents and UI copy. Findings never perform a rename.

**Evidence:** core and connected tests cover phrase/case matching, UTF-8 ranges,
audience boundaries, temporal policy, exclusions, incomplete evidence and work
limits. Malformed quoted exclusions refuse rather than produce a clean result.
No generic linter runtime is required by this native checker.

## GG-12 Studio review

**Implemented:** `/glossary` exposes read-only discovery, exact definitions,
evidence, modeled usage, candidate comparison, review/freshness, refresh inspection
and revision-bound correction handoff. Positive exact versions and kind checks
prevent modeled-usage substitution. Navigation uses accessible controls.

**Evidence:** component/client/bridge tests and authenticated Chromium acceptance
exercise the real Rust/NATS backend, production bridge and Studio. The bridge
refuses unauthenticated reads and model writes; inspection grants no authority.

## GG-13 Connected acceptance

**Implemented:** maintained core, backend, CLI, MCP and authenticated native
browser fixtures cover the profile and its failure boundaries. The
[acceptance reference](generated-glossary-acceptance.md) identifies commands,
assertions, measurements and the limits of the available receipts.

**Boundary:** local controlled acceptance is separate from hosted CI, production
operation, representative-corpus evaluation and physical human expert review.
Those outcomes must be recorded where deployed before claiming them.
