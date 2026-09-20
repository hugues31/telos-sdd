# Native plans and resumable work

Telos 0.15 governs every versioned repository file through an approved plan:
code, tests, dependencies, configuration, assets, CI, documentation and the
specification. Plans and progress live in Git as `.tel` records. No database or
chat transcript is needed to resume them.

## Start a plan

The generated `telos` router invokes `telos-brainstormer` for a new request, then
`telos-challenger` to prepare the contract. The brainstormer uses the existing
specification, proposes alternatives, and asks only unresolved questions that
matter to scope or acceptance. It records user decisions and open questions in
the plan brief. Small changes still use a small plan.

```console
telos plan open "Document installation" --json
```

Use the returned `PLN-<uuid>` as `$PLAN`. `plan edit` reads the **complete new
definition** on stdin. Keep the input outside the governed repository while it
is being prepared, or pipe it directly; the saved `.tel` record is canonical.

```json
{
  "title": "Document installation",
  "request": "Explain how to install the project",
  "goal": "A contributor can install and run the project",
  "success_criteria": ["Installation commands are correct"],
  "brief": {
    "summary": "Document the existing setup without changing its behavior",
    "brainstormed": true,
    "decisions": [],
    "questions": []
  },
  "scope": ["README.md"],
  "tasks": [{
    "id": "TSK-001",
    "title": "Write installation instructions",
    "kind": "docs",
    "allowed_paths": ["README.md"],
    "depends_on": [],
    "spec_delta": "",
    "acceptance": ["The documented commands match the repository"],
    "validation": [{"kind": "review", "name": "content"}],
    "next_action": "Inspect the existing installation commands"
  }],
  "validation": [{"kind": "review", "name": "final"}]
}
```

```console
telos plan edit "$PLAN" < /tmp/installation-plan.json
telos plan diff "$PLAN" --json
telos plan approve "$PLAN" --expected-digest '<digest from plan diff>'
telos plan task start "$PLAN" TSK-001 --json
```

Approval covers that exact revision, its paths, task dependencies, specification
deltas and validators. Execution inside that scope needs no additional human
approval. A new revision requires approval again. Never turn an agent proposal
into an accepted user decision: use `source: "agent", state: "proposed"` until
there is an actual answer. An unanswered blocking question prevents approval.

Task kinds are `behavior`, `refactor`, `tests`, `tooling`, `docs`, `integration`
and `recovery`; `bootstrap` is reserved for explicit initialization. Tasks can
use direct argv commands (`{"kind":"command","name":"tests","argv":["cargo",
"test"]}`), reconciled scenarios (`{"kind":"scenario","id":"SCN-0107"}`),
or reviews with a concrete evidence note. Every task and the final plan require
validation. A task depends on completed tasks, not on cancelled ones.

## Prepare a behavioral delta

Before approval, `plan task prepare` creates the task's draft change. Use the
returned `CHG-<uuid>` as `$CHANGE` with the existing staging commands, then
import that delta into the plan revision:

```console
telos plan task prepare "$PLAN" TSK-001 --json
printf '%s\n' '{"status":"active"}' | telos edit intent INT-0042 --change "$CHANGE"
telos plan task import "$PLAN" TSK-001
telos plan diff "$PLAN" --json
telos plan approve "$PLAN" --expected-digest '<digest from plan diff>'
telos plan task start "$PLAN" TSK-001
```

Preparation changes only work records. It does not authorize implementation.
The task and plan scopes must both cover every path affected by the delta,
including generated context binding files. Approved deltas are immutable.
`change approve` only verifies an inherited, executing plan contract; it cannot
supply a missing plan approval.

## Execute, checkpoint and finish

After starting, modify files inside the approved scope. Behavior changes retain
the existing same-byte red/green scenario witnesses, ownership, architecture
checks and binding rules. A documentation or tooling task can have an empty
specification delta and still produces an attributed receipt.

```console
telos plan checkpoint "$PLAN" --summary "Instructions written" --next-action "Verify commands"
telos change reconcile "$CHANGE" --request-id installation-seal
telos plan verify "$PLAN" --task TSK-001 content --review "Checked all commands against the current project"
telos plan task finish "$PLAN" TSK-001
telos plan verify "$PLAN" final --review "All success criteria met"
telos plan complete "$PLAN"
telos check --sealed --planned
```

The percentage is `floor(100 × completed tasks / non-cancelled tasks)`. An
empty plan has no percentage. **100% is not completion**: final validation and
`plan complete` are still required. Validation results apply to exact repository
contents and the approved revision. Changing either makes the evidence stale.
If a reconciled task needs more work, start that same task again before finishing;
Telos allocates a fresh change and retains the earlier receipt.

