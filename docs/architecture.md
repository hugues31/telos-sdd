# Architecture

Telos v0.6 is a certified-state transition system layered over Git. The full
design rationale lives in [design-v2.md](design-v2.md); this file maps it to
the code.

## Layers

1. **Git substrate** (`internal/gitx`) — plumbing only, zero Telos
   semantics: trees, refs (transactional `update-ref --stdin`), notes,
   worktrees, temp-index tree writing. Nothing above it runs git directly.
2. **Deterministic kernel** (`internal/kernel`, with `contract`, `evidence`,
   `provenance`, `constraints`, `policy`, `glob`, `coded`) — the trusted
   core: certificates, the Change lifecycle, digest-bound approvals,
   witnessed red/green, findings gates, policy evaluation, atomic promotion,
   salvage/rebase. Kernel invariants KERNEL-001..010 are Go code, not
   configuration.
3. **Knowledge layer** (`internal/graph` contract, `internal/index` SQLite
   implementation, `internal/gosrc`, `internal/ctxpack`) — a derived,
   disposable, root-bound projection used for retrieval, never authority.
4. **Surface** (`internal/telos` CLI + guard, `internal/view` web server,
   `bundle/` agent assets generated from `tools/gen-bundle/roles.go`).

## The certified state

A certified state IS a commit on the target branch whose git note under
`refs/notes/telos` holds a sealed certificate: canonical JSON payload
(project, commit, tree, parent_certified, change, contract tree +
requirement ids, policy blob + hash, approvals, verification with evidence
entries, toolchain) plus an HMAC-SHA256 seal over the exact payload bytes.
The payload names its own commit, so a note cannot be copied onto another
one; the chain is the commit graph. `Seal` only accepts a verified
transition — "certify the current filesystem" is unrepresentable.

Protected content is everything git-tracked (including `telos.toml` and
`policies/`); derived content is gitignored under `.telos/`. Byte identity
is Git's (blob OIDs post-filters): no parallel hashing exists.

## The Change transaction

`telos change start` creates a candidate worktree on `telos/CHG-NNN` from
the certified tip; `changes/CHG-NNN/` (committed, retained after promotion)
holds intent, the contract delta (telos:op markers folded as a pure function
— the folded spec tree OID is the approval digest), findings, evidence
records, and promotion-time provenance. Promotion recomputes every gate
(state → exact base → digest-bound approval → open reds → blocking findings
→ obligations → constraints → suite with content-addressed reuse), builds
one commit from the exact base, and moves the branch and the notes ref in a
single ref transaction with a compare-and-swap on the base: a lost race is
`TELOS_BASE_STALE`, never partial state.

Suite runs happen in throwaway detached worktrees of the exact tree under
proof; the candidate is never mutated by a run. Mutation evidence uses
`go test -overlay` for the same reason.

## Derived knowledge

`telos index rebuild` derives the complete graph (contract nodes and edges,
changes, provenance relations, evidence, findings, Go symbols and imports)
from the certified artifacts at HEAD into `.telos/cache/index.db` —
deterministic, atomically replaced, bound to the indexed commit. Deleting it
loses nothing. The CLI query commands, the context compiler, and the web
view consume the same `graph.Querier`; SQL never leaves `internal/index`.

## Provider boundary

Both providers install the same generated Skill and role files. The guard is
a fail-open PreToolUse hook: in the certified root it denies direct writes
and non-broker shell; in a candidate it allows free work except protected
paths, and gates the human decisions (`approve`, `adopt`, `abort`,
`restore`, `confirm`/`resolve` of findings, re-`init`) as native permission
prompts with digest binding where applicable. No provider adapter contains
integrity logic.
