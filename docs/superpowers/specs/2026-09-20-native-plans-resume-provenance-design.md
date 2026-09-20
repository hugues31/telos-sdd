# Native plans, resumption and change provenance

Status: approved implementation design for Telos 0.15.0.

## Product contract

Every change to a versioned repository belongs to a plan: application code,
tests, configuration, dependencies, lockfiles, workflows, assets, documentation
and specifications. A small change uses a small plan. Technical work may have
an empty behavioral delta; it does not require invented business scenarios.

The human approves the exact plan revision. The agent then works autonomously
inside its approved scope. Only a material scope change needs new approval;
covered changes inherit their plan's approval. Brainstorming must clarify the
request and offer useful alternatives without adding ceremonial questions.
Existing decisions and permissions remain valid across interruptions.

Plans, progress and provenance are native `.tel` records committed with the
repository. They require neither SQLite nor a chat transcript. The web view and
CLI project the same persisted records. No migration of earlier workspaces is
provided by this breaking 0.x release.

## Storage boundaries

| Path | Purpose |
|---|---|
| `telos/plans/PLN-<uuid>.tel` | Definition revisions and append-only events |
| `telos/changes/CHG-<uuid>.tel` | Prepared or executing specification delta and proof journal |
| `telos/history/CHG-<uuid>.tel` | Immutable reconciled change receipt, including the complete change |
| `telos/ledger.tel` | Initial observation, current inventory, stable entity identities and receipt frontier |
| `telos/telos.lock` | Functional specification and proof seal |
| `telos/.runtime/` | Local publication journal, commit decision and process lock; ignored by Git |

A completed change leaves the open-change directory and remains in its receipt.
An abandoned change's complete bytes remain in its owning plan event. Neither
retains an active claim. Plan management is governed by its own event protocol,
so recording progress does not require a recursive outer plan.

The plan/receipt grammar uses named native records with nested objects, arrays,
quoted strings, booleans, numbers and nulls. Duplicate fields, unknown fields,
invalid identifiers, broken event chains and unsupported formats are errors.
Functional `.tel` syntax remains the specification delta language.

```text
plan "PLN-<uuid>" {
  format 1
  id "PLN-<uuid>"
  revisions [ { ... } ]
  events [ { ... } ]
}
```

The canonical digest uses deterministic JSON values derived from the native
record; text formatting does not determine approval. Revisions are immutable.
Events carry a UUID, UTC timestamp, monotonically increasing journal version,
revision, optional task, operation data, request identity and predecessor digest.

## Definition and brainstorming brief

A definition contains `title`, original `request`, `goal`, `success_criteria`,
`brief`, repository `scope`, `tasks`, and final `validation`.

The brief persists its summary, users, exclusions, decisions, questions and
`brainstormed` completion marker. Decisions record their text, alternatives,
source (`user`, `existing_instruction`, `agent`) and state (`proposed`,
`accepted`, `rejected`, `deferred`). An agent proposal cannot be represented as
an accepted user decision. Questions retain their blocking flag and nullable
answer. Unanswered blocking questions prevent approval. Resume retains both
answers and open questions; it never substitutes an inferred user answer.

A task contains:

- A plan-local `TSK-NNN` identity, title and kind: behavior, refactor, tests,
  tooling, docs, integration, recovery or bootstrap.
- Dependencies, entity targets, allowed path globs and the exact `spec_delta`.
- Acceptance criteria, named validations and a concrete next action.
- Optional approved identity moves for explicit renames and a cancellation flag.
- An optional preallocated change UUID for reproducible imports; ordinarily the
  engine allocates one randomly. A used identity cannot be allocated again.

The dependency graph must be acyclic. A cancelled dependency does not count as
completed. Changing paths, outcomes, specification deltas, dependencies or
validators creates a new revision and invalidates executable approval. Completed
tasks cannot be rewritten; additional work is a new task.

Validation forms are direct argv commands, scenario references and reviews with
concrete evidence notes. They have unique names within their task or plan. Every
executable task and the final plan need validation. Commands run without an
implicit shell. Generated reports and build artifacts should be ignored; tracked
artifacts remain governed even when an ignore rule matches them.

## Approval and execution

`plan diff` presents the definition, prior revision, task graph and exact digest.
`plan approve --expected-digest` checks that digest, the preparation inventory,
blocking questions, dependency graph, paths and combined specification deltas.
Native agent rules request approval for this plan revision. They do not prompt
again for each covered change.

Only one task owns a worktree at a time. Separate worktrees can execute separate
plans. `plan task start` checks approval, dependency completion, baseline and
ownership, then atomically records the start and creates its approved change.
The task scope and plan scope both apply to every changed path. Changes cannot
expand their specification delta independently of the approved task.

Preparing a delta before approval is supported through `plan task prepare`,
existing `add`, `edit`, `move`, `remove`, `config` and `map` commands, then
`plan task import`. The import freezes that draft delta into a new revision.
No application write is authorized by preparation alone.

