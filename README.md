# Atlas

[![OpenSSF Scorecard](https://api.securityscorecards.dev/projects/github.com/TrogonStack/atlas/badge)](https://scorecard.dev/viewer/?uri=github.com/TrogonStack/atlas)

**Atlas is the shared board where teams direct AI agents to design a product end to end.** Personas, screens, sitemaps, storyboards and the system behind them live in one model, with Event Modeling as the backbone that ties them together. People inspect the model, communicate intent, and review the work; agents perform the tasks.

**Atlas's backend is the source of truth; every client presents a projection.** Human-facing interfaces are read-only tools for inspection and communication with AI. Agents apply changes through `trogon-atlas` or the MCP tools the server serves at `/mcp`, using one API backed by NATS JetStream. Changes are drafted on branches, validated, reviewed as a diff against the live model, and landed atomically as recorded changesets.

**Atlas exists because a design spread across whiteboards, documents and tickets drifts away from the system it describes.** With the event model as the source of truth, a screen stays tied to the command behind it and the events it produces, so a change shows its impact before it lands. Humans and agents can work at the same time without overwriting each other.

**Atlas is for product teams that design with Event Modeling and want AI agents to carry out the work.** Product engineers, architects and designers share one model through views suited to their roles and preferences. Connected clients support people and multiple agents working together, while the backend validates and exposes the model's evolving state.

To try Atlas, [run it locally with Docker Compose](docs/tutorials/run-atlas-locally.md) and load the example model.

Read the [Atlas principles](docs/explanation/principles/README.md) for the ideas behind the product, its modeling language, and its codebase. The [documentation](docs/README.md) connects those ideas to design explanations, task guides, and technical contracts.
