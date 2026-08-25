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

The same spec also reads as Cucumber. `telos gherkin` needs no lock, so it
works on this untouched tree:

```console
telos gherkin
```

```gherkin
# telos/features/billing/settlement/INT-0042.feature
@INT-0042
Feature: Invoice payment marks it settled
  Customers must see immediately that their debt is cleared.

  @SCN-0107
  Scenario: full payment settles the invoice
    Given the invoice with state open and balance 120.00 EUR
    When the payment is received with amount 120.00 EUR
    Then the invoice state is settled
```

Nothing there was authored as prose. Each step is `the ` plus the notion's
`phrase` plus that step's own field values, so the sentence cannot disagree
with the typed data behind it. No `.feature` file is committed here: with
`[gherkin] enabled` in `telos.toml`, `change reconcile` writes them under
`telos/features/` and seals them, which makes them a build product rather
than one of this tree's spec owners.

The first full reconcile is an honest spec-only bootstrap. There is no active
behavioral obligation yet, so it runs zero tests and zero checks:

```console
telos change reconcile --full --json
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
telos change open "rebuild INT-0017" --json
printf '%s\n' '{"status":"active"}' | telos edit intent INT-0017 --change CHG-0001 --json
printf '%s\n' '{"check":"cargo test --test invoice_issued domain_does_not_import_adapter_modules -- --exact"}' | telos edit constraint CON-0003 --change CHG-0001 --json
telos change diff CHG-0001 --json
telos change approve CHG-0001 --expected-digest '<digest returned by diff>' --json
```

After approval, the external implementer creates a manifest, application code,
and a test whose discovered name begins with `scn_0091_`. Record a red witness
before implementation, bind every covered code input to `INT-0017`, record a
green witness on unchanged test bytes, and reconcile:

```console
telos test SCN-0091 --json
telos bind '<code path>' INT-0017 --json
telos test SCN-0091 --json
telos change reconcile CHG-0001 --json
telos rebuild status --json
```

Progress must now be `1/2`.

For the second batch, repeat the same reviewed loop for `INT-0042` and a test
whose discovered name begins with `scn_0107_`:

```console
telos change open "rebuild INT-0042" --json
printf '%s\n' '{"status":"active"}' | telos edit intent INT-0042 --change CHG-0002 --json
telos change diff CHG-0002 --json
telos change approve CHG-0002 --expected-digest '<digest returned by diff>' --json
telos test SCN-0107 --json
telos bind '<code path>' INT-0042 --json
telos test SCN-0107 --json
telos change reconcile CHG-0002 --json
```

The executable `CON-0003` check must genuinely reject a domain-to-adapter
dependency. If reconcile reports `TELOS_CONSTRAINT_FAILED`, repair only the
external implementation and reconcile again; do not edit the Telos-owned tree
or the already witnessed test.

Finish by verifying `2/2`, the coherent seal, and the optional view:

```console
telos rebuild status --json
telos check --sealed --json
telos view --port 3000
telos view --export site
```

The repository test `cargo test -p telos --test rebuild_demo` is a
protocol/conformance harness. Its private test fixture models an external
implementer and performs two fresh `0/2 → 1/2 → 2/2` reconstructions through
the public CLI. It demonstrates protocol determinism; it does not claim that
the CLI generated the application.
