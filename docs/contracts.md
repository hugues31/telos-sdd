# telos CLI contracts

This document is the frozen reference for everything an agent (an M3 skill,
or any other tool) routes on without interpretation: the `--json` envelope
shape, the 17 error codes and their canonical hints, the `status --json`
schema, `check`'s semantics, and — as of M2 — the whole change/transaction
surface (`show`, `change open|list|abandon|diff|approve|reconcile`,
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
  `"config"`, `"status"`, `"view"`, `"check"`, `"show"`, `"list"`,
  `"query"`, `"impact"`, `"context"`, `"rebuild"`, `"change"`, `"add"`,
  `"edit"`, `"remove"`, `"adopt"`, `"revert"`, `"test"`, `"bind"`. All six
  `change` subcommands (`open|list|abandon|diff|approve|reconcile`) answer
  under the single `"change"` value — the envelope names the command a
  caller invoked, and `telos change …` is one command with subcommands, the
  same way `telos query …` is one `"query"`.
- `result` — the command's payload on success; `null`, never absent, on
  failure.
- `error` — `null`, never absent, on success; on failure, the frozen
  three-key error body below.
- `next_actions` — suggested follow-up invocations, e.g.
  `["telos adopt", "telos revert"]`. Empty, never absent, when there is
  nothing to suggest — always empty on failure.

No key is ever omitted (no `skip_serializing_if` anywhere in the
implementation): a consumer indexes every key unconditionally instead of
checking whether it is there.

### Canonical `command` values

This table is the complete public set in 0.7. Subcommands share their parent
value: both `rebuild plan` and `rebuild status` answer as `"rebuild"`, and
every `change` subcommand answers as `"change"`. The hidden `agent-guard`
host-hook entry point is intentionally excluded: it is not a public JSON
envelope surface.

| Value | CLI surface |
|---|---|
| `version` | `telos version` |
| `init` | `telos init` |
| `config` | `telos config [--change CHG-NNNN]` |
| `status` | `telos status` |
| `view` | `telos view [--port N] [--export DIR]` |
| `check` | `telos check [--sealed]` |
| `show` | `telos show` |
| `list` | `telos list` |
| `query` | `telos query` |
| `impact` | `telos impact` |
| `context` | `telos context` |
| `rebuild` | `telos rebuild plan` or `telos rebuild status` |
| `change` | `telos change ...` |
| `add` | `telos add` |
| `edit` | `telos edit` |
| `remove` | `telos remove` |
| `adopt` | `telos adopt` |
| `revert` | `telos revert` |
| `test` | `telos test` |
| `bind` | `telos bind` |

### The error body

```json
{ "code": "TELOS_DRIFT_DETECTED", "message": "...", "hint": "..." }
```

- `code` — one of the 17 [error codes](#error-codes) below, as
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

Seventeen codes: the 9 frozen by spec §8, 7 M1 extensions, and M3's
`TELOS_TEST_NOT_FOUND`. M3 now emits `TELOS_SCENARIO_RED_EXPECTED` and
`TELOS_TEST_SEALED` while reconciling strict TDD changes. Variants are never
renamed or removed, only added — this is the whole contract an M3 skill
routes on.

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

### Detailed emission cases

| Emission | When | Hint |
|---|---|---|
| `TELOS_DRIFT_DETECTED` | The project's state is `drifted` — *not* merely "not `coherent`": a `changing` project (an open change, nothing unclaimed) does **not** trigger this code, only genuine unclaimed drift does (a sealed path modified or missing, or an unsealed spec file on disk). Emitted by `check --sealed` in M1; from M2 on, also gates `change open`, `add`/`edit`/`remove`, `change approve`, and `change reconcile` *without* `--full` (`--full` never reads the lock, so it is exempt — see the `change reconcile` section below). `change diff`/`list`/`abandon`, `status`, `check` without `--sealed`, and `show` never gate on it — they read, or they clean up, and a drifted project is exactly when a caller needs them most. | `` run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert` `` |
| `TELOS_APPROVAL_STALE` | (M2) `change reconcile`'s digest gate: a change's approval no longer matches its ops digest, because the delta was staged into again (`add`/`edit`/`remove`) after `telos change approve` — staging into an approved change is deliberately allowed. | `` re-approve with `telos change approve CHG-0001` `` (id-carrying, not a bare instruction to re-run `diff`) |
| `TELOS_REFERENCE_UNKNOWN` | A reference in the spec — a notion, an attribute, an enum symbol, an intent/scenario/constraint id — does not resolve. Emitted by the semantic pass on `load_model` (rule §3.3 №1); from M2 on, also rejected at write time (`add`/`edit` payloads, and the whole delta a staged change describes). | None. The engine folds its best guess directly into `message` (`` ; closest is `Invoice` ``) when one is close enough; there is nothing to add. |
| `TELOS_REFERENCE_UNKNOWN` | A `show`/`impact` argument, or `query`'s `--using`/`--triggered-by`, is a well-formed id or notion name that resolves to nothing in the loaded spec (message `` unknown notion `Invoice` ``, `` unknown intent `INT-9999` ``, `` unknown scenario `SCN-9999` ``, or `` unknown constraint `CON-9999` ``). | `` closest is `Invoice` `` (edit distance, for a notion name, backtick-quoted) or `closest is INT-0042` (numeric distance, for a typed id, *not* backtick-quoted) — present only when a candidate is close enough; `null` otherwise. |
| `TELOS_REFERENCE_UNKNOWN` | A `show`/`impact` argument is neither a typed id nor a valid notion name at all (message `` cannot parse `x` as an id or notion name ``). | None. |
| `TELOS_REFERENCE_UNKNOWN` | An `impact` argument names a change (`CHG-…`) — a change is a transaction record, not a node of the spec graph, so it has no relations to walk (message `` `impact` does not apply to changes ``). `show CHG-…`, unlike `impact`, *does* resolve — it reads the change store directly rather than the graph; see the `show` section below. | None. |
| `TELOS_REFERENCE_UNKNOWN` | (M2) `change abandon`/`change diff`/`change approve`/`change reconcile <id>`/`add\|edit\|remove --change`/`adopt --into` is given a value that does not even parse as a `CHG-NNNN` id — a distinct, earlier check from the next row's "well-formed but unknown" (message `` cannot parse `x` as a change id ``). The same family covers `edit`/`remove`'s `<key>` argument for an intent or a constraint (message `` cannot parse `x` as an intent id `` / `` cannot parse `x` as a constraint id ``) and a notion (message `` cannot parse `x` as a notion name ``) — one dedicated message per expected kind, since the command already knows which kind it asked for. | None. |
| `TELOS_REFERENCE_UNKNOWN` | (M2) `show`/`change abandon`/`change diff`/`change approve`/`change reconcile <id>`/`add\|edit\|remove --change`/`adopt --into` name a well-formed `CHG-NNNN` id the store does not hold (message `` unknown change `CHG-9999` ``). | `closest is CHG-0001` (numeric distance) — present only when another change exists; `null` otherwise. |
| `TELOS_SCENARIO_RED_EXPECTED` | (M3) `reconcile` under `policy.tdd = "strict"` requires an intact sealed red witness for a scenario before its green run; none exists. | Run `telos test SCN-…` to record a red witness before implementing. |
| `TELOS_TEST_SEALED` | (M3) The bytes of a test file sealed as a red witness changed before the scenario went green — the witness no longer proves anything. | The red witness is invalid; run `telos test SCN-…` again on the current bytes before reconciling. |
| `TELOS_TEST_NOT_FOUND` | (M3) No `[test] cmd` is configured; discovery finds zero or more than one file containing the scenario's `scn_NNNN` convention; or `--file` names no file. | The exact cases follow this table. |
| `TELOS_ORPHAN_CODE` | (M2) `change reconcile`'s no-code-without-telos gate (rule §3.3 №5, over the delta's post model): a file matched by `[code]`/`[tests]` globs in `telos.toml` is not covered by any `implements`/`proves` binding (message names which of the two families and the binding relation it's missing). | Bind it with `telos bind <path> <INT-id>`, or remove it from the `telos.toml` globs if it isn't spec-governed. |
| `TELOS_CONSTRAINT_FAILED` | (M2) `change reconcile`'s constraint-checks gate: a constraint's `check` shell command exited non-zero, or could not even be spawned (message `` CON-0001 check failed: `<cmd>` ``). The command's own output is deliberately *not* included — it is not reproducible across machines (a git version, a locale, `$PATH`), so it cannot be frozen contract. | Run the constraint's `check` command directly to see its output. |
| `TELOS_CHANGE_STATE_INVALID` | (M2) `change reconcile <id>` on a change whose status is not `approved`/`implementing` (message `` change CHG-0001 is not approved; approve it first ``). | `` run `telos change diff CHG-0001` then `telos change approve CHG-0001` `` |
| `TELOS_CHANGE_STATE_INVALID` | (M2) `change approve` on a change with no staged ops — `open`, with nothing added yet (message `` change CHG-0001 has no staged operations ``). | `stage operations with telos add\|edit\|remove first` |
| `TELOS_CHANGE_STATE_INVALID` | (M2) `adopt`/`revert` run when the project has *not* drifted — both commands exist only to leave the drifted state (message `` nothing to adopt: the project has not drifted `` or `` nothing to revert: the project has not drifted ``). | `` run `telos status` to see the project's state `` |
| `TELOS_CHANGE_STATE_INVALID` | (M2) `check --sealed` on a project that is `changing` — "sealed and unmodified" cannot be true while a change is open, and that is a different remedy from drift, hence its own code (message `open changes; reconcile or abandon them`). | `` run `telos change list` `` |
| `TELOS_FILE_CLAIMED` | (M2) A file targeted by `add`/`edit`/`remove`, or by `adopt`'s plan, is already claimed by a different, concurrently open change — one file, one change (message `` <path> is already claimed by CHG-0001 `` — the path is **not** backtick-quoted inside the message). | `` reconcile or abandon CHG-0001 first, or work within it `` (id-carrying) |
| `TELOS_NOT_INITIALIZED` | No `telos/telos.toml` found walking up from the current directory. | `` run `telos init` at the repository root `` |
| `TELOS_NOT_INITIALIZED` | `telos/telos.toml` exists, but `telos.lock` is missing (`status`, `check --sealed`). `telos init` always seals, so this is not "unsealed" — it's abnormal. | `` the project was never sealed; run `telos init` in a fresh repository or restore telos.lock from git `` |
| `TELOS_ALREADY_INITIALIZED` | `telos init` run on a project that already has `telos/telos.toml`. | `` project already initialized; see `telos status` `` |
| `TELOS_PARSE_ERROR` | A `.tel` file (or `telos.lock`, or `telos.toml`) is syntactically invalid (`load_model`, `check`, `change diff`'s base parse, …). | None today — `message` names the offending file and, when the parser can determine it, the line and column. **Exception:** `adopt` on a drifted `.tel` file it cannot parse forces this same code but replaces the hint with `ADOPT_PARSE_HINT`; see the `adopt` section below. |
| `TELOS_PARSE_ERROR` | (M2) An `add`/`edit` payload on stdin is not a JSON object, or its shape does not match the payload schemas section below (Annex D) (`message` prefixed `` payload: `` — e.g. `` payload: missing required field `title` in intent payload ``). A handful of exact wordings are frozen verbatim without that prefix: an unknown key (`` unknown key `titel` in notion payload ``), an unknown closed-set word (`` unknown attribute type `txt`; expected one of string, int, decimal, money, bool, date, datetime, enum, ref ``), a `decimal` value sent as a JSON number, and a malformed `set` action. | None today. |
| `TELOS_INTEGRITY_VIOLATION` | A rule §3.3 violation with no dedicated hint: `seal` finding a binding to a code file that doesn't exist on disk, an entity declared twice, or (M2) `remove`/`adopt` leaving a still-referenced entity behind (`cannot remove <entity>: <referrer>`). | None today — `message` names the offending path or entity. |
| `TELOS_INTEGRITY_VIOLATION` | (M2) `change reconcile`'s accept-OID gate: an `accept` op's path changed, or vanished, since `adopt` recorded its OID (message `` `<path>` changed since it was accepted `` or `` `<path>` was accepted but no longer exists ``). | `` re-run `telos adopt` to accept the current bytes of `<path>` `` |
| `TELOS_INTEGRITY_VIOLATION` | (M2) `change reconcile`'s test gate: the `[test] cmd` run for an impacted scenario's `proves` target (or, under `--full`, the whole suite once when at least one intent is active) exited non-zero or could not be spawned (message `` the test run for `<target>` failed: `<substituted cmd>` ``). A full reconcile with only draft/deprecated intents invokes no runner. The command's own stdout/stderr is deliberately not included, for the same reproducibility reason as `TELOS_CONSTRAINT_FAILED`. | `run the command directly to see why it fails, then reconcile again` |
| `TELOS_INTEGRITY_VIOLATION` | (M2) An `edit notion` payload changes the notion's `name` — a staged op cannot rename an entity, since the op's target path is derived from the entity's identity (message `` cannot rename notion `<from>` to `<to>` ``). | `` stage `remove notion <from>` and an `add` of the new one instead `` |
| `TELOS_INTEGRITY_VIOLATION` | (M2) `adopt` cannot express the deletion of a file that carries no entity of its own: a bound code file (message `` cannot adopt: bound file `<path>` was deleted ``) or an unbound opaque file such as `telos.toml` (message `` cannot adopt: `<path>` was deleted ``). | `` restore it with `telos revert`, or remove its binding `` for a bound file; `` restore it with `telos revert` `` for an unbound one. |
| `TELOS_INTEGRITY_VIOLATION` | (M2) `adopt` finds a drifted `.tel` file whose declared entity belongs at another path — adopting it as-is would capture the wrong path and leave the real drift uncaptured (message `` `<path>` declares an entity that belongs in `<declared-path>` ``). | `` rename the file to match the entity it declares, or the entity to match the file `` |
| `TELOS_INTEGRITY_VIOLATION` | (M2) `adopt` finds a *missing* entity file whose file name is not a valid identity, so not even its deletion can be expressed (message `` cannot read an entity identity from `<path>` ``). | `` restore `<path>` with `telos revert` `` |
| `TELOS_INTEGRITY_VIOLATION` | (M2) `revert` finds a drifted path (`Modified`/`Missing`) the lock does not seal — defensive, since `compute_state` should not be able to produce this (message `` `<path>` is not sealed; there is nothing to restore it from ``). | `` run `telos change reconcile --full` to reseal the project `` |
| `TELOS_CYCLE_DETECTED` | A cycle exists on `requires` or `refines`. | None today — `message` renders the cycle's path (`` INT-0001 → INT-0002 → INT-0001 ``). |
| `TELOS_GIT_ERROR` | `git rev-parse --show-toplevel` failed (most commonly: not inside a git repository). | `` not a git repository; run `git init` `` |
| `TELOS_GIT_ERROR` | The `git` binary itself could not be spawned (missing from `PATH`). | None — `message` names the underlying I/O error. |
| `TELOS_GIT_ERROR` | (M2) `revert`'s `git cat-file blob <oid>` fails — the sealed OID names a blob the object store does not hold (a project sealed but never committed; message `` `git cat-file blob <oid>` failed: <stderr> ``). **Not** `TELOS_INTEGRITY_VIOLATION` — a seal records OIDs, it never writes objects, so a missing blob is git's own diagnosis, not a spec integrity one. | the frozen `MISSING_BLOB_HINT`: `` the sealed content is not in the git object store; commit the sealed state or restore the file by hand `` |
| `TELOS_INTERNAL` | An internal invariant broke — a bug, not a spec or usage problem. | None. |

`TELOS_TEST_NOT_FOUND` has four exact M3 forms. No runner is
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

### `result` schema

```json
{
  "state": "coherent",
  "changes": [],
  "drift": null,
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
  (an unclaimed drift outranks it — see `drift` below); `"changing"` never
  occurred in M1, since no command opened a change yet, but does from M2 on
  (`change open`, or `adopt`).
- `changes` — open changes, best-effort (an unparseable change file still
  appears, with `status: "open"` and a repair obligation, rather than
  blocking `status`). Always `[]` in M1; from M2 on, really produced, one
  item per open change: `{"id": "CHG-0007", "status": "implementing", "obligations": ["..."]}`.
  `obligations` is the frozen, status-keyed list of what remains — see the
  `change` section below.
- `drift` — `null` when `state` isn't `"drifted"`; otherwise:
  ```json
  { "paths": ["telos/notions/Invoice.tel"], "suggestion": "telos adopt" }
  ```
  `paths` is sorted, and lists every drifted path — modified, missing, or
  unsealed-but-present — without distinguishing which kind (that
  distinction exists internally as `DriftKind`, but M1's frozen schema
  exposes paths only).
- `coverage` — exact counts off the loaded model, or all zeros if the spec
  didn't parse. `scenarios_proved` counts scenarios with ≥ 1 `proves`
  binding; `intents_implemented` counts intents with ≥ 1 `implements`
  binding.

`next_actions` is `["telos adopt", "telos revert"]` when `state` is
`"drifted"`; `["telos change list"]` when `state` is `"changing"`; `[]` when
`"coherent"`.

## `check [--sealed]`

Parses the spec and checks its integrity (rules §3.3 №1, №3, №4 — the ones
that hold at read time; №2 and №5 are write-time/reconcile-time concerns,
M2).

### Without `--sealed`

`check` never touches `telos.lock`. It calls `load_model`:

- **All parses, all resolves**: `ok: true`, `result: {"diagnostics": []}`.
- **One or more diagnostics**: `ok: false`, `result: null`, exit `1`.
  `error` is the *first* diagnostic, converted to the frozen error triple
  (`code`, `message`, `hint`).

  **M1 limitation**: the frozen envelope has room for exactly one error, but
  `load_model` collects *every* diagnostic in one pass, never just the
  first. To keep all of them visible without growing the envelope past its
  frozen five keys, `error.message` becomes multi-line when there is more
  than one diagnostic — one `file: message` line per diagnostic, in the
  order they were found, starting with the same line `error.code` and
  `error.hint` describe. An agent reading only `error.code`/the first line
  of `error.message` gets the primary diagnosis and can re-run `check`
  after fixing it; a human reading `error.message` (or stderr in
  human-mode) sees everything found in this run. A future milestone may
  promote this into `result.diagnostics` on failure instead — this is
  explicitly an M1 shape, not a permanent one.

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
4. (M2) Otherwise, if `state == "changing"` (at least one change is open) →
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

## `context <INT-id|SCN-id>`

`context` returns the bounded implementation pack for one intent. A scenario
argument resolves to its owning intent; notions, constraints, and changes are
well-formed but inapplicable (`TELOS_REFERENCE_UNKNOWN`, message
`` `context` applies to intents and scenarios ``, null hint). It is a read
command: `next_actions` is always `[]`.

```json
{
  "id": "INT-0042", "change": null,
  "canonical": "intent INT-0042 …\n",
  "scenarios": [{"id": "SCN-0107", "title": "…", "proved": true}],
  "notions": [{"name": "Invoice", "canonical": "notion Invoice …\n"}],
  "constraints": [{"id": "CON-0003", "scope": "global", "canonical": "constraint CON-0003 …\n"}],
  "bindings": {"implements": ["src/billing/invoice.rs"],
               "proves": [{"scenario": "SCN-0107", "test": "tests/billing.rs::scn_0107"}]},
  "neighbors": [{"id": "INT-0017", "title": "…", "rel": "requires", "direction": "out"}]
}
```

`change` is the owning open change id for a staged add/edit and `null` for
the disk model. Its post-overlay model applies the owner’s operations
idempotently, folds the owner’s journal into bindings, then re-runs semantic
validation. Thus `proved`, `implements`, and `proves` expose live
journalled evidence; context never uses the entire spec as a prompt. Scenarios
remain in the intent’s order; notions sort by name; constraints by id;
implements by path; proves by `(scenario, test)`; neighbours by
`(relation, id, direction)`. Neighbours are only one-hop `refines`,
`requires`, or `excludes` intent edges.

## `test <SCN-id|--all> [--file <path>]`

Runs the configured `[test] cmd` and appends an immutable witness journal
record to the approved or implementing change whose staged intent owns the
scenario. A non-zero runner exit is **red evidence, not a command failure**:
the command still exits zero and records the exact blob OID of the test file
that was run. A zero exit records green. The test command does not parse test
runner output, so it cannot distinguish a zero-test run from green.

Without `--file`, discovery requires exactly one `[tests]`-globbed file whose
contents contain the identifier-boundary convention `scn_NNNN`; zero or more
than one match is `TELOS_TEST_NOT_FOUND`. `--file <path>` selects that file
directly and still derives the matching function name when present. A single
run returns:

```json
{"scenario":"SCN-0108","witness":"red|green",
 "test":"tests/billing.rs::scn_0108_x","change":"CHG-0001",
 "command":"cargo test scn_0108_x"}
```

Red returns `next_actions: ["telos test SCN-0108"]`; green returns
`["telos change reconcile CHG-0001"]`. `test --all` witnesses every scenario
an open approved/implementing change owes, in scenario-id order, as
`{"runs":[…]}`, and has no next action. It requires exactly one of a
scenario and `--all` (usage errors remain exit 2 without an envelope).

The drift carve-out is exact-path only: a test run admits drift of the test
path it records, but refuses any other unclaimed drift. A journal record
moves `approved` to `implementing`; journal records are digest-inert and
therefore do not invalidate a prior approval. Re-running appends evidence.

## `bind <path> <INT-id>`

Records that an existing repository-relative code path implements an intent
in the approved or implementing change that stages that intent. `path` cannot
be absolute, escape through `..`, or name anything under `telos/`; the file
must exist. The ownership change must add or edit the intent, never merely
remove it. Its result is:

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
reseal. `implementing` (M3) is the same as `approved` for every gate below —
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
`["repair telos/changes/CHG-NNNN.tel (unparseable)"]`.

### `change open <motivation>`

Allocates the next `CHG-NNNN`, writes an empty change. Gated on *unclaimed
drift only* (`TELOS_DRIFT_DETECTED` when the project's state is `drifted`)
— **not** on `changing`: a second, third, … change may `open` freely while
another is already in flight, as long as nothing is unclaimed drift.

`result`: `{"id": "CHG-0001", "status": "open"}`. `next_actions`:
`["telos add intent --change CHG-0001"]`.

### `change list`

Every change the store holds, best-effort (an unparseable change file still
gets an entry — `status: "open"`, empty claims, the repair obligation of
the "Obligations" subsection above — rather than blocking the listing).
Never gated on drift or on anything else: cleanup and inspection stay
available in every project state.

`result`: `{"changes": [{"id", "status", "motivation", "obligations"}, ...]}`.
`next_actions`: always `[]`.

### `change abandon <id>`

Reads the change first (so a mistyped id gets `` unknown change `CHG-9999`
``, never a silent no-op), then deletes its file. Not gated on drift —
abandoning is one of the two ways out of a mess, not more mutation of the
spec.

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
at that path yet. `next_actions`: `["telos change approve CHG-0001"]` while
`status` is `open`/`drafted`, or while `stale` is `true`; otherwise
`["telos change reconcile CHG-0001"]`.

### `change approve <id>`

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
| 5 | effective configuration validation, then the overlay (the delta's post-spec must parse and resolve — rules 1/2/3/4 of §3.3) | invalid globs/configuration use their exact `TELOS_PARSE_ERROR` or `TELOS_INTEGRITY_VIOLATION`; otherwise whatever `TELOS_*` diagnostic the semantic pass raises first |
| 6 | rule 5, no code without telos, over the post model | `TELOS_ORPHAN_CODE` |
| 7 | sealed code coverage: every path in the previous lock's `code` table remains bound in the folded post-model, unless this delta stages `telos/bindings.tel` | `TELOS_INTEGRITY_VIOLATION`, `` sealing would drop `<path>` from the code table: no binding covers it and this change does not stage telos/bindings.tel ``; hint `` the bindings shrank outside this change; reconcile or abandon the change that claims telos/bindings.tel, or restore them with `telos revert` `` |
| 8 | sealed red/green witness for every new or changed scenario | `TELOS_SCENARIO_RED_EXPECTED` or `TELOS_TEST_SEALED` under strict policy; warnings under advisory policy |
| 9 | sealable structure: every active scenario has at least one `proves`, and any active obligation has a nonblank runner | `TELOS_INTEGRITY_VIOLATION`, ``active scenario SCN-NNNN has no `proves` binding``; then `TELOS_TEST_NOT_FOUND` for no runner |
| 10 | constraint checks, for the constraints this delta puts in scope | `TELOS_CONSTRAINT_FAILED` |
| 11 | tests, one run per distinct `proves` target of the impacted scenarios | `TELOS_INTEGRITY_VIOLATION`, `` the test run for `<target>` failed `` |

Only once gate 11 passes does anything reach disk: the spec `.tel` files
(through the emitter, in staged order), then the canonical folded
`telos/bindings.tel`, then `telos.lock`, then the change file's deletion.
`counters.toml` is never touched by reconcile — every id a transaction spends
was already persisted when the op was staged. Journal records are digest-inert:
they move an approved change to `implementing` but do not stale its approval.

Gate 8 is strict versus advisory: `policy.tdd = "strict"` refuses on the
first missing red witness, missing green witness, or changed witness bytes;
`"advisory"` reconciles and returns each same frozen verdict in
`result.witness_warnings`. Both `approved` and `implementing` changes owe
reconciliation, and a journal is folded into the post-model before gates 5–11.

#### The carry-over: drift another open change claims is never sealed here

Gate 1 admits drift *any* open change claims, not just this one's — a
concurrent change (from M3, an implementing change drifts its code files for
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
`result.ops_applied` is always `0` under `--full` (no ops — nothing was
staged, the state was simply found and re-proved). Open changes are
tolerated and left untouched (their files, still open), and the seal this
produces has `sealed_by: null` — no transaction produced it.

## `add`/`edit`/`remove <notion|intent|constraint>`

Stages one operation into an open change; nothing is written under `telos/`
until that change's `reconcile`. `add`/`edit` read a JSON payload from
stdin (Annex D, below); `remove` takes no payload. `--change CHG-0001` is
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

`add`/`edit`: `{"change": "CHG-0001", "entity": "intent", "id": "INT-0043", "scenario_ids": ["SCN-0108"], "claims": ["telos/intents/INT-0043.tel"]}`.
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

### Payload schemas (`add`/`edit`, Annex D)

One JSON object on stdin; an unknown top-level or nested key is refused
(`TELOS_PARSE_ERROR`, `` unknown key `x` in <kind> payload ``). `add` never
carries an id — `intent` and `constraint` get theirs from the allocator, a
notion's identity is its `name` field. `edit <kind> <key>` accepts the same
keys, all optional: a key present in the payload replaces that field
**wholesale** (a list field like `attrs`/`requires`/`scenarios` is the whole
new list, never a delta against the old one); a key absent keeps the base
entity's current value. `remove <kind> <key>` takes no payload.

**`add notion`**:
```json
{ "name": "Invoice", "kind": "entity",
  "def": "A bill issued to a Customer for delivered work.",
  "attrs": [ {"name": "state", "type": "enum", "values": ["open", "settled"]},
             {"name": "balance", "type": "money"},
             {"name": "customer", "type": "ref", "target": "Customer"} ],
  "rels":  [ {"name": "issued-to", "target": "Customer"} ] }
```
`attrs`/`rels` default to `[]` when absent.
`type` ∈ `string|int|decimal|money|bool|date|datetime|enum|ref`;
`enum` requires `values` (≥ 1 entry); `ref` requires `target`.

**`add intent`** (no `id` on the intent or on any scenario — every id is
allocated and reported back in `result`; steps carry their state under
`fields`):
```json
{ "title": "Invoices can be settled", "status": "active",
  "telos": "Customers must see immediately that their debt is cleared.",
  "statement": { "template": "event-driven", "when": "PaymentReceived",
                 "on": "Invoice", "action": "set Invoice.state = settled" },
  "refines": [], "requires": ["INT-0017"], "excludes": [],
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
`then` expressions are strings of the mini-language, parsed the same way
`.tel` source is. `action` is a string: if it starts with `set ` it must
parse to exactly `set <Notion.attr> = <literal>` (a dedicated parse error
otherwise); any other string is a free clause.

Typing a `fields` value against its attribute's declared type: `string` →
JSON string → `Str`; `int` → JSON integer → `Int`; `decimal` → JSON
**string** lexeme (`"120.50"` — a JSON number is refused, to avoid float
hazard); `money` → JSON string `"120.00 EUR"`; `bool` → JSON bool; `date`/
`datetime` → JSON string lexeme; `enum` → JSON string, the symbol itself, →
`Symbol`; `ref` → refused in M2. An unknown attribute is
`TELOS_REFERENCE_UNKNOWN` with a suggestion.

**`add constraint`**:
```json
{ "kind": "architecture", "title": "Hexagonal boundaries",
  "rule": {"text": "Domain code must not import adapter modules."},
  "scope": "global", "check": "scripts/check-imports.sh --layer domain" }
```
`rule` is `{"text": …}` **or** `{"expr": "Invoice.balance >= 0"}`; `scope`
is `"global"` or an array of intent ids; `check` is optional.

**`edit`**: same keys as the matching `add`, all optional, each replacing
its field wholesale when present. In `scenarios`, an entry carrying an
`"id": "SCN-0107"` replaces that scenario in place; an entry with no `id`
is newly allocated; a scenario of the base absent from the new list is
dropped. `"check": null` on a constraint explicitly clears it (an absent
`check` leaves it untouched). **`remove`**: no payload.

## `adopt [--into CHG-NNNN]` / `revert`

The two exits from drift (spec §6): capture it, or throw it away. Both are
gated the opposite way from every mutating command above —
`TELOS_CHANGE_STATE_INVALID`, `` nothing to adopt: the project has not
drifted `` / `` nothing to revert: the project has not drifted ``, hint
`` run `telos status` to see the project's state `` — since both exist only
to leave a `drifted` project, never a `coherent` or `changing` one.

### `adopt`

Turns every *unclaimed* drifted path into one staged op (`edit`/`add`/
`remove`/`accept`, chosen by where the path is and how it drifted) of a
change — a new one, or `--into`'s existing one. Canonicalizing: the op
carries the re-parsed entity, so `reconcile` writes back canonical bytes
and whatever whitespace the out-of-protocol edit introduced never reaches
the seal. After a successful `adopt` the project is `changing`, not
`coherent` — the drift is claimed, not yet resealed.

`result`: `{"change": "CHG-0002", "ops": 1, "paths": ["telos/notions/Invoice.tel"]}`.
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
safe to re-run). Needs the sealed content in the object store — a seal
records OIDs, it does not write objects — so a project sealed but never
committed gets **`TELOS_GIT_ERROR`** (not `TELOS_INTEGRITY_VIOLATION` — a
missing blob is git's own diagnosis, `git cat-file blob` failing, not a
spec integrity one) with the frozen `MISSING_BLOB_HINT` rather than
silently restoring nothing.

`result`: `{"restored": ["telos/notions/Invoice.tel"], "deleted": []}`.
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
hooks. Codex repository configuration is not assumed active: before relying
on the generated guard or rules, open `/hooks`, review and trust the repository
`.codex` layer, and verify the exact `telos agent-guard --host codex` hook.
Until that review and trust is complete, `.codex/hooks.json` and
`.codex/rules/telos.rules` must be treated as inactive. Once active, the guard
refuses direct agent writes to the repository `telos/` tree and accepts only
CLI-mediated mutations. Generated Codex rules request native human
confirmation for `telos change approve`, `telos adopt`, and `telos revert`.
Before approval, the challenger presents `change diff`’s `result.digest`;
before adopt/revert, the router presents the relevant drift paths. The rules
themselves are static prompts and carry neither value. The generated context
deliberately remains a portable bounded `telos context` pack, never a
whole-spec or host-specific prompt dump.

For a direct canonical human-action command, the guard independently reads
the current repository state before it permits a decision prompt: approval
context is `change CHG-NNNN digest sha256:...`; adopt/revert context contains
the sorted current drift paths and the sealed spec digest. It never uses an
agent-supplied tool description for either value, and denies the action if it
cannot resolve that context. Claude returns its supported PreToolUse `ask`
decision with this context in the reason. The official Codex PreToolUse hook
contract rejects `permissionDecision: "ask"`; it does accept top-level
`systemMessage` and `hookSpecificOutput.additionalContext`. Consequently the
Codex guard returns no unsupported `ask`: it sends the same repository-derived
context through those supported fields and the generated static `.rules` entry
owns the native `prompt`.

## Configuration and M4–M5 surfaces (0.7)

The sections above remain the frozen M1–M3 contract. This section adds the
configuration, view, rebuild, and GitHub CI surfaces shipped by M4–M5. Every
success and failure still uses the same five-key envelope; the parent command,
not its mode or subcommand, is the `command` value.

### M4–M5 result schema registry

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
    "test": {"cmd": "cargo test {filter}"},
    "policy": {"tdd": "strict"},
    "agents": {"hosts": ["claude", "codex"]}
  },
  "error": null,
  "next_actions": []
}
```

Write mode reads one complete JSON object of that same nested shape from
stdin. Partial objects, unknown fields, an invalid `policy.tdd`, or an invalid
glob are `TELOS_PARSE_ERROR`. `policy.tdd` is exactly `"strict"` or
`"advisory"`. The normalized, sorted/deduplicated `agents.hosts` must equal
the current value: `telos config` preserves this `init --agents` project
metadata and never installs or deletes host artifacts.

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
      "test": {"cmd": "cargo test {filter}"},
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

| Boundary | Glob validation | agents.hosts validation | Refusal effect |
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

### `view [--port N] [--export DIR]`

Live and export consume the same immutable `ViewSnapshot` and the same HTML
renderer. The five page families and their exact mappings are:

### Live routes and export files

| Page | Live route | Export file |
|---|---|---|
| Dashboard | `/` | `index.html` |
| Graph | `/graph` | `graph.html` |
| Intent | `/intent/INT-NNNN` | `intents/INT-NNNN.html` |
| Glossary | `/glossary` | `glossary.html` |
| Coverage | `/coverage` | `coverage.html` |

The Dashboard exposes state, open changes, and drift; Graph exposes all eight
relation filters; Intent exposes canonical statement, scenarios, applicable
constraints, `implements`, and `proves`; Glossary exposes notions; Coverage
is the intent × scenario × test matrix. All pages cross-link through the
mapping above. An unknown or malformed intent route is HTTP 404.

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

Every page is rendered in memory before publication. Under the filesystem
publication threat model below, the exporter writes a unique sibling staging
directory and promotes it atomically with a no-replacement primitive: no
ordinary concurrent owner is overwritten, and any render, write, or
finalization error leaves no final destination. `files` is sorted by
repository-style `/` path. HTML is byte-deterministic and self-contained: CSS
and JavaScript are inline and no page contains an external URL, font, image,
stylesheet, script, or network request.

### Export publication matrix

| Condition | Final destination | Existing owner | Staging cleanup |
|---|---|---|---|
| success | one complete atomic publication | n/a | staging consumed |
| destination exists before start | absent from Telos | preserved byte-for-byte | no staging published |
| destination appears before publish | absent from Telos | preserved byte-for-byte | authenticated Telos staging cleaned |
| render/write/finalization error | absent | n/a | authenticated Telos staging cleaned |
| staging identity mismatch | absent | replacement preserved | foreign/replaced entry never cleaned |

```json
{
  "ok": true,
  "command": "view",
  "result": {
    "mode": "export",
    "destination": "site",
    "files": [
      "coverage.html",
      "glossary.html",
      "graph.html",
      "index.html",
      "intents/INT-0017.html",
      "intents/INT-0042.html"
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

The recursive watcher ignores `.git`, `target`, `.superpowers`, and exporter
staging paths. It coalesces bursts, rebuilds the complete state/model off the
read lock, and atomically replaces the snapshot only after successful
validation. An invalid edit retains the last good pages and adds an escaped
reload-error banner; a later valid reload clears it. Watcher errors are
reported in the banner rather than terminating the server. Serving and
reloading never write project or Telos bytes.

### `rebuild plan|status`

Both subcommands are read-only, have `command: "rebuild"`, make no LLM call,
and never generate application code. They are the only model/graph consumers
that admit spec-only mode. Config read also works without a lock, but reads
project metadata rather than loading the model or graph. If a lock entry
exists, it must be readable and the project may be `coherent` or `changing`;
`drifted` returns `TELOS_DRIFT_DETECTED`. In `changing`, every parseable open
change is folded by ascending change ID, journals are folded, cross-change
claims and semantic integrity are revalidated, and each context pack still
follows the exact public `telos context` owner resolution.

#### Plan

`result.steps` contains every intent, including draft and deprecated intents.
The order is a deterministic topological order over `requires`: each direct
prerequisite precedes its dependent, and ready ties use ascending intent ID.
Each row has exactly `n` (one-based), `intent`, `requires` (sorted direct
prerequisites), and `context`. `context` is the complete frozen `telos context
INT-NNNN` result: `id`, `change`, `canonical`, `scenarios`, `notions`,
`constraints`, `bindings`, and one-hop `neighbors`.

```json
{
  "ok": true,
  "command": "rebuild",
  "result": {
    "steps": [
      {"n": 1, "intent": "INT-0017", "requires": [], "context": {"id": "INT-0017", "change": null, "canonical": "...", "scenarios": [{"id": "SCN-0091", "title": "a newly issued invoice is open", "proved": false}], "notions": ["..."], "constraints": ["..."], "bindings": {"implements": [], "proves": []}, "neighbors": ["..."]}},
      {"n": 2, "intent": "INT-0042", "requires": ["INT-0017"], "context": {"id": "INT-0042", "change": null, "canonical": "...", "scenarios": [{"id": "SCN-0107", "title": "full payment settles the invoice", "proved": false}], "notions": ["..."], "constraints": ["..."], "bindings": {"implements": [], "proves": []}, "neighbors": ["..."]}}
    ]
  },
  "error": null,
  "next_actions": []
}
```

The `"..."` entries above denote values with the exact nested `telos context`
schema, not omitted response keys; executable contract tests compare every
real step to the full public context result.

#### Status and real measurement

`rebuild status` executes the configured `[test] cmd` once for every distinct
`proves` target, in structural `(path, optional test name)` order, with the
target substituted for `{filter}`. A scenario is green iff it has at least
one proof and **all** proof targets are safe, present, resolvable, and exit
zero. No proof, a missing/unsafe file, a stale named test, or any non-zero
runner exit produces an explanatory red row, not a command failure. A
missing or blank runner is `TELOS_TEST_NOT_FOUND` because progress cannot be
measured. Each test row has exactly `test`, `green`, and the literal
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

### Proof and constraint execution matrix

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

The init and export guarantees cover negligence and ordinary, non-adversarial
filesystem concurrency. Consistent with the storage model in design §5, they
do not claim resistance to a same-UID adversary able to observe a CSPRNG
staging name and substitute a path entry between syscalls.

| Surface | Covered environment | Guarantee |
|---|---|---|
| init | negligence and ordinary concurrent filesystem owners | no owner overwritten; failed partial init authenticated and safely resumable |
| export | negligence and ordinary concurrent filesystem owners | no owner overwritten; no final destination on error |
| init and export | same-UID adversary substituting a path between syscalls | excluded by the §5 threat model |

### `init [--agents ...] [--ci github]`

`--ci github` participates in one preflight with every requested agent host.
All target bytes, parent directories, JSON/text merges, path types, and
collisions are validated before the first initialization write. A preflight
failure leaves `telos/`, `.gitattributes`, counters, agent artifacts,
workflow, and init marker absent or byte-for-byte unchanged.

After preflight, `.telos-init.json` records a versioned transaction, normalized
agent/CI options, exact core snapshots, and an authenticated phase. Persisted
CAS transitions authorize core publication and then integrations. Within the
filesystem publication threat model above, every created artifact is fully
staged and synced under a random sibling before a no-replace publish; user
configuration merges are recomputed/compared before atomic replacement. No
final file is opened with truncate; any late file, directory, live symlink,
dangling symlink, parent replacement, or staging-name owner introduced by
ordinary concurrency is preserved rather than overwritten or removed.

If publication fails after the marker exists, a retry is allowed only with
the same normalized `--agents`/`--ci` options and only while the marker,
phase, core bytes, directory identities, and already-published Telos artifacts
remain exact. Exact artifacts are safe no-ops, user merges are idempotent, and
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
      - uses: dtolnay/rust-toolchain@stable
      - name: Install Telos v0.7.0
        run: cargo install --git https://github.com/hugues31/telos-sdd --tag v0.7.0 --locked telos
      - name: Verify sealed Telos state
        run: telos check --sealed
```

The install tag is derived from the CLI package version. Shipping 0.7.0
therefore requires publishing repository tag `v0.7.0`; without that tag the
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
creates the exact Cargo/source/test bytes from the demo README. The
architecture function shares the covered `SCN-0091` test target. The batch
records an unchanged red-to-green witness, binds `Cargo.toml`, generated
`Cargo.lock`, and every `src/**/*.rs` file to the intent, records the canonical
`proves`, and reconciles to `1/2`.

CHG-0002 stages the real `INT-0042` `draft` → `active` edit and follows the
same red/green/bind/reconcile lifecycle. Real forbidden `crate::adapters`
imports make the constraint return exact `TELOS_CONSTRAINT_FAILED`; removing
only those imports lets the same approved change reconcile to `2/2`.
Every `[tests]` file has a canonical proof binding, both changes disappear,
and final `telos check --sealed` plus `rebuild status` prove the reconstructed
tree. Rebuild proves behavioral conformity; source-byte identity depends on
how fully constraints capture architecture.

### Billing reconstruction checkpoints

| Checkpoint | Intent statuses | tests_run | checks_run | Rebuild status |
|---|---|---|---|---|
| untouched spec-only | draft, draft | no process | no process | 0/2 |
| change reconcile --full bootstrap | draft, draft | 0 | 0 | 0/2 |
| CHG-0001 reconciled | active, draft | one distinct scenario proof | staged architecture check | 1/2 |
| CHG-0002 reconciled | active, active | one distinct scenario proof | architecture check | 2/2 |
