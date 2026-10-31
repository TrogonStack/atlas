# Test a historical client against the current native contract

Use a disposable, isolated checkout containing the acceptance test and Docker
for the real JetStream backend. Reserve its build queue. Do not edit files while
Cargo runs. Keep evidence outside version control.

Set `ATLAS_ACCEPTANCE_SCRATCH` to an existing task-specific disposable directory.
It holds only the copied historical executable. Start from a clean checkout;
commit the acceptance fixture locally first if necessary.

Run from the repository root in a shell with `mise` available:

```sh
set -eu
: "${ATLAS_ACCEPTANCE_SCRATCH:?set a task-specific disposable directory}"
test -d "$ATLAS_ACCEPTANCE_SCRATCH"
test -z "$(git status --porcelain)"
prepared_branch=$(git symbolic-ref --short HEAD)
prepared_sha=$(git rev-parse HEAD)
export ATLAS_HISTORICAL_SOURCE=416a6c7e02034390c53e96f459439e7e1f913dfe
mkdir -p .trogonai/analysis
record="$(git rev-parse --show-toplevel)/.trogonai/analysis/$(date +%s)-historical18.internal.trogonai.md"
trap 'git switch "$prepared_branch"' EXIT
{
  printf 'prepared source: %s\nhistorical source: %s\n' "$prepared_sha" "$ATLAS_HISTORICAL_SOURCE"
  git switch --detach "$ATLAS_HISTORICAL_SOURCE"
  test "$(git rev-parse HEAD)" = "$ATLAS_HISTORICAL_SOURCE"
  grep '^pub const .*REVISION: u32 = 18;' rsworkspace/crates/trogon-atlas-proto/src/lib.rs
  test "$(grep -c '^pub const .*REVISION: u32 = 18;' rsworkspace/crates/trogon-atlas-proto/src/lib.rs)" = 3
  git grep -n ensure_can_mutate -- rsworkspace/crates/trogon-atlas-client
  (cd rsworkspace && mise exec -- cargo build -p trogon-atlas-cli)
  export ATLAS_HISTORICAL_CLI="$ATLAS_ACCEPTANCE_SCRATCH/client18"
  cp rsworkspace/target/debug/trogon-atlas "$ATLAS_HISTORICAL_CLI"
  export ATLAS_HISTORICAL_SHA256=$(shasum -a 256 "$ATLAS_HISTORICAL_CLI" | cut -d ' ' -f 1)
  printf 'historical binary sha256: %s\n' "$ATLAS_HISTORICAL_SHA256"
  git switch "$prepared_branch"
  test "$(git rev-parse HEAD)" = "$prepared_sha"
  mise exec -- cargo test --manifest-path rsworkspace/Cargo.toml -p trogon-atlas-cli --test historical_native_compat -- --ignored --nocapture
} > "$record" 2>&1
rm "$ATLAS_HISTORICAL_CLI"
trap - EXIT
```

Retain the commands along with the record. If a command fails, inspect the record,
restore the prepared branch, and remove the copied executable after investigation.
Never substitute a modern binary with an edited revision constant. No target
directory overrides are needed. The source identity and SHA-256 inputs attest the
locally built artifact through this procedure; they are not signed provenance.

The ignored test must be explicitly executed. A default suite reporting it ignored
provides no historical acceptance evidence. Missing inputs, a wrong source identity,
a hash mismatch, and an incompatible executable fail the explicit run.

## Assess the assertions

`historical_18_apply_refuses_native_current_without_writes` seeds a Specification and
Schema using current CLI apply through BearerAuth and AuthzLayer over NatsStore.
The native binding contains ordered `Blue`, `green` environments and explicitly
present `isDefault: false`. It exports returned member identities before proposing
a title edit, so the proposal has valid caller intent.

- The exact historical error identifies client 18, required revision 20, and the
  instruction to upgrade this client. This exercises historical `ensure_can_mutate`
  through the CLI's public apply command.
- The recorded HTTP RPC paths equal only `GetServerInfo`. No mutation RPC reaches
  the stack. The recorder forwards requests unchanged and does not replace auth.
- Full stored entity equality includes UID, member and binding IDs, native field
  presence and ordering. The stored etag and global change sequence also stay equal.
- The current CLI applies the identical proposed manifest successfully. Stored title
  changes to `After`, etag changes, and the sequence advances. Member/binding fields,
  environments, explicit default presence, and UID remain intact. The recorded
  paths include `BatchMutate`.

This covers native schema variants on baseline apply. It does not establish Product
association, branch, deployed-server, packaged-release, or other historical-client
coverage. Raw direct RPC callers are outside the historical CLI gate being tested.
