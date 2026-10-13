# Translation Layers

When you map an existing codebase into an Event Model, the codebase's own
abstractions don't translate 1:1 to Event Modeling concepts. The runtime tools
the code uses (Temporal, gRPC, NATS, message brokers, databases, schedulers,
caches) bake in their own units of execution and their own atomicity
guarantees. Naively mapping "what the code does" onto "what the EM board
declares" produces models that look right but lie about what's actually
happening.

This directory captures the translation rules for specific runtime tools:
what each layer's primitives ARE in EM terms, and what they are NOT.

## Why these matter

The validator enforces **structural** invariants: one command → one stream,
every command has a trigger, every event field has a source, every member is
in the store. It cannot enforce **narrative** truth. Two events both sitting
on one lane look fine to the validator. Whether they actually fire at the
same moment is a translation-layer question the validator cannot answer.

Get the translation wrong and the model passes validation while hiding
sagas. Get it right and the model says what's actually happening at
runtime, which is the whole point of building one.

## Translation guides

- [`temporal-and-event-modeling.md`](./temporal-and-event-modeling.md): Temporal
  activities/workflows vs EM commands/sagas. **A Temporal activity is not a
  command.** Loops inside an activity hide sagas.
- [`grpc-handler-is-not-a-command.md`](./grpc-handler-is-not-a-command.md): gRPC
  handlers vs EM commands. **A gRPC handler is not a command.** The handler is
  transport; the command is the business intent it carries.
- [`pubsub-message-is-not-an-event.md`](./pubsub-message-is-not-an-event.md): Pub/Sub
  messages vs EM events. **A pub/sub message is not always an event.** Sometimes
  the message IS the delivery of an event; sometimes the message is a notification
  and the real event lives in a different system.

## Open / future translations

Add a guide for each as it comes up in real mapping work:

- **HTTP route ≠ command.** Same reasoning as gRPC. Often the route dispatches the
  real command into a workflow or queue.
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

## Convention for each guide

1. **Primitives.** Name the runtime tool / pattern and list what it gives
   you.
2. **Mapping.** Map each primitive to its EM equivalent (or "not an EM
   concept").
3. **The trap.** What naive translation breaks. The thing that looks
   right and is wrong.
4. **Spotting it.** Code-level signals that the trap is present.
5. **How to model it.** The corrected shape.
6. **Concrete example.** From this codebase or a mapped one, with names of
   the actual entities involved.

## Rule of thumb across all translations

> If a single "unit of code execution" in the tool produces more than one
> business-meaningful state change, the EM model needs more than one
> command. The runtime tool is not the modelling unit.
