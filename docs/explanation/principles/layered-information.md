# Information is layered over an evolving graph

Part of the [Atlas principles](README.md).

Every feature must account for the entities and relationships it adds to,
reads from, or projects out of the graph. Strategy, journeys, screens,
delivery tracking, and reliability are connected layers of information.
Their relationships make impact and context discoverable instead of
leaving each layer as an isolated document.

The graph can be incomplete, disconnected, inconsistent, or broken.
Interfaces and agents must be able to inspect that state and understand
what is missing or contradictory. Backend validation helps identify the
state and guide the next action. An unexplained disconnect, an intentional
deferral, an unanswered question, and a stale reference carry different
meanings and need to remain visible.

Validation findings are information about the model. Structural validity
alone does not establish business truth, and clearing a finding must not
hide the behavior being described. Repairs need dependency context and a
reasoned change, with backend write guards still enforced. See
[annotations](../../../proto/trogonatlas/eventmodel/v1alpha1/annotations.proto),
[questions and assumptions](../../../proto/trogonatlas/annotation/v1alpha1/annotations.proto),
and [repair patterns](../validation-repair-patterns.md).
