# How to model a multi-step workflow

Model a workflow that runs several automated steps in order, with
compensation when a step fails. Atlas has no process-manager kind. A
progress read model and one AutomationSlice for each step carry the
routing. The
[`acme-order-workflow`](../../examples/manifests/acme-order-workflow/)
namespace is the finished result: tax, then payment, then shipping, and a
cancellation when payment fails.

## Why there is no process-manager kind

A process manager reads events and decides which automation runs next. The
design rules in `proto/trogonatlas/eventmodel/v1alpha1/event_model.proto`
already place that job:

- Decision 10: "Storyboard entry = ReadModel + Observer." An automation
  observes a read model. There is no second category of trigger, so no
  entity takes events and starts automations.
- Decision 11: an AutomationSlice "reads ReadModels only (never raw events)
  and emits at most ONE command per execution. Multi-step orchestration =
  multiple AutomationSlices chained through the graph."
- Decision 12: six edges are allowed: Command to Event, Event to ReadModel,
  ReadModel to UI, ReadModel to Processor, UI to Command, and Processor to
  Command. No edge goes from an event to a processor, so a router that
  starts automations cannot exist.

The routing rule "`payment.captured` starts the shipper" is therefore data
in the progress read model. The shipper observes the row, and the row says
when it is the shipper's turn.

## 1. Add a progress read model

One read model is the todo list for one order. Every step reads it, and
every step's event updates it. Declare the lane's `step` as `state` and
project it into the read model:

```yaml
kind: ReadModel
metadata: { name: order-progress, namespace: acme-order-workflow }
spec:
  key: [order_id]
  sourceEvents: # order.placed, tax.calculated, payment.captured,
                # payment.failed, order.shipped, order.cancelled
  # ...
```

Four automations read this read model, so name them in a
`SharedConsumersAnnotation`. Without it the validator reports
`READ_MODEL_MULTIPLE_AUTOMATIONS` (Info). With it the finding is
`READ_MODEL_SHARED_CONSUMERS` (Info).

## 2. Add one AutomationSlice for each step

Each step has its own Processor, its own AutomationSlice, and exactly one
command. The tax step (YAML in this guide is abbreviated, and `...` stands for the
`namespace` and `version` of the reference):

```yaml
kind: AutomationSlice
metadata: { name: tax-calculator, namespace: acme-order-workflow }
spec:
  sourceReadModels: [{ readModel: { id: { slug: order-progress, ... } } }]
  processor: { processor: { id: { slug: tax-worker, ... } } }
  emittedCommand: { command: { id: { slug: calculate-tax, ... } } }
  scenarios:
  - id: ok
    given: [{ readModel: { id: { slug: order-progress, ... } } }]
    whenDoc: The order row says tax is next.
    then: { command: { id: { slug: calculate-tax, ... } } }
```

The payment, shipping and cancellation steps have the same shape. Each
command needs its own CommandSlice, and each emitted event needs a
ReadModelSlice that projects it into `order-progress`, placed next to the
CommandSlice. See
[start an event model from scratch](start-a-model-from-scratch.md) for
these rules.

Compensation is another AutomationSlice. `order-canceller` reads the row
that says payment failed and issues `cancel-order`. To undo more steps, add
one AutomationSlice for each undo step.

## 3. State the outcomes with `then` and `noCommand`

Each scenario names its outcome. `then` is the command the automation
issues. `noCommand` states that the automation deliberately does nothing:

```yaml
kind: AutomationSlice
metadata: { name: shipper, namespace: acme-order-workflow }
spec:
  scenarios:
  - id: failed
    title: Payment failed
    given: [{ readModel: { id: { slug: order-progress, ... } } }]
    whenDoc: The order row says payment.failed.
    noCommand: { doc: 'Given payment.failed, the shipper does not run.' }
```

A scenario with neither outcome is an error (`SCENARIO_MISSING_THEN`). The
`noCommand` scenarios are where the routing is checked: each step names the
rows on which it stays idle.

## 4. Let one command emit either outcome

Issue 362 asks for a worker that sends `CompletePayment` or `FailPayment`. The
rule is "a processor issues exactly one command"
(`PROCESSOR_MULTIPLE_COMMANDS`, Error), so two commands need two
processors. Use one command instead. The payment gateway decides the
outcome, and the lane's transitions declare both roads:

```yaml
kind: Swimlane
spec:
  transitions:
  - after: { id: { slug: tax.calculated, ... } }
    next:
    - { event: { id: { slug: payment.captured, ... } }, doc: The gateway accepts the payment. }
    - { event: { id: { slug: payment.failed, ... } }, doc: The gateway declines the payment. }
```

The `capture-payment` CommandSlice has one scenario for each event. After
`tax.calculated`, exactly one of the two events follows. A scenario that
emits an event the lane does not allow next is reported
(`SCENARIO_TRANSITION_VIOLATION`, Warning).

Use one automation for each outcome only when the read model decides the
outcome before the worker runs. In this workflow that is the shipper
(row says paid) and the canceller (row says failed): they issue different
commands, so they are different automations.

## How routing shows on the board

The storyboard lists the slices in time order. Each AutomationSlice sits
after the projection of the event that made its row true. The board reads
as the chain: `calculate-tax`, the progress projection, `capture-payment`,
and then two roads from the payment projection: `ship-order` and
`cancel-order`. The lane's transitions give the fork, and the `noCommand`
scenarios show which road each automation refuses.

## Validate

```sh
trogon-atlas apply -f examples/manifests/acme/ examples/manifests/acme-order-workflow/
trogon-atlas validate acme-order-workflow/acme-order-workflow-model
```

The finished model reports no errors and no warnings. Its Info findings are
`READ_MODEL_SHARED_CONSUMERS`, `SLICE_NO_SCENARIOS` for the read model
slices, and `SUBDOMAIN_UNREALIZED` for subdomains of other examples.

## Related

- [Temporal and Event Modeling](../explanation/temporal-and-event-modeling.md)
- [Translation layers](../explanation/translation-layers.md)
- [Model runtime code as an Event Model](model-runtime-code-as-an-event-model.md)
- [Start an event model from scratch](start-a-model-from-scratch.md)
