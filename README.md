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

v0.7 M1 (engine) and M2 (transactions) complete — M3 agents, M4 view, M5
rebuild proof upcoming.

The three acceptance loops from the spec (§14) are the project's executable
roadmap: **feature** (open → challenge → approve → red/green → reconcile →
`coherent`), **drift** (out-of-protocol edit → `drifted` → `adopt` → same
loop → `coherent`), **merge** (two sealed branches → lock conflict →
`reconcile --full` → `coherent`). They live in
[`crates/telos/tests/acceptance_loops.rs`](crates/telos/tests/acceptance_loops.rs)
and are un-ignored one at a time as the milestone implementing their last
command lands. **merge** is green as of M2; **feature** and **drift** wait
on M3's `test`/`bind`. List what is still ignored:

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
