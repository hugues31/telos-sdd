# Billing reconstruction demo

This directory is intentionally a spec-only Telos project. It contains no
manifest, dependency lock, application source, test source, generated checker,
build artifact, or hidden solution. Both intents start as `draft`. Telos
supplies a prerequisite-first bounded plan and verifies the result; an external
implementer writes the application and tests without ever writing below
`telos/`. The deterministic CLI makes no LLM call and generates no application
code.

Inspect the untouched demo and its initial `0/2` progress:

```console
telos rebuild plan --json
telos rebuild status --json
```

Explicit initialization observes the spec-only bootstrap. There is no active
behavioral obligation yet, so it runs zero tests and zero checks:

```console
telos init --from-spec --json
telos status --json
```

## External implementation workflow

Give the full output of `telos rebuild plan --json` to a trusted external
implementer (for example, an agent using the generated `telos-implementer`
skill). The implementer should handle one plan step at a time, write only
normal repository files outside `telos/`, and discover its own design from the
context pack. No solution bytes are provided here.

For the first batch, activate `INT-0017` and make the declarative architecture
constraint executable:

<!-- intent-activation:start -->
```json
{"status":"active"}
```
<!-- intent-activation:end -->

<!-- constraint-check-patch:start -->
```json
{"check":"cargo test --test invoice_issued domain_does_not_import_adapter_modules -- --exact"}
```
<!-- constraint-check-patch:end -->

```console
telos plan open "rebuild INT-0017" --json
# Define TSK-001 with the intended source, test, manifest and spec paths.
telos plan edit "$PLAN" < /tmp/billing-plan.json
telos plan task prepare "$PLAN" TSK-001 --json
printf '%s\n' '{"status":"active"}' | telos edit intent INT-0017 --change "$CHANGE" --json
printf '%s\n' '{"check":"cargo test --test invoice_issued domain_does_not_import_adapter_modules -- --exact"}' | telos edit constraint CON-0003 --change "$CHANGE" --json
telos change diff "$CHANGE" --json
telos plan task import "$PLAN" TSK-001
telos plan diff "$PLAN" --json
telos plan approve "$PLAN" --expected-digest '<digest returned by plan diff>' --json
telos plan task start "$PLAN" TSK-001
```

After approval, the external implementer creates a manifest, application code,
and a test whose discovered name begins with `scn_0091_`. Record a red witness
before implementation, bind every covered code input to `INT-0017`, record a
green witness on unchanged test bytes, and reconcile:

```console
telos test SCN-0091 --json
telos bind '<code path>' INT-0017 --json
telos test SCN-0091 --json
telos change reconcile "$CHANGE" --json
telos rebuild status --json
```

Use a scenario validator for the task and a final acceptance review. After
reconciliation, run `telos plan verify "$PLAN" --task TSK-001 SCN-0091`,
`telos plan task finish "$PLAN" TSK-001`, the final validator and
`telos plan complete "$PLAN"`. Progress must now be `1/2`.

The [native plan guide](../../docs/plans.md) supplies the complete definition
schema. The returned `$PLAN` and `$CHANGE` are UUID identities. Include
`Cargo.toml`, `Cargo.lock`, `src/**`, `tests/**` and the affected
`telos/contexts/**` paths in both the plan and task scopes. No implementation
file may be written before approval and task start.

For the second batch, repeat the same reviewed loop for `INT-0042` and a test
whose discovered name begins with `scn_0107_`:

```console
telos plan open "rebuild INT-0042" --json
telos plan edit "$PLAN" < /tmp/settlement-plan.json
telos plan task prepare "$PLAN" TSK-001 --json
printf '%s\n' '{"status":"active"}' | telos edit intent INT-0042 --change "$CHANGE" --json
telos change diff "$CHANGE" --json
telos plan task import "$PLAN" TSK-001
telos plan diff "$PLAN" --json
telos plan approve "$PLAN" --expected-digest '<digest returned by plan diff>' --json
telos plan task start "$PLAN" TSK-001
telos test SCN-0107 --json
telos bind '<code path>' INT-0042 --json
telos test SCN-0107 --json
telos change reconcile "$CHANGE" --json
```

Finish and validate this task and its plan the same way, using `SCN-0107`.

The executable `CON-0003` check must reject a domain-to-adapter
dependency. If reconcile reports `TELOS_CONSTRAINT_FAILED`, repair only the
external implementation and reconcile again; do not edit the Telos-owned tree
or the already witnessed test.

Finish by verifying `2/2`, the coherent seal, and the optional view:

```console
telos rebuild status --json
telos check --sealed --planned --json
telos view --port 3000
telos view --export /tmp/billing-site
```

The repository test `cargo test -p telos --test rebuild_demo` is a
protocol/conformance harness. Its private test fixture models an external
implementer and performs two fresh `0/2 → 1/2 → 2/2` reconstructions through
the public CLI. It demonstrates protocol determinism; it does not claim that
the CLI generated the application.
