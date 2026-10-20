# Atlas principles

Atlas is an AI-first, multiplayer product built around Event Modeling.
People communicate intent, inspect the model, and direct the work. AI
agents carry out tasks against a shared backend, while interfaces give each
participant an appropriate view of the same information.

These principles govern product and codebase decisions. The linked
contracts explain current mechanisms and limits; the principles also set
direction for capabilities that still need to be built. They do not imply
that every desired interface, profile, or agent workflow already exists.

Atlas is currently greenfield: breaking schema/API changes and recreating
development data are acceptable. Prefer the intended model over legacy
compatibility layers. See the [development policy](../wire-compatibility.md#current-development-policy).

## Principles

- [Backend authority](backend-authority.md): Keep authoritative state and rules in the backend.
- [Connected clients](connected-clients.md): Require service connectivity while allowing local hosting and local AI models.
- [AI-first work](ai-first.md): Agents execute tasks; people inspect the model and communicate intent.
- [Layered information](layered-information.md): Work with relationships and make incomplete or broken graphs inspectable.
- [Multiplayer](multiplayer.md): Support people, individual agents, and agent swarms working concurrently.
- [Multiple roles](multiple-roles.md): Adapt views to roles and preferences while preserving a complete, coherent model for every affected role.
- [Event Modeling](event-modeling.md): Anchor every layer in the model's behavioral primitives.
- [Code is the truth](code-is-the-truth.md): Once a slice is implemented the model follows the code; before that it may run ahead as a proposal.
