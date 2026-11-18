# Event Modeling is the foundation

Part of the [Atlas principles](README.md).

The model ultimately comes down to Event Modeling primitives: commands
express intent, events record facts, read models expose information, UIs
let people observe and act, and processors issue commands in response to
read models. Slices connect those moments; storyboards give them causal
order; swimlanes state consistency boundaries.

Time moves forward. Recurrence is a new occurrence of an entity, so a
projection must preserve storyboard order and repeated moments. A screen
can compose contributions from several contexts without becoming a new
kind of behavioral slice. Delivery trackers and reliability objectives
layer information onto the model without rewriting its behavioral
primitives.

Strategic design explains the problems and boundaries around that
behavior. Domains and subdomains describe business problems; bounded
contexts declare language and responsibility; explicit mappings and seams
connect them. The model describes decisions and data that outlive any one
implementation. Design-tool links and vendor bindings connect outward from
that description. A code binding marks a slice as implemented, and from
then on the [code is the truth](code-is-the-truth.md) about that slice's
behavior.

Schemas declare shapes, and entity references preserve exact
breaking-change versions so contracts can evolve without silently
retargeting consumers. New layers should remain connected to these
identities and primitives rather than introduce a competing behavioral
language. See the [modeling contract](../../../proto/trogonatlas/eventmodel/v1alpha1/event_model.proto),
[slice grammar](../../../proto/trogonatlas/eventmodel/v1alpha1/slices.proto),
[translation guides](../translation-layers.md), and
[reliability overlays](../reliability-overlays.md).