During execution, preserve the existing domain boundaries, binding coverage,
constraint checks and strict same-byte red/green witnesses. A technical task
without behavior changes still obtains a receipt and repository validation.

To revise an active task, pause first, retain the task identity, and present a
new revision for approval. Approval rebuilds its exact change contract and
requires new proof evidence. It preserves code on disk. A checkpoint, proof,
blocker or next-action update changes only the journal and does not invalidate
an otherwise current approval.

After reconciliation, validate the task against the current repository and
finish it explicitly. After all tasks finish, run final validators and complete
the plan. Evidence is stale when its input inventory or approval revision changes.
Cancelling a plan does not undo already applied work. Open changes and dirty
files must be resolved before cancellation can release their ownership.

## Repository governance

Git's tracked inventory is authoritative regardless of Telos code/test globs.
Include ignored-but-tracked files, additions, deletions, symlink targets,
executable modes and submodule revisions. Include new non-ignored files so work
is visible before staging. Reject unmerged index entries and unsafe paths.

Plan/receipt/lock metadata is validated by its own protocol rather than recursively
included in its inventory. The initialization marker and runtime files are local
protocol data. Every other versioned file is governed, including agent skills
and their host configuration.

The CLI prevents unsupported operations. Agent hooks prevent direct metadata
writes and require an approved task for repository edits. Arbitrary external
editors are not a filesystem security boundary: `check --planned` detects their
unattributed results. `check --sealed --planned` additionally verifies the
functional seal and absence of open changes.

Adoption prepares an explicit recovery-plan delta for approval; it is identified
as recovery, not as work approved before it happened. Revert requires an approved
recovery task and an exact drift token. Full reconciliation requires an active
integration or recovery task and produces a normal attributed receipt. None is
an anonymous route around planning.

Initialization is an explicit bootstrap operation. It observes pre-existing
files and creates a completed bootstrap plan and initial ledger. Historical
implementation dates are unknown; bootstrap never fabricates them.

## Resumption, concurrency and durable publication

`plan resume` is read-only. It returns the current revision and approval, task
states and ready dependencies, the last checkpoint, changed files since that
checkpoint, changes outside scope, Git context, interrupted attempts and the
next action. It runs no tests or constraints. A task baseline that no longer
matches the receipt frontier requires review instead of blind continuation.

A checkpoint persists its summary, next action, inventory and observed Git HEAD.
If the user edits code after it, resume shows those bytes as additional work.
Proofs already in an open change remain available to another agent.

Mutation request identities provide retry protection. Repeating a completed
request with the same operation and arguments returns its stored result before
checking an optional expected version. Reusing an identity for different input
is an error. Optimistic versions and a process-scoped worktree lock prevent
lost updates. Random plan, change, event and run UUIDs avoid branch allocation
collisions. The lock is automatically released when the process exits.

Runner start is durable before execution. The result is persisted only after
the command returns and its inputs are checked. An interrupted command has an
unknown outcome, never an inferred green. The user or agent must inspect its
effects before an explicit retry with a new identity; retrying the original
identity cannot execute it a second time.

Multi-file publication first records every preimage and postimage in a local
journal and flushes a commit decision. Recovery discards an uncommitted
preparation or completes a committed publication. Every destination must match
its preimage or postimage. A conflicting external edit is preserved and reported,
not overwritten. The functional seal, generated specification, receipt, ledger,
plan event and open-change removal share this publication boundary.

Readers do not certify an in-progress publication. Live projection retains its
last valid snapshot and reports the reload error until it can read a coherent
state. Static export uses the same verified projection. Reading model data never
runs a validator; a local lock file may be created after cloning.

File data is flushed on every platform; parent directory publication is flushed
on Unix. Recovery covers process termination on all supported platforms. Power
loss durability also depends on filesystem and hardware guarantees; Windows
parent-directory flush semantics are not represented as a POSIX guarantee.

A clone resumes ordinary unfinished work from committed `.tel`, code and test
files, without runtime files or chat. A locally incomplete publication must be
recovered on its original storage before those records are committed or copied.

## Entity and file provenance

Receipts retain the plan, revision, approved digest, task, UTC reconciliation
time, observed Git HEAD, parent receipt references, complete change, exact file
before/after OIDs and modes, and entity events.

The identity registry gives each context, capability, notion, intent, scenario
and constraint an `ENT-<uuid>` independent of its display selector. Explicit moves
preserve it. Approved rename mappings preserve it when a remove/add delta names
the same continuing concept. Deletion leaves history; recreation receives a new
identity. The registry is verified against the initial observation and receipts.

History distinguishes definition changes, moves, removals and implementation
changes. Changing an implementation file does not alter a notion's definition
date. A context's own definition history is distinct from activity in its
intents. Initial observation establishes no implementation date. Timestamps are
engine observations; Git HEAD records what was visible then, not an invented
future commit or a guaranteed merge date.

`history <entity|path|plan|change>` and `show <entity> --history` expose attribution.
Plan pages and entity panels link dates and changes back to their plans. Querying
a recreated display name follows the current identity; an explicit historical
UID remains queryable.

