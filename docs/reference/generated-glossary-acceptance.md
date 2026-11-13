# Generated glossary acceptance

This reference maps the [native contract](generated-glossary-contracts.md) to
executable checks. The [delivery coverage](generated-glossary-gaps.md) retains
the original gap identifiers. Local controlled acceptance establishes the
tested native behavior; hosted CI, deployed operation and domain-expert
evaluation require their own receipts.

## Executable assertion map

| Surface | Maintained checks | Assertions |
| --- | --- | --- |
| Core | [glossary tests](../../rsworkspace/crates/trogon-atlas-core/tests/glossary.rs) | Generated-entry admission, exact source bytes/UID, canonical candidate identity, review binding, calendar policy, reference traversal, scoped comparison and bounded UTF-8 usage findings. |
| Backend | [main fixture](../../rsworkspace/crates/trogon-atlas-server/tests/glossary_backend.rs) | Current accepted version selection, draft isolation, ordinary-write and Agent review refusal, source incarnation, selected-source refresh and pre-mutation reply limits. |
| Backend boundaries | [boundary fixture](../../rsworkspace/crates/trogon-atlas-server/tests/glossary_boundaries_backend.rs) | Atomic forged-review refusal, accepted immutability, preview, audience families, stale etags/page tokens, restricted visibility, safe deletion, branch landing, source recreation, retirement and shared traversal budgets. |
| Lifecycle, policy and refresh | [lifecycle acceptance](../../rsworkspace/crates/trogon-atlas-server/tests/glossary_lifecycle_acceptance.rs) | Explicit rejection, retirement after changed/deleted evidence or a deleted replacement target, safe deletion and incoming-reference protection, self/missing/cyclic replacement refusal, and exact preservation of expert corrections and publication after failed or partial refresh. |
| Missing evidence | [missing-evidence budget](../../rsworkspace/crates/trogon-atlas-server/tests/glossary_missing_evidence_budget.rs) | Missing-source lookups count toward the shared traversal budget, including exact-limit success and overflow refusal. |
| CLI | [connected CLI tests](../../rsworkspace/crates/trogon-atlas-cli/tests/glossary_cli_integration.rs) | Native manifest/source roundtrip, review, published index, exact retrieval, comparison, usage and refresh, malformed-request exit and exact supplied code bytes. |
| History and export | [CLI history acceptance](../../rsworkspace/crates/trogon-atlas-cli/tests/glossary_history_cli_integration.rs) | Correction/review history through authenticated gRPC, native CLI export, unchanged reapply, exact source retention and refusal to replay review authority into a new incarnation. |
| MCP | [connected MCP test](../../rsworkspace/crates/trogon-atlas-mcp/tests/glossary_mcp_integration.rs) | Real HTTP MCP authoring, NEEDS_CHANGES correction, stale receipt, explicit acceptance, compact retrieval, scoped policy, context comparison and changed-source refresh. Reader discovery excludes review mutation. |
| MCP discovery | [scope tests](../../rsworkspace/crates/trogon-atlas-mcp/tests/glossary_scope_mcp_integration.rs), [prompt test](../../rsworkspace/crates/trogon-atlas-mcp/tests/glossary_prompt.rs) | Namespace-filtered search, opt-in latest versions and discoverable source/review/refresh procedure. |
| Studio | [page tests](../../tsworkspace/webapps/studio/src/components/GlossaryPage.test.tsx), [HTTP tests](../../tsworkspace/webapps/studio/src/lib/glossary-api.test.ts), [bridge tests](../../tsworkspace/webapps/studio/server/glossary-bridge.test.mjs) | Explicit context, exact identity, stale-request isolation, source status distinct from review, modeled-usage substitution refusal, branch forwarding and correction handoff. |
| Authenticated browser | [native Playwright test](../../tsworkspace/webapps/studio/e2e/glossary.native.spec.ts) | Real Rust/NATS, bridge and Chromium; anonymous refusal, read-only model boundary, exact published/draft/source/usage inspection, published-only filtering and credential-free correction context. |

Core and transport bounds are described in the contract. Limit tests assert
explicit refusal rather than silent truncation or an unchecked clean result.
The browser's controlled User review is a fixture assertion, not a claim of
physical human review or subject-matter expertise.

## Running the checks

From the repository root, using the configured toolchain:

