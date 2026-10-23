# Run Atlas locally with Docker Compose

In this tutorial you start the whole Atlas stack on your machine, load the
example Acme storefront model into it, and look at that model from the
command line, from Studio, and from an AI agent. By the end you have a
running server you can keep experimenting with.

The stack runs with authentication turned off, so keep it on your own
machine.

## Before you start

You need:

- [Docker](https://docs.docker.com/get-started/get-docker/) with Compose v2
  (`docker compose version` prints a version).
- [Git](https://git-scm.com/).
- [mise](https://mise.jdx.dev/getting-started.html), which installs the Rust
  toolchain used to build the `trogon-atlas` command line tool.
- Free host ports 50069, 8788, 8080 and 50070.
- About 4 GB of memory available to Docker.

Atlas does not publish a CLI binary yet, so you build it from the same
checkout.

## Get the code

```sh
git clone https://github.com/TrogonStack/atlas.git
cd atlas
```

Run every command in this tutorial from the root of that checkout.

## Start the stack

```sh
docker compose -f devops/docker/compose/dev/compose.yaml up -d --build
```

The first run compiles the server from source, which takes several minutes.
Later runs reuse the built image.

Compose starts these services:

| Service | What it does | Reachable at |
| --- | --- | --- |
| `trogon-atlas-server` | The Atlas API (gRPC) and the MCP endpoint for agents | `http://127.0.0.1:50069` |
| `trogon-atlas-studio` | Studio, the read-only web view of the model | `http://127.0.0.1:8788` |
| `nats` | NATS JetStream, where the model is stored | internal |
| `spicedb`, `spicedb-db` | Authorization decisions and their Postgres database | internal |
| `spicedb-migrate` | Prepares the SpiceDB database, then exits | internal |

Check that everything is up:

```sh
docker compose -f devops/docker/compose/dev/compose.yaml ps
```

Wait until `trogon-atlas-server` and `trogon-atlas-studio` both report
`healthy`. Studio waits for the server and then warms up, so it is the last
one to get there. `ps` lists only running services, so `spicedb-migrate` is
missing once it has finished.

Open <http://127.0.0.1:8788>. Studio loads, but the store is empty, so there
is nothing to look at yet.

## Install the CLI

```sh
mise trust
mise install rust
mise exec -- cargo install --locked --path rsworkspace/crates/trogon-atlas-cli
```

`mise trust` lets mise read the repository's `mise.toml`, and `mise install
rust` fetches the pinned Rust toolchain. `cargo install` puts `trogon-atlas`
in cargo's bin directory, usually `~/.cargo/bin`. Check that your shell finds
it:

```sh
trogon-atlas --version
```

If the shell reports `command not found`, add the directory that
`cargo install` named in its output to your `PATH`.

The CLI talks to `http://127.0.0.1:50069` by default, which is where the
stack you started is listening.

## Load the demo model

The [example manifests](../../examples/manifests/README.md) describe a
fictional retailer, Acme. Apply the problem space first, then the
storefront, the fulfillment model that builds on it, and the reviews model
that builds on that:

```sh
trogon-atlas apply -f examples/manifests/acme/
trogon-atlas apply -f examples/manifests/acme-storefront/
trogon-atlas apply -f examples/manifests/acme-fulfillment/
trogon-atlas apply -f examples/manifests/acme-reviews/
```

Each `apply` lands its directory as one atomic change and prints `created`
for every entity. Run the same command again and every entity prints
`unchanged`, because the server already matches the files.

## Explore the model

List the namespaces the server now holds:

```sh
trogon-atlas namespaces
```

Print the storefront's structure: its storyboards and slices, and what each
slice connects. The business itself, with its subdomains, lives in the `acme`
namespace:

```sh
trogon-atlas tree acme-storefront
trogon-atlas tree acme
```

Look at one slice in detail, with the entities it refers to and the ones
that refer to it:

```sh
trogon-atlas describe command_slice/acme-storefront/place-order-slice
```

Ask the server whether the model holds together:

```sh
trogon-atlas validate acme-storefront/acme-storefront-model
```

It reports no errors or warnings, because the examples are kept valid. The
informational findings point at places the model could say more.

Now go back to <http://127.0.0.1:8788> and reload. The overview lists Acme
Fulfillment, Acme Reviews and Acme Storefront. Open Acme Storefront: its
board shows the Checkout and Payment storyboards, from adding an item to the
cart to capturing the payment. Acme Fulfillment picks up from the paid
order.

## Connect an AI agent

Agents change the model through the MCP endpoint the server serves at
`http://127.0.0.1:50069/mcp`. Point any MCP client at that URL using its
Streamable HTTP transport. With Claude Code, for example:

```sh
claude mcp add --transport http --scope user atlas http://127.0.0.1:50069/mcp
```

`--scope user` makes the server available to Claude Code in every
directory, not only the one you ran the command from.

Then ask the agent something about the model, such as "List the slices in
the acme-storefront namespace and explain the checkout flow." It answers by
calling the same API the CLI uses, and anything it changes shows up in
Studio.

## Stop the stack

Stop the services and keep the model for next time:

```sh
docker compose -f devops/docker/compose/dev/compose.yaml down
```

To start over with an empty store, also delete the volumes:

```sh
docker compose -f devops/docker/compose/dev/compose.yaml down -v
```

## Next steps

- [Start an event model from scratch](../how-to/start-a-model-from-scratch.md)
  to write your own manifests.
- [Draft and land a new entity version](../how-to/draft-then-land.md) to
  change a model on a branch and review it before it goes live.
- [Issue an API key to a client](../how-to/issue-an-api-key.md) before you
  run Atlas anywhere other people can reach it.
- Read the [Atlas principles](../explanation/principles/README.md) for the
  ideas behind the product.
