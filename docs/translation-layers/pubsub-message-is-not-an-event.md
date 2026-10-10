# Pub/Sub Message Is Not an Event

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

In NATS JetStream terms (this codebase): subjects, streams (JetStream persistence
layer), consumer groups, ack, and nack map directly onto the concepts above.

## What each maps to in Event Modeling

| Pub/Sub concept      | EM equivalent                                                                                                                            |
|----------------------|------------------------------------------------------------------------------------------------------------------------------------------|
| Topic / subject      | **Not an EM stream.** A NATS subject is a transport channel. The EM stream is an aggregate's identity boundary. They are orthogonal concepts. |
| Message              | Depends on role (see the two cases below). Not automatically an EM event.                                                                |
| Publisher            | The process executing a command's side effect (writing the event record) OR an infrastructure forwarder.                                 |
| Subscriber           | A process reading a read model that is sourced from events, OR pure infrastructure.                                                      |
| Consumer group       | Infrastructure. Determines which process handles delivery. Not an EM concept.                                                            |
| Acknowledgement      | Infrastructure. Exactly-once delivery semantics. Not an EM concept.                                                                      |
| Dead-letter topic    | Infrastructure. Operational fallback. Not an EM concept.                                                                                 |

### The two cases for a message

**Case 1 -- the message IS the event being delivered.**

The upstream system records a business-meaningful state change into an
event store or durable log, then publishes a message whose payload IS that
event record (or a subset of it). The message is how subscribers learn the
event happened. In EM terms, the event lives in the source aggregate's
stream; the message is just the transport that carries the change record to
interested consumers. The EM board shows the event; the pub/sub delivery is
invisible.

**Case 2 -- the message is a notification; the event lives elsewhere.**

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
from the message alone. The event -- the state change with all its fields --
is in the store. Putting the notification on the EM board as the event makes
the model lie about where information lives and what the downstream reads.

The result: validation passes (the event has the fields the notification
carries), but the board says the downstream command is triggered by a
self-contained event when in fact the downstream must first issue a
`GetEntity` call. That read-then-act pattern is invisible to the validator
and to anyone reading the board.

## How to spot it

Three code-level signals that a message is being misread as an event:

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

## How to model it

**For Case 1 (message carries the event):**

1. The event belongs to the source aggregate's swimlane. Place it there.
2. The pub/sub delivery is infrastructure. The EM board does not show it.
3. The downstream consumer's reaction is an AutomationSlice whose trigger
   is a read model sourced from the event, not from the message. The
   automation runs when the read model says "there is a new event to act on."
4. The source aggregate's stream and the NATS subject are two different things.
   Name them differently on the board and in code comments.

**For Case 2 (message is a notification):**

1. Model the upstream state change as an event on the source aggregate.
2. Model the downstream reaction separately: a read model that watches the
   source aggregate (updated via the notification or a poll), then an
   AutomationSlice that fires a command when the read model changes.
3. The message itself disappears from the board. It is the delivery mechanism
   for the read model update, not the event.
4. If the downstream system cannot directly observe the source aggregate's
   stream, model the notification as an integration event at the boundary:
   an event on a "notification received" swimlane, carrying only what the
   notification contained, followed by a command that fetches the rest. That
   makes the read explicit in the model.

## Concrete example from this codebase

`trogon-atlas-server` publishes change records over a JetStream broadcast
channel (`ChangeRecord`) when a mutation lands. Downstream subscribers
(the `StreamChanges` RPC) receive these records.

The NATS subject is not an EM stream. The EM streams are the individual
aggregates (`CommandSlice`, `ReadModelSlice`, etc.). The JetStream channel
is transport that fans out all aggregate changes to subscribers.

A `ChangeRecord` carries the full `StoredEntity` payload -- Case 1. The
downstream consumer does not need to call `GetEntity` to reconstruct state;
it has the event fields. The EM board should show the individual aggregate
events (e.g. `command-slice.updated`) not the transport message.

If a future integration published only `{ "kind": "COMMAND_SLICE", "id": "...",
"revision": 7 }` -- Case 2 -- the consumer would have to call `GetEntity`
to learn what changed. In that case the EM board would model a
"change-notification-received" event on an integration swimlane, followed by
a fetch-and-react command, keeping the gap between notification and state
visible.

## Rule of thumb

> Ask: can the downstream command be executed using only the message payload?
> If yes, the message carries the event (Case 1) -- show the event on the
> board, hide the transport. If no, the message is a notification (Case 2) --
> show the fetch-then-act as explicit steps and make the read visible.

> One pub/sub topic is never one EM stream unless it carries exactly one
> aggregate's events. When in doubt, identify the aggregate first, then find
> which topics carry its events.
