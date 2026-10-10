# How to migrate legacy entity fields into `schema`

Use this when upgrading a store written by a server older than contract
revision 8. Those servers stored an Event's, Command's or ReadModel's field
list in an inline `fields` list. That list is retired: its field numbers are
reserved, and current servers only read the `schema` Any. Unmigrated, those
entities would look as if they declare no fields, and the first write to one
of them would drop the old list for good.

To prevent that, a current server checks the store before it serves anything.
A store that still holds a retired list stops the server at startup with:

```text
N stored entity image(s) still use the retired `fields` layout; stop every
trogon-atlas-server process on this store, run `trogon-atlas-server
migrate-legacy-fields` to preview and `trogon-atlas-server migrate-legacy-fields
--apply` to rewrite them, then start this version again
```

This applies to every `--role`, `reader` included. A reader would serve a
model whose field lists look empty, and a client that reads it from a reader
and writes it back through a writer loses the lists all the same.

The store records that it has been migrated in its reserved `_schema` key.
A new, empty store is marked at creation and never needs this migration. A
store that holds data but no retired list is marked the first time a current
server opens it. Once marked, servers older than this version refuse to open
the store, so an old server left running cannot write the retired layout
back.

The `migrate-legacy-fields` subcommand of `trogon-atlas-server` opens the
store even while servers refuse it. It reads the stored bytes directly, so it
sees the retired list that the gRPC API can no longer return. It covers
baseline entities and branch deltas. The revision log is history and keeps
the bytes it was written with.

## Stop writers

Run the migration with no server writing to the store. Every rewrite is a
compare-and-set against the revision it read, so a concurrent write makes
the command stop with an error rather than overwrite it; rerun it once the
writer is gone.

## Preview the changes

Point the subcommand at the store the same way the server is configured
(`--store`, `--nats-url`, `--nats-bucket`, or their `TROGON_ATLAS_*`
environment variables). Without `--apply` it only reports and never marks
the store. Pass `--role reader` so the preview never competes for the writer
lease:

```bash
trogon-atlas-server --role reader migrate-legacy-fields
```

Each entity image that would change prints on its own line, followed by a
count of every image seen, by shape:

```text
would rewrite baseline command.acme.place-order.1 (both-equal)
would rewrite baseline event.acme.order-placed.1 (fields-only)
CONFLICT baseline event.acme.order-refunded.1: fields and schema disagree, left untouched
would rewrite branch feature.event.acme.order-placed.1 (base) (fields-only)
would rewrite branch feature.event.acme.order-placed.1 (ours) (fields-only)
summary: empty=1 schema-only=1 fields-only=3 both-equal=1 both-different=1
dry run: nothing written; pass --apply to write
```

| Shape | Meaning | What `--apply` does |
| --- | --- | --- |
| `not-field-bearing` | A kind that never had `fields` | Nothing |
| `empty` | An Event, Command or ReadModel with neither | Nothing |
| `schema-only` | Already migrated | Nothing |
| `fields-only` | Only the retired list | Packs it into an anonymous `Schema` in `schema` |
| `both-equal` | Both, declaring the same fields | Drops the retired list |
| `both-different` | Both, disagreeing | Nothing; reported as `CONFLICT` |

## Apply them

```bash
trogon-atlas-server migrate-legacy-fields --apply
```

A branch delta whose base pointed at a baseline entity this run rewrote is
moved to the new revision, so the branch does not see the migration as a
baseline change.

When the run leaves no retired list behind it marks the store and prints
`store marked migrated; servers can open it`. Start the new servers then.

## Resolve conflicts

A `CONFLICT` row carries both lists, and they disagree. The command leaves it
untouched, prints `store not marked migrated; servers still refuse to open
it`, and exits non-zero, so servers stay off the store until you choose.

Rerun with `--keep-schema-on-conflict` to keep each conflicting entity's
`schema` and discard its retired list:

```bash
trogon-atlas-server migrate-legacy-fields --apply --keep-schema-on-conflict
```

If the retired list was the right one for an entity, read it with the old
server before upgrading, run the command above, then set that entity's
`schema` to the list through the API once the new server is up.

## Confirm

A rerun reports nothing to rewrite, keeps the store marked, and exits zero.
