# The code is the ultimate truth

Part of the [Atlas principles](README.md).

The model is a claim about what the system does; the code is what the
system does. The backend owns what the model says, and the code owns what
the system does. Once a slice is implemented, the model must not disagree
with the code. When they differ, the model is wrong, and Atlas reconciles
the model to the code rather than the other way around. People should not
re-sync the model by hand: agents and tooling detect drift, apply the
reconciliation, and report what changed and what it puts at risk. Drift is
only meaningful against a specific revision, so an implemented slice
records the code revision it was last reconciled with.

Before a change lands, the model may run ahead of the code. A proposed
slice, schema version, or storyboard change is a question asked of the
model: which read models, consumers, contexts, and scenarios are affected,
and what would break. The same question can be asked of a proposed code
change. Either way, the proposal stays marked as proposed until the code
confirms it. If the code lands differently, the code wins, the model
follows it, and the gap between the proposal and what shipped remains
visible as a finding.

Truth is about behavior, not code structure. The model follows what the
code does, read through the translation layers, not how the code is
organized. Code decides the behavioral primitives: commands, events, read
models, processors, slices, storyboard order, schemas, and consistency
boundaries. Strategic design, questions, assumptions, deferrals,
reliability objectives, and delivery tracking are layers people own; they
can go stale against the code, and that staleness is a finding, but they
are not rewritten from it. See [translation layers](../translation-layers.md),
[layered information](layered-information.md), and
[backend authority](backend-authority.md).
