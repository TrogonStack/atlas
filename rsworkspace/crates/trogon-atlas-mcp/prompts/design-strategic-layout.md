# Design the strategic layout

Follow these rules when placing bounded contexts, the problem space and their
language. Write the entities with put_entity_json.

- Bounded contexts touch at exactly ONE seam: an upstream EVENT flowing into a
  downstream subscription READ MODEL's source_events (the translation pattern).
  A cross-model link is NOT a continuation: no shared stream means no fork, no
  guard, no single timeline; it is async pub/sub with fan-out. The DOWNSTREAM
  declares the dependency (its consumer-shaped view names the upstream event);
  the upstream never knows its consumers. Any other cross-namespace structural
  ref is rejected (CROSS_MODEL_BOUNDARY_VIOLATION): translate, don't reference.
  A namespace IS a bounded context; its root document is the BoundedContext
  entity (slug == namespace, validated) carrying the context charter, links,
  and metadata. Event models are free-form curations WITHIN a context (a
  feature, a flow, a release), and a namespace may hold many.
- Model DECISIONS AND DATA, never implementation. In scope: strategic design
  (contexts, charters, seams, investment) and the behavioral contract (facts,
  order, fields). Out of scope, always: classes, services, repositories,
  frameworks, persistence, code structure. Litmus test: would it survive
  reimplementing the system in another language?
- The problem space is a knowledge graph: Domain (the business, usually one)
  contains Subdomains (problems the business must be good at, classified
  CORE/SUPPORTING/GENERIC, a budget document). They live in a namespace named
  after the domain, which is NOT a bounded context. A Subdomain never contains
  contexts: BoundedContext.realizes maps solution to problem (cross-namespace,
  legal: knowledge, not coupling). One context realizing several subdomains is
  a monolith confessing; realizing subdomains of different classifications also
  warns (REALIZES_MIXED_CLASSIFICATIONS), because one boundary then carries two
  investment levels. A Domain declared in a context's namespace warns
  (DOMAIN_IN_CONTEXT_NAMESPACE): give it its own problem-space namespace.
  Classification goes on the SUBDOMAIN, never the context. Layout: the Project,
  the Domain and ALL Subdomains live in ONE problem-space namespace (Domain and
  Project slug == that namespace); each context namespace holds only its
  BoundedContext, which realizes ONE subdomain across namespaces. Never create
  a Project or Domain per context namespace to silence DOMAIN_MISSING_PROJECT;
  point the contexts at the one problem space instead. Declare a relationship
  intent (see the relationship rule) for every cross-context seam.
- Seams carry data; relationships carry intent. The DOWNSTREAM BoundedContext
  declares relationships: upstream context + intent (CUSTOMER_SUPPLIER /
  CONFORMIST / PARTNERSHIP / SEPARATE_WAYS) + doc. Intent and data must agree:
  integration intent without a seam warns, SEPARATE_WAYS with a seam errors, a
  seam with no declared intent nudges (Info).
- Terms and Ambiguities are language data. A Term lives in the bounded context
  whose language it defines and points outward through embodied_by to the
  entities carrying that meaning. An Ambiguity lives in the problem-space
  namespace and records a real ruling across two or more Term refs: HOMONYM or
  SYNONYM, with DELIBERATE, RENAME, or MERGE. Do not store unresolved language
  work as an Ambiguity.

Mutation responses from put_entity_json return non-blocking validation
findings, strategic ones included, in an `issues` array: read it and fix them.

After editing only problem-space entities (Project, Domain, Subdomain) or a
BoundedContext there may be no EventModel to validate: run validate_project
with the namespace or project scope instead. Fix all Errors and Warnings it
reports.
