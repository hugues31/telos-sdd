# telos CLI contracts

This document is the frozen reference for everything an agent or other tool
routes on without interpretation: the `--json` envelope shape, the 18 error
codes and their canonical hints, the `status --json` schema, `check`'s
semantics, and the whole change/transaction surface (`show`,
`change open|list|abandon|diff|approve|reconcile`,
`add`/`edit`/`remove`, `adopt`, `revert`), including the JSON payload
schemas `add`/`edit` read from stdin. The 0.7 freeze also includes typed
configuration, live/export view, rebuild planning/progress, and resumable
GitHub CI initialization. Nothing here is prose to be summarized by an LLM —
it is matched on literally (`error.code ==
"TELOS_DRIFT_DETECTED"`, `result.state == "drifted"`), the same way a
compiler's exit code is.

Everything below is locked by a test in `crates/telos/tests/`. If this
document and the code ever disagree, the code is the bug — but so is a
future change to the code that isn't reflected here.

## Inspection scope metadata: additive contract revision

`status.result.coverage_scope` and successful `check.result.scope` are new
required metadata in this revision. Existing result fields, counter meanings,
error codes, and the five-key envelope remain unchanged. Consumers that
validate the exact set of result keys must update their schemas; consumers
should allow additional result metadata. This is an explicit extension of
the previously frozen result schemas, not a change to validation or proof gates.

## The `--json` envelope

Every command, whatever it does and however it fails, answers with exactly
five keys, in this order:

```json
{
  "ok": true,
  "command": "status",
  "result": { "...": "..." },
  "error": null,
  "next_actions": []
}
```

- `ok` — `true` on success, `false` on failure.
- `command` — the invoked command's name: one of `"version"`, `"init"`,
  `"config"`, `"map"`, `"status"`, `"view"`, `"check"`, `"show"`, `"list"`,
  `"query"`, `"impact"`, `"pack"`, `"rebuild"`, `"change"`, `"add"`,
  `"edit"`, `"move"`, `"remove"`, `"adopt"`, `"revert"`, `"test"`, `"bind"`. All six
  `change` subcommands (`open|list|abandon|diff|approve|reconcile`) answer
  under the single `"change"` value — the envelope names the command a
  caller invoked, and `telos change …` is one command with subcommands, the
  same way `telos query …` is one `"query"`.
- `result` — the command's payload on success; `null`, never absent, on
  failure.
- `error` — `null`, never absent, on success; on failure, the frozen
  three-key error body below.
- `next_actions` — suggested follow-up invocations, e.g. the token-bound
  `["telos adopt --expected-state sha256:...", "telos revert --expected-state sha256:..."]`.
  Empty, never absent, when there is
  nothing to suggest — always empty on failure.

No key is ever omitted (no `skip_serializing_if` anywhere in the
implementation): a consumer indexes every key unconditionally instead of
checking whether it is there.

### Canonical `command` values

This table is the complete public set in 0.9. Subcommands share their parent
value: both `rebuild plan` and `rebuild status` answer as `"rebuild"`, and
every `change` subcommand answers as `"change"`. The hidden `agent-guard`
host-hook entry point is intentionally excluded: it is not a public JSON
envelope surface.

| Value | CLI surface |
|---|---|
| `version` | `telos version` |
| `init` | `telos init` |
| `config` | `telos config [--change CHG-NNNN]` |
| `map` | `telos map [--change CHG-NNNN]` |
| `status` | `telos status` |
| `view` | `telos view [--port N] [--export DIR] [--open]` |
| `check` | `telos check [--sealed]` |
| `show` | `telos show` |
| `list` | `telos list` |
| `query` | `telos query` |
| `impact` | `telos impact` |
| `pack` | `telos pack` |
| `rebuild` | `telos rebuild plan` or `telos rebuild status` |
| `change` | `telos change ...` (including `approve <id> [--expected-digest SHA256]`) |
| `add` | `telos add` |
| `edit` | `telos edit` |
| `move` | `telos move` |
| `remove` | `telos remove` |
| `adopt` | `telos adopt [--into CHG-NNNN] [--expected-state SHA256]` |
| `revert` | `telos revert [--expected-state SHA256]` |
| `test` | `telos test` |
| `bind` | `telos bind` |

### The error body

```json
{ "code": "TELOS_DRIFT_DETECTED", "message": "...", "hint": "..." }
```