## Branches and trusted CI

Receipt identities and content hashes survive squash and rebase. Git SHAs are
additional context, not the primary attribution key.

For a branch integration, retain the primary ledger and resolve application
conflicts under an approved integration task. `plan integrate --source <commit>`
imports the other branch's immutable receipts and compatible plan histories.
It checks the shared initial observation and rejects divergent edits to the
same plan journal or receipt. The final integration receipt references both
histories and records the resolved files under the approved integration scope.
Imported provenance is never rewritten to manufacture a linear history.

`check --planned --base <trusted-commit>` compares against a CI-supplied base.
The baseline and previous receipts are immutable; plan revisions and events must
extend their prior prefixes. The current inventory must equal the recorded
frontier. Missing base objects, missing provenance, historical rewrites and
unattributed deletions are failures. Initial deployment requires an explicitly
trusted bootstrap baseline; an untrusted proposed workflow cannot choose its
own audit base. Branch protection and the trusted workflow enforce this policy.

## Agent orchestration

The router invokes `telos-brainstormer` for new work or a material new question.
The skill persists a proportionate brief, explores useful alternatives and
asks only material questions. It neither depends on BMAD nor changes scope
without user acceptance.

The challenger checks domain language and boundaries, then prepares exact tasks,
paths, deltas, dependencies and validation. The implementer executes the approved
plan, checkpoints work, preserves proof requirements, reconciles changes and
records task/final validation. Routine resumption does not restart brainstorming.

Generated Claude and Codex integration installs all four skills. Approval hooks
bind the prompt to the exact plan digest. Supported host hooks and protected CI
complement the CLI; they do not claim to constrain every external tool.

## Web experience

The dashboard shows active plans independently of functional project state.
Each card offers **View plan**, state, current task and progress. Plan pages show
brief, decisions, unanswered questions, approval revision, task dependencies,
blockers, allowed paths, acceptance criteria, specification deltas, checkpoints,
validation activity and linked receipts. Completed/cancelled plans remain listed.

Progress is `floor(100 × done / non-cancelled tasks)`. In-progress and blocked
tasks contribute zero completed tasks; a blocked task stays in the denominator.
No tasks means **To be planned** and a null percentage. Three finished tasks out
of five means 60%. A 100% task count with missing final validation remains active.
The percentage measures task completion, not estimated time or effort.

## Acceptance criteria

| ID | Requirement |
|---|---|
| AC-01 | Native records round-trip revisions, decisions and events without loss. |
| AC-02 | Precise requests receive proportionate brainstorming without artificial questions. |
| AC-03 | Unanswered questions survive interruption without invented answers. |
| AC-04 | Rejected or merely proposed ideas do not become authorized tasks. |
| AC-05 | Application mutations require approval and an active task; external violations are detected. |
| AC-06 | Docs, CI, lockfiles, assets and executable modes are governed. |
| AC-07 | Ignore rules and Telos globs cannot hide tracked changes. |
| AC-08 | Covered deltas inherit approval; new scope needs a reviewed revision. |
| AC-09 | Checkpoints and proofs preserve approval; contract edits invalidate it. |
| AC-10 | Dependency completion is required; cancellation is no substitute. |
| AC-11 | Red witnesses and unfinished code survive restart with an actionable next step. |
| AC-12 | Interrupted runners remain unknown rather than passing. |
| AC-13 | A completed request can be replayed without another event or allocation. |
| AC-14 | Conflicting requests and stale versions cannot overwrite progress. |
| AC-15 | Publication interruption retains the old state or recovers the complete new state. |
| AC-16 | Recovery preserves conflicting external edits. |
| AC-17 | A committed unfinished task resumes in a clone without runtime or chat. |
| AC-18 | Reconciled and abandoned changes retain audit history without active claims. |
| AC-19 | Technical changes do not fabricate specification-definition changes. |
| AC-20 | Moves/renames preserve identity; deletion and recreation do not reuse it. |
| AC-21 | Context definition history is distinct from implementation activity. |
| AC-22 | Progress counts completed tasks with an explicit denominator. |
| AC-23 | Empty plans and pending final validation cannot appear completed. |
| AC-24 | Active plans remain visible in coherent projects, live views and exports. |
| AC-25 | Observation runs no validators and makes no versioned changes. |
| AC-26 | Recovery, full reconciliation and initialization have explicit attribution. |
| AC-27 | Adopted and observed work do not acquire fabricated historical dates. |
| AC-28 | Trusted-base CI rejects rewritten history and unattributed changes. |
| AC-29 | Content-addressed attribution survives squash/rebase; missing Git context stays explicit. |
| AC-30 | Integration retains both branch histories and attributes resolutions. |
| AC-31 | Cancellation preserves already applied effects and requires resolving unfinished work. |
| AC-32 | Incompatible baselines prevent blind task resumption. |
| AC-33 | Final validation applies to the actual reconciled bytes and becomes stale when they change. |
