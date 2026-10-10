# How to ship a breaking proto change

Use this when `mise run proto:check-breaking` (the `Proto - lint /
breaking` CI job) reports `BREAKING:` findings and the break is intentional.
If it is not intentional, change the proto instead: reserve removed field
numbers and names, add a new field rather than retyping an old one, and keep
JSON names and enum value names stable. See
[wire compatibility](../explanation/wire-compatibility.md) for why the check
is strict.

## Run the check locally

From `atlas/`, with the baseline branch fetched (`mise.toml` pins buf):

```bash
git fetch origin main:main
mise install
mise run proto:check-breaking
```

Set `TROGON_ATLAS_PROTO_BASELINE_REF` to compare against something other than
`main`, such as a release tag.

## Decide what the break costs

For every finding, write down what stops working:

- Stored entities and `Any` annotation payloads that no longer decode, or
  decode into a different meaning.
- Manifests and agent prompts that spell a renamed field or enum value.
- Released trogon-atlas and MCP binaries that send the old shape.
- Studio, which loads its vendored copy of the protos.

If stored data is affected, ship the data migration (a one-off repair
command, or a server-side backfill) in the same pull request or before it.
[Migrating legacy entity fields](migrate-legacy-entity-fields.md) is an
example of such a command.

## Acknowledge each finding

1. Copy every `BREAKING:` line the check printed into
   `rsworkspace/crates/trogon-atlas-proto/acknowledged-breaking-changes.txt`, without the
   `BREAKING: ` prefix. One finding per line, verbatim; the check matches
   lines exactly, so an unrelated later break is never covered by accident.
2. Raise `CONTRACT_REVISION` in `rsworkspace/crates/trogon-atlas-proto/src/lib.rs`.
3. Raise `MIN_CLIENT_CONTRACT_REVISION` to the new revision when older
   clients would send requests the new server misreads, and
   `MIN_SERVER_CONTRACT_REVISION` when the new clients would misread an older
   server. Clients and servers on the wrong side then refuse to mutate with a
   message naming which one to upgrade, instead of corrupting data.
4. Run `mise run proto:generate` and commit the regenerated `src/gen/`
   directories. CI fails when they are out of date with the protos.
5. Rerun the check. Acknowledged findings print as `acknowledged:` and the
   script exits zero.

In the pull request description, state why the break is necessary and which
of the costs above apply. The diff to the acknowledgement file is what
reviewers approve.

## Clean up after merge

Once the change is on the baseline, the check prints each of its lines as a
`stale acknowledgement`. Delete those lines in the next pull request that
touches the protos, so the file only ever lists breaks still in flight.
