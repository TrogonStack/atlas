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
published-only lean term pointers, compact exact definitions, richer review
indexes and revision-bound paging. Lean definitions omit full Entities,
metadata, quotations and review bodies while retaining policy, permitted use
period, source status and exact incarnation/read revisions.
Default publication selects the highest currently accepted version per
namespace/slug/audience family. A newer draft does not replace it. General MCP
search exposes namespace filters and listing has opt-in latest-version selection.

**Evidence:** connected tests cover audience families, restricted visibility,
page-token scope, source/output budgets and smaller pointer/definition payloads
than the inspection responses. Large evidence-bearing records remain available
through lean projections without first encoding a full inspection response.
The independent Reader run measured the earlier inspection index and selective
full-entry bytes and latency. Model-token cost is model dependent and is not
inferred from byte counts.

## GG-06 Agent context loading

**Implemented:** agent bootstrap guidance starts with scoped published term
pointers and loads only relevant compact exact definitions, retaining ambiguity,
publication and freshness warnings. It keeps organizational, code and customer
meanings separate.

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
schedule periodic review externally. The procedure records automated refresh
and human review cadences and responsible participants independently; a
scheduled run does not supply human approval.

**Evidence:** connected tests detect a newly selected source version, preserve
old publication, refuse review against changed evidence and prevent rebinding to
a recreated source. Partial input does not authorize removal. Accepted retirement
requires an explicit review disposition before deletion.

## GG-09 Glossary comparison

**Implemented:** `compare_glossaries` returns bounded candidates for equal labels
with different text, equal definitions with different labels and missing
counterparts. Exact entries and evidence remain available for interpretation;
the guide covers organizational/code and customer comparisons. Studio selects
the audience on each side independently, including code and human vocabulary
inside the same namespace.

**Evidence:** core and connected tests plus the supplied-corpus run preserve
contextual meanings. Lexical candidates do not establish semantic equivalence,
conflict or an Ambiguity ruling; agents and experts decide those explicitly.

## GG-10 Terminology policy

**Implemented:** native policy supports ALLOWED, DEPRECATED and PROHIBITED usage,
exact preferred replacement, rationale, audience selection and optional half-open
policy activation intervals. A separate reviewed `usagePeriod` declares when
the term itself may be used; use outside that period is prohibited even with
an ALLOWED, inactive or absent rule. References participate in authorization,
impact, lifecycle, branches and manifests. Descriptive `deprecated_on` retains
its existing meaning.

**Evidence:** tests cover inclusive start/exclusive end, independent policy and
term-use intervals, review binding, unknown enums, self/cyclic replacements,
reference traversal and safe deletion. The supplied corpus proposes Consignment
as unsuitable operational copy with Parcel as its exact replacement. Retained
customer-language policy and connected CLI acceptance preserve Booking internally
while prohibiting it in customer copy with Pickup slot as the exact replacement.

## GG-11 Code and UI checks

**Implemented:** `check_glossary_usage` checks an explicit text revision against
published audience-scoped policies, returning exact Terms, UTF-8 byte spans,
reasons and preferred replacements. Explicit exclusions and quoted-text skipping
are bounded. Prose phrases normalize whitespace. The separate
`check_glossary_code_usage` also recognizes snake_case, camelCase, PascalCase
and acronym boundaries. The generation guide applies the appropriate checker
to supplied code, documents and UI copy. Findings never perform a rename.

**Evidence:** core and connected tests cover identifier/phrase/case matching,
UTF-8 ranges, audience boundaries, temporal policy, exclusions, incomplete
evidence and work limits. Malformed quoted exclusions refuse rather than produce a clean result.
No generic linter runtime is required by this native checker.

## GG-12 Studio review

**Implemented:** `/glossary` exposes read-only discovery, exact definitions,
evidence, modeled usage, candidate comparison, review/freshness, refresh inspection
and revision-bound correction handoff. Comparison audiences are independent and
permitted term-use dates are separate from rule activation. Positive exact
versions and kind checks prevent modeled-usage substitution. Navigation uses
accessible controls.

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
