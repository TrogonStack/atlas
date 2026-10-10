# How to replan after a stale refusal

Use this when an apply or a conflict resolution is refused because the state
it was planned from moved. trogon-atlas prints `stale, replan:` and exits 3; the MCP
tools fail with `data.category` set to `stale_state`. Nothing was written.
[Stale plans](../explanation/stale-plans.md) explains why the server refuses.

## Replan an apply

1. Read the refusal. It names the entity, the revision your plan expected
   and the revision the server holds now (`absent`, or `null` over MCP,
   means the entity does not exist on that side).
2. Look at what the other writer did before reapplying, because your
   manifest may now undo their change:

   ```sh
   trogon-atlas diff -f manifests/
   ```

   Over MCP, call `diff_manifests` with the same YAML.
3. Update your manifests if the other change should be kept, then apply
   again. The new apply plans from fresh state:

   ```sh
   trogon-atlas apply -f manifests/
   ```

   Over MCP, call `apply_manifests` again. Do not reuse anything from the
   refused call.

## Re-decide a conflict resolution

1. Diff the branch again and find the entry:

   ```sh
   trogon-atlas branch diff my-branch
   ```

   Each entry prints a line such as
   `state: base=12,ours=40,theirs=15`. Over MCP, call `diff_branch` and read
   the entry's `state` object, or use `actual_state` from the refusal's
   `data`.
2. Compare the entry's current `base`, `ours` and `theirs` content with what
   you decided from. If the decision no longer holds, choose again.
3. Resolve with the state you just read:

   ```sh
   trogon-atlas branch resolve my-branch --kind event --namespace shop \
     --slug order.placed --keep-ours \
     --expect-state base=12,ours=40,theirs=15
   ```

   Over MCP, pass the entry's `state` object unchanged as
   `expected_state` to `resolve_branch_entry`.

If the refusal says the branch no longer holds an entry for the key, the
conflict is gone (someone resolved it or rebased the branch). Diff again
before doing anything else.
