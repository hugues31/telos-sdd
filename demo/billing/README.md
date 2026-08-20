# Billing reconstruction demo

This directory is intentionally a spec-only Telos project. It ships no
`Cargo.toml`, application source, test, `telos.lock`, generated site, build
artifact, or hidden solution. Telos produces the ordered, bounded work plan
and verifies the result; an external `telos-implementer` agent writes the
application. The `telos` CLI itself makes no LLM call and does not generate
application code.

Inspect the prerequisite-first plan and its context packs from this directory:

```console
telos rebuild plan --json
telos rebuild status --json
```

Before implementation, create the target project's minimal build manifest so
the configured future `cargo test {filter}` runner is executable, then perform
the one-time bootstrap seal. At this point there is still no application source,
test, or binding:

```console
telos change reconcile --full --json
telos status --json
```

The external implementer then handles each planned intent in order using the
ordinary protocol. It opens a batch, sends a complete byte-equivalent intent
payload through `edit intent` to claim the existing intent, displays the
non-empty operation digest, and approves exactly that digest:

```console
telos change open "rebuild INT-0017" --json
telos edit intent INT-0017 --change CHG-0001 --json < INT-0017.json
telos change diff CHG-0001 --json
telos change approve CHG-0001 --json
```

It writes the smallest discoverable behavioral test containing the literal
`scn_0091` token, records the real failure, writes only the needed application
code, binds that code, reruns the unchanged test to green, and reconciles:

```console
telos test SCN-0091 --json
telos bind src/lib.rs INT-0017 --json
telos test SCN-0091 --json
telos change reconcile CHG-0001 --json
telos rebuild status --json
```

Repeat the same batch for `INT-0042` / `SCN-0107`. Progress is measured by
executing the current proof targets and advances from `0/2` to `1/2`, then
`2/2`; bindings alone never count as green.

Verify and inspect the reconstructed project with the public commands:

```console
telos check --sealed --json
telos rebuild status --json
telos view --port 3000
telos view --export site
```

The repository acceptance test `cargo test -p telos --test rebuild_demo`
performs this complete sequence twice in fresh git repositories and compares
the observable plan, red/green runs, and progress results.
