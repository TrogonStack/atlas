# AI performs the tasks; people inspect and direct

Part of the [Atlas principles](README.md).

AI-first means agents must be effective at completing the work. The product
must give them the context, tools, and feedback needed to investigate,
propose, apply, and verify a change. It should not depend on a person
performing manual model edits to finish the agent's task.

Human-facing interfaces are read-only with respect to the model. They are
inspection and communication tools: a person can navigate the graph,
examine a finding, compare a proposal, and communicate a goal or correction
to an agent. Those interactions guide the work; the agent performs the
model mutation through backend tools. This boundary applies to future
interfaces as well as Studio.

The read-only boundary applies to human presentation surfaces. Agent tools
such as CLI and MCP must retain mutation capabilities, governed by backend
authorization and validation. Discoverable capabilities, exact graph
context, structured results, and actionable refusals make those tools
effective. See the [MCP contract](../../../rsworkspace/crates/trogon-atlas-mcp/README.md),
[wire compatibility](../wire-compatibility.md), and
[repair workflow](../../how-to/repair-validation-findings.md).