- `code` — one of the stable [error codes](#error-codes) below, as
  `SCREAMING_SNAKE_CASE`.
- `message` — a human-readable, non-localized description. Correctional
  where possible (e.g. `` unknown notion `invoice`; closest is `Invoice` ``)
  rather than merely diagnostic.
- `hint` — an actionable next step, as a string, or `null` — present and
  `null`, never absent, when the error carries none.

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Success. |
| `1` | A domain error: `ok: false`, an `error` body is present. This includes `check` reporting diagnostics — `check` finding problems is `check` doing its job correctly, but the *process* still fails so that `check` is usable as a CI gate. |
| `2` | A usage error (unknown command, bad flag). Produced by `clap` before any command runs; no envelope is printed even in `--json` mode. |

## Error codes

The eighteen codes below are stable. Strict TDD reconciliation uses
`TELOS_SCENARIO_RED_EXPECTED` and `TELOS_TEST_SEALED`; test discovery uses
`TELOS_TEST_NOT_FOUND`. Variants are never renamed or removed, only added —
this is the whole contract agent tooling routes on.

### Canonical error-code set

| Code |
|---|
| `TELOS_DRIFT_DETECTED` |
| `TELOS_APPROVAL_STALE` |
| `TELOS_REFERENCE_UNKNOWN` |
| `TELOS_SCENARIO_RED_EXPECTED` |
| `TELOS_TEST_SEALED` |
| `TELOS_ORPHAN_CODE` |
| `TELOS_CONSTRAINT_FAILED` |
| `TELOS_CHANGE_STATE_INVALID` |
| `TELOS_FILE_CLAIMED` |
| `TELOS_NOT_INITIALIZED` |
| `TELOS_ALREADY_INITIALIZED` |
| `TELOS_PARSE_ERROR` |
| `TELOS_INTEGRITY_VIOLATION` |
| `TELOS_CYCLE_DETECTED` |
| `TELOS_GIT_ERROR` |
| `TELOS_INTERNAL` |
| `TELOS_TEST_NOT_FOUND` |
| `TELOS_TEST_NOT_EXECUTED` |

### Detailed emission cases

| Emission | When | Hint |
|---|---|---|
| `TELOS_DRIFT_DETECTED` | The project's state is `drifted` — *not* merely "not `coherent`": a `changing` project (an open change, nothing unclaimed) does **not** trigger this code, only genuine unclaimed drift does (a sealed path modified or missing, or an unsealed spec file on disk). Emitted by `check --sealed`; it also gates `change open`, `add`/`edit`/`remove`, `change approve`, and `change reconcile` *without* `--full` (`--full` never reads the lock, so it is exempt — see the `change reconcile` section below). `change diff`/`list`/`abandon`, `status`, `check` without `--sealed`, and `show` never gate on it — they read, or they clean up, and a drifted project is exactly when a caller needs them most. | `` run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert` `` |
| `TELOS_APPROVAL_STALE` | `change reconcile`'s digest gate: a change's approval no longer matches its ops digest, because the delta was staged into again (`add`/`edit`/`remove`) after `telos change approve` — staging into an approved change is deliberately allowed. | `` re-approve with `telos change approve CHG-0001` `` (id-carrying, not a bare instruction to re-run `diff`) |
| `TELOS_REFERENCE_UNKNOWN` | A reference in the spec — a notion, an attribute, an enum symbol, or an intent/scenario/constraint id — does not resolve. Emitted by the semantic pass on `load_model` and also rejected at write time (`add`/`edit` payloads and the whole delta a staged change describes). | None. The engine folds its best guess directly into `message` (`` ; closest is `Invoice` ``) when one is close enough; there is nothing to add. |
| `TELOS_REFERENCE_UNKNOWN` | A `show`/`impact` argument, or `query`'s `--using`/`--triggered-by`, is a well-formed id or notion name that resolves to nothing in the loaded spec (message `` unknown notion `Invoice` ``, `` unknown intent `INT-9999` ``, `` unknown scenario `SCN-9999` ``, or `` unknown constraint `CON-9999` ``). | `` closest is `Invoice` `` (edit distance, for a notion name, backtick-quoted) or `closest is INT-0042` (numeric distance, for a typed id, *not* backtick-quoted) — present only when a candidate is close enough; `null` otherwise. |
| `TELOS_REFERENCE_UNKNOWN` | A `show`/`impact` argument is neither a typed id nor a valid notion name at all (message `` cannot parse `x` as an id or notion name ``). | None. |
| `TELOS_REFERENCE_UNKNOWN` | An `impact` argument names a change (`CHG-…`) — a change is a transaction record, not a node of the spec graph, so it has no relations to walk (message `` `impact` does not apply to changes ``). `show CHG-…`, unlike `impact`, *does* resolve — it reads the change store directly rather than the graph; see the `show` section below. | None. |
| `TELOS_REFERENCE_UNKNOWN` | `change abandon`/`change diff`/`change approve`/`change reconcile <id>`/`add\|edit\|remove --change`/`adopt --into` is given a value that does not even parse as a `CHG-NNNN` id — a distinct, earlier check from the next row's "well-formed but unknown" (message `` cannot parse `x` as a change id ``). The same family covers `edit`/`remove`'s `<key>` argument for an intent or a constraint (message `` cannot parse `x` as an intent id `` / `` cannot parse `x` as a constraint id ``) and a notion (message `` cannot parse `x` as a notion name ``) — one dedicated message per expected kind, since the command already knows which kind it asked for. | None. |
| `TELOS_REFERENCE_UNKNOWN` | `show`/`change abandon`/`change diff`/`change approve`/`change reconcile <id>`/`add\|edit\|remove --change`/`adopt --into` name a well-formed `CHG-NNNN` id the store does not hold (message `` unknown change `CHG-9999` ``). | `closest is CHG-0001` (numeric distance) — present only when another change exists; `null` otherwise. |
| `TELOS_SCENARIO_RED_EXPECTED` | `reconcile` under `policy.tdd = "strict"` requires an intact sealed red witness for a scenario before its green run; none exists. | Run `telos test SCN-…` to record a red witness before implementing. |
| `TELOS_TEST_SEALED` | The bytes of a test file sealed as a red witness changed before the scenario went green — the witness no longer proves anything. | The red witness is invalid; run `telos test SCN-…` again on the current bytes before reconciling. |
| `TELOS_TEST_NOT_FOUND` | No `[test] cmd` is configured; discovery finds zero or more than one file containing the scenario's `scn_NNNN` convention; or `--file` names no file. | The exact cases follow this table. |
| `TELOS_TEST_NOT_EXECUTED` | `telos test` with `[test] report` configured: the report is missing, invalid, names no testcase for the scenario, or every such testcase was skipped (message: one of the four sentences in the `test` section). Nothing is journalled; under `--all` the loop stops there. | `` make the runner execute the test named after `scn_NNNN` and write the report, then run `telos test SCN-NNNN` again `` |
| `TELOS_TEST_NOT_EXECUTED` | Gate 8 under strict policy with `[test] report` configured: the scenario's witnesses were taken by exit status (message `` scenario SCN-NNNN's witness was taken by exit status; `[test] report` is configured ``). A warning under advisory. | `` run `telos test SCN-NNNN` again to record a report-backed red and green `` |
| `TELOS_TEST_NOT_EXECUTED` | Gate 11 or `--full` with `[test] report` configured: a run's report does not prove an impacted (respectively active) scenario (message `` the test run for `<target>` did not execute SCN-NNNN: <reason> ``, `<target>` being `the whole suite` under `--full`). | `` run the configured executable with the displayed arguments and inspect the report, then reconcile again `` |
| `TELOS_ORPHAN_CODE` | `change reconcile`'s unbound-code gate, evaluated over the delta's post model: a file matched by `[code]`/`[tests]` globs in `telos.toml` is not covered by any `implements`/`proves` binding (message names which of the two families and the binding relation it's missing). | Bind it with `telos bind <path> <INT-id>`, or remove it from the `telos.toml` globs if it isn't spec-governed. |
| `TELOS_CONSTRAINT_FAILED` | `change reconcile`'s constraint-checks gate: a constraint's `check` shell command exited non-zero, or could not even be spawned (message `` CON-0001 check failed: `<cmd>` ``). The command's own output is deliberately *not* included — it is not reproducible across machines (a git version, a locale, `$PATH`), so it cannot be frozen contract. | Run the constraint's `check` command directly to see its output. |
| `TELOS_CHANGE_STATE_INVALID` | `change reconcile <id>` on a change whose status is not `approved`/`implementing` (message `` change CHG-0001 is not approved; approve it first ``). | `` run `telos change diff CHG-0001` then `telos change approve CHG-0001` `` |
| `TELOS_CHANGE_STATE_INVALID` | `change approve` on a change with no staged ops — `open`, with nothing added yet (message `` change CHG-0001 has no staged operations ``). | `stage operations with telos add\|edit\|remove first` |
| `TELOS_CHANGE_STATE_INVALID` | `adopt`/`revert` run when the project has *not* drifted — both commands exist only to leave the drifted state (message `` nothing to adopt: the project has not drifted `` or `` nothing to revert: the project has not drifted ``). | `` run `telos status` to see the project's state `` |
| `TELOS_CHANGE_STATE_INVALID` | `check --sealed` on a project that is `changing` — "sealed and unmodified" cannot be true while a change is open, and that is a different remedy from drift, hence its own code (message `open changes; reconcile or abandon them`). | `` run `telos change list` `` |
| `TELOS_FILE_CLAIMED` | A file targeted by `add`/`edit`/`remove`, or by `adopt`'s plan, is already claimed by a different, concurrently open change — one file, one change (message `` <path> is already claimed by CHG-0001 `` — the path is **not** backtick-quoted inside the message). | `` reconcile or abandon CHG-0001 first, or work within it `` (id-carrying) |
| `TELOS_NOT_INITIALIZED` | No `telos/telos.toml` found walking up from the current directory. | `` run `telos init` at the repository root `` |
| `TELOS_NOT_INITIALIZED` | `telos/telos.toml` exists, but `telos.lock` is missing (`status`, `check --sealed`). `telos init` always seals, so this is not "unsealed" — it's abnormal. | `` the project was never sealed; run `telos init` in a fresh repository or restore telos.lock from git `` |
| `TELOS_ALREADY_INITIALIZED` | `telos init` run on a project that already has `telos/telos.toml`. | `` project already initialized; see `telos status` `` |
| `TELOS_PARSE_ERROR` | A `.tel` file (or `telos.lock`, or `telos.toml`) is syntactically invalid (`load_model`, `check`, `change diff`'s base parse, …). | None today — `message` names the offending file and, when the parser can determine it, the line and column. **Exception:** `adopt` on a drifted `.tel` file it cannot parse forces this same code but replaces the hint with `ADOPT_PARSE_HINT`; see the `adopt` section below. |
| `TELOS_PARSE_ERROR` | An `add`/`edit` payload on stdin is not a JSON object, or its shape does not match the payload schemas section below (`message` prefixed `` payload: `` — e.g. `` payload: missing required field `title` in intent payload ``). A handful of exact wordings are frozen verbatim without that prefix: an unknown key (`` unknown key `titel` in notion payload ``), an unknown closed-set word (`` unknown attribute type `txt`; expected one of string, int, decimal, money, bool, date, datetime, enum, ref ``), a `decimal` value sent as a JSON number, and a malformed `set` action. | None, except the two grammar cases: an expression field that does not parse (hint names the mini-language grammar; see `add intent` below) and an `enum` symbol that is not lower-kebab-case (hint names the symbol grammar; see `add notion` below). |
| `TELOS_INTEGRITY_VIOLATION` | An integrity violation with no dedicated hint: `seal` finding a binding to a code file that doesn't exist on disk, an entity declared twice, or `remove`/`adopt` leaving a still-referenced entity behind (`cannot remove <entity>: <referrer>`). | None today — `message` names the offending path or entity. |
| `TELOS_INTEGRITY_VIOLATION` | `change reconcile`'s accept-OID gate: an `accept` op's path changed, or vanished, since `adopt` recorded its OID (message `` `<path>` changed since it was accepted `` or `` `<path>` was accepted but no longer exists ``). | `` re-run `telos adopt` to accept the current bytes of `<path>` `` |
| `TELOS_INTEGRITY_VIOLATION` | `change reconcile`'s test gate: the `[test] cmd` run for an impacted scenario's `proves` target (or, under `--full`, the whole suite once when at least one intent is active) failed. Without `[test] report`, that run exited non-zero. With `[test] report` configured, a testcase named after the impacted (respectively active) scenario failed in that run's report (message `` the test run for `<target>` failed: `<substituted cmd>` ``). A runner that cannot be spawned, or a stale report that cannot be removed before the run, is `TELOS_INTERNAL` instead — `run_proof`'s own message — not this code. A full reconcile with only draft/deprecated intents invokes no runner. The command's own stdout/stderr is deliberately not included, for the same reproducibility reason as `TELOS_CONSTRAINT_FAILED`. | `run the configured executable with the displayed arguments, then reconcile again` |
| `TELOS_CHANGE_STATE_INVALID` | `change approve <id> --expected-digest <digest>` reaches a mutation boundary whose live delta digest differs (message `` change CHG-0001 no longer matches the expected digest ``). The check is repeated after validation immediately before the write. | `` run `telos change diff` again and review the new digest `` |
| `TELOS_CHANGE_STATE_INVALID` | `adopt` or `revert` reaches a mutation boundary whose exact sealed lock plus sorted drift paths/kinds differs from `--expected-state` (message `` project drift no longer matches the expected state token ``). | `` run `telos status` again and review the new drift scope `` |
| `TELOS_INTEGRITY_VIOLATION` | An `edit notion` payload changes the notion's `name` — a staged op cannot rename an entity, since the op's target path is derived from the entity's identity (message `` cannot rename notion `<from>` to `<to>` ``). | `` stage `remove notion <from>` and an `add` of the new one instead `` |
| `TELOS_INTEGRITY_VIOLATION` | `adopt` cannot express the deletion of a file that carries no entity of its own: a bound code file (message `` cannot adopt: bound file `<path>` was deleted ``) or an unbound opaque file such as `telos.toml` (message `` cannot adopt: `<path>` was deleted ``). | `` restore it with `telos revert`, or remove its binding `` for a bound file; `` restore it with `telos revert` `` for an unbound one. |
| `TELOS_INTEGRITY_VIOLATION` | `adopt` finds a drifted `.tel` file whose declared entity belongs at another path — adopting it as-is would capture the wrong path and leave the real drift uncaptured (message `` `<path>` declares an entity that belongs in `<declared-path>` ``). | `` rename the file to match the entity it declares, or the entity to match the file `` |
| `TELOS_INTEGRITY_VIOLATION` | `adopt` finds a *missing* entity file whose file name is not a valid identity, so not even its deletion can be expressed (message `` cannot read an entity identity from `<path>` ``). | `` restore `<path>` with `telos revert` `` |
| `TELOS_INTEGRITY_VIOLATION` | `revert` finds a drifted path (`Modified`/`Missing`) the lock does not seal — defensive, since `compute_state` should not be able to produce this (message `` `<path>` is not sealed; there is nothing to restore it from ``). | `` run `telos change reconcile --full` to reseal the project `` |
| `TELOS_CYCLE_DETECTED` | A cycle exists on `requires` or `refines`. | None today — `message` renders the cycle's path (`` INT-0001 → INT-0002 → INT-0001 ``). |
| `TELOS_GIT_ERROR` | `git rev-parse --show-toplevel` failed (most commonly: not inside a git repository). | `` not a git repository; run `git init` `` |
| `TELOS_GIT_ERROR` | The `git` binary itself could not be spawned (missing from `PATH`). | None — `message` names the underlying I/O error. |
| `TELOS_GIT_ERROR` | `revert`'s `git cat-file blob <oid>` fails — the sealed OID names a blob the object store does not hold (a lock sealed by an older `telos`, or an unreachable object git has pruned; message `` `git cat-file blob <oid>` failed: <stderr> ``). Every seal writes its objects (`git hash-object -w`), so a project sealed but never committed is *not* this case. **Not** `TELOS_INTEGRITY_VIOLATION` — a missing blob is git's own diagnosis, not a spec integrity one. | the frozen `MISSING_BLOB_HINT`: `` the sealed content is not in the git object store; commit the sealed state or restore the file by hand `` |
| `TELOS_INTERNAL` | An internal invariant broke — a bug, not a spec or usage problem. | None. |

`TELOS_TEST_NOT_FOUND` has four exact forms. No runner is
`` no `[test] cmd` is configured in telos/telos.toml `` with hint
`` set [test] cmd, e.g. `cargo test {filter}` ``. Zero discovery matches is
`` no file matched by the [tests] globs contains `scn_NNNN` `` with hint
`` name the test after the scenario id (`scn_NNNN_…`) in a file the [tests] globs cover, or pass `--file <path>` ``
(substituting the requested scenario id, for example `scn_0108`). Multiple
matches have the exact template
`` `scn_NNNN` appears in more than one test file: `<path>`, `<path>` ``
(with the sorted matched paths substituted) and hint
`` pass `--file <path>` to pick one ``.
Finally, an explicit absent file is
`` the file passed with --file does not exist: `<path>` `` with a present,
null hint.

## `status`

Reports the project's state against its seal, and a coverage snapshot of the
spec. `status` still reports drift and parse-broken drift rather than failing
on those conditions. It can fail on `TELOS_NOT_INITIALIZED` (no workspace or
lock), `TELOS_GIT_ERROR`, an unreadable lock, invalid sealed configuration,
or a lock produced by an older Telos whose otherwise matching model is not
sealable. In that last coherent-only case it returns the same first
`TELOS_INTEGRITY_VIOLATION`/`TELOS_TEST_NOT_FOUND` as current reconcile; it
never labels active scenarios without proofs and a runner `coherent`.

Order of operations, and why it matters: [`compute_state`] runs *first* and
never parses a `.tel` file — it only compares git blob OIDs — so a corrupted
spec still gets a state answer. Loading the model for `coverage` is
best-effort *after* that: if the spec fails to parse, `coverage` is reported
as all zeros rather than blocking the command. (This is a deliberate choice
where the spec left the case ambiguous — "coverage computed over what
parses" doesn't have an obvious meaning when *nothing* parses.)

### `telos.lock`

Format version `3`: `version`, `tool`, optional `sealed_by`, `spec_digest`,
`proof_evidence` (`"exit-status"` | `"report"`, required), then the `[spec]`
and `[code]` tables. A lock of any other version is `TELOS_PARSE_ERROR` with
hint `` run `telos reconcile --full` to regenerate telos.lock `` — `--full`
never reads the lock, so the hint is always actionable. `init`, a per-change
reconcile (from the effective configuration), and `--full` all write
`proof_evidence`.

### `result` schema

```json
{
  "state": "coherent",
  "changes": [],
  "drift": null,
  "proof_evidence": "exit-status",
  "coverage_scope": {
    "model": "working-tree",
    "includes_open_changes": false,
    "model_loaded": true
  },
  "coverage": {
    "notions": 4,
    "constraints": 1,
    "intents_total": 2,
    "intents_active": 2,
    "scenarios_total": 2,
    "scenarios_proved": 1,
    "intents_implemented": 1
  }
}
```

- `state` — `"coherent"`, `"drifted"`, or `"changing"`. `"changing"` requires
  at least one open change and every current drift claimed by one of them
  (an unclaimed drift outranks it — see `drift` below). It is produced by
  `change open` or `adopt` while their transactions remain open.
- `changes` — open changes, best-effort (an unparseable change file still
  appears, with `status: "open"` and an `abandon` obligation, rather than
  blocking `status`). Contains one item per open change:
  `{"id": "CHG-0007", "status": "implementing", "obligations": ["..."]}`.
  `obligations` is the frozen, status-keyed list of what remains — see the
  `change` section below.
- `drift` — `null` when `state` isn't `"drifted"`; otherwise:
  ```json
  { "paths": ["telos/contexts/billing/notions/Invoice.tel"], "suggestion": "telos adopt", "token": "sha256:..." }
  ```
  `paths` is sorted, and lists every drifted path — modified, missing, or
  unsealed-but-present — without distinguishing which kind (that
  distinction exists internally as `DriftKind`, but the public schema
  exposes paths only). The `token` field authenticates the complete sealed
  spec/code OID tables, the exact sorted `(path, drift kind)` scope, and the live blob
  OID of every present drift entry, so a path whose bytes or kind changes
  receives another token even when the displayed path list is unchanged.
- `proof_evidence` — `"exit-status"` or `"report"`, read from `telos.lock`:
  how every proof the current seal rests on was judged. `"report"` means the
  sealing configuration had a `[test] report`, so each sealed green is a
  testcase named after its scenario that executed and passed; `"exit-status"`
  means the runner's exit code alone was read, which cannot distinguish a
  zero-test run from green. It reports what the seal proved, not what the
  configuration says now: the two differ only between turning the report on
  and the next reconcile.
- `coverage_scope` — counters inspect the current **working-tree model on
  disk**, which may have drifted from the seal. `includes_open_changes` is
  always `false`: proposals stored in open changes are excluded.
  `model_loaded: false` distinguishes unavailable coverage from a genuinely
  empty model; the counters are then fallback zeros.
- `coverage` — exact counts off that loaded model, or all zeros if loading
  fails. `scenarios_proved` counts scenarios with ≥ 1 `proves` binding;
  `intents_implemented` counts intents with ≥ 1 `implements` binding.
  These are binding counts, not fresh test results. `status` runs no tests.
  An empty disk model can report zero intents/scenarios while an open change
  proposes several: inspect `telos change diff <CHG-id>` and
  `telos pack <INT-id>` (which includes the intent's owning change overlay).

`next_actions` is
`["telos adopt --expected-state sha256:...", "telos revert --expected-state sha256:..."]`
when `state` is `"drifted"`, using the exact same `drift.token` in both;
`["telos change list"]` when `state` is `"changing"`, followed by one
`telos change abandon CHG-NNNN` per open change whose file does not parse
(ascending by id) — the one command that can clear that change's obligation
without a hand edit; `[]` when `"coherent"`.

## `check [--sealed]`

Parses the spec, resolves references, validates active intents and events, and
type-checks literals in the current **working-tree model on disk**. Open-change
proposals are excluded. Neither form executes tests or executable constraint
checks, and success does not establish proof readiness for a pending change.
Inspect proposals with `telos change diff <CHG-id>` and `telos pack <INT-id>`.
Deletion safety and code coverage are enforced at the write and reconcile
boundaries.

On success, `result` is:

```json
{
  "diagnostics": [],
  "scope": {
    "model": "working-tree",
    "includes_open_changes": false,
    "seal_verified": false,
    "tests_executed": false,
    "constraint_checks_executed": false
  }
}
```

`seal_verified` is `true` only for a successful `check --sealed`. The other
scope fields have the values shown for either invocation. Failures retain
`result: null` and the existing error contract.

### Without `--sealed`

`check` never touches `telos.lock`. It calls `load_model`:

- **All parses, all resolves**: `ok: true`, with the result above and
  `scope.seal_verified: false`.
- **One or more diagnostics**: `ok: false`, `result: null`, exit `1`.
  `error` is the *first* diagnostic, converted to the frozen error triple
  (`code`, `message`, `hint`).

  **Current limitation**: the frozen envelope has room for exactly one error, but
  `load_model` collects *every* diagnostic in one pass, never just the
  first. To keep all of them visible without growing the envelope past its
  frozen five keys, `error.message` becomes multi-line when there is more
  than one diagnostic — one `file: message` line per diagnostic, in the
  order they were found, starting with the same line `error.code` and
  `error.hint` describe. An agent reading only `error.code`/the first line
  of `error.message` gets the primary diagnosis and can re-run `check`
  after fixing it; a human reading `error.message` (or stderr in
  human-mode) sees everything found in this run. A future contract revision
  may promote this into `result.diagnostics` on failure.

### With `--sealed`

Additionally requires the project to be sealed, unmodified, and free of open
changes. **State is checked before parsing**: `compute_state` never parses
anything, so a spec that is *both* drifted *and* syntactically broken reports
`TELOS_DRIFT_DETECTED`, never `TELOS_PARSE_ERROR` — drift is the more
actionable diagnosis (and the more likely actual cause: an in-progress
edit, a bad merge).

Order:

1. `Workspace::discover` (→ `TELOS_NOT_INITIALIZED` if no `telos/`).
2. Read `telos.lock` (→ `TELOS_NOT_INITIALIZED`, distinctly worded, if
   `telos/` exists but the lock doesn't).
3. `compute_state`. If `state == "drifted"` →
   `TELOS_DRIFT_DETECTED` with the hint:
   ```
   run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert`
   ```
4. Otherwise, if `state == "changing"` (at least one change is open) →
   `TELOS_CHANGE_STATE_INVALID`, message `` open changes; reconcile or abandon them ``,
   hint `` run `telos change list` ``. Checked in this order —
   unclaimed drift outranks an open change — because a project that is
   somehow both reports the more urgent diagnosis: drift is damage, an open
   change is only work in progress.
5. Only once state is confirmed `coherent`: validate configuration, parse +
   integrity exactly as without `--sealed`, then require sealable structure:
   each active scenario has at least one `proves` and any active obligation
   has a nonblank runner.

`telos init --ci github` wires `telos check --sealed` into CI. GitHub makes
that job merge-required only when repository branch protection separately
requires job `sealed`.

## `show <id|Name>`

Prints one entity's canonical block plus its relations. `target` is a typed
id (`INT-0042`, `SCN-0107`, `CON-0003`, `CHG-0001`) or a bare notion name
(`Invoice`); anything else is `` cannot parse `x` as an id or notion name ``
(`TELOS_REFERENCE_UNKNOWN`, no hint).

### `result` schema

```json
{
  "entity": { "...": "..." },
  "canonical": "notion Invoice entity {\n  ...\n}\n",
  "relations": { "out": [{"rel": "uses", "to": "Customer"}], "in": [] }
}
```

- `entity` — the parsed entity, serialized as-is (a `Notion`, `Intent`,
  `Scenario`, or `Constraint`, each its own JSON shape). For `CHG-…` it is
  instead `{"id", "status", "motivation", "ops": [...]}`, `ops` being one
  `{"n", "op", "entity", "key"}` descriptor per staged op, in staged order
  starting at `n: 1`.
- `canonical` — for a notion, intent, constraint, or scenario (a scenario
  reports its *owning* intent's block), the exact bytes `telos_core::emit`
  would produce for it — reused byte for byte, never reformatted, so `show`
  and the emitter can never disagree about what "canonical" means. **For a
  change (`CHG-…`), this is instead the literal bytes of
  `telos/changes/<id>.tel` as they are on disk.** A change is outside the
  seal (`telos/changes/` is never scanned by `spec_files`), so a hand-edited
  but still-parseable change file is legal state, and `show` has to report
  what is really there rather than a re-emission of the parse. For any file
  telos itself wrote the two are identical byte for byte (the round-trip
  invariant); the distinction only shows up when a change file was
  hand-edited outside the emitter.
- `relations` — `{"out": [{"rel", "to"}], "in": [{"rel", "from"}]}`, in graph
  order. **Always `{"out": [], "in": []}` for a change**: a change is a
  transaction record, not a node of the spec graph, so it has no edge to
  report — the key is still present, so a consumer reads every `show`
  answer the same way regardless of what it was pointed at.

`show CHG-9999` (a change id the store does not hold) is
`TELOS_REFERENCE_UNKNOWN`, `` unknown change `CHG-9999` ``, with a
numeric-nearest hint when another change exists. `next_actions` is always
`[]`.

## `pack <INT-id|SCN-id>`

`pack` returns the bounded implementation pack for one intent. A scenario
argument resolves to its owning intent; notions, constraints, and changes are
well-formed but inapplicable (`TELOS_REFERENCE_UNKNOWN`, message
`` `pack` applies to intents and scenarios ``, null hint). It is a read
command: `next_actions` is always `[]`.

```json
{
  "id": "INT-0042", "owner": {"context": "billing", "capability": "settlement"}, "change": null,
  "canonical": "intent INT-0042 in billing/settlement …\n",
  "scenarios": [{"id": "SCN-0107", "title": "…", "proved": true}],
  "notions": [{"name": "billing/Invoice", "canonical": "notion billing/Invoice …\n"}],
  "constraints": [{"id": "CON-0003", "scope": "context", "canonical": "constraint CON-0003 in context billing …\n"}],
  "bindings": {"implements": ["src/billing/invoice.rs"],
               "proves": [{"scenario": "SCN-0107", "test": "tests/billing.rs::scn_0107"}]},
  "mappings": [],
  "neighbors": [{"id": "INT-0017", "title": "…", "rel": "requires", "direction": "out"}]
}
```

`change` is the owning open change id for a staged add/edit and `null` for
the disk model. Its post-overlay model applies the owner’s operations
idempotently, folds the owner’s journal into bindings, then re-runs semantic
validation. Thus `proved`, `implements`, and `proves` expose live
journalled evidence; pack never uses the entire spec as a prompt and never
exposes supplier internals. Required context-map mappings are published as
contracts. Scenarios
remain in the intent’s order; notions sort by name; constraints by id;
implements by path; proves by `(scenario, test)`; neighbours by
`(relation, id, direction)`. Neighbours are only one-hop `refines`,
`requires`, or `excludes` intent edges.

## `test <SCN-id|--all> [--file <path>] [--diagnostics]`

Runs the configured `[test] cmd` and appends an immutable witness journal
record to the approved or implementing change whose staged intent owns the
scenario. Without `[test] report`, a non-zero runner exit is **red evidence, not a
command failure**: the command still exits zero and records the exact blob
OID of the test file that was run, and a zero exit records green. That
reading cannot distinguish a zero-test run from green, and the run line, the
seal and the result say so (`exit-status`). With `[test] report` configured
the verdict is the report's — see "Report-backed evidence" below — and a run
that proves nothing is `TELOS_TEST_NOT_EXECUTED` with no journal line.

`--diagnostics` prints the scenario, substituted runner command, exit status,
and captured stdout/stderr to **stderr** after each run, including runs refused
for missing/empty/invalid reports and genuine assertion failures. A signal is
reported as exit status `-1`. It applies to individual scenarios and `--all`;
use `telos test SCN-0108 --diagnostics --json 2>runner.log` to keep a log of
that invocation. Output is buffered until the runner exits, not streamed live.
It is not persisted automatically, and diagnostics from an earlier invocation
cannot be recovered unless captured then. Invalid UTF-8 is displayed lossily.
Diagnostics do not change witness classification, journal admission, result
keys, error messages or hints. Machine-readable stdout remains one JSON
envelope. Without the flag, runner output stays suppressed as before.

Before the process starts, Telos hashes the selected proof file. It re-hashes
it after the process exits and journals only when the OID is unchanged; a
runner that rewrites its own proof is refused and leaves the journal untouched.
Thus the witness names the exact executed code/proof OIDs, never bytes observed
only after execution.

Without `--file`, discovery requires exactly one `[tests]`-globbed file whose
contents contain the identifier-boundary convention `scn_NNNN`; zero or more
than one match is `TELOS_TEST_NOT_FOUND`. `--file <path>` selects that file
directly and still derives the matching function name when present. A single
run returns:

```json
{"scenario":"SCN-0108","witness":"red|green",
 "test":"tests/billing.rs::scn_0108_x","change":"CHG-0001",
 "command":"cargo nextest run --profile telos scn_0108_x",
 "evidence":"report|exit-status","executed":1}
```

`evidence` says how the verdict was decided. `executed` is the number of
testcases named after the scenario that ran (passed plus failed) under
`report`, and `null` under `exit-status`. The journal line ends in the same
evidence word: `` run  SCN-0108 green "tests/billing.rs::scn_0108_x" "<oid>" report ``.

Red returns `next_actions: ["telos test SCN-0108"]`; green returns
`["telos change reconcile CHG-0001"]`. `test --all` witnesses every scenario
an open approved/implementing change owes, in scenario-id order, as
`{"runs":[…]}`, and has no next action. It requires exactly one of a
scenario and `--all` (usage errors remain exit 2 without an envelope).

The drift carve-out is exact-path only: a test run admits drift of the test
path it records, but refuses any other unclaimed drift. A journal record
moves `approved` to `implementing`; journal records are digest-inert and
therefore do not invalidate a prior approval. Re-running appends evidence.

### Report-backed evidence

`[test] report = "<path>"` names the JUnit XML report the runner writes,
repository-relative and outside `telos/`. `{report}` in `[test] cmd` is
substituted with that path as argument data exactly like `{filter}`; a
runner that always writes to a fixed path needs no placeholder. Before every
run Telos deletes the report if it exists; after the run it reads it back.
The exit status is then diagnostic only.

A `testcase` is named after the scenario when its `name` attribute contains
`scn_NNNN` at an identifier boundary — the same predicate as discovery.
`classname` is ignored. A testcase with a `failure` or `error` child is
failed; with a `skipped` child, skipped; otherwise passed. Over the
testcases named after the scenario, in this order: any failed → **red**;
otherwise any skipped → not executed; otherwise any passed → **green**;
otherwise not executed. Every `testcase` in the document counts, whether the
root is `testsuites` or `testsuite`.

"Not executed" is `TELOS_TEST_NOT_EXECUTED`, nothing is journalled, and the
message is one of four frozen sentences (`<path>` the configured report,
`scn_NNNN` the scenario's pattern):

| Reason | Message |
|---|---|
| no file at the report path after the run | `` the runner did not write the report at `<path>` `` |
| unreadable or malformed XML | `` the report at `<path>` is not valid JUnit XML: <parser error> `` |
| no testcase named after the scenario | `` the report at `<path>` contains no testcase named after `scn_NNNN` `` |
| testcases named after the scenario, none failed, `<n>` skipped | `` <n> testcase(s) named after `scn_NNNN` were skipped in the report at `<path>` `` |

The hint is always
`` make the runner execute the test named after `scn_NNNN` and write the report, then run `telos test SCN-NNNN` again ``.
A compile error, a missing dependency, or a runner that selected nothing all
land here rather than as red or green. Under `--all` the first such verdict
aborts the loop; runs already taken stay journalled. The parser refuses XML
that carries a `<!DOCTYPE>` declaration — entity expansion is never
processed — so such a report reads as `not valid JUnit XML`.

Wiring a report: `cargo nextest run --profile <p> {filter}` with a junit
profile, `pytest --junitxml={report} -k {filter}`, `gotestsum --junitfile
{report} -- -run {filter}` (behind a runner script, since pipes are refused),
`jest --ci --reporters=jest-junit -t {filter}` with `JEST_JUNIT_OUTPUT_FILE`,
`phpunit --log-junit {report} --filter {filter}`, `dotnet test --logger
"junit;LogFilePath={report}" --filter {filter}`. Keep the report path out of
the `[code]`/`[tests]` globs and in `.gitignore`.

### Display and runner-template execution

The `command` result preserves literal `{filter}` substitution and
trailing-whitespace trim for stable display bytes. That display is diagnostic,
not a shell-replay instruction. `[test] cmd` is parsed into one direct process
argument vector: whitespace separates words, single/double quotes group a
word, backslash quotes one following non-newline character, and `{filter}` may
be one whole argument or part of a word such as `module::{filter}`. The filter
is inserted as uninterpreted argument data and is never evaluated by a shell.

The template fails closed with `TELOS_PARSE_ERROR` if it contains shell
operators, command/backtick or arithmetic substitution, unmatched quotes,
controls, `eval`/`call`, an environment trampoline, or a nested Unix/Windows
interpreter (`sh -c`, `cmd /C`, PowerShell, and equivalents). Filter controls
(including CR, LF, and NUL) are refused; quotes and metacharacters in a filter
remain bytes in one argument. Put any needed shell program in a dedicated
runner script and configure that script as the direct executable.

### Grouped red/green ordering

The witness protocol is per scenario, not a global red-green alternation.
Within one approved intent/change, callers may record red for several new
scenarios, implement their common behavior once, and then record green for
each. Every scenario must still have its own genuine red followed by green
on identical proof-file bytes. Finish all tests in a shared file before its
first red; adding another function later changes the sealed file too.
Grouping does not alter the approved delta, require another approval, or
weaken report-backed execution checks. Reconcile still re-runs impacted
existing scenarios and requires witnesses for new/changed active scenarios.
The generated implementer includes a two-scenario example.

An already-satisfied scenario with no prior red cannot claim a strict pair.
Report it to the challenger; any move to the existing advisory policy is a
separately reviewed configuration change, not an automatic fallback. Never
fabricate a failure or use full reconciliation to avoid a strict witness.

## `bind <path> <INT-id>`

Records that an existing repository-relative code path implements an intent
in the approved or implementing change that stages that intent. All untrusted
CLI, `.tel` binding, change-journal, and lock paths must be portable normalized
relative components: non-empty `/`-separated normal components only, with no
root/prefix, `.`, `..`, doubled/trailing separator, backslash/drive ambiguity,
colon, NUL, or control byte. `bind` and proof selection additionally reject
anything under `telos/`; the file must exist. Hash/read/write/restore repeats
validation and uses capability-anchored, no-follow traversal below the opened
repository root, so an in-repository symlink cannot redirect an operation to
an outside owner. The ownership change must add or edit the intent, never
merely remove it. Its result is:

```json
{"change":"CHG-0001","path":"src/billing/invoice.rs","intent":"INT-0042"}
```

`next_actions` is `["telos change reconcile CHG-0001"]`. Like `test`, bind
admits drift only of its exact claimed path, transitions `approved` to
`implementing`, and leaves the approval digest fresh. Rebinding the identical
`(path, intent)` pair is idempotent: it returns the same result and adds no
second journal line. Journal bindings are folded into context and reconcile’s
post-model; they are not staged spec operations and do not take the
one-file-one-change claim lock.

## `change`

The whole lifecycle of a staged transaction is
`open → drafted → approved → implementing → reconciled` (or `abandoned` from
any still-open state). `reconciled` is the successful terminal transition:
the change file is deleted after the seal is written. From the empty change
`open` allocates to the seal `reconcile` closes it with:

```
open --(add|edit|remove)--> drafted --approve--> approved --reconcile--> (sealed, file deleted)
```

`add`/`edit`/`remove` may also stage into an already-`approved` change (its
status stays `approved`, but `change diff` starts reporting `stale: true`)
or into a `drafted` one; `abandon` deletes the file from any status, no
reseal. `implementing` is the same as `approved` for every gate below —
an approved change in flight.

### Obligations

`change list` and `status.changes[]` both report `obligations`: the frozen,
status-keyed list of what remains before a change is done
(`Change::obligations`):

| status | obligations |
|---|---|
| `open` | `["stage the delta", "approve", "reconcile"]` |
| `drafted` | `["approve", "reconcile"]` |
| `approved` / `implementing` | `["reconcile"]` |
| `abandoned` | `[]` (unreachable in practice — an abandoned change's file is gone before anything could report it) |

A change file that fails to parse at all is a different, best-effort case
(`open_change_infos`, not `Change::obligations`): it still gets an entry,
`status: "open"`, empty `claims`, and the one-item obligations list
`["abandon (telos/changes/CHG-NNNN.tel is unparseable)"]` — `abandon`
because it is the one command that clears it: the file is never repaired by
hand, and `change abandon` does not need it to parse.

### `change open <motivation>`

Allocates the next `CHG-NNNN`, writes an empty change. Gated on *unclaimed
drift only* (`TELOS_DRIFT_DETECTED` when the project's state is `drifted`)
— **not** on `changing`: a second, third, … change may `open` freely while
another is already in flight, as long as nothing is unclaimed drift.

`result`: `{"id": "CHG-0001", "status": "open"}`. `next_actions`:
`["telos add intent --change CHG-0001"]`.

### `change list`

Every change the store holds, best-effort (an unparseable change file still
gets an entry — `status: "open"`, empty claims, the `abandon` obligation of
the "Obligations" subsection above — rather than blocking the listing).
Never gated on drift or on anything else: cleanup and inspection stay
available in every project state.

`result`: `{"changes": [{"id", "status", "motivation", "obligations"}, ...]}`.
`next_actions`: always `[]`.

### `change abandon <id>`

Deletes the change's file without reading it: abandoning means throwing
the change away, and nothing about that decision depends on the file's
content — so a change whose file no longer parses (a truncated write, a bad
merge of `telos/changes/`) is abandoned like any other, instead of
`TELOS_PARSE_ERROR` blocking the one command that can clear its obligation.
A mistyped id still gets `` unknown change `CHG-9999` `` (from the delete
itself), never a silent no-op. Not gated on drift — abandoning is one of the
two ways out of a mess, not more mutation of the spec.

`result`: `{"id": "CHG-0001", "status": "abandoned"}`. `next_actions`: `[]`.

### `change diff <id>`

Reports the change's staged ops against the base: one before/after pair per
op, the live ops digest, the frozen `approved_digest` (if any), and whether
the two disagree. The base is disk truth, not necessarily the *sealed*
one — `telos/`'s files as they parse right now, whatever they currently
are. Never gated on drift, deliberately: a change's own delta is judged
against that live base whatever state the rest of the project is in
(`coherent`, `changing`, even `drifted`), which is exactly the moment a
caller most needs to see it.

`result`:
```json
{
  "id": "CHG-0001", "status": "drafted", "digest": "sha256:...",
  "approved_digest": null, "stale": false,
  "ops": [
    {"n": 1, "op": "add", "entity": "notion", "key": "Invoice",
     "before": null, "after": "notion Invoice entity {\n...}\n"}
  ]
}
```
`before`/`after` are canonical emitted text — `null` for `remove`'s `after`
and `accept`'s `after`, and `null` for `before` when the base holds nothing
at that path yet. `next_actions`:
`["telos change approve CHG-0001 --expected-digest sha256:..."]` while
`status` is `open`/`drafted`, or while `stale` is `true`, using the exact
`result.digest`; otherwise
`["telos change reconcile CHG-0001"]`.

### `change approve <id> [--expected-digest SHA256]`

Freezes the change's ops digest — the review a later `reconcile` checks the
base against. Gated on drift, like `open`. Refuses a change with zero
staged ops: `TELOS_CHANGE_STATE_INVALID`, `` change CHG-0001 has no staged
operations ``, hint `stage operations with telos add|edit|remove first`.
Idempotent otherwise. Re-approval accepts both `approved` and `implementing`
changes, preserves the entering status in `result.status` (`approved` or
`implementing`), refreshes `approved_digest` from the current ops digest, and
makes the next `change diff` report `stale: false`. In particular,
re-approving after implementation evidence was journalled never moves an
`implementing` change backward to `approved`.

Generated skills and guards must pass `--expected-digest` with the exact value
just displayed by `change diff`; a missing or stale value fails closed at the
guard, and the command rejects a mismatch with `TELOS_CHANGE_STATE_INVALID`.
The canonical illustrated spelling is
`telos change approve CHG-0001 --expected-digest sha256:...`.
The CLI keeps a deliberate interactive-human compatibility route when the flag
is omitted: it binds itself to the digest first read, validates, then re-reads
and compares that digest at the mutation boundary. Omitting the flag never
authorizes a delta saved during validation, but automation must not rely on
this compatibility route.

`result`: `{"id": "CHG-0001", "digest": "sha256:...", "status": "approved"|"implementing"}`.
`next_actions`: `["telos change reconcile CHG-0001"]`.

### `change reconcile <id>|--full`

Applies an approved change (writes its spec files, reseals, deletes the
change file) — or, given `--full` instead of an id, re-proves the whole
project from the files on disk and reseals it regardless of what
`telos.lock` currently says.

`result` per invocation: `{"id": "CHG-0001"|null, "full": false|true, "ops_applied": 3, "checks_run": 1, "tests_run": 0, "witness_warnings": []}`.
`id` is `null` (present, never absent) under `--full`; `full` is `false` by
construction for an id invocation (clap refuses an id and `--full`
together). `next_actions`: always `["telos status"]`.

#### The eleven gates, frozen order

`change reconcile <id>` runs eleven gates, in this order, before writing a
single byte — an agent fixing what reconcile complains about must converge,
which only happens if the complaint is always the *first* thing wrong:

| # | Gate | Refusal |
|---|---|---|
| 1 | drift (unclaimed paths only — a path *any* open change claims is expected to differ, that is a change in progress, not damage; what another change claims is admissible here but not sealable here, see the carry-over below) | `TELOS_DRIFT_DETECTED`, message names the drifted paths, same frozen hint as `check --sealed` |
| 2 | status (`approved`/`implementing` only) | `TELOS_CHANGE_STATE_INVALID`, `` change CHG-0001 is not approved; approve it first `` |
| 3 | digest (the delta must still be the one that was approved) | `TELOS_APPROVAL_STALE`, `` re-approve with `telos change approve CHG-0001` `` |
| 4 | accepted bytes (each `accept` op's blob OID must still match) | `TELOS_INTEGRITY_VIOLATION`, `` `<path>` changed since it was accepted `` / `` `<path>` was accepted but no longer exists `` |
| 5 | effective configuration validation, then the overlay (the delta's post-spec must parse, resolve references, validate active intents and events, type-check literals, and preserve referential integrity) | invalid globs/configuration use their exact `TELOS_PARSE_ERROR` or `TELOS_INTEGRITY_VIOLATION`; otherwise whatever `TELOS_*` diagnostic the semantic pass raises first |
| 6 | no unbound code, evaluated over the post model | `TELOS_ORPHAN_CODE` |
| 7 | sealed code coverage: every path in the previous lock's `code` table remains bound in the folded post-model, unless this delta stages its owning `telos/contexts/<context>/bindings.tel` | `TELOS_INTEGRITY_VIOLATION`; the error names the uncovered path and owning bindings file |
| 8 | sealed red/green witness for every new or changed scenario | `TELOS_SCENARIO_RED_EXPECTED` or `TELOS_TEST_SEALED` under strict policy; `TELOS_TEST_NOT_EXECUTED` when `[test] report` is configured and the scenario's witnesses were taken by exit status; warnings under advisory policy |
| 9 | sealable structure: every active scenario has at least one `proves`, and any active obligation has a nonblank runner | `TELOS_INTEGRITY_VIOLATION`, ``active scenario SCN-NNNN has no `proves` binding``; then `TELOS_TEST_NOT_FOUND` for no runner |
| 10 | constraint checks, for the constraints this delta puts in scope | `TELOS_CONSTRAINT_FAILED` |
| 11 | tests, one run per distinct `proves` target of the impacted scenarios | `TELOS_INTEGRITY_VIOLATION`, `` the test run for `<target>` failed ``; with `[test] report`, `TELOS_TEST_NOT_EXECUTED`, `` the test run for `<target>` did not execute SCN-NNNN: <reason> `` |

Immediately before gates 10–11, reconcile records a snapshot captured before
checks/tests: the complete current spec path/OID map and every bound code/proof
OID. It runs each obligation once, then requires both the same spec path set
and the same OIDs. A runner or ordinary save that changes either tree refuses
before any journal, lock, or change removal. The lock carries the exact executed
code/proof OIDs from that proven snapshot. After canonical spec ops are written,
Telos hashes their deterministic post-state and revalidates both tables
immediately before and after lock publication; any later edit is observable as
drift rather than silently becoming part of the successful seal.

Only once gate 11 passes does anything reach disk: the spec `.tel` files
(through the emitter, in staged order), then the canonical folded per-context
`bindings.tel` files, then `telos.lock`, then the change file's deletion.
If ordinary reconciliation returns an error during publication, it restores
all files it attempted to write or remove to their pre-call bytes, including
the prior lock and any moved/deleted spec files. Newly created files, including
journal-derived bindings, are removed. The change is deleted last and remains
open on failure, so the same reviewed delta can be retried after fixing the
cause. Restoration does not require a writable Git object store. The original
error code/message are retained; if restoration itself fails, the error hint
names every path requiring recovery. This is returned-error rollback, not
crash durability: process termination/power loss still require Git recovery;
empty directories and unreachable Git objects may remain.

`counters.toml` is never touched by reconcile — every id a transaction spends
was already persisted when the op was staged. Journal records are digest-inert:
they move an approved change to `implementing` but do not stale its approval.

Gate 8 is strict versus advisory: `policy.tdd = "strict"` refuses on the
first missing red witness, missing green witness, or changed witness bytes;
`"advisory"` reconciles and returns each same frozen verdict in
`result.witness_warnings`. Both `approved` and `implementing` changes owe
reconciliation, and a journal is folded into the post-model before gates 5–11.

With `[test] report` configured, gate 8 reads only `report` runs. When the
filtered verdict is not intact and the journal holds an `exit-status` run
for the scenario, the refusal is `TELOS_TEST_NOT_EXECUTED` with message
`` scenario SCN-NNNN's witness was taken by exit status; `[test] report` is configured ``
and hint `` run `telos test SCN-NNNN` again to record a report-backed red and green ``
(a warning under `advisory`). Gate 11 runs each impacted target once and
judges the run for every impacted scenario the target proves, in scenario-id
order: a red keeps the integrity refusal above; a report that does not prove
the scenario is `TELOS_TEST_NOT_EXECUTED` with message
`` the test run for `<target>` did not execute SCN-NNNN: <reason> `` — `<reason>`
one of the four sentences of the `test` section — and hint
`` run the configured executable with the displayed arguments and inspect the report, then reconcile again ``.
The seal records `proof_evidence = "report"` from the effective
configuration.

#### The carry-over: drift another open change claims is never sealed here

Gate 1 admits drift *any* open change claims, not just this one's — a
concurrent change (an implementing change drifts its code files for
its whole life) must not hold an unrelated reconcile hostage. The seal draws
the line the gate does not: **a spec or code path that is both drifted and
claimed by another open change is sealed at its previously sealed OID**, not
re-hashed from disk — and stays out of the new lock entirely if the previous
lock never held it (an adopted-but-unreconciled untracked file). The drift
therefore survives the reconcile and resurfaces the moment the claiming
change goes: `status` still reports it, still claimed (so the project is
`changing`, not `coherent`), and `change abandon` on the claiming change
turns it straight back into `drifted`. Bytes that arrived out of protocol are
sealed by the change that reviews them — an `adopt`/`approve`/`reconcile` of
their own — and by nothing else. A change's *own* claims are re-hashed
normally: its ops have just rewritten them, which is the point.

`--full` is the deliberate exception and stays unchanged: it re-proves the
whole tree from disk and seals what it finds, open adopt-changes included.
That is total proof, not a bypass — the drift it seals has passed every gate
a spec on its own can be held to: the applicable gates below, plus the whole
suite once when at least one intent is active, or no runner invocation when
all intents are draft/deprecated. This is why `--full` is the exit from a
conflicted lock.

#### `--full`

Structurally skips gates 1–4, 7, and 8 rather than passing them vacuously: there is
no change, so no drift/status/digest/accept-OID judgement to make, and
`--full` deliberately never reads `telos.lock` at all — a lock left
conflicted by a merge, or a spec tree that was never sealed, is exactly what
it exists for. Gates 5, 6, 9, 10, and 11 run, but adapted to having no delta
to filter against: configuration validates before 5's disk model; 6 (orphan
code) is unchanged; 9 requires all active proof bindings and a runner; 10
runs the `check` of **every** constraint that has one. Gate 11 invokes
`[test] cmd` with `{filter}` empty exactly once when the model contains at
least one active intent, and zero times when all intents are draft/deprecated.
With `[test] report` configured that single run's report is judged for every
active scenario that has a `proves` binding, in scenario-id order, with the
same two refusals as gate 11 and `<target>` being `the whole suite`.
`result.ops_applied` is always `0` under `--full` (no ops — nothing was
staged, the state was simply found and re-proved). Open changes are
tolerated and left untouched (their files, still open), and the seal this
produces has `sealed_by: null` — no transaction produced it.

Full reconcile uses the same pre-execution spec/code snapshot and post-run
OID equality checks. Its lock is built directly from that proven snapshot;
it never re-hashes changed post-test bytes into a successful seal.

## `add`/`edit`/`remove <notion|intent|constraint>`

Stages one operation into an open change; nothing is written under `telos/`
until that change's `reconcile`. `add`/`edit` read a JSON payload from
stdin (see the payload schemas below); `remove` takes no payload. `--change CHG-0001` is
required on all three.

**No status gate.** Staging is allowed on `open` (which becomes `drafted`
on the first op), on `drafted`, and on an already-`approved` change too —
nothing is lost by staging into an approved change: the approval's digest
stays as it was, `change diff` starts reporting `stale: true`, and
`reconcile` refuses with `TELOS_APPROVAL_STALE` until the change is
re-approved. Staleness is `reconcile`'s gate to enforce, not staging's to
forbid.

Gated on unclaimed drift (`TELOS_DRIFT_DETECTED`) — staging on top of a base
nobody reviewed is refused the same way `change open` is.

### Claims: one file, one change

Every op targets exactly one file (a function of the entity's kind and
id/name, never of where a file happens to sit). A change's *claims* are the
set of those paths across every op it holds (`add` then `edit` of the same
entity claims it once, not twice). A second, different open change may not
stage an op whose target path is already claimed: `TELOS_FILE_CLAIMED`,
`` <path> is already claimed by CHG-0001 `` (the path is bare, not
backtick-quoted, inside the message), hint `` reconcile or abandon
CHG-0001 first, or work within it ``. A path a change claims is that change
in progress, not drift — `compute_state` never reports a claimed path as
drifted, so nothing stops a caller from staging further into that *same*
change, or from any command that reads rather than mutates.

### `result` schema

`add`/`edit` result (the IDs below are illustrative):
```json
{"change": "CHG-0001", "entity": "intent", "id": "INT-0043", "scenario_ids": ["SCN-0108"], "claims": ["telos/contexts/billing/capabilities/settlement/intents/INT-0043.tel"]}
```
Ownership is required in the relevant creation payloads; it is not a separate
field of the `add`/`edit` result. `pack` exposes structured intent ownership.
`scenario_ids` is always present, `[]` when the op allocated none (every
kind but `add intent`/`edit intent` growing a scenario). `claims` is the
*whole change's* claim set with the new op counted, not just this op's own
path. `remove`: the shorter `{"change", "entity", "id"}` — no
`scenario_ids` or `claims`. `next_actions` is always
`["telos change diff CHG-0001"]`.

### Counters (`telos/changes/counters.toml`)

Four persisted high-water marks — `intent`, `scenario`, `constraint`,
`change` — **never decremented**: an id, once handed out, is never handed
out again, so `remove`ing an entity or abandoning the change that added it
never frees its id for reuse. The file is only a fast path, never the
single source of truth: every allocation computes a *floor* fresh from the
sealed model, every open change's ops, and (for `change`) the change ids on
disk plus the change that produced the current seal, then starts from
`max(persisted, floor)` — so a stale or missing `counters.toml` self-heals
on the very next allocation rather than ever reissuing an id. Of the three
entity counters, only `add intent`, `add constraint`, and an `edit intent`
that grows a scenario ever mint an id; notions are named, not numbered, so
no notion op touches a counter. The fourth counter, `change`, is minted by
two different commands: `change open` always, and `adopt` too — but only
when it opens a *new* change; `adopt --into` spends no id, since the change
it appends to was already allocated and persisted when it was opened.

### Payload schemas (`add`/`edit`)

One JSON object on stdin; an unknown top-level or nested key is refused
(`TELOS_PARSE_ERROR`, `` unknown key `x` in <kind> payload ``).
Creation identity and ownership depend on the entity kind:

| Entity | Identity in the creation payload | Required `owner` |
|---|---|---|
| context | caller supplies `id`, e.g. `billing` | none |
| capability | caller supplies local `id`, e.g. `settlement` | context, e.g. `billing` |
| notion | caller supplies `name`, e.g. `Invoice` | `context` or `context/capability` |
| intent | no `id`; allocated as `INT-NNNN` | `context/capability` |
| scenario nested in a new intent | no `id`; allocated as `SCN-NNNN` | inherited from the intent |
| constraint | no `id`; allocated as `CON-NNNN` | `context` or `context/capability` |

Owners are strings, not objects. Context/capability IDs use lower-kebab-case.
`edit context <id>` and `edit capability <context/id>` require the complete
creation payload, with unchanged identity/ownership. For notion, intent and
constraint, `edit <kind> <key>` accepts the same keys, all optional: a key
present in the payload replaces that field
**wholesale** (a list field like `attrs`/`requires`/`scenarios` is the whole
new list, never a delta against the old one); a key absent keeps the base
entity's current value. `remove <kind> <key>` takes no payload.

The following creation payloads form a complete bootstrap, in document order.
Start in a fresh Git repository with `telos init` and
`telos change open "Settle invoices"`. Pass each JSON block unchanged on stdin
to `telos add <kind> --change CHG-0001 --json`, using the entity kind named
above the block. Read allocated IDs from each response; this empty-project
example allocates `INT-0001`, `SCN-0001`, and `CON-0001`. These commands only
stage a proposal: implementation, proof configuration and approval still
belong to the normal change workflow.

**`add context`**:
```json
{ "id": "billing", "kind": "core", "title": "Billing",
  "def": "Owns invoice rules." }
```

**`add capability`**:
```json
{ "owner": "billing", "id": "settlement", "title": "Settlement",
  "def": "Settles invoices after payment." }
```

**`add notion` prerequisite: `Customer`**:
```json
{ "owner": "billing", "name": "Customer", "kind": "entity",
  "def": "The customer receiving an invoice." }
```

**`add notion`**:
```json
{ "owner": "billing", "name": "Invoice", "kind": "entity",
  "def": "A bill issued to a Customer for delivered work.",
  "attrs": [ {"name": "state", "type": "enum", "values": ["open", "settled"]},
             {"name": "balance", "type": "money"},
             {"name": "customer", "type": "ref", "target": "Customer"} ],
  "rels":  [ {"name": "issued-to", "target": "Customer"} ] }
```
`attrs`/`rels` default to `[]` when absent.
`type` ∈ `string|int|decimal|money|bool|date|datetime|enum|ref`;
`enum` requires `values` (≥ 1 entry); `ref` requires `target`. Every
`values` entry is an enum symbol: ASCII lower-kebab-case (`playing`,
`x-wins`), because the symbol is written bare into the change file and
read back by the parser as a `lower-ident`. Any other spelling (`X`) is
refused at staging, so `add` never writes a change it cannot read back:
`` payload: attribute `Board.outcome` has type `enum`, but `X` is not an
enum symbol; symbols are lower-kebab-case like `x-wins` `` — the one
shape error whose `hint` is set, naming the accepted grammar.

**`add notion` prerequisite: `PaymentReceived`**:
```json
{ "owner": "billing/settlement", "name": "PaymentReceived", "kind": "event",
  "def": "A payment received for an invoice.",
  "attrs": [ {"name": "amount", "type": "money"} ] }
```

**`add intent`** (no `id` on the intent or on any scenario — every id is
allocated and reported back in `result`; steps carry their state under
`fields`):
```json
{ "owner": "billing/settlement", "title": "Invoices can be settled", "status": "active",
  "telos": "Customers must see immediately that their debt is cleared.",
  "statement": { "template": "event-driven", "when": "PaymentReceived",
                 "on": "Invoice", "action": "set Invoice.state = settled" },
  "refines": [], "requires": [], "excludes": [],
  "scenarios": [
    { "title": "a full payment settles the invoice",
      "given": [ {"notion": "Invoice", "fields": {"state": "open", "balance": "120.00 EUR"}} ],
      "when":  {"notion": "PaymentReceived", "fields": {"amount": "120.00 EUR"}},
      "then":  ["Invoice.state == settled"] } ] }
```
Statement templates: `{"template":"ubiquitous","action":…}` ·
`{"template":"event-driven","when":"Event","on":"Notion"?,"action":…}` ·
`{"template":"state-driven","while":"Invoice.state == open","action":…}`
(`while` must parse to exactly `Ref == literal`) ·
`{"template":"unwanted","if":"Invoice.balance < 0","action":…}` ·
`{"template":"optional","where":"dark-mode","action":…}`. `while`/`if`/
`then` (and a constraint's `rule.expr`) are strings of the mini-language,
parsed the same way `.tel` source is — comparisons `Notion.attr <op>
literal`, membership `Notion.attr in (a, b)`, combined with `and`/`or`/
`not`, on ASCII identifiers. They are a grammar, never prose: a string of
natural language is a `TELOS_PARSE_ERROR` whose message names the exact
field that failed (`` payload.scenarios[0].then[1]: unexpected character
`é` ``) and whose `hint` names the grammar. `action` is a string: if it
starts with `set ` it must parse to exactly `set <Notion.attr> =
<literal>` (a dedicated parse error otherwise); any other string is a free
clause.

Typing a `fields` value against its attribute's declared type: `string` →
JSON string → `Str`; `int` → JSON integer → `Int`; `decimal` → JSON
**string** lexeme (`"120.50"` — a JSON number is refused, to avoid float
hazard; so is a string that is not a `decimal-lit` of the form `-?\d+\.\d+`,
because the lexeme is written verbatim to the change file and `"2"` would
read back as an `int`: `` payload: field `Product.price` has type `decimal`,
but `2` is not a decimal of the form `120.50`; a whole number is written
`2.0` ``); `money` → JSON string `"120.00 EUR"`; `bool` → JSON bool; `date`/
`datetime` → JSON string lexeme; `enum` → JSON string, the symbol itself, →
`Symbol`; `ref` → refused. An unknown attribute is
`TELOS_REFERENCE_UNKNOWN` with a suggestion.

**`add constraint`**:
```json
{ "owner": "billing", "kind": "architecture", "title": "Hexagonal boundaries",
  "rule": {"text": "Domain code must not import adapter modules."},
  "scope": "global", "check": "scripts/check-imports.sh --layer domain" }
```
`rule` is `{"text": …}` **or** `{"expr": "Invoice.balance >= 0"}`; `scope`
is `"global"` or an array of intent ids; `check` is optional. The sample
check script must be supplied by the project before reconciliation.

**`edit`**: context and capability require the full matching `add` payload.
For notion, intent and constraint, keys are optional, each replacing its
field wholesale when present; omitted ownership is retained. In `scenarios`, an entry carrying an
`"id": "SCN-0107"` replaces that scenario in place; an entry with no `id`
is newly allocated; a scenario of the base absent from the new list is
dropped. `"check": null` on a constraint explicitly clears it (an absent
`check` leaves it untouched). **`remove`**: no payload.

## `adopt [--into CHG-NNNN] [--expected-state SHA256]` / `revert [--expected-state SHA256]`

The two exits from drift are to capture it or throw it away. Both are
gated the opposite way from every mutating command above —
`TELOS_CHANGE_STATE_INVALID`, `` nothing to adopt: the project has not
drifted `` / `` nothing to revert: the project has not drifted ``, hint
`` run `telos status` to see the project's state `` — since both exist only
to leave a `drifted` project, never a `coherent` or `changing` one.

Generated skills and guards always take the token from the same `status`
response and invoke `telos adopt --expected-state sha256:...` or
`telos revert --expected-state sha256:...` (with `--into` before or after the
token where applicable). The command re-scans changes and re-hashes the exact
drift scope at the mutation boundary; a stale token refuses before allocation,
write, restore, or deletion. As with approval, a direct human may deliberately
omit the compatibility flag; that route binds itself to the first observed
token and still repeats the boundary check. Agent automation fails closed when
the flag is missing.

### `adopt`

Turns every *unclaimed* drifted path into one staged op (`edit`/`add`/
`remove`/`accept`, chosen by where the path is and how it drifted) of a
change — a new one, or `--into`'s existing one. Canonicalizing: the op
carries the re-parsed entity, so `reconcile` writes back canonical bytes
and whatever whitespace the out-of-protocol edit introduced never reaches
the seal. After a successful `adopt` the project is `changing`, not
`coherent` — the drift is claimed, not yet resealed.

`result`: `{"change": "CHG-0002", "ops": 1, "paths": ["telos/contexts/billing/notions/Invoice.tel"]}`.
`next_actions`: `["telos change diff CHG-0002", "telos change approve CHG-0002"]`.

Four refusals, each handing the caller a next step (frozen wordings in the
error-code table above): a drifted `.tel` file that no longer parses
(`TELOS_PARSE_ERROR`, hint `` fix the file or run `telos revert` ``); the
deletion of a file that carries no entity of its own — a bound code file or
an opaque file like `telos.toml` (`TELOS_INTEGRITY_VIOLATION`); a `.tel`
file whose declared entity belongs at another path
(`TELOS_INTEGRITY_VIOLATION`); a *missing* entity file whose file name is
not even a valid identity, so not even its deletion can be expressed
(`TELOS_INTEGRITY_VIOLATION`, message `` cannot read an entity identity
from `<path>` ``, hint `` restore `<path>` with `telos revert` ``).

### `revert`

The mirror image: every sealed path is restored from the blob its OID
names, every unsealed path is deleted. Destructive (no undo beyond what git
already holds) and not atomic (a failure part-way leaves what was already
restored restored — strictly closer to the seal than where it started, and
safe to re-run). Needs the sealed content in the object store, and every
seal puts it there (`git hash-object -w` at `init` and at each reconcile),
so a project sealed but never committed reverts like any other. The objects
stay unreachable until a commit names them; should git prune them first
(`gc.pruneExpire`, two weeks by default), or should the lock predate this
behaviour, `revert` gets **`TELOS_GIT_ERROR`** (not
`TELOS_INTEGRITY_VIOLATION` — a missing blob is git's own diagnosis, `git
cat-file blob` failing, not a spec integrity one) with the frozen
`MISSING_BLOB_HINT` rather than silently restoring nothing.

Every restoration/deletion uses the same validated repository path contract
as `bind`, plus capability-rooted no-follow mutation. A symlink or parent-path
substitution therefore refuses and never writes the outside target.

`result`: `{"restored": ["telos/contexts/billing/notions/Invoice.tel"], "deleted": []}`.
`next_actions`: `["telos status"]`.

## `init [--agents claude,codex] [--ci github]`

`init` still creates and seals the empty Telos tree. With `--agents`, its
comma-delimited host list is sorted and deduplicated, then it installs the
same three canonical skill files — `telos`, `telos-challenger`, and
`telos-implementer` — for each requested host. Claude receives them under
`.claude/skills/`; Codex receives them under `.agents/skills/` plus a managed
Telos block in `AGENTS.md`. Existing host configuration is parsed before any
project write; malformed JSON is `TELOS_PARSE_ERROR` with the repair-and-rerun
hint.

The exact JSON result is identical with or without `--agents`:
`result`: `{"root": "telos", "sealed": true}`.
`next_actions`: `["telos status"]`.

It merges one owned `PreToolUse` command hook without deleting unrelated
hooks. Every text owner must contain exactly zero or one well-formed Telos-owned
block with one start before one end marker. Orphaned, duplicate, reversed, or
crossed markers in `AGENTS.md`, Codex rules, or another owned-block consumer
are a preflight error before any write; a partial-init retry applies the same
check and preserves all user bytes. Existing merged files publish with a real
content-and-identity CAS: the candidate atomically displaces the owner, checks
the displaced bytes/identity, restores on mismatch, and never loses an IDE save
that landed after validation. Codex repository configuration is not assumed
active: before relying on the generated guard or rules, open `/hooks`, review
and trust the repository `.codex` layer, and verify the exact
`telos agent-guard --host codex` hook.
Until that review and trust is complete, `.codex/hooks.json` and
`.codex/rules/telos.rules` must be treated as inactive. Once active, the guard
refuses direct agent writes to the repository `telos/` tree and accepts only
CLI-mediated mutations. Generated Codex rules request native human
confirmation for `telos change approve`, `telos adopt`, and `telos revert`.
Fresh Codex integrations also install native `prompt` rules for the exact
`rtk telos ...` and `rtk proxy telos ...` spellings of those three actions.
These wrappers preserve the same required digest/state token and human
prompt; other wrappers, wrapper options, nesting and compound commands are
refused with an explicit native-rule-coverage diagnostic.

For an existing installation, updating the binary alone does **not** enable
RTK decisions: the guard requires the intact shipped
[`codex-rtk.rules`](../crates/telos/assets/codex-rtk.rules) block in
`.codex/rules/telos.rules`. Copy that block into the existing Telos-owned
section without removing unrelated rules, then review/trust the repository
rules as above. Missing/outdated blocks are refused explicitly. Do not rerun
`init` on an already initialized project or relax prompting to `allow`.
Until the block is installed and active, projects that mandate RTK need an
explicit project-instruction exception for the three direct human-action
commands. Other project commands can continue using RTK.

Before approval, the challenger presents `change diff`’s `result.digest` and
passes that exact value as `--expected-digest`; before adopt/revert, the router
presents the relevant drift paths/token and passes the exact `--expected-state`.
The rules themselves are static prompts, while the token is a command argument
the guard verifies independently. A token-less, stale, compound, nested, or
environment-wrapped human-action command fails closed. The generated work pack
deliberately remains a portable bounded `telos pack`, never a
whole-spec or host-specific prompt dump.

For a canonical token-bound human-action command, the guard independently
reads the current repository state before it permits a decision prompt:
approval context is `change CHG-NNNN digest sha256:...`; adopt/revert context contains
the sorted current drift paths and the sealed spec digest. It never uses an
agent-supplied tool description for either value, and denies the action if it
cannot resolve that context. Claude returns its supported PreToolUse `ask`
decision with this context in the reason. The official Codex PreToolUse hook
contract rejects `permissionDecision: "ask"`; it does accept top-level
`systemMessage` and `hookSpecificOutput.additionalContext`. Consequently the
Codex guard returns no unsupported `ask`: it sends the same repository-derived
context through those supported fields and the generated static `.rules` entry
owns the native `prompt`.

## Configuration, view, rebuild, and CI

Every success and failure on these surfaces uses the same five-key envelope;
the parent command, not its mode or subcommand, is the `command` value.

### Configuration, view, rebuild, and CI result schemas

Object keys listed here are exact and always present for the named success
mode. Their nested schemas are defined below.

| Invocation | command | Exact result keys |
|---|---|---|
| `config` | `config` | `code, tests, test, policy, agents` |
| `config --change` | `config` | `change, path, config` |
| `view --export` | `view` | `mode, destination, files` |
| `view --port` | `view` | `mode, url` |
| `rebuild plan` | `rebuild` | `steps` |
| `rebuild status` | `rebuild` | `scenarios_green, scenarios_total, scenarios` |
| `init --ci github` | `init` | `root, sealed` |

### State admission matrix

`Spec-only` means a parseable `telos/telos.toml` and validated Telos model
with no `telos.lock`. For `config read`, only the TOML must be readable; no
model or lock is loaded. A `changing` project with any unclaimed drift is
classified and handled as `drifted`, so damage wins over work in progress.
The `coherent` column assumes valid configuration and sealable structure;
sealed consumers refuse legacy locks that fail either predicate.

| Surface | Spec-only | coherent | changing | drifted |
|---|---|---|---|---|
| `config read` | allow | allow | allow | allow |
| `config write` | refuse: TELOS_NOT_INITIALIZED | n/a: needs an open change | allow: target open/drafted | refuse: TELOS_DRIFT_DETECTED |
| `view live` | refuse: TELOS_NOT_INITIALIZED | allow | allow | allow |
| `view export` | refuse: TELOS_NOT_INITIALIZED | allow | refuse: TELOS_CHANGE_STATE_INVALID | refuse: TELOS_DRIFT_DETECTED |
| `rebuild plan` | allow | allow | allow | refuse: TELOS_DRIFT_DETECTED |
| `rebuild status` | allow | allow | allow | refuse: TELOS_DRIFT_DETECTED |

### `config [--change CHG-NNNN]`

Read mode starts as soon as `telos/telos.toml` can be discovered and parsed.
It deliberately does not require or inspect `telos.lock`, project state, the
model, or open changes. Human output is the canonical TOML in section order
`code`, `tests`, `test`, `policy`, `agents`, with exactly one final newline.
JSON output is the complete typed configuration:

```json
{
  "ok": true,
  "command": "config",
  "result": {
    "code": {"globs": ["src/**/*.rs"]},
    "tests": {"globs": ["tests/**/*.rs"]},
    "test": {"cmd": "cargo test {filter}", "report": ""},
    "policy": {"tdd": "strict"},
    "agents": {"hosts": ["claude", "codex"]}
  },
  "error": null,
  "next_actions": []
}
```

`test.report` is the repository-relative path of the JUnit XML report the
runner writes (`""` when unset — exit-status evidence; see the `test`
section). It must be a code path outside `telos/`: a path under the spec
tree is `TELOS_PARSE_ERROR` with message
`` invalid [test] report: `<path>` is under the spec tree `` and hint
`` write the report outside telos/, e.g. `target/telos-report.xml` ``. A
`{report}` placeholder in `test.cmd` requires it: otherwise
`TELOS_PARSE_ERROR` with message
`` invalid [test] cmd: `{report}` is used but `[test] report` is not configured ``
and hint
`` set [test] report to the repository-relative path the runner writes its JUnit XML report to ``.
Both checks run wherever the configuration is validated (the validation
matrix below), so no surface executes a runner under an incoherent `[test]`.
On the write side `test.report` is optional and defaults to `""`.

Write mode reads one complete JSON object of that same nested shape from
stdin. Partial objects (`test.report` excepted), unknown fields, an invalid
`policy.tdd`, or an invalid glob are `TELOS_PARSE_ERROR`. `policy.tdd` is
exactly `"strict"` or `"advisory"`. The normalized, sorted/deduplicated
`agents.hosts` must equal the current value: `telos config` preserves this
`init --agents` project metadata and never installs or deletes host
artifacts.

The target change must exist and be `open` or `drafted`. The ordinary
unclaimed-drift and one-file/one-change claim gates run first. Success stages
one typed `edit config` operation claiming `telos/telos.toml`, moves an open
change to `drafted`, and changes the operation digest. It does not write the
base configuration. Only approval followed by reconcile writes the canonical
TOML and seals it; the effective staged config already controls that
reconcile's globs, test runner, and TDD policy.

```json
{
  "ok": true,
  "command": "config",
  "result": {
    "change": "CHG-0001",
    "path": "telos/telos.toml",
    "config": {
      "code": {"globs": ["src/**/*.rs"]},
      "tests": {"globs": ["tests/**/*.rs"]},
      "test": {"cmd": "cargo test {filter}", "report": ""},
      "policy": {"tdd": "advisory"},
      "agents": {"hosts": ["claude", "codex"]}
    }
  },
  "error": null,
  "next_actions": ["telos change diff CHG-0001"]
}
```

### Configuration validation matrix

The same canonical validators are authoritative at every trust boundary; a
hand-edited change cannot bypass the CLI staging checks.

| Boundary | Glob and `[test]` validation | agents.hosts validation | Refusal effect |
|---|---|---|---|
| `config --change` | compile with runtime walker semantics | normalized value must equal base | config/change/counters unchanged |
| `change approve` | revalidate effective config | revalidate transition from base | change stays drafted and gains no digest |
| change approve again | revalidate effective config | revalidate transition from base | existing approved digest is preserved |
| ordinary change reconcile | revalidate before checks/tests/writes | revalidate transition from base | config/change/lock unchanged |
| `change reconcile --full` | validate persisted config | validate normalized persisted value | no checks/tests/write before refusal |
| sealed consumers | validate persisted config | validate normalized persisted value | never report/publish an invalid coherent seal |

An approved, fresh `EditConfig` is global: its effective runner, globs, and
policy are used by reconcile, `telos test`, and `rebuild status`. At ordinary
reconcile it marks every intent and scenario impacted, so all applicable
constraint checks and every distinct `proves` target run once before the
configuration is written. A configuration edit cannot narrow its own proof
or constraint gate.

### `view [--port N] [--export DIR] [--open]`

Live and export consume the same immutable `ViewSnapshot`. The binary embeds
one SPA application shell and all of its assets; live serves that shell with a
generated `/data.js`, while export writes the same shell and assets with the
same deterministic snapshot payload in `data.js`.

`--open` launches the generated view in the operating system's default web
browser: the advertised loopback URL for live mode, or the exported
`index.html` for `--export`. A browser-launch failure returns `TELOS_INTERNAL`
with a hint containing the target to open manually. It does not alter either
mode's success envelope.

The SPA owns navigation. Its six frontend routes are:

### Frontend hash routes

| Page | Hash route |
|---|---|
| Dashboard | `#/` |
| Intents | `#/intents` |
| Intent detail | `#/intent/INT-NNNN` |
| Graph | `#/graph` |
| Glossary | `#/glossary` |
| Coverage | `#/coverage` |

### Live routes and export files

| Resource | Live path | Export path |
|---|---|---|
| Application shell | `/` | `index.html` |
| Snapshot payload | `/data.js` | `data.js` |
| Embedded asset | `/<path>` | `<path>` |
| Live status | `/live.json` | — |
| Pages marker | — | `.nojekyll` |

Every other server HTTP path returns HTTP 404. In particular, frontend hash
routes are never direct server paths.

The Dashboard exposes state, open changes, and drift; Intents lists every
intent; Graph exposes all eight relation filters; Intent detail exposes the
canonical statement, scenarios, applicable constraints, `implements`, and
`proves`; Glossary exposes notions; Coverage is the intent × scenario × test
matrix. All pages cross-link through the hash routes above.

#### Static export

`telos view --export DIR` admits only `coherent` with valid sealed
configuration and sealable active proof/runner structure. Drift returns the existing
`TELOS_DRIFT_DETECTED` form. Open changes return
`TELOS_CHANGE_STATE_INVALID`, message `open changes; reconcile or abandon
them`, hint ``run `telos change list```; drift is checked first. The
destination must be valid UTF-8 and must not exist in any form, including a
file, directory, live symlink, or dangling symlink. A collision returns
`TELOS_CHANGE_STATE_INVALID`, message ``export destination `DIR` already
exists``, hint `choose an empty path that does not exist`.

Export first loads the model, then freshly re-scans changes and recomputes the
sealed OID state before constructing the immutable projection. Consequently it
publishes an authenticated single sealed snapshot: a normal save between the
earlier state/model reads either leaves the exact sealed model eligible or is
refused as drift; newly saved model bytes are never labelled coherent. The
snapshot itself owns the already-authenticated model bytes, so later working
tree edits cannot change what is rendered.

Every file is assembled in memory before publication. Under the filesystem
publication threat model below, the exporter writes a unique sibling staging
directory and promotes it atomically with a no-replacement primitive: no
ordinary concurrent owner is overwritten, and any render, write, or
finalization error leaves no final destination. `files` is sorted by
repository-style `/` path. For a given binary and snapshot, the complete
export tree is byte-deterministic and self-contained: every script, style,
font, and image it uses is present in the export, and no exported page makes a
network request. Every published path is exactly `index.html`, `data.js`,
`.nojekyll`, or is below `assets/`.

The exporter also captures the no-follow identity of the full destination
parent chain. It reopens that pathname chain and compares every identity
immediately before promotion; renaming/replacing any parent refuses, cleans
only authenticated staging, leaves the advertised final absent, and preserves
the replacement owner.

### Export publication matrix

| Condition | Final destination | Existing owner | Staging cleanup |
|---|---|---|---|
| success | one complete atomic publication | n/a | staging consumed |
| destination exists before start | absent from Telos | preserved byte-for-byte | no staging published |
| destination appears before publish | absent from Telos | preserved byte-for-byte | authenticated Telos staging cleaned |
| render/write/finalization error | absent | n/a | authenticated Telos staging cleaned |
| staging identity mismatch | absent | replacement preserved | foreign/replaced entry never cleaned |

### Export envelope

```json
{
  "ok": true,
  "command": "view",
  "result": {
    "mode": "export",
    "destination": "site",
    "files": [
      ".nojekyll",
      "assets/app.css",
      "assets/app.js",
      "assets/logo.png",
      "data.js",
      "index.html"
    ]
  },
  "error": null,
  "next_actions": []
}
```

#### Live lifecycle

`telos view` is a foreground, read-only local server. It requires a readable
lock and model but admits `coherent`, `changing`, and `drifted`. It binds only
IPv4 loopback `127.0.0.1`; `--port 0` asks the OS for a free port. After the
listener, watcher, and initial snapshot are ready, it prints and flushes
exactly one startup line, then serves until Ctrl-C or process termination:

```json
{"ok":true,"command":"view","result":{"mode":"server","url":"http://127.0.0.1:<allocated>/"},"error":null,"next_actions":[]}
```

The live HTTP boundary is tied to that exact advertised authority. Every
request must carry `Host: 127.0.0.1:<allocated>`; a missing or different Host
is refused with HTTP 421, including `localhost` or another port. A successful
`GET /` establishes a per-process 256-bit CSPRNG session as the host-only
session cookie `telos_view_session_<allocated>`, with
`HttpOnly; SameSite=Strict; Path=/`. Including the port in the name lets
multiple live servers coexist even though cookies themselves have no port
scope. `Secure` is intentionally absent because the advertised loopback URL
is HTTP. The credential is never placed in the URL or shell body.

`/data.js` and `/live.json` require that exact session cookie. They also reject
HTTP 403 when a present `Sec-Fetch-Site` is anything except `same-origin` or
`none`; this includes `same-site` requests from another loopback port, where a
Strict cookie alone would not create an origin boundary. A non-browser client
may omit Fetch Metadata, but still needs the exact Host and must first obtain
the session through `/`. Sensitive successes and refusals are `no-store`, and
all live resources carry `Cross-Origin-Resource-Policy: same-origin`.

This boundary prevents a web origin from importing the executable live
snapshot or polling its status. It does not attempt to isolate the model from
another local process running as the user, because such a process can make the
same two raw loopback requests. Static export has no session boundary: its
`data.js` remains a self-contained file suitable for `file://` and GitHub
Pages.

The recursive watcher ignores root `.git`, the repository-root `target`, and
exporter staging paths. A nested project path such as
`examples/target/...` remains relevant and triggers reload. It coalesces bursts, rebuilds the complete state/model off the
read lock, and atomically replaces the snapshot only after successful
validation. Watcher errors are reported through `/live.json` rather than
terminating the server.

### Live status lifecycle

| Event | Snapshot | generation | reload_error | watcher_error |
|---|---|---|---|---|
| Initial state | initial good snapshot | `0` | `null` | `null` |
| Successful relevant batch at sequence S | replaced atomically | increment once, saturating at u64::MAX | `null` | clear only when S > recorded watcher-error sequence; otherwise unchanged |
| Invalid reload | last good snapshot retained | unchanged | reload error message | unchanged |
| Watcher failure at sequence W | last good snapshot retained | unchanged | unchanged | watcher error message recorded with W |
| Later successful relevant batch at sequence S > W | replaced atomically | increment once, saturating at u64::MAX | `null` | `null` |

Event sequences increase monotonically. Thus a successful relevant batch
clears a watcher error only when its sequence is strictly greater than the
recorded watcher-error sequence; an older or same-sequence success may clear
`reload_error` and increment `generation`, but it retains `watcher_error`.

### Initial live status

The status response initially has the exact three-field shape:

```json
{"generation":0,"reload_error":null,"watcher_error":null}
```

Serving and reloading never write project or Telos bytes.

### `rebuild plan|status`

Both subcommands are read-only, have `command: "rebuild"`, make no LLM call,
and never generate application code. They are the only model/graph consumers
that admit spec-only mode. Config read also works without a lock, but reads
project metadata rather than loading the model or graph. If a lock entry
exists, it must be readable and the project may be `coherent` or `changing`;
`drifted` returns `TELOS_DRIFT_DETECTED`. In `changing`, every parseable open
change is folded by ascending change ID, journals are folded, cross-change
claims and semantic integrity are revalidated, and each work pack still
follows the exact public `telos pack` owner resolution.

Spec-only discovery calls the same `config.validate_self()` used by sealed
consumers before loading a model; an invalid glob cannot be smuggled through
`rebuild plan` or `rebuild status` merely because no lock exists. Runner
grammar is validated when a surface is about to execute it. Config write
retains the one discovered workspace/config snapshot instead of opening a
second discovery window.

#### Plan

`result.steps` contains every intent, including draft and deprecated intents.
The order is a deterministic topological order over `requires`: each direct
prerequisite precedes its dependent, and ready ties use ascending intent ID.
Each row has exactly `n` (one-based), `intent`, `requires` (sorted direct
prerequisites), and `pack`. `pack` is the complete frozen `telos pack
INT-NNNN` result: `id`, `owner`, `change`, `canonical`, `scenarios`, `notions`,
`constraints`, `bindings`, `mappings`, and one-hop `neighbors`.

```json
{
  "ok": true,
  "command": "rebuild",
  "result": {
    "steps": [
      {"n": 1, "intent": "INT-0017", "requires": [], "pack": {"id": "INT-0017", "owner": {"context": "billing", "capability": "invoicing"}, "change": null, "canonical": "...", "scenarios": [{"id": "SCN-0091", "title": "a newly issued invoice is open", "proved": false}], "notions": ["..."], "constraints": ["..."], "bindings": {"implements": [], "proves": []}, "mappings": [], "neighbors": ["..."]}},
      {"n": 2, "intent": "INT-0042", "requires": ["INT-0017"], "pack": {"id": "INT-0042", "owner": {"context": "billing", "capability": "settlement"}, "change": null, "canonical": "...", "scenarios": [{"id": "SCN-0107", "title": "full payment settles the invoice", "proved": false}], "notions": ["..."], "constraints": ["..."], "bindings": {"implements": [], "proves": []}, "mappings": [], "neighbors": ["..."]}}
    ]
  },
  "error": null,
  "next_actions": []
}
```

The `"..."` entries above denote values with the exact nested `telos pack`
schema, not omitted response keys; executable contract tests compare every
real step to the full public pack result.

#### Status and real measurement

`rebuild status` executes the configured `[test] cmd` once for every distinct
`proves` target globally, in structural `(path, optional test name)` order,
with the target substituted for `{filter}`. One target shared by multiple
scenarios is still invoked once globally; the identical cached outcome and
display command are projected into every owning row. A scenario is green iff
it has at least one proof and **all** proof targets are safe, present,
resolvable, and exit zero. With `[test] report` configured, "exit zero"
becomes "the run's report gives the row's scenario a green verdict" (the
`test` section's rule); a target shared by several scenarios is still run
once, and its cached report is judged once per scenario, so two rows on one
target may differ. A run that proves nothing, no proof, a missing/unsafe file, a stale named
test, or any non-zero runner exit all produce an explanatory red row, not a
command failure.
A missing or blank runner is `TELOS_TEST_NOT_FOUND` because progress cannot
be measured. Each test row has exactly `test`, `green`, and the literal
substituted `command`; each scenario row has `id`, aggregate `green`, and
`tests`.

```json
{
  "ok": true,
  "command": "rebuild",
  "result": {
    "scenarios_green": 1,
    "scenarios_total": 2,
    "scenarios": [
      {"id": "SCN-0091", "green": false, "tests": []},
      {"id": "SCN-0107", "green": true, "tests": [{"test": "tests/billing.rs::scn_0107_full_payment_settles_the_invoice", "green": true, "command": "git hash-object .green-scn_0107_full_payment_settles_the_invoice"}]}
    ]
  },
  "error": null,
  "next_actions": []
}
```

The configured runner is trusted project code and can have effects; Telos
only contains its filter as data and reports its exit status. `check` and
`status` do not execute scenario proofs or constraint `check` commands.
`rebuild status` executes scenario proofs only. Constraint checks execute
during ordinary reconcile when impacted and during full reconcile for every
configured constraint. `tests_run` counts runner invocations, not scenario
declarations.

For an ordinary reconcile, a production path claimed by the current change
through an `accept` op or a journal `bind` makes every intent it implements
impacted. Gate 11 therefore reruns the distinct proof targets of every
scenario on all those intents before the path's current bytes may enter the
new seal. A path claimed only by another open change is still carried over at
its previously sealed OID and does not enter this transaction's impacted set.

### Proof and constraint execution matrix

Every scenario/test execution above is judged by the exit status without
`[test] report` and by the report with it (`test` section).

| Surface | Scenario/test execution | Constraint check execution |
|---|---|---|
| `status` | none | none |
| `check` | none | none |
| `check --sealed` | none | none |
| `rebuild plan` | none | none |
| `rebuild status` | every distinct bound proof target | none |
| ordinary reconcile without EditConfig | distinct impacted proof targets | impacted applicable constraints |
| ordinary reconcile with EditConfig | every distinct proof target | every applicable constraint |
| full reconcile, all intents draft/deprecated | none (tests_run: 0) | every configured constraint check |
| full reconcile, at least one active intent | whole suite once (tests_run: 1) | every configured constraint check |

### Sealability matrix

| Effective model/config | Seal verdict |
|---|---|
| no active intent | proof bindings and runner not required |
| active scenario without proves | refuse TELOS_INTEGRITY_VIOLATION before checks/tests/writes |
| all active scenarios proved, runner blank/whitespace | refuse TELOS_TEST_NOT_FOUND before checks/tests/writes |
| all active scenarios proved, runner nonblank | admit the later constraint/test gates |

### Filesystem publication threat model

Repository data paths are separately constrained by portable parsing plus
capability-rooted/no-follow access. Init, merged agent owners, and export cover
negligence and ordinary, non-adversarial filesystem concurrency, including an
IDE save in the documented validation-to-publication windows. Staging and
backup entries use 128-bit CSPRNG sibling names and are identity-authenticated
before cleanup; randomness avoids accidental collision, but is not treated as
an authorization secret. Consistent with Telos's storage model,
publication does not claim resistance to a malicious same-UID process able to
observe a name and substitute a path entry between syscalls.

| Surface | Covered environment | Guarantee |
|---|---|---|
| init | negligence and ordinary concurrent filesystem owners | no owner overwritten; failed partial init authenticated and safely resumable |
| agent merged owners | ordinary IDE save after validation | content/identity CAS refuses and restores without losing either version |
| export | negligence and ordinary concurrent filesystem owners | no owner overwritten; no final destination on error |
| repository proof/hash/read/write/restore | malformed paths and symlink redirection | portable normalization plus capability-anchored, no-follow traversal remains below the opened repository root |
| init, agent merge, and export | adversarial same-UID path substitution between syscalls | excluded by the documented threat model |

### `init [--agents ...] [--ci github]`

Fresh init admits only an absent `telos/` tree or the empty canonical
`notions/`, `intents/`, `constraints/`, and `changes/` directories. Before it
creates `.telos-init.json`, any other file, directory, byte-bearing entry,
live/dangling symlink, core owner (`telos.toml`, bindings, counters, lock), or
active-unproved prepopulation is `TELOS_ALREADY_INITIALIZED`; the entire tree
and integrations remain byte-for-byte unchanged. Only an authenticated marker
with exact options, phase, core definitions, bytes, and canonical directory
shapes can resume. The generated empty configuration/model passes `validate_self`,
semantic integrity, and the same sealability predicate as full reconcile before
the initial lock is published.

`--ci github` participates in one preflight with every requested agent host.
All target bytes, parent directories, JSON/text merges, path types, and
collisions are validated before the first initialization write. A preflight
failure leaves `telos/`, `.gitattributes`, counters, agent artifacts,
workflow, and init marker absent or byte-for-byte unchanged.

After preflight, `.telos-init.json` records a versioned transaction, normalized
agent/CI options, exact core snapshots, and an authenticated phase. Persisted
CAS transitions authorize core publication and then integrations. Within the
filesystem publication threat model above, every created artifact is fully
staged and synced under a CSPRNG sibling before a no-replace publish; user
configuration merges are recomputed/compared before atomic replacement. No
final file is opened with truncate; any late file, directory, live symlink,
dangling symlink, parent replacement, or staging-name owner introduced by
ordinary concurrency is preserved rather than overwritten or removed.

If publication fails after the marker exists, a retry is allowed only with
the same normalized `--agents`/`--ci` options and only while the marker,
phase, core bytes, canonical directory shapes, and already-published Telos
artifacts remain exact. Exact artifacts are safe no-ops, user merges are idempotent, and
the retry completes the missing agents/workflow before removing the marker.
Different options or any foreign byte/path owner are refused without further
publication. Once the marker is gone, ordinary repeat init remains
`TELOS_ALREADY_INITIALIZED`.

### Init publication and resume matrix

| Starting condition | Result | Overwrite policy |
|---|---|---|
| clean, all preflights valid | seal core, publish requested agents/CI, remove marker | create-only or validated atomic merge |
| preflight error/collision | refuse before marker/core writes | every owner preserved |
| authenticated incomplete init, same options | resume exact phase and finish | exact artifacts are no-ops; missing artifacts no-replace |
| authenticated incomplete init, different options | refuse | partial authenticated state preserved |
| marker/core/parent changed or foreign owner appears | refuse | foreign and prior bytes preserved |
| completed init, no marker | TELOS_ALREADY_INITIALIZED | completed project preserved |

The success envelope remains the frozen init envelope, independent of agent
hosts or CI:

```json
{"ok":true,"command":"init","result":{"root":"telos","sealed":true},"error":null,"next_actions":["telos status"]}
```

An occupied workflow target returns exactly:

```json
{"ok":false,"command":"init","result":null,"error":{"code":"TELOS_CHANGE_STATE_INVALID","message":"`.github/workflows/telos.yml` already exists","hint":"preserve or move the existing workflow before retrying"},"next_actions":[]}
```

The generated file is exactly:

```yaml
name: Telos

on:
  pull_request:
  push:
    branches: [main]

permissions:
  contents: read

jobs:
  sealed:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - name: Install Telos v0.13.0
        run: |
          version=0.13.0
          asset="telos_${version}_linux_amd64.tar.gz"
          base="https://github.com/hugues31/telos-sdd/releases/download/v${version}"
          cd "$RUNNER_TEMP"
          curl -fsSLO "${base}/${asset}"
          curl -fsSLO "${base}/checksums.txt"
          sha256sum --check --ignore-missing checksums.txt
          tar -xzf "${asset}"
          install -D -m 0755 telos "$HOME/.local/bin/telos"
          echo "$HOME/.local/bin" >> "$GITHUB_PATH"
      - name: Verify sealed Telos state
        run: telos check --sealed
```

The downloaded release version is derived from the CLI package version.
Shipping 0.13.0 therefore requires release `v0.13.0` to carry the
`telos_0.13.0_linux_amd64.tar.gz` and `checksums.txt` assets; without them the
generated install step cannot succeed. The workflow reports a check but does
not itself make GitHub treat it as required: repository branch protection
must separately require job `sealed` before merges.

### Spec-only bootstrap and public reconstruction

Spec-only mode is intentionally narrow: `rebuild plan` and `rebuild status`
can inspect it, while live/export and ordinary sealed-project commands still
require a lock. The public Billing base contains only README and Telos files:
no lock, Cargo manifest/lock, source, tests, generated checker/site, journal,
binding, build artifact, hidden solution, or LLM call. Both intents are
`draft`; the architecture constraint is declarative; the future runner is
already configured as `cargo test {filter}`.

On the untouched copy, `rebuild plan` orders `INT-0017` before `INT-0042` and
`rebuild status` returns `0/2` without launching a process because neither
scenario has a proof target. The first seal uses the real CLI spelling
`telos change reconcile --full --json`. With no active intent and no
constraint `check`, it returns `tests_run: 0`, `checks_run: 0`, creates the
lock, and leaves measured progress `0/2`.

An external `telos-implementer` then executes ordinary prerequisite-ordered
batches. CHG-0001 stages a real complete `INT-0017` `draft` → `active` edit
and adds the machine `CON-0003.check`. Outside `telos/`, the implementer
chooses and creates its own Cargo/source/test solution from the bounded context;
the demo README contains no manifest, source, test, or extractable solution
bytes. The batch records an unchanged red-to-green witness, binds every covered
implementation input to the intent, records the discovered `proves`, and
reconciles to `1/2`.

CHG-0002 stages the real `INT-0042` `draft` → `active` edit and follows the
same red/green/bind/reconcile lifecycle. Real forbidden `crate::adapters`
imports make the constraint return exact `TELOS_CONSTRAINT_FAILED`; removing
only those imports lets the same approved change reconcile to `2/2`.
Every `[tests]` file has a canonical proof binding, both changes disappear,
and final `telos check --sealed` plus `rebuild status` prove the reconstructed
tree. Rebuild proves behavioral conformity; source-byte identity depends on
how fully constraints capture architecture.

The repository's `rebuild_demo` test is a protocol/conformance harness. Its
private fixture constants live under `crates/telos/tests/`, outside the public
demo, and model one possible external implementer twice. They prove the public
CLI lifecycle and `0/2 → 1/2 → 2/2` checkpoints, not that Telos generated code
or that the demo disclosed a preferred solution.

### Billing reconstruction checkpoints

| Checkpoint | Intent statuses | tests_run | checks_run | Rebuild status |
|---|---|---|---|---|
| untouched spec-only | draft, draft | no process | no process | 0/2 |
| change reconcile --full bootstrap | draft, draft | 0 | 0 | 0/2 |
| CHG-0001 reconciled | active, draft | one distinct scenario proof | staged architecture check | 1/2 |
| CHG-0002 reconciled | active, active | one distinct scenario proof | architecture check | 2/2 |