```sh
mise exec -- cargo test --manifest-path rsworkspace/Cargo.toml -p trogon-atlas-core --test glossary
mise exec -- cargo test --manifest-path rsworkspace/Cargo.toml -p trogon-atlas-server --test glossary_backend --test glossary_boundaries_backend --test glossary_lifecycle_acceptance --test glossary_missing_evidence_budget
mise exec -- cargo test --manifest-path rsworkspace/Cargo.toml -p trogon-atlas-cli --test glossary_cli_integration --test glossary_history_cli_integration
mise exec -- cargo test --manifest-path rsworkspace/Cargo.toml -p trogon-atlas-mcp --test glossary_mcp_integration --test glossary_scope_mcp_integration --test glossary_prompt
mise exec -- cargo test --manifest-path rsworkspace/Cargo.toml -p trogon-atlas-client --lib manifest_schema::tests::committed_schema_file_matches_generator
```

The ignored `glossary_native_fixture` keeps a backend alive for controlled browser
and independent-agent runs. Ordinary test execution intentionally skips that
fixture; the native Playwright test invokes it explicitly.

From `tsworkspace`, run the focused Studio checks:

```sh
mise exec -- pnpm --filter trogon-atlas-studio exec vitest run src/components/GlossaryPage.test.tsx src/components/OverviewPage.test.tsx src/lib/glossary-api.test.ts server/glossary-bridge.test.mjs src/App.test.tsx
mise exec -- pnpm --filter trogon-atlas-studio run typecheck
```

For the native browser run, create a fresh evidence directory below the repository
`.trogonai/` directory. Set `ATLAS_NATIVE_EVIDENCE_DIR` to that absolute path and
`ATLAS_GLOSSARY_FIXTURE_TOKEN` to the controlled fixture Reader credential through
the child environment. Do not print or persist the credential. From
`tsworkspace/webapps/studio`:

```sh
mise exec -- pnpm exec playwright test --config playwright.native.config.ts e2e/glossary.native.spec.ts
```

The harness retains assertion results, exact API responses, source revision,
browser errors and child cleanup status. Do not overwrite an earlier run's
receipt directory. A browser response does not replace a backend mutation or
history receipt.

## Connected agent procedure evidence

The controlled source corpus was applied through the real CLI. An authoring
agent inspected pinned material, proposed contextual Terms and retained a run
Document. Generated entries began as DRAFT with current evidence and an empty
published selection. Blank-definition and unsupported-quote writes refused.

A synthetic User then recorded explicit guarded acceptance. A separate Reader
agent started with a compact index and selectively fetched exact Parcel and
readiness definitions. It did not read the extraction plan or fetch the entire
source corpus. It preserved these meanings:

| Exact entry | Audience | Retrieved meaning |
| --- | --- | --- |
| `glossary-fulfillment/parcel@1` | internal | A shipment handled by fulfillment. |
| `glossary-customer/parcel@1` | customer | A package a customer can track. |
| `glossary-fulfillment/ready-to-ship@1` | internal | Accepted payment and completed packing are required. |
| `glossary-customer/ready-to-ship@1` | customer | The package awaits handoff to a carrier. |
| `glossary-fulfillment/ready-to-ship-code@1` | code | The predicate requires both `payment_accepted` and `packing_complete`. |

All selected entries were published with current source evidence. Recorded
review reasons explicitly identified synthetic protocol acceptance. The code
definition supports the internal readiness conditions in this supplied snapshot;
the customer status remains a separate meaning. A lexical difference alone does
not establish a domain conflict.

## Measurements and receipt limits

The independent Reader's successful local run returned an index of eight entries
in 3,587 stdout UTF-8 bytes and about 0.063 seconds. Selective exact retrieval of
the five needed definitions used 9,997 bytes in about 0.313 seconds in total.
These are controlled-corpus CLI measurements, not production latency guarantees
or model-token counts.

An earlier consumer attempt used an incorrect request shape and then encountered
an expired fixture. Both failures were retained. The successful run reconstructed
fresh source and Term incarnations, obtained new synthetic review decisions and
used fresh Reader requests. Historical review UIDs and etags were not copied.

Private run receipts retain commands, exits, hashes, exact revisions and
before/after content. Product documentation names maintained checks rather than
embedding credentials or machine-specific receipts. Source traceability,
authenticated review authority and domain truth remain separate assertions.

## Deployment evaluation

The native implementation provides the tools for these evaluations. Record
deployment-specific results before adopting a generated glossary for that use:

| Evaluation | Required evidence |
| --- | --- |
| Domain quality | Use a representative authorized corpus with named domain experts. Retain corrected and rejected definitions, conflicting passages, unresolved meanings, missed specialized vocabulary and separate organizational, code and customer interpretations. Record actual review decisions through authenticated User operations. |
| Consumption and scale | Name the consuming model and tokenizer revision when measuring token cost. Record compact-index and selective-definition tokens, request counts and latency, source/output limits, and Studio paging and keyboard navigation over representative context sizes. Keep these results alongside exact input revisions and environment details. |

The controlled synthetic review and local byte measurements above remain useful
regression receipts; each deployment supplies its own expert and scale evidence.
