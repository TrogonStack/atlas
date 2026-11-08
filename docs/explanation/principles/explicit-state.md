# Make intent and state explicit

Part of the [Atlas principles](README.md).

Atlas favors explicit intent and state wherever their meaning affects a
decision. People and agents should be able to understand the model from
what it declares, without reconstructing hidden conventions, prior
interactions, or assumptions held by another participant. This makes
decisions inspectable and preserves their meaning across views and tools.

Absence can carry intent. An unanswered question, an incomplete definition,
and a deliberate decision that nothing should happen have different
meanings. Making those distinctions visible lets a reviewer tell what is
known, what remains open, and what has been intentionally ruled out. An
omitted value alone cannot communicate which meaning was intended.

For example, an automation scenario describes a WHEN condition and its
expected outcome. If the automation should issue no command, that is an
outcome worth stating, together with why. A deliberate "no command
expected" tells a different story from a missing THEN. The
[scenario contract](../../../proto/trogonatlas/eventmodel/v1alpha1/scenarios.proto)
expresses this distinction through a command outcome or an explicit
no-command outcome.

Explicitness serves understanding. Favor representations that preserve
meaningful distinctions in state, relationships, defaults, and outcomes,
while keeping routine detail easy to work with. The aim is a model whose
intent survives inspection, change, and collaboration, with uncertainty
and intentional absence visible wherever they affect interpretation.
