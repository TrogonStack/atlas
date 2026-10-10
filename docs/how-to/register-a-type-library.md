# How to register a type library and point an event at it

This guide registers a tenant protobuf message and makes an Event use it as
its payload. For why types live in the entity store, see
`docs/explanation/tenant-types.md`.

## Write the library manifest

Pick a package prefix the namespace does not already use. That prefix is the
library's `name`, and every file's `package` must start with it.

```yaml
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: TypeLibrary
metadata:
  namespace: shop
  name: shop.orders.v1
spec:
  title: Order types
  files:
    - path: shop/orders/v1/orders.proto
      content: |
        syntax = "proto3";
        package shop.orders.v1;

        message OrderPlaced {
          string order_id = 1;
          int64 total_cents = 2;
        }
```

To import another library's files, list it under `spec.dependencies` with
the same `id` shape every other reference uses.

## Check it before writing

```sh
trogon-atlas apply --dry-run -f orders-types.yaml
```

A dry run compiles the text on the server and reports compiler errors with
their file, line and column. Studio's inspector does the same with its
Compile button, and MCP clients can call `compile_type_library`.

## Register it

```sh
trogon-atlas apply -f orders-types.yaml
```

Add `--branch <name>` to register it on a branch first. The types are then
visible only to writes on that branch until it merges.

## Point the event at the type

Set the event's `schema` to the message in proto3 JSON, with `@type` naming
it. Fill in the fields to record an example payload, or leave them out to
declare the type alone.

```yaml
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata:
  namespace: shop
  name: order-placed
spec:
  title: Order placed
  schema:
    "@type": type.googleapis.com/shop.orders.v1.OrderPlaced
    orderId: o-1
    totalCents: "1250"
```

```sh
trogon-atlas apply -f orders-types.yaml -f order-placed.yaml
```

The library and the event may go in one apply, even in one file. The CLI
reads the namespace's live libraries from the server, compiles the libraries
in the apply on top of them, and writes the libraries ahead of the events in
one atomic batch. `trogon-atlas diff` and `trogon-atlas export` render the
payload the same way, so an exported namespace applies back unchanged.

MCP clients send the same JSON through `put_entity_json` or
`batch_mutate_json`, and `get_entity_json` returns it. gRPC clients build the
Any themselves with `PutEntity` or `BatchMutate`.

The server refuses the event if no live library in its namespace declares
the type, or if the payload does not decode as it. Against a server that
does not advertise type libraries, clients transcode with the built-in types
only, and a tenant `@type` fails with `message not found`.

To confirm the link, list the library's incoming references: the event shows
up with the field `schema`. Studio's inspector lists the message names a
namespace declares under Payload type.

## Change the type later

Adding fields and other wire compatible edits go in a new version of the same
library. An incompatible edit fails with `BREAKING_CHANGE`. Put that shape in
a new package, such as `shop.orders.v2`, as its own library with
`supersedes` set to the old one, then repoint the events. The old library can
be deleted once no schema and no other library uses it.
