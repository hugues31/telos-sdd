# telos CLI contracts

This document is the frozen reference for everything an agent (an M3 skill,
or any other tool) routes on without interpretation: the `--json` envelope
shape, the 16 error codes and their canonical hints, the `status --json`
schema, and `check`'s semantics. Nothing here is prose to be summarized by
an LLM — it is matched on literally (`error.code == "TELOS_DRIFT_DETECTED"`,
`result.state == "drifted"`), the same way a compiler's exit code is.

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
- `command` — the invoked command's name, e.g. `"status"`.
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

### The error body

```json
{ "code": "TELOS_DRIFT_DETECTED", "message": "...", "hint": "..." }
```

- `code` — one of the 16 [error codes](#error-codes) below, as
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

Sixteen codes: the 9 frozen by spec §8 (present from M1 even though most are
only ever emitted starting M2, once the change flow and `reconcile` exist),
plus 7 M1 extensions frozen in turn as of this document. Variants are never
renamed or removed, only added — this is the whole contract an M3 skill
routes on.

| Code | When | Hint |
|---|---|---|
| `TELOS_DRIFT_DETECTED` | The project is not `coherent`: a sealed path was modified or went missing, or an unsealed spec file exists on disk. Emitted by `check --sealed` in M1; from M2 on, also gates `open`/`approve`/`reconcile`/`rebuild`/`view --export`. | `` run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert` `` |
| `TELOS_APPROVAL_STALE` | (M2) A change's approval no longer matches its delta digest — the delta was edited after `telos change approve`. | Re-run `telos change diff` to review the new delta, then `telos change approve` again. |
| `TELOS_REFERENCE_UNKNOWN` | A reference in the spec — a notion, an attribute, an enum symbol, an intent/scenario/constraint id — does not resolve. Emitted by the semantic pass on `load_model` (rule §3.3 №1); from M2 on, also rejected at write time. | None. The engine folds its best guess directly into `message` (`` ; closest is `Invoice` ``) when one is close enough; there is nothing to add. |
| `TELOS_REFERENCE_UNKNOWN` | A `show`/`impact` argument, or `query`'s `--using`/`--triggered-by`, is a well-formed id or notion name that resolves to nothing in the loaded spec (message `` unknown notion `Invoice` ``, `` unknown intent `INT-9999` ``, `` unknown scenario `SCN-9999` ``, or `` unknown constraint `CON-9999` ``). | `` closest is `Invoice` `` (edit distance, for a notion name, backtick-quoted) or `closest is INT-0042` (numeric distance, for a typed id, *not* backtick-quoted) — present only when a candidate is close enough; `null` otherwise. |
| `TELOS_REFERENCE_UNKNOWN` | A `show`/`impact` argument is neither a typed id nor a valid notion name at all (message `` cannot parse `x` as an id or notion name ``). | None. |
| `TELOS_REFERENCE_UNKNOWN` | A `show`/`impact` argument names a change (`CHG-…`) — changes are not resolvable entities until M2 (message `changes are not supported in M1`). | None. |
| `TELOS_SCENARIO_RED_EXPECTED` | (M2) `reconcile` under `policy.tdd = "strict"` requires an intact sealed red witness for a scenario before its green run; none exists. | Run `telos test SCN-…` to record a red witness before implementing. |
| `TELOS_TEST_SEALED` | (M2) The bytes of a test file sealed as a red witness changed before the scenario went green — the witness no longer proves anything. | The red witness is invalid; run `telos test SCN-…` again on the current bytes before reconciling. |
| `TELOS_ORPHAN_CODE` | (M2) `reconcile`'s no-code-without-telos check (rule §3.3 №5): a file matched by `[code]`/`[tests]` globs in `telos.toml` is not covered by any `implements`/`proves` binding. | Bind it with `telos bind <path> <INT-id>`, or remove it from the `telos.toml` globs if it isn't spec-governed. |
| `TELOS_CONSTRAINT_FAILED` | (M2) A constraint's `check` shell command exited non-zero during `reconcile`. | Run the constraint's `check` command directly to see its output. |
| `TELOS_CHANGE_STATE_INVALID` | (M2) An operation was attempted on a change from a state that doesn't allow it (the state machine is `open → drafted → approved → implementing → reconciled`, or `abandoned` at any point). | Inspect the change with `telos change list` and drive it through its states in order. |
| `TELOS_FILE_CLAIMED` | (M2) A file targeted by a change is already claimed by a different, concurrently open change. | Resolve or abandon the other change first — a file can only be claimed by one open change at a time. |
| `TELOS_NOT_INITIALIZED` | No `telos/telos.toml` found walking up from the current directory. | `` run `telos init` at the repository root `` |
| `TELOS_NOT_INITIALIZED` | `telos/telos.toml` exists, but `telos.lock` is missing (`status`, `check --sealed`). `telos init` always seals, so this is not "unsealed" — it's abnormal. | `` the project was never sealed; run `telos init` in a fresh repository or restore telos.lock from git `` |
| `TELOS_ALREADY_INITIALIZED` | `telos init` run on a project that already has `telos/telos.toml`. | `` project already initialized; see `telos status` `` |
| `TELOS_PARSE_ERROR` | A `.tel` file (or `telos.lock`, or `telos.toml`) is syntactically invalid. | None today — `message` names the offending file and, when the parser can determine it, the line and column. |
| `TELOS_INTEGRITY_VIOLATION` | A rule §3.3 violation other than an unknown reference or a cycle — e.g. `seal` finding a binding to a code file that doesn't exist on disk, or (M2) a delete of a still-referenced entity. | None today — `message` names the offending path or entity. |
| `TELOS_CYCLE_DETECTED` | A cycle exists on `requires` or `refines`. | None today — `message` renders the cycle's path (`` INT-0001 → INT-0002 → INT-0001 ``). |
| `TELOS_GIT_ERROR` | `git rev-parse --show-toplevel` failed (most commonly: not inside a git repository). | `` not a git repository; run `git init` `` |
| `TELOS_GIT_ERROR` | The `git` binary itself could not be spawned (missing from `PATH`). | None — `message` names the underlying I/O error. |
| `TELOS_INTERNAL` | An internal invariant broke — a bug, not a spec or usage problem. | None. |

## `status`

Reports the project's state against its seal, and a coverage snapshot of the
spec. Always answers — `status` reports, it never fails because the project
happens to be drifted or unparseable. It can still fail on `TELOS_NOT_INITIALIZED`
(no workspace, or a workspace with no lock) or `TELOS_GIT_ERROR` (not a git
repository).

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

- `state` — `"coherent"`, `"drifted"`, or `"changing"`. `"changing"` cannot
  occur in M1: it requires an open change, and no command opens one yet.
- `changes` — open changes. Always `[]` in M1. From M2 on, each item is
  `{"id": "CHG-0007", "status": "implementing", "obligations": ["..."]}` —
  this shape is frozen now even though nothing produces it yet.
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
`"drifted"`, `[]` otherwise.

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

Additionally requires the project to be sealed and unmodified. **State is
checked before parsing**: `compute_state` never parses anything, so a spec
that is *both* drifted *and* syntactically broken reports
`TELOS_DRIFT_DETECTED`, never `TELOS_PARSE_ERROR` — drift is the more
actionable diagnosis (and the more likely actual cause: an in-progress
edit, a bad merge).

Order:

1. `Workspace::discover` (→ `TELOS_NOT_INITIALIZED` if no `telos/`).
2. Read `telos.lock` (→ `TELOS_NOT_INITIALIZED`, distinctly worded, if
   `telos/` exists but the lock doesn't).
3. `compute_state`. If `state != "coherent"` →
   `TELOS_DRIFT_DETECTED` with the hint:
   ```
   run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert`
   ```
4. Only once state is confirmed coherent: parse + integrity, exactly as
   without `--sealed`.

`telos init --ci github` wires `telos check --sealed` into CI: a merge to
main requires a coherent, integral, sealed state.
