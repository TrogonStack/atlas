# Temporal and Event Modeling

> A Temporal **activity** is NOT an Event Modeling **command**.
> One activity often hides many commands.

## Primitives

| Temporal                | What it is at runtime                                                           |
|-------------------------|---------------------------------------------------------------------------------|
| Workflow                | A durable, replay-safe state machine. One `workflow_id` per execution.          |
| Activity                | One unit of code executed inside a workflow. Retried with its own timeouts.     |
| Signal                  | An out-of-band message into a running workflow.                                 |
| Query                   | A read against the workflow's current state (synchronous, no side effects).     |
| Schedule                | A cron / interval that starts workflows.                                        |
| Side effect / SDK call  | Anything the activity uses to interact with the outside world.                  |

## What each maps to in Event Modeling

| Temporal                | EM equivalent                                                                                                                       |
|-------------------------|-------------------------------------------------------------------------------------------------------------------------------------|
| Workflow                | A SWIMLANE. The `workflow_id` is the stream id. `Swimlane.transitions` describe the workflow's lifecycle state machine.             |
| Activity                | **NOT a command.** Often a wrapper around multiple commands. See the trap.                                                          |
| Activity input          | Some becomes a command's payload, some becomes the trigger AutomationSlice's source-RM data. The runtime mixes both freely.         |
| Activity output         | Maybe one event's data, maybe many events emitted over time. Each "state change" in the activity is its own event in EM terms.      |
| Signal                  | A COMMAND. The signal IS a request that changes the workflow's state.                                                               |
| Query                   | A READ MODEL. Or part of one: queries return projections of the workflow's history.                                                |
| Schedule                | An AutomationSlice triggered by a clock RM (an external-source RM representing time).                                               |
| Side effect             | Not an EM primitive directly. Each side effect that represents a business-meaningful state change should be its own command+event.  |

## The trap

The temptation:

> One activity = one CommandSlice with N emitted events.

This is right ONLY when the activity is a single atomic act: read inputs, emit
one logical state change, return. The moment the activity loops, calls an LLM
iteratively, makes multiple decisions, or publishes streaming side effects, **it
contains a saga, not one command**.

If you model the activity as one command:

- The validator still passes (especially with `swimlane.state` as the escape
  hatch for derived fields).
- The board looks tidy.
- But the board lies about *when* each event happens. Downstream readers can't
  tell that event B was emitted three seconds and four LLM calls after event A.

## How to spot it

Three code-level signals that an activity is hiding a saga:

1. **A `for` / `while` / `loop` inside the activity that emits events
   per iteration.** Each iteration is its own command-effect; the next
   iteration depends on the previous one's outcome. That is automation
   reading a read model to fire another command, by definition.

2. **The activity calls an external decision-maker mid-execution.** LLMs,
   rule engines, ML inference, human approvals via signals, anything that
   pauses to ask "what should I do next?"; each decision is a new
   command. The activity bundles them; the model should split them.

3. **The activity publishes intermediate events that aren't terminal.**
   "Started", "tool requested", "tool result returned", "finished" are four
   distinct moments of state change. Each deserves a command.

## How to model it

1. Identify the **business-meaningful state transitions** the activity drives.
   Drop the ones that are pure infrastructure (NATS publish, cache writes,
   log lines, tool-server bootstrap, retry attempts).

2. **One command per transition.** Each command lives on the same swimlane
   the activity's events live on, OR on the appropriate aggregate's swimlane
   if the transition is cross-aggregate (then a saga is also needed).

3. **Connect them with AutomationSlices.** Each command's emitted event
   flows into a read model that the next command's automation watches.
   The processor for each automation is the part of the activity that
   does the work between events.

4. The original "activity" disappears as a modelling concept. It becomes
   implementation glue between the commands. The EM model says nothing
   about whether four commands run in one Temporal activity or four;
   that's a deployment / atomicity decision, not a modelling one.

5. Signals into the workflow become commands. Map each signal handler to
   a CommandSlice on the workflow's swimlane.

## Concrete example

Take a Temporal activity, `run_agent_turn`, that drives an LLM agent
loop. One activity, but the loop:

- starts the turn
- builds the per-turn MCP tool server (infrastructure)
- (loop:) the LLM picks a tool
- (loop:) the tool runs and returns
- (loop:) intermediate NATS publishes per delta (infrastructure)
- ... repeat ...
- the LLM emits its final answer
- the turn finishes

The first model treated this as one `CommandSlice`
(`agent-runner--run-agent-turn`) emitting seven events. The validator
passed (all seven on the agent-turn lane; LLM-picked fields covered by
`swimlane.state`). But it lies: those seven events span seconds to minutes,
not one atomic moment.

The honest decomposition: four commands, three automations, one loop edge:

```
[workflow-started-view RM]
    ↓ automation: turn-runner
run-agent (command)
    → agent-turn.streaming-started (event)

[agent-turn-progress RM, when streaming-started OR tool-result.returned]
    ↓ automation: tool-requester
request-tool-use (command, fields: workflow_id, tool_name)
    → tool-use.requested (event)

[tool-use-pending RM]
    ↓ automation: tool-executor
return-tool-result (command, fields: workflow_id, tool_name)
    → tool-result.returned (event)
    ↘ loop edge: tool-result.returned re-feeds agent-turn-progress
                 → tool-requester fires request-tool-use again
                 → until SDK signals done

[last-tool-result RM, when signal=done]
    ↓ automation: turn-finisher
finish-agent-turn (command)
    → agent-turn.finished (event)
```

The infrastructure events (`tool-server.built`, `chat-event.published`,
`chat-event.delivered`) leave the model entirely. They are operational
detail: bytes on wires, not business facts the aggregate cares to record.

## The aggregate-state caveat

`Swimlane.state` exists for fields the aggregate truly owns: generated UUIDs,
derived counters, computed phases, things that genuinely aren't any upstream
command's responsibility. Once you decompose the saga properly, most fields
you were tempted to put in `state` migrate back to being command inputs (the
upstream RM carries them). Reserve `state` for what survives that migration:

- A UUID minted by the activity at execution time (`turn_id` on an agent turn).
- A state machine's current phase that nothing else sets.
- An auto-incrementing counter the aggregate owns.

`tool_name` and `event_kind` were in `swimlane.state` BEFORE the saga
decomposition because they were "the LLM picks one per loop." Once each
loop iteration is its own command, `tool_name` becomes a command field
(sourced from the trigger RM that observes the prior tool-result), and
falls off `swimlane.state` entirely.

## Rule of thumb

> When an Event Modeling board has a CommandSlice that emits more than
> two or three events, ask: are those emissions one moment or many?
> If many, you're looking at a saga the model has collapsed into a
> single command. Decompose it.

> When a Temporal activity has a loop, ask: does each iteration produce
> a business-meaningful state change? If yes, the loop is hidden
> automation. Each iteration is its own command, triggered by the
> previous iteration's event.

## What the validator can and cannot catch

- **Can catch:** the structural invariants: one command writes to one
  stream, every command has a trigger, every event field has a source.
- **Cannot catch:** whether multi-event emissions are atomic. The
  validator sees a CmdSlice with N emitted events and assumes they
  happen together. Temporal collapsing many commands into one activity
  is invisible to it.

So this translation is enforced by review and convention, not by the
validator. The rule of thumb above is the test.
