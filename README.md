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

v0.7 M1 (engine) complete — M2 transactions, M3 agents, M4 view, M5 rebuild
proof upcoming.

The three acceptance loops from the spec (§14) are the project's executable
roadmap: **feature** (open → challenge → approve → red/green → reconcile →
`coherent`), **drift** (out-of-protocol edit → `drifted` → `adopt` → same
loop → `coherent`), **merge** (two sealed branches → lock conflict →
`reconcile --full` → `coherent`). They are committed today as `#[ignore]`d
end-to-end tests in
[`crates/telos/tests/acceptance_loops.rs`](crates/telos/tests/acceptance_loops.rs),
scripting commands M2/M3 haven't implemented yet, and are un-ignored one at
a time as those milestones land. List them:

```sh
cargo test --workspace -- --ignored --list
```

M1 ships the `.tel` parser, the in-memory model with full referential
integrity, the lock/seal engine (git blob OIDs), and the read/query CLI
surface: `telos version | init | status | check [--sealed] | show | list |
query | impact`, all behind a frozen `--json` envelope.

## Quickstart

```sh
cargo build --workspace
cd your-repo
telos init
telos status
```

## Docs

- Design spec: [`docs/specs/2026-08-19-telos-sdd-design.md`](docs/specs/2026-08-19-telos-sdd-design.md)
- CLI contracts — `--json` envelope, error codes, `status` schema:
  [`docs/contracts.md`](docs/contracts.md)

## License

MIT — see [`LICENSE`](LICENSE).
