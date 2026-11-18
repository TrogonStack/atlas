# How to model runtime code as an Event Model

Turn the units of execution an existing codebase uses (Temporal activities,
gRPC handlers, pub/sub messages) into commands, events, read models, and
automations that say what actually happens at runtime. For why the runtime
unit is not the modelling unit, see
[translation layers](../explanation/translation-layers.md).

## Decompose a Temporal activity

Use this when an activity loops, calls an external decision-maker, or
publishes intermediate events. See
[Temporal and Event Modeling](../explanation/temporal-and-event-modeling.md)
for the signals that an activity hides a saga.

1. **List the business-meaningful state transitions the activity drives.**
   Drop the ones that are pure infrastructure: NATS publishes, cache writes,
   log lines, tool-server bootstrap, retry attempts.

2. **Create one CommandSlice per transition.** Each command lives on the
   swimlane the activity's events live on, or on the appropriate aggregate's
   swimlane if the transition is cross-aggregate (then a saga is also
   needed).

3. **Connect the commands with AutomationSlices.** Each command's emitted
   event flows into a read model that the next command's automation watches.
   The processor for each automation is the part of the activity that does
   the work between events.

4. **Drop the activity from the model.** It becomes implementation glue
   between the commands. Whether those commands run in one Temporal activity
   or several is a deployment and atomicity decision, not a modelling one.

5. **Map each signal handler to a CommandSlice** on the workflow's swimlane.

6. **Move fields out of `swimlane.state`.** Once the saga is decomposed, most
   fields placed in `swimlane.state` become command inputs carried by the
   upstream read model. Keep in `state` only what the aggregate truly owns:
   UUIDs minted at execution time, a phase nothing else sets, counters the
   aggregate increments.

## Split a gRPC handler into commands

Use this when mapping an RPC onto the board. See
[gRPC handler is not a command](../explanation/grpc-handler-is-not-a-command.md)
for why the handler and the command differ.

1. **List the business-meaningful state transitions the handler drives.**
   A handler that calls one `store.put()` drives one command. A handler that
   calls `store.create()` then `store.update()` in sequence drives two.

2. **Name each command for the business act, not the RPC.** `put-entity` is
   a transport name. `define-command-slice`, `supersede-read-model`, or
   `delete-automation` are business names.

3. **Map request fields to the command payload and the trigger separately.**
   Fields the model cares about become the CommandSlice's named fields.
   Pure transport fields (auth headers, idempotency keys, page tokens) stay
   off the board.

4. **Model the response as a read model.** The handler returns a view of the
   post-command state. Map it to a ReadModelSlice whose projection is sourced
   by the command's emitted event.

5. **Model server-streaming handlers as change-feed read models.** The stream
   delivers change records, each containing an event. The board shows the
   event; the streaming RPC is only how the client learns about it.

## Place a pub/sub message on the board

Use this when a subscriber reacts to a message. See
[pub/sub message is not an event](../explanation/pubsub-message-is-not-an-event.md)
for how to tell a delivered event from a notification.

Ask whether the downstream command can run using only the message payload.

### When the message carries the event

1. Place the event on the source aggregate's swimlane.
2. Leave the pub/sub delivery off the board.
3. Model the downstream reaction as an AutomationSlice whose trigger is a
   read model sourced from the event, not from the message.
4. Name the source aggregate's stream and the transport subject differently,
   on the board and in code comments.

### When the message is a notification

1. Model the upstream state change as an event on the source aggregate.
2. Model the downstream reaction as a read model that watches the source
   aggregate (updated through the notification or a poll), followed by an
   AutomationSlice that fires a command when the read model changes.
3. Leave the message off the board. It delivers the read model update; it is
   not the event.
4. If the downstream system cannot observe the source aggregate's stream
   directly, model the notification as an integration event at the boundary:
   an event on a "notification received" swimlane carrying only what the
   notification contained, followed by a command that fetches the rest. That
   makes the read explicit.