One task owns a worktree at a time. Use separate worktrees for parallel work.
Pause before editing an active plan, keep its task identity, and obtain approval
for the new revision. Completed tasks are immutable; append a new task for
additional work. Cancel does not undo applied changes. Abandon retains the draft
in history and refuses to silently discard dirty repository work.

## Resume after interruption

```console
telos status --json
telos plan resume "$PLAN" --json
```

Resume runs no runner and changes no versioned files. It returns the brief,
approval, dependencies, active task, last checkpoint, changes since that
checkpoint, files outside scope, interrupted attempts and a suggested next
action. It also works in a fresh clone containing the saved work records.

Use the same `--request-id` and input to retry a lost mutation response. Native
plan mutations replay before checking `--expected-version`; a reused identity
with different input is refused. Explicit request identities also protect the
staging, proof and reconciliation CLI commands. Keep IDs under 120 characters
because execution result records append a suffix.

A runner start is durable before execution. Missing results mean **unknown**,
not success. Inspect its effects; retry with a new request identity and, for
`plan verify`, `--retry-unknown`. Telos does not automatically repeat arbitrary
commands that may have external effects.

```console
telos recover
```

Use recovery when Telos reports `TELOS_RECOVERY_REQUIRED`. A publication journal
contains before/after bytes and a durable commit decision. Recovery either drops
an uncommitted preparation or finishes a committed publication. External edits
that match neither version are preserved as `TELOS_RECOVERY_CONFLICT`; resolve
the named conflict before retrying. Never delete the journal to bypass it.

## History and the web view

```console
telos history "$PLAN" --json
telos history README.md --json
telos show INT-0042 --history --json
telos view --port 3000 --open
```

The dashboard displays active plans and **View plan**. Plan pages expose scope,
tasks, blockers, progress, checkpoints and activity. Entity pages show dates,
plan revisions and tasks that changed their definitions or implementations.
Initial entities have an observation date; Telos does not invent an earlier
implementation date. Stable entity identities survive explicit moves; deletion
and recreation produce distinct identities. Use `identity_moves` to approve an
explicit selector rename when the move cannot be derived unambiguously.

Receipts retain file transitions, the full reconciled change, semantic entity
changes, approval digest, timestamp and observed Git HEAD. Git commit ancestry
provides the commit context; a receipt does not predict a future commit hash.
Static export contains the same history. Export outside the repository or to an
ignored directory; an unignored export is itself a governed addition.

## Initialization, recovery plans and branch integration

`telos init` explicitly observes the existing repository and records a completed
bootstrap plan. `telos init --from-spec` observes and seals a copied specification
written in the current format, such as the Billing demo. It refuses open changes.
There is no legacy workspace migration. Neither command fabricates provenance
for work done before initialization.

Unplanned edits require an explicit recovery plan. Prepare its task change and
use `telos adopt --into "$CHANGE" --expected-state '<status token>'`, then import,
approve, start and reconcile it. Restoration uses an approved recovery task and
`telos revert --expected-state '<status token>'`, followed by reconciliation.
Automatic restoration supports regular files; restore symlinks, submodules or
mode transitions explicitly inside the approved recovery scope. Inventory and
CI still detect and govern all these kinds of changes.

To integrate branches, approve and start an `integration` task before merging.
Resolve code conflicts inside its approved scope. Preserve the primary branch's
`telos/ledger.tel` and derived `telos.lock` while resolving metadata conflicts,
then stage the resolved index and import the source's immutable records:

```console
telos plan integrate "$PLAN" --source '<source commit>'
telos change reconcile --full
```

The integration receipt records the resolved tree and both history parents.
Conflicting receipts or divergent progress for the same plan fail explicitly;
never choose one journal arbitrarily. Full reconciliation requires an empty
delta on an approved recovery or integration task and rechecks the whole model.

## CI and trust

```console
telos check --sealed --planned --base '<trusted base SHA>'
```

This checks all tracked files regardless of ignore rules or code/test globs,
plus non-ignored new files, deletions, executable bits, symlinks and submodule
revisions. The base must come from trusted CI context. It checks retained receipt
ancestry and immutable prefixes of existing plan histories. Local hashes detect
corruption; they are not human signatures. Host prompts and protected review/CI
establish the approval trust boundary.

The generated GitHub workflow fetches history and supplies the PR base or prior
push SHA. Review and land the initial bootstrap as an explicit trusted baseline
before enabling that required check; it deliberately fails if no governed base
exists. Subsequent changes cannot silently replace the baseline.

Generated local runtime files are ignored and never form the progress source of
truth. File data is flushed on supported platforms; parent directories are also
flushed on Unix. Process termination is recoverable across platforms. Power-loss
guarantees additionally depend on the operating system, filesystem and hardware.

Runtime recovery material under `telos/.runtime/` and initialization markers
must never be tracked. Planned-state checks reject these paths in the index
and in a trusted base commit instead of allowing a governance bypass.
