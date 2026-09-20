# Native plans implementation

Release target: **0.15.0**. The user approved implementation, committing and
pushing the release. All project content must be in English.

Approved product decisions:

- Every versioned repository file is governed by a plan, including documentation.
- Approve the plan once; covered changes inherit that approval. Scope changes
  require a new approval.
- Persist plans and progress in `.tel`, support recovery and fresh-agent resume,
  retain entity/file provenance, and expose the same state in the web view.
- Invoke a dedicated brainstorming skill for new work.
- Breaking changes are permitted; no legacy migration is required.

| Work package | Status | Acceptance |
|---|---|---|
| Baseline and executable design | Complete | Existing checks understood; English design and explicit formats |
| Durable storage and native plan model | Complete | Round trips, event integrity, request replay, concurrency, crash recovery |
| Plan CLI and mandatory change ownership | Complete | Prepare, approve, execute, checkpoint, resume, verify and complete |
| Repository inventory and provenance | Complete | All paths covered, retained receipts, stable identities, history and CI |
| Agent skills and guards | Complete | Brainstorming and inherited plan approval across supported hosts |
| Web view and static export | Complete | Plans, progress, resume information and entity history |
| Fixtures, acceptance tests and docs | Complete | Full Rust/frontend checks and reconstruction demo |
| Version, commit, push and release | Ready to publish | 0.15.0 committed, pushed, release verified |

## Implementation notes

Completed changes now produce immutable receipts. Durable multi-file
transactions recover interrupted publication and preserve conflicting external
edits. Existing scenario proof and domain-boundary rules remain enforced.

The web view was checked in a live browser with three of five tasks complete,
an active fourth task, a persisted checkpoint, and three attributed receipts.
The dashboard showed 60% and linked to the same plan and resume instructions.

This implementation tracker is development documentation. Runtime project plans
are provided by the new native plan protocol.

## Release verification

- Rust workspace: 1,248 tests passed, including public CLI reconstruction,
  interrupted-process recovery, fresh-clone resume, branch integration and
  trusted-base history checks.
- Clippy with warnings denied and rustfmt checks passed.
- Frontend: 152 tests, TypeScript validation and production build passed.
- Brainstorming skill metadata validated.
- Live browser review confirmed dashboard progress, plan details, checkpoints
  and attributed history. Navigation collapses before the additional Plans
  entry can overflow the header.

The workspace version is 0.15.0. Publish the implementation commit to `main`,
verify the CI matrix, then push the `v0.15.0` tag to build the release archives.

A Windows CI failure exposed platform-dependent resolution of relative runner
executables when the core API is called from outside the repository. Explicit
relative paths now resolve against the repository before spawning, for both
scenario proofs and plan validators. A real-script regression covers a path
with spaces, and Windows CI runs that check and report tests before the full
workspace suite.
