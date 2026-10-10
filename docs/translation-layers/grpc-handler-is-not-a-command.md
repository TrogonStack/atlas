# gRPC Handler Is Not a Command

> A gRPC **handler** is NOT an Event Modeling **command**.
> The handler is transport glue; the command is the business intent it carries.

## Primitives

| gRPC concept             | What it is at runtime                                                                                |
|--------------------------|------------------------------------------------------------------------------------------------------|
| Service definition       | A protobuf contract declaring RPC names, request types, and response types.                          |
| Handler (server method)  | The server-side function that receives a decoded request, does work, and returns a response.         |
| Request message          | The wire-format input. May carry a mix of business intent, routing hints, auth tokens, and metadata. |
| Response message         | The wire-format output. May bundle the result of multiple operations.                                |
| Unary RPC                | One request in, one response out. The call is synchronous from the client's perspective.             |
| Server-streaming RPC     | One request in, a stream of responses out.                                                           |
| Interceptor / middleware | Cross-cutting logic (auth, logging, tracing) that runs around every handler.                         |

## What each maps to in Event Modeling

| gRPC concept             | EM equivalent                                                                                                                                              |
|--------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Handler                  | **Not a command.** The handler is an infrastructure boundary. It deserializes, validates format, checks auth, then dispatches the real command.             |
| Request message fields   | Some fields become the command payload. Auth/metadata fields do not appear in the command at all; they are transport concerns.                              |
| Business intent in the request | ONE command per business-meaningful state transition the handler drives.                                                                              |
| Response message         | The read model or projection that is returned after the command lands. If the handler returns the mutated entity, that is a read of the event-sourced view.  |
| Server-streaming RPC     | A long-lived subscription, not a command. Maps to a ChangeRecord / StreamChanges read model: the stream delivers events, it does not produce them.          |
| Interceptor              | Not an EM concept. Pure infrastructure.                                                                                                                    |

## The trap

The temptation:

> `PutEntity` is an RPC, so `PutEntity` is the command.

This is wrong. `PutEntity` is the transport name. The command is the business act:
*"Replace the stored definition of this event-model entity."* The handler for
`PutEntity` also:

- validates the request format (not a state transition)
- checks the auth token (not a state transition)
- runs scenario validation (a guard, not a transition)
- calls `store.put()` (the real state transition)
- invalidates the snapshot cache (infrastructure side effect)
- fires the git-mirror side effect (infrastructure side effect)

If you model `PutEntity` as one command that emits "entity.stored" AND
"git-mirror.applied" AND "snapshot.invalidated", the board lies. The git
mirror and cache invalidation are infrastructure; only the store write is the
business-meaningful state change.

The more dangerous version: a handler that drives **two** business transitions.
A `RegisterAndActivate` RPC that creates a resource AND immediately activates
it bundles two commands into one wire call. The EM board must split them.

## How to spot it

Three code-level signals that a handler is hiding multiple commands or
non-command logic:

1. **The handler calls more than one store mutation.** Each mutation is
   a separate state transition. One wire call may legitimately carry a
   batch, but each batch element is its own command in EM terms.

2. **The handler's request message contains fields that only make sense
   for routing, auth, or idempotency.** Those fields are transport metadata;
   strip them when mapping to the command payload. The command captures only
   the business intent.

3. **The handler performs validation that can reject the request before any
   mutation.** Format checks, auth checks, and precondition guards are not
   commands. They are the guard before the command. In EM, guards live in the
   AutomationSlice's trigger logic or the CommandSlice's own preconditions,
   not as separate events on the board.

## How to model it

1. Identify the **business-meaningful state transitions** the handler drives.
   A handler that calls one `store.put()` drives one command. A handler that
   calls `store.create()` then `store.update()` in sequence drives two.

2. **Name the command for the business act, not the RPC.** `put-entity` is a
   transport name. `define-command-slice`, `supersede-read-model`, or
   `delete-automation` are business names.

3. **Map request fields to command payload and command trigger separately.**
   Fields the EM model cares about become the CommandSlice's named fields.
   Fields that are pure transport (auth headers, idempotency keys, page
   tokens) do not appear on the board.

4. **The response is a read model, not an event.** The handler returns a
   view of the post-command state. Map that to a ReadModelSlice whose
   projection is sourced by the command's emitted event.

5. **Server-streaming handlers become change-feed read models.** The stream
   delivers change records; each record contains an event. The EM board shows
   the event; the streaming RPC is just how the client learns about it.

## Concrete example from this codebase

`PutEntity` handler in `trogon-atlas-server/src/service.rs`:

The wire call is `PutEntity(PutEntityRequest { entity, validate_only, if_match, create_only })`.

The EM command is one of:
- **create-entity** (when `create_only=true`): a request to mint a new entity.
  The `entity` field is the payload. `validate_only` and `if_match` are transport guards.
- **update-entity** (when `if_match` is set): a request to replace an existing
  entity at a specific ETag. The `entity` and `if_match` are payload.
- **upsert-entity** (when neither flag is set): an unconditional write.

These are three distinct commands with different preconditions and different
failure modes. The single `PutEntity` RPC carries all three; the EM board
separates them.

The handler's auth-token check, scenario validation, snapshot invalidation,
and git-mirror apply are **not** on the board. They are infrastructure wrapped
around the store write.

## Rule of thumb

> If a gRPC handler touches more than one store mutation, or if its request
> message carries fields that have nothing to do with business state, the
> handler is wrapping more than one command. Name the commands for the
> business acts, not for the RPC names.
