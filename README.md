# Atlas

[![OpenSSF Scorecard](https://api.securityscorecards.dev/projects/github.com/TrogonStack/atlas/badge)](https://scorecard.dev/viewer/?uri=github.com/TrogonStack/atlas)

**Atlas is the shared board where teams and their AI agents design a product end to end.** Personas, screens, sitemaps, storyboards and the system behind them live in one model, with Event Modeling as the backbone that ties them together.

**Atlas keeps that model on a server that every surface reaches the same way.** People write through `trogon-atlas` manifests and review in the Studio, agents write through MCP tools, and all of them go through one gRPC API backed by NATS JetStream. Changes are drafted on branches, validated, reviewed as a diff against the live model, and landed atomically as recorded changesets.

**Atlas exists because a design spread across whiteboards, documents and tickets drifts away from the system it describes.** With the event model as the source of truth, a screen stays tied to the command behind it and the events it produces, so a change shows its impact before it lands. Humans and agents can work at the same time without overwriting each other.

**Atlas is for product teams that design with Event Modeling and want AI agents as real collaborators.** Product engineers, architects and designers share one picture of the product, and platform teams can run their own instance from the Helm charts in this repository.
