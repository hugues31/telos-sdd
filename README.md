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

v0.7 is complete across M1–M5: engine, transactions, agent workflow,
live/static view, deterministic rebuild measurement, GitHub CI generation,
and the public spec-only Billing reconstruction proof.

The three acceptance loops from the spec (§14) are the project's executable
roadmap: **feature** (open → challenge → approve → red/green → reconcile →
`coherent`), **drift** (out-of-protocol edit → `drifted` → `adopt` → same
loop → `coherent`), **merge** (two sealed branches → lock conflict →
`reconcile --full` → `coherent`). They live in
[`crates/telos/tests/acceptance_loops.rs`](crates/telos/tests/acceptance_loops.rs)
and all three run in the ordinary test suite. The ignored-test list is
expected to be empty; verify it with:

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

M4 adds a shared read-only projection rendered either by the foreground
loopback server (`telos view`) or as a deterministic, self-contained sealed
site (`telos view --export`). M5 adds prerequisite-first rebuild plans, real
scenario progress, non-destructive GitHub CI setup, and the public Billing
proof under [`demo/billing`](demo/billing).

## Quickstart

```sh
cargo build --workspace
cd your-repo
telos init --agents codex
telos status
```

Initialize agent integrations and the GitHub sealed-state workflow together:

```sh
telos init --agents claude,codex --ci github
```

This creates `.github/workflows/telos.yml` only when that path and its parents
are safe; it never replaces an existing workflow. The generated job installs
the release tag `v0.7.0` and runs `telos check --sealed`. Publish that tag
before relying on generated CI, and configure GitHub branch protection if job
`sealed` must be required for merges. If publication is interrupted after the
authenticated init marker is written, rerun the exact same `--agents`/`--ci`
options to resume safely; different options or foreign bytes are refused.

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
their preventive guards. For Codex, repository configuration is not assumed
active: before relying on the guard or rules, open `/hooks`, review and trust
the repository `.codex` layer, and verify the exact
`telos agent-guard --host codex` hook. Until that review and trust is complete,
treat `.codex/hooks.json` and `.codex/rules/telos.rules` as inactive. Once
active, the guard blocks direct agent edits under `telos/`; use the CLI for
spec mutations. Before approval, the challenger presents the `change diff`
digest and passes the exact displayed token with `--expected-digest`; for
adopting or reverting, the router presents the relevant drift paths and passes
the exact `status` token with `--expected-state`. Missing or stale tokens fail
closed. Direct humans may omit these flags as an explicit compatibility route;
the CLI still binds the mutation to the state first observed and rechecks it at
the write boundary. The native prompts themselves are static confirmations.

## Configuration

Read the complete canonical configuration without requiring a lock:

```sh
telos config --json
```

Configuration writes are ordinary reviewed changes. Send the complete object;
`agents.hosts` is preserved exactly because host installation remains owned by
`init --agents`:

```sh
telos change open "use advisory TDD"
printf '%s\n' '{"code":{"globs":["src/**/*.rs"]},"tests":{"globs":["tests/**/*.rs"]},"test":{"cmd":"cargo test {filter}"},"policy":{"tdd":"advisory"},"agents":{"hosts":["claude","codex"]}}' \
  | telos config --change CHG-0001 --json
telos change diff CHG-0001
telos change approve CHG-0001 --expected-digest '<digest from diff>'
telos change reconcile CHG-0001
```

Only reconcile writes `telos/telos.toml`. The same glob and `agents.hosts`
validators run at staging, approval, reconcile, full reconcile, and sealed
consumers. An approved config edit is global: its effective settings drive
`telos test`/`rebuild status`, and reconcile reruns all applicable constraints
and every distinct proof before writing.

## View

Serve the current readable model on loopback, including coherent, changing,
or drifted state. Port `0` requests a free port and the foreground process
prints its actual URL before serving:

```sh
telos view --port 0 --json
```

Export is stricter: only a coherent sealed project can publish the static
snapshot, and the destination must not exist:

```sh
telos view --export site --json
```

Both modes expose Dashboard, Graph, Intent, Glossary, and Coverage. Exported
HTML is self-contained, uses inline assets, and contains no external URLs.
Export renders and verifies a sibling staging tree, then publishes it with one
atomic no-replace operation; collision or any render/write/finalization error
publishes no Telos-owned destination and preserves the existing owner.

## Rebuild proof

The rebuild surface plans and measures; it never calls an LLM or writes
application code. Plans are deterministic and prerequisite-first, while
status executes every distinct bound proof once globally (including targets
shared by scenarios) and reports a scenario green only when at least one proof
exists and all its proofs pass:

```sh
telos rebuild plan --json
telos rebuild status --json
```

The committed [`demo/billing`](demo/billing) is deliberately spec-only: no
lock, Cargo manifest/lock, source, tests, generated site, build output, hidden
solution, or application template. Both intents begin `draft`, the constraint
is declarative, and the future Cargo runner is configured but has no proof
target to execute. An external `telos-implementer` follows the bounded plan
and writes its own solution. The demo README provides only the reviewed CLI
lifecycle and placeholders for externally chosen code/test paths; it contains
no manifest, source, test, or extractable solution bytes. The repository's
private `rebuild_demo` fixture is a protocol/conformance harness, not evidence
that the CLI generated application code.

The one-time seal uses the real command:

```sh
telos change reconcile --full --json
```

The untouched plan orders `INT-0017` before `INT-0042`; status is `0/2`
without launching a process. Because there is no active intent and no machine
constraint yet, bootstrap returns `tests_run: 0`, `checks_run: 0`, and leaves
progress at `0/2`. The two later batches stage real `draft` → `active` edits;
the first also stages the architecture check and binds all runner inputs.
Unchanged red/green proofs advance `0/2 → 1/2 → 2/2`, followed by:

```sh
telos check --sealed --json
telos rebuild status --json
telos view --port 0 --json
telos view --export site --json
```

Configured test and constraint commands are trusted project code and may have
effects. Test filters are passed to one validated direct executable argument
vector and never reinterpreted by a shell; the displayed command remains D10's
literal diagnostic substitution. Ordinary `telos check` and `telos status` do
not replay constraints; constraint checks run at reconcile, while `rebuild
status` runs scenario proofs only. A seal is refused if any active scenario
lacks a `proves` binding or if active obligations have no nonblank runner. Full
reconcile runs the whole suite once when at least one intent is active and zero
times for a draft-only model.

## Docs

- Design spec: [`docs/specs/2026-08-19-telos-sdd-design.md`](docs/specs/2026-08-19-telos-sdd-design.md)
- CLI contracts — `--json` envelope, error codes, `status` schema:
  [`docs/contracts.md`](docs/contracts.md)

## License

MIT — see [`LICENSE`](LICENSE).
