# Where tenant types live

An Event, Command or ReadModel describes its payload in `schema`, a
`google.protobuf.Any`. The Any can hold the built-in `Schema` message, a flat
list of fields, or a real protobuf message a tenant defined. This page explains
where those tenant messages are kept, how the server learns about them, and
why the design avoids an external schema registry.

## The registry is the entity store

Tenant types are entities. A `TypeLibrary` is stored in the same entity store
as every Event and ReadModel, under the same namespace, the same branch scope
and the same change feed. There is no second service to deploy, back up,
authorize or keep in sync, and a branch that edits a library and the events
using it reviews and merges them together.

`id.slug` is the protobuf package prefix the library owns. Every file's
`package` must equal the slug or start with it followed by a dot, and no two
live libraries in one namespace and branch may claim overlapping prefixes.
That makes ownership answerable from a type name alone: the library that owns
`shop.orders.v1.OrderPlaced` is the one whose slug is `shop.orders.v1` or
`shop.orders` or `shop`, and there is at most one.

## Source text is the source of truth

A library stores `.proto` files as plain text, path and content, and nothing
else. There is no stored descriptor set, no generated artifact, and no blob
on the side. Anything derived from the text can be derived again, so storing
it would only add a second copy that can disagree with the first.

`provenance` records where the text came from, such as a Buf module commit
or a git revision. It is a note for people. The server never fetches what it
names: a client pulls the files and sends them as text.

## Compiled on demand

The server compiles a library's text in memory whenever it needs the types:
to check a write, to validate a payload, or to answer a client. The compiled
pools are cached and evicted freely, because recompiling is always possible.
A library can import other libraries in the same namespace through
`dependencies`, and the well-known types are always available.

Clients use the same compiler before they write:

- `CompileTypeLibrary` reports diagnostics as `path:line:col` and any wire
  compatibility violations, without writing anything.
- `ResolveType` returns the library that declares a type and the descriptors
  needed to encode it.
- `GetTypeLibraryDescriptorSet` returns every live type in a namespace along
  with the list of message names, which is what a picker or code generator
  needs.

## Payloads in JSON

Manifests and the MCP `*_json` tools write a tenant-typed `schema` as proto3
JSON with an `@type`. To transcode it, a client asks the server for the
namespace's live descriptors with `GetTypeLibraryDescriptorSet` and layers
them over the built-in ones. Libraries written in the same apply or batch
are compiled on the client against those descriptors, so a type and its
first users can land together. The server still checks every write on its
own; the client only needs enough types to turn JSON into bytes and back.

## What the server enforces

- A write naming a tenant type in `schema` is refused unless a live library
  in the entity's namespace declares that type, and a non-empty `value` must
  decode as that type with no fields the type does not declare.
- A new version of a library must stay wire compatible with the previous
  one. A change that would make stored payloads decode differently fails with
  `BREAKING_CHANGE`.
- A library that another library depends on, or whose types entity schemas
  use, cannot be deleted, even with `mode=FORCE`. The schemas show up as
  references to the library, so impact analysis lists them too.

## Breaking changes become new packages

Compatibility is checked per slug, so an incompatible change cannot be a new
version of the same library. Publish the new shape under a new package, for
example `shop.orders.v2`, as its own library, set `supersedes` to the old
library, and repoint the events. Stored payloads keep their old type URL and
keep decoding, because the old library stays until nothing uses it.

## Why not an external schema registry

The Buf Schema Registry and similar services are good at publishing shared
modules. Atlas does not depend on one at runtime:

- A tenant must be able to define and change a type on the fly, from a string,
  inside a branch, without publishing anything anywhere.
- Every write is validated against the types the store holds at that moment.
  A remote lookup would make validation depend on another service being up
  and on the remote state matching the branch being written.
- Branches, merges, authorization and the change feed already exist for
  entities. A type kept outside the store would need its own copy of each.

A Buf module can still be the origin of a library. Pull it on the client,
send the files as text, and record the module and commit in `provenance`.

## Current limits

- `trogon-atlas fmt` works offline, so it formats only manifests whose
  `schema` uses the built-in types.
- Validators that read field names from `schema` read them only from the
  built-in `Schema` message and the built-in types.
- A schema reference points at the latest version of the owning library.
