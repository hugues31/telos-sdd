# telos-sdd

Specification-driven development where the spec is a typed, queryable
database — not prose a human has to keep in sync with the code by hand.

telos-sdd keeps a typed, referentially-integral base of intents — notions,
intents, scenarios, constraints — versioned in git, as a project's sole
source of truth. Code is one possible solution to that base, never the
other way around. The goal-state test: delete `src/` and rebuild a project
that is fully conformant — every scenario green, every constraint satisfied
— from the intent base alone.

The `telos` CLI is the only legitimate write path into the base: it
validates referential integrity on every mutation and seals coherent states
by git blob hash. Nobody, human or agent, edits `telos/*.tel` by hand.

## Status

v0.7 M1 (engine), M2 (transactions), and M3 (agent workflow) are complete —
M4 view and M5 rebuild proof upcoming.

The three acceptance loops from the spec (§14) are the project's executable
roadmap: **feature** (open → challenge → approve → red/green → reconcile →
`coherent`), **drift** (out-of-protocol edit → `drifted` → `adopt` → same
loop → `coherent`), **merge** (two sealed branches → lock conflict →
`reconcile --full` → `coherent`). They live in
[`crates/telos/tests/acceptance_loops.rs`](crates/telos/tests/acceptance_loops.rs)
and are un-ignored one at a time as the milestone implementing their last
command lands. List what is still ignored:

```sh
cargo test --workspace -- --ignored --list
```

M1 ships the `.tel` parser, the in-memory model with full referential
integrity, the lock/seal engine (git blob OIDs), and the read/query CLI
surface: `telos version | init | status | check [--sealed] | show | list |
query | impact`, all behind a frozen `--json` envelope.

M2 ships the write path: the change transaction (`telos change
open|list|diff|approve|reconcile|abandon`), staged mutations (`add`, `edit`,
`remove`) validated against an overlay before anything touches disk,
digest-bound approvals, drift capture and restore (`adopt`, `revert`), and
`change reconcile --full` — the way out of a merge-conflicted lock, and the
way to seal a pre-existing spec tree. The CLI contracts are frozen in
[`docs/contracts.md`](docs/contracts.md).

M3 adds the bounded agent work pack (`telos context INT-…`), journalled
implementation evidence (`telos test`, `telos bind`), strict or advisory
red/green witness enforcement at reconcile, and optional host integrations
from `telos init --agents`. Context is intentionally intent-sized and
portable: an agent receives one intent’s scenarios, used notions, applicable
constraints, bindings, and one-hop neighbours — never the whole spec.

## Quickstart

```sh
cargo build --workspace
cd your-repo
telos init --agents codex
telos status
```

## M3 workflow

```sh
# Inspect the bounded pack before implementing one approved change.
telos context INT-0042 --json

# Start with a red witness, implement, then record green evidence.
telos test SCN-0108 --file tests/billing.rs --json
telos bind src/billing/invoice.rs INT-0042 --json
telos test SCN-0108 --json
telos change reconcile CHG-0001 --json
```

`telos test` records red when the configured runner exits non-zero; that is
evidence, not a failed CLI command. In strict projects reconciliation requires
an intact red/green witness. `bind` and `test` journal evidence into the
approved change and leave its reviewed delta digest fresh.

`telos init --agents claude,codex` installs the same `telos`,
`telos-challenger`, and `telos-implementer` skills for the selected hosts and
their preventive guards. The guard blocks direct agent edits under `telos/`;
use the CLI for spec mutations. Before approval, the challenger presents the
`change diff` digest; for adopting or reverting, the router presents the
relevant drift paths. The native prompts themselves are static confirmations.

## Docs

- Design spec: [`docs/specs/2026-08-19-telos-sdd-design.md`](docs/specs/2026-08-19-telos-sdd-design.md)
- CLI contracts — `--json` envelope, error codes, `status` schema:
  [`docs/contracts.md`](docs/contracts.md)

## License

MIT — see [`LICENSE`](LICENSE).
