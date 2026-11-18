# Pub/sub message is not an event

> A Pub/Sub **message** is NOT an Event Modeling **event**.
> Sometimes the message IS the delivery of an event; sometimes the message is
> a notification and the real event lives in a different system entirely.

## Primitives

| Pub/Sub concept      | What it is at runtime                                                                                   |
|----------------------|---------------------------------------------------------------------------------------------------------|
| Topic / subject      | A named channel. Publishers write to it; subscribers read from it.                                      |
| Message              | A byte payload with optional headers, published to a topic and delivered to subscribers.                 |
| Publisher            | The process that writes a message to the topic.                                                          |
| Subscriber           | The process that reads messages from the topic.                                                          |
| Consumer group       | A set of subscribers that share delivery; each message goes to one member.                               |
| At-least-once / exactly-once | Delivery guarantees at the transport layer.                                                    |
| Acknowledgement      | The subscriber signals it has processed a message; the broker stops redelivering it.                    |
| Dead-letter topic    | Where messages land when all delivery retries are exhausted.                                             |

In NATS JetStream terms, as this codebase uses them: subjects, streams (JetStream persistence
layer), consumer groups, ack, and nack map directly onto the concepts above.

## What each maps to in Event Modeling

| Pub/Sub concept      | EM equivalent                                                                                                                            |
|----------------------|------------------------------------------------------------------------------------------------------------------------------------------|
| Topic / subject      | **Not an EM stream.** A NATS subject is a transport channel. The EM stream is an aggregate's identity boundary. They are orthogonal concepts. |
| Message              | Depends on its role (see the cases below). Not automatically an EM event.                                                                |
| Publisher            | The process executing a command's side effect (writing the event record) OR an infrastructure forwarder.                                 |
| Subscriber           | A process reading a read model that is sourced from events, OR pure infrastructure.                                                      |
| Consumer group       | Infrastructure. Determines which process handles delivery. Not an EM concept.                                                            |
| Acknowledgement      | Infrastructure. Exactly-once delivery semantics. Not an EM concept.                                                                      |
| Dead-letter topic    | Infrastructure. Operational fallback. Not an EM concept.                                                                                 |

### What a message can be

**The message IS the event being delivered.**

The upstream system records a business-meaningful state change into an
event store or durable log, then publishes a message whose payload IS that
event record (or a subset of it). The message is how subscribers learn the
event happened. In EM terms, the event lives in the source aggregate's
stream; the message is just the transport that carries the change record to
interested consumers. The EM board shows the event; the pub/sub delivery is
invisible.

**The message is a notification; the event lives elsewhere.**

The message says "something changed; go look." The payload may carry only an
ID or a hint, not the full event record. The downstream consumer must query
the source system to learn what actually changed. In EM terms there are two
events: one in the source system (the state change that triggered the
notification) and potentially one in the consuming system (the reaction). The
message itself is not on the board.

## The trap

The temptation:

> We publish a NATS message when an entity is updated. The message is the
> event. Put it on the EM board as an event and connect it directly to the
> downstream command.

This mixes transport with model in two ways:

**Conflating subject with stream.** A single NATS subject might carry change
records for every entity kind in every namespace. One Kafka topic might carry
events from ten different aggregates. Modelling the topic as the EM stream
creates a fake aggregate whose identity is "everything published to this
topic," which is meaningless as a consistency boundary.

**Conflating notification with event.** If the message carries only an entity
ID and a timestamp, the downstream consumer cannot reconstruct business state
from the message alone. The event (the state change with all its fields)
is in the store. Putting the notification on the EM board as the event makes
the model lie about where information lives and what the downstream reads.

The result: validation passes (the event has the fields the notification
carries), but the board says the downstream command is triggered by a
self-contained event when in fact the downstream must first issue a
`GetEntity` call. That read-then-act pattern is invisible to the validator
and to anyone reading the board.

## How to spot it

Code-level signals that a message is being misread as an event:

1. **The subscriber's first action after receiving a message is an RPC or
   database read.** If the message payload alone is not enough to execute the
   downstream command, the message is a notification. The real event is
   whatever the subscriber just fetched.

2. **A single topic has messages from multiple aggregate types.** A NATS
   subject like `atlas.changes` that carries `CommandSlice`,
   `ReadModelSlice`, and `Storyboard` change records is not one EM stream.
   Each entity kind is a separate aggregate; the topic is a fan-out
   transport layer.

3. **The message schema includes only keys and timestamps, no business
   fields.** A payload of `{ "entity_id": "...", "revision": 42 }` is a
   notification, not an event. The business fields live in the store at that
   revision; the message is a pointer to them.

## Modelling it

When the message carries the event, the board shows the event on the source
aggregate and hides the transport. When the message is a notification, the
board shows the upstream event and makes the downstream read explicit. The
steps are in [place a pub/sub message on the board](../how-to/model-runtime-code-as-an-event-model.md#place-a-pubsub-message-on-the-board).

## Concrete example from this codebase

When a mutation lands, `trogon-atlas-store` publishes a `ChangeEvent` to the
JetStream stream `TROGON_ATLAS_CHANGES`, on subjects under `atlas.changes`
(`atlas.changes.{put|del}.{namespace}.{slug}`). The server tails that stream
into an in-process broadcast channel, and the `StreamChanges` RPC forwards
each change to its subscribers.

The NATS subject is not an EM stream. The EM streams are the individual
aggregates (`CommandSlice`, `ReadModelSlice`, and so on). The JetStream
stream is transport that fans out every aggregate's changes to subscribers.

A `ChangeEvent` carries the change kind (`KIND_PUT` or `KIND_DELETED`), the
`EntityRef`, a resume token, the timestamp, the changeset, and the author. It
does not carry the entity's fields, so it is a notification: a consumer that
needs the new definition must call `GetEntity` at that ref. A board that
models a consumer of this feed shows the source aggregate's event, then a
read model fed by the notification, then the automation that fetches and
reacts, keeping the gap between notification and state visible. Putting the
`ChangeEvent` itself on the board as the business event would hide that read.

## Rule of thumb

> Ask: can the downstream command be executed using only the message payload?
> If yes, the message carries the event: show the event on the board, hide
> the transport. If no, the message is a notification: show the
> fetch-then-act as explicit steps and make the read visible.

> One pub/sub topic is never one EM stream unless it carries exactly one
> aggregate's events. When in doubt, identify the aggregate first, then find
> which topics carry its events.

## Related

- [Translation layers](translation-layers.md)
- [How to model runtime code as an Event Model](../how-to/model-runtime-code-as-an-event-model.md)
