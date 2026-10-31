# How to protect a published entity

Use this when an entity other teams already depend on, such as a published
event contract, must stop changing until someone decides otherwise. You need
an `admin` token. [Entity protection](../explanation/entity-protection.md)
explains what the server enforces; the
[command reference](../reference/entity-protection-commands.md) lists every
option and output field.

## Protect it

1. Check the identity exists on baseline. Protection covers every version, so
   name it without `@version`:

   ```sh
   trogon-atlas protection get event/orders/placed
   ```

2. Preview, then protect, with the reason refused writers will see:

   ```sh
   trogon-atlas protection protect event/orders/placed \
     --reason "published to billing" --validate-only
   trogon-atlas protection protect event/orders/placed \
     --reason "published to billing"
   ```

   Over MCP, call `protect_entity` with `kind`, `namespace`, `slug` and
   `reason`. `changed: false` means it was already protected; read the
   returned `protection.reason` before assuming it is yours.

Unset `--branch` and `TROGON_ATLAS_BRANCH` first: protection is
baseline-only and the command refuses a branch.

## When a write is refused

A write to the identity fails with `ENTITY_PROTECTED`: the CLI exits `4`,
and an MCP tool error has `data.code` set to `ENTITY_PROTECTED` and
`data.next_action` to `request_unprotect`. The refusal names the identity,
its reason and the admin who protected it. Ask that admin to lift it; do not
retry.

A refusal with `PROTECTION_STATE_UNKNOWN` (CLI exit `75`) means the server
could not read the protection. Retry later, or see
[repair an unreadable row](#repair-an-unreadable-row).

## Lift it

1. Read the current row and note its `revision`:

   ```sh
   trogon-atlas --format json protection get event/orders/placed
   ```

2. Lift only that row, so a protection someone set again in the meantime is
   not removed by mistake:

   ```sh
   trogon-atlas protection unprotect event/orders/placed --if-match 42
   ```

   Exit `3` (`STALE_PRECONDITION`) means the row changed since you read it.
   Read it again and decide again. Over MCP, call `unprotect_entity` with
   `if_match`.

## Audit every protection

```sh
trogon-atlas protection list --namespace orders
```

Exit `11` means the listing is partial: some rows could not be read
(`unknown`), or the server could not enumerate the whole registry
(`complete: false`). Do not treat an identity missing from a partial
listing as unprotected. Over MCP, `list_entity_protections` reports the
same through `verdict`.

## Repair an unreadable row

An `unknown` entry with `liftable: true` has a row the server cannot decode.
Writes to that identity are refused until it is gone. Lift it by its
revision, then protect the identity again if it should stay protected:

```sh
trogon-atlas protection unprotect event/orders/placed --if-match 57
trogon-atlas protection protect event/orders/placed --reason "published to billing"
```

An entry with `liftable: false` has no row revision because the registry did
not answer. Retry the listing later.
