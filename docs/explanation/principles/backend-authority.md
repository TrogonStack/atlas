# The backend is the source of truth

Part of the [Atlas principles](README.md).

The backend owns authoritative model state, identity, relationships,
validation, and accepted changes. Everything presented outside it is a
projection of that state. A board, inspector, report, search index, or
export provides a perspective on the model, and must remain traceable to
the backend's identities and scope.

Clients can cache or transform information for presentation. They cannot
establish a competing model or decide independently that a change has
landed. A manifest is proposed input until the backend accepts it; an AI
conclusion is analysis until it is adopted into the model. Backend
projections and caches must also remain reproducible from authoritative
content.

Shared schema and core helpers keep identity, reference traversal, and
semantic comparison consistent across surfaces. See
[snapshot identity](../snapshot-identity.md), [tenant types](../tenant-types.md),
and the [core responsibilities](../../../rsworkspace/crates/trogon-atlas-core/README.md).
