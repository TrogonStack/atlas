# Translation layers

When you map an existing codebase into an Event Model, the codebase's own
abstractions don't translate 1:1 to Event Modeling concepts. The runtime tools
the code uses (Temporal, gRPC, NATS, message brokers, databases, schedulers,
caches) bake in their own units of execution and their own atomicity
guarantees. Naively mapping "what the code does" onto "what the EM board
declares" produces models that look right but lie about what's actually
happening.

A translation layer is the rule that says what a runtime tool's primitives
ARE in EM terms, and what they are NOT.

## Why these matter

The validator enforces **structural** invariants: one command → one stream,
every command has a trigger, every event field has a source, every member is
in the store. It cannot enforce **narrative** truth. Two events both sitting
on one lane look fine to the validator. Whether they actually fire at the
same moment is a translation-layer question the validator cannot answer.

Get the translation wrong and the model passes validation while hiding
sagas. Get it right and the model says what's actually happening at
runtime, which is the whole point of building one.

## Translations

- [Temporal and Event Modeling](temporal-and-event-modeling.md): **a Temporal
  activity is not a command.** Loops inside an activity hide sagas.
- [gRPC handler is not a command](grpc-handler-is-not-a-command.md): the
  handler is transport; the command is the business intent it carries.
- [Pub/sub message is not an event](pubsub-message-is-not-an-event.md):
  sometimes the message IS the delivery of an event; sometimes the message is
  a notification and the real event lives in a different system.

To apply these while modelling, follow
[how to model runtime code as an Event Model](../how-to/model-runtime-code-as-an-event-model.md).

## Translations not yet written up

The same reasoning applies to other runtime primitives. Each gets its own
page once real mapping work runs into it:

- **HTTP route ≠ command.** Same reasoning as gRPC. Often the route dispatches
  the real command into a workflow or queue.
- **DB transaction ≠ command.** A transaction may bundle the side effects of
  one command, or it may span several saga steps (when it shouldn't).
- **Background job / cron ≠ automation.** Background jobs are infrastructure;
  automations are EM constructs reading RMs to fire commands. Some jobs ARE
  automations (cron-triggered reactions to a clock RM); some are not (cache
  warmers, garbage collection).
- **Cache invalidation ≠ projection.** Caches sit beside read models, not
  inside the event-modeling graph.
- **Stream / topic ≠ stream (the EM aggregate).** A NATS subject or Kafka
  topic is a transport channel. The EM stream is an aggregate's identity. One
  Kafka topic may carry many aggregates' events; one aggregate may publish to
  many topics.
- **Idempotency key ≠ aggregate identity.** Idempotency keys are about
  exactly-once delivery at the transport layer. Aggregate identity is about
  the consistency boundary at the model layer.

## Shape of a translation page

Each translation page covers the same ground, so readers can compare tools:

- **Primitives:** the runtime tool's concepts and what each gives you.
- **Mapping:** each primitive's EM equivalent, or "not an EM concept".
- **The trap:** what naive translation breaks; the thing that looks right
  and is wrong.
- **Spotting it:** code-level signals that the trap is present.
- **Concrete example:** from this codebase or a mapped one, naming the
  actual entities involved.
- **Rule of thumb:** the one-line test a reviewer can apply.

The corrected modelling steps go in the
[how-to guide](../how-to/model-runtime-code-as-an-event-model.md), not on the
translation page.

## Rule of thumb across all translations

> If a single "unit of code execution" in the tool produces more than one
> business-meaningful state change, the EM model needs more than one
> command. The runtime tool is not the modelling unit.
