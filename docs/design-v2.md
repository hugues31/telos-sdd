# Telos V2 — Certified-State Development

**Status:** adopted design for v0.6
**Supersedes:** the V1 model documented in `architecture.md` (spec/code roots in `.telos/state.json`, `telos: RULE-*` annotations, OBJ/RULE vocabulary).
**Primary invariant:** every state accepted as valid by Telos is a coherent, verifiable alignment of intent/contract, implementation, evidence, and policy.

This document is the authoritative design for the v0.6 rewrite. It revises the original V2 proposal with the arbitrations recorded during design review: Git is the substrate (no parallel hash universe), the "contract clarification" transition category is removed, evidence is content-addressed, findings carry proposed severities that only policy or a human can make blocking, and salvage is a one-gesture normal path.

---

## 0. Acceptance loops

These three loops are the acceptance criteria for the whole design. They exist as executable end-to-end tests (`cmd/telos/e2e_test.go`: `TestLoopFeature`, `TestLoopSalvage`, `TestLoopConcurrent`), committed skipped at M0 and un-skipped milestone by milestone. If daily usage ever requires the human to think about certificates, roots, worktrees, or evidence IDs, the abstraction has failed: those concepts live in the kernel, not in the user's mental load.

**Loop 1 — feature** (`TestLoopFeature`):

```text
I ask for a feature
  → Telos challenges the need
  → I settle 1–2 material questions
  → I approve the contract delta (native permission prompt, digest-bound)
  → the agent develops in an isolated candidate
  → Telos produces the evidence (witnessed red/green)
  → the verifier critiques
  → Telos certifies
  → promotion: one new certified commit on main
```

**Loop 2 — accidental edit** (`TestLoopSalvage`):

```text
I accidentally modify a file in the certified worktree
  → CORRUPTED
  → "Capture this edit as CHG-042?"
  → yes
  → same loop as above; the certified worktree is restored,
    the message says exactly where my work went
```

**Loop 3 — concurrent changes** (`TestLoopConcurrent`):

```text
CHG-A and CHG-B are in flight
  → A promotes
  → B's promotion reports a moved base
  → B rebases; only evidence whose dependency closure intersects
    the rebased diff is re-run — disjoint changes re-certify almost free
  → B promotes
```

---

## 1. Thesis

Telos V2 is not primarily a workflow framework and not primarily a documentation generator. It is a **certified-state transition system for software repositories modified by humans and AI agents**.

> **Any modification to protected repository content outside an authorized Telos transition invalidates the current certified state.**

A human editing one character of the contract, a coding agent modifying source directly, an IDE rewriting a test, or a script changing a policy all have the same consequence: `CERTIFIED → CORRUPTED`. Telos does not guess whether the modification was reasonable. The only way to obtain a new certified state is a **Change**: created from the last certified state, developed in isolation, satisfying every hard constraint and proof obligation, carrying the required digest-bound human approvals, and promoted atomically.

```text
Certified State N ──(Change)──► Candidate ──(verify + seal)──► Certified State N+1
```

The intended property of the main branch:

> **Every commit accepted on `main` is a certified state.**

Intermediate red/green states exist only in an isolated candidate worktree; they are never represented as a valid Telos state.

## 2. Goals

1. **Byte-level integrity** — protected content has a declared certified identity; any unexpected change invalidates certification immediately, without an LLM.
2. **Atomic semantic evolution** — contract, implementation, evidence, and policy evolve together through named Changes.
3. **Human intent remains authoritative** — semantic contract changes require approval bound to the exact reviewed content.
4. **LLMs are untrusted reasoning components** — useful for judgment (clarifying, critiquing, implementing, inspecting), never trusted to define validity. Hard validity belongs to the deterministic kernel.
5. **Evidence is first-class** — a requirement is proven by explicit, reproducible evidence, not by an identifier appearing in a test file.
6. **Durable knowledge does not depend on agent context** — no correctness-relevant fact exists only in a conversation.
7. **Efficient reasoning on large projects** — a derived semantic graph and a bounded context compiler feed agents the relevant neighborhood, not the whole repository.
8. **Human-readable global documentation** — a local web application projects the certified knowledge model; it is never a second source of truth.

## 3. Non-goals

Telos V2 does not claim to prove semantic equivalence between natural-language intent and code. It is not a privilege boundary against a determined attacker running under the same OS account. It does not require formal methods for every requirement, does not require every code edit to cause a spec edit (behavior-preserving refactors and conformance fixes exist), does not require CI/CD or any hosted service, and never makes SQLite, the web UI, or LLM summaries authoritative.

---

## 4. Core concepts

| Concept | ID | Role |
|---|---|---|
| **Intent** | `INT-*` | Why a change is desired: motivation, desired outcome, context. Causal, not executable. |
| **Requirement** | `REQ-*` | Stable normative assertion with a class (`behavior`, `security`, `invariant`, `concurrency`, `performance`, `architecture`) and executable Gherkin scenarios. |
| **Decision** | `DEC-*` | Technical or product decision used to satisfy requirements. |
| **Change** | `CHG-*` | The only normal unit of evolution between certified states. |
| **Evidence** | `EVD-*` | Reproducible, content-addressed proof that implementation conforms to contract. |
| **Finding** | `FND-*` | Ambiguity, contradiction, unproven claim, policy violation, or verification concern. |
| **Policy** | — | Project-specific certification requirements (CUE). Kernel invariants are not policy and cannot be weakened by it. |

The chain Telos maintains is explicit — it never claims `Intent == Code`:

```text
Intent ──(human-approved interpretation)──► Contract
Contract ──(proof obligations)──► Evidence
Evidence ──(verification)──► Implementation
```

Three questions are kept distinct and never collapsed into "spec matches code": **consistency** (are the requirements mutually compatible?), **feasibility** (can they be satisfied under known constraints?), and **conformance** (does the implementation satisfy them?).

---

## 5. Git is the substrate

V2 deliberately does not reimplement integrity primitives Git already provides. There is no parallel hash universe, no homemade Merkle roots, no internal ledger.

- **A certified state IS a git commit.** Promotion creates the commit. The certificate chain is the commit graph: no homemade parent pointers, no sequence numbers.
- **The certificate is a git note** on the certified commit, under `refs/notes/telos` (§13). Storing it outside the tree resolves the self-hashing circularity; a 1:1 certificate↔commit mapping falls out.
- **Protected content = everything git-tracked.** `telos.toml` is tracked and therefore protected — closing the V1 gap where `test_commands` could change silently between a red witness and its green. Derived and disposable content is gitignored: `.telos/cache/**`, candidate worktrees, generated web assets.
- **Canonicalization (KERNEL-010) inherits Git's answer.** Byte identity is what Git stores: blob OIDs after `.gitattributes` filters. No `normalize()` function participates in integrity.
- **SEALED vs ATTESTED.** The default seal is an HMAC-SHA256 over the canonical certificate payload with a secret embedded in the binary. It detects out-of-protocol edits and prevents trivial manual rewriting of certificates; it is **not** a boundary against an adversary holding the same binary. This mode is called `SEALED`, never "trusted". A future `ATTESTED` mode can use signed commits/tags or external attestation; it is out of scope for 0.6 but the `seal.mode` field leaves room.

The critical API constraint stands: the sealing primitive only accepts a verified transition object —

```go
func (k *Kernel) Seal(vt *VerifiedTransition) (Certificate, error)
```

`VerifiedTransition` has unexported fields constructible only inside the kernel package, so "seal the current filesystem" is unrepresentable (KERNEL-002 by construction). There is no `telos sign-current-state`.

## 6. Corruption and salvage

**Corruption detection at the root worktree:** the worktree differs from the certified HEAD (`git status --porcelain` non-empty), or HEAD carries no valid certificate note (an out-of-band commit). Either way: `CORRUPTED`. Recovery is only ever `salvage` (preserve) or `restore` (discard). There is no `adopt`, `certify-current`, or `rebaseline` command; the one sanctioned adoption is the explicit, guard-gated genesis at `telos init` (and a deliberate destructive re-init, gated with elevated wording).

**Salvage is a one-gesture normal path, not a recovery ceremony:**

1. `telos status` on a corrupted root immediately proposes the conversion — `"Capture this edit as CHG-042?"` — and, when the dirty paths overlap an open Change's diff, offers routing into it (`--into CHG-041`).
2. `telos salvage` stashes the diff (`git stash push -u`), verifies the root is back to certified bytes, creates (or reuses) the candidate, applies the stash there, and commits it on the candidate branch. On conflict (only possible with `--into` on a diverged candidate) the stash is preserved and the conflicted paths are named.
3. The result always says where the work went: the candidate worktree path, and that the root was restored. The user's editor buffers point at reverted files — the message must make the relocation obvious.

`telos restore` discards the diff back to the certified state; it is guard-gated (destructive, lists paths).

## 7. Change transaction model

**Base (KERNEL-001).** Every Change declares the exact certified commit it starts from. Promotion is only possible when the declared base equals the current `main` tip; the promotion ref-transaction compare-and-swaps on it, so a lost race is `TELOS_BASE_STALE`, never partial state.

**Candidate isolation.** A Change is developed in a git worktree (sibling directory, branch `telos/CHG-NNN`) created from the base commit. The certified worktree is never a scratchpad. Legitimate intermediate states — failing test, partial implementation — exist only there.

**Categories.** Exactly two normal categories, plus two special ones:

| Category | Contract | Requires |
|---|---|---|
| `behavior_change` | changes | digest-bound human approval of the exact target contract |
| `behavior_preserving` | byte-identical | human-gated claim + revalidation of affected evidence |
| `policy_change` | — | elevated approval (KERNEL-009); flagged `privileged` |
| `genesis` | — | init only |

**The "contract clarification" category is removed.** Any change to normative contract bytes requires a new approval. An LLM is never asked to certify semantic equivalence in order to skip a human gate. A future deterministic split between normative and editorial content must be *syntactic* (the contract format itself declaring normative spans), never inferred.

**Promotion** recomputes everything against the exact candidate (KERNEL-007, with content-addressed reuse, §9), folds the contract delta into canonical `spec/`, writes all objects first (tree, commit, certificate blob, notes tree), then moves `refs/heads/main` and `refs/notes/telos` together in one `git update-ref --stdin` transaction. A crash before the transaction leaves only unreachable objects; after it, worktree cleanup is idempotent (`telos doctor` repairs).

**Rebase.** When the base moved: `git rebase --onto` inside the candidate, then bookkeeping — the base is updated, every evidence closure digest is recomputed on the rebased tree (unchanged digest ⇒ evidence survives; this is where content-addressing pays), and a contract approval survives if and only if the folded contract tree OID is unchanged (if someone else touched `spec/`, the human re-approves — correctly).

## 8. Contract model

`spec/` always describes the **current certified state**. Future semantics live exclusively in a Change's delta until promotion.

**Grammar** (evolving the V1 regex parser):

- `spec/PRODUCT.md` — product intent: `### INT-NNN — Title` sections.
- `spec/<domain>.md` — requirements: `### REQ-NNN — Title` sections, each with required lines
  `Class: behavior|security|invariant|concurrency|performance|architecture` and
  `Motivated by: INT-NNN[, INT-NNN…]`, and a ```` ```gherkin ```` block (required for classes
  `behavior`/`security`/`invariant`/`concurrency`; warning-only for `performance`/`architecture`).
  An optional ```` ```telos-constraint ```` block attaches structured constraints (§20).
- `spec/DECISIONS.md` — `### DEC-NNN — Title` with `Status: accepted|superseded by DEC-NNN`.

IDs are unique across the repository. Structural validation (duplicates, placement, dangling `Motivated by`, missing scenario blocks) ports directly from V1's `loadSpec`.

**Contract delta** (`changes/CHG-NNN/contract.delta.md`): standard grammar sections, each preceded by an operation marker:

```markdown
<!-- telos:op add file: spec/auth.md -->
### REQ-007 — Sessions expire
Class: security
Motivated by: INT-002
...

<!-- telos:op replace file: spec/auth.md -->
### REQ-003 — ...

<!-- telos:op remove id: REQ-004 -->
```

**Fold** is a pure function: materialize the base `spec/` tree in a temporary index, apply the ops, `write-tree`. **The approval digest is the folded `spec/` tree OID**, computed identically at `change review` and at promotion — KERNEL-004 is an OID equality.

## 9. Evidence

Evidence is first-class and **content-addressed**. KERNEL-007 means "recompute the validity of the exact candidate", not "blindly rerun every proof".

**Record** (committed at `changes/CHG-NNN/evidence/EVD-*.json`):

```json
{"evidence":1,"id":"EVD-<first 12 hex of key>",
 "kind":"suite|witnessed_red_green|adversarial_test|benchmark|static_check|mutation|smt|command",
 "requirements":["REQ-007"],
 "command":"go test ./...","cwd":".",
 "depends_on":{
   "closure":"go_packages|tracked_tree",
   "closure_digest":"<sha256 over sorted 'path\\0blobOID\\n'>",
   "packages":["./internal/auth"],
   "contract":"<spec tree oid, contract-sensitive kinds only>",
   "policy":"<policy hash>",
   "toolchain":{"go":"go1.24.1","os":"linux","arch":"amd64"}},
 "result":{"status":"pass|fail","exit_code":0,"output_digest":"<sha256>","output_tail":"…","duration_ms":812},
 "witness":{"red":{"baseline_tree":"<oid>","failed_tree":"<oid>",
                   "sealed_tests":[{"path":"tests/x_test.go","blob":"<oid>"}],"output_tail":"…"},
            "green":{"tree":"<oid>","sealed_tests_intact":true}},
 "reusable":true,"adopted":false,"change":"CHG-042","created_at":"RFC3339"}
```

Reuse key = `sha256(canonicalJSON({kind, command, cwd, depends_on}))`.

**Reuse (modeled on `go test`'s cache):** at `ready`/`promote`, the dependency closure digest is recomputed on the exact tree being certified and looked up among the Change's own records and the certified ancestors'. A pass with a matching digest is reused (`reused:true` in the certificate). If Telos cannot determine dependencies, it conservatively invalidates. Hermeticity is per class: Go evidence uses the import-graph closure (`go list -deps -json`, falling back to the whole tracked tree on any failure); `benchmark` is **never** reusable.

**Witnessed red/green** (V1's strongest idea, re-homed onto git objects): a new behavioral requirement gets a test; the broker witnesses that test failing on a green-verified baseline; the test bytes are sealed as blob OIDs; implementation changes; the same bytes pass. Suite runs happen in throwaway detached worktrees of the exact tree — the candidate is never mutated by a run, which structurally removes V1's rollback and premature-green-persist bugs. Between red and green, sealed tests may change only through a new red witness. Adoption of already-correct behavior (`evidence adopt`, V1's `--expect-pass`) stays human-gated.

**Flaky evidence is never certifying evidence.** A proof that fails intermittently becomes a Finding *against the test*, whose resolution requires a Change that fixes the test. `retry-until-green` exists nowhere in the system, not even as an option.

**Obligations (KERNEL-005).** Defaults hardcoded until policy lands: every certified state needs a green `suite` record on the certified tree; every REQ must be referenced by at least one test file; every REQ added or modified by a `behavior_change` needs `witnessed_red_green` (or gated adoption); `behavior_preserving` changes recompute the suite and must keep `contract.tree` identical to the base's. Policy (§20) extends obligations per requirement class.

## 10. Findings

LLM critics propose; policy and humans dispose.

```json
{"finding":1,"id":"FND-007","change":"CHG-042",
 "source":{"kind":"critic|human|kernel","name":"consistency-critic"},
 "target":{"requirements":["REQ-042"],"paths":[],"evidence":[]},
 "proposed_severity":"info|minor|major|blocking","confidence":0.8,"rationale":"…",
 "severity":"blocking","escalated_by":"policy|human|null",
 "status":"open|resolved",
 "resolution":{"kind":"real|not_an_issue|duplicate","by":"human","duplicate_of":"","note":"…"},
 "created_at":"RFC3339"}
```

A critic can only ever set `proposed_severity`. Deterministic policy rules may auto-escalate (`escalated_by:"policy"`); otherwise a human confirms. An open finding with `severity:"blocking"` forbids certification (KERNEL-006). Resolutions carry a taxonomy — `real`, `not_an_issue`, `duplicate` — so the critic false-positive rate (`resolved_not_an_issue / resolved_total`) is computable; it is a tracked health metric, because the failure mode of critic noise is alarm fatigue, not abuse.

## 11. Provenance

`telos: RULE-*` source annotations are **removed entirely** — invasive, file-level, easy to make decorative, duplicating what the broker already knows.

Provenance is recorded from certified Changes at promotion (`changes/CHG-NNN/provenance.json`): `REQ → implemented_by (symbols) / verified_by (tests) / changed_by (CHG)`. For Go, symbols are extracted with stdlib `go/parser` + `go/ast`; other languages fall back to file-level relations. Every relation carries `authority` (`canonical` | `derived` | `candidate`) and `origin`; an LLM-inferred relation can never silently become canonical.

**Durable identity is `REQ → Change → Evidence`.** Symbol names are derived projections: a rename does not break identity, the graph re-derives the projection.

## 12. The kernel and its invariants

The kernel is the small, auditable, deterministic trusted core. It owns certified-state validation, Change base validation, candidate lifecycle, approval digest checks, proof-obligation evaluation, policy evaluation, atomic promotion, provenance recording, and sealing. Package layout:

```text
internal/
  gitx/         git plumbing (worktrees, notes, ref transactions, trees) — zero Telos semantics
  kernel/       certified-state machine: Certificate, Seal(VerifiedTransition), Status,
                StartChange/Review/Approve/Verify/Promote, Salvage/Restore, genesis
  contract/     spec parser (INT/REQ/DEC), delta parser, Fold
  evidence/     records, closures, red/green witness, reuse, adopt
  provenance/   go/ast symbols, REQ→symbol/test/change relations
  graph/        node/edge types + Querier interface (no SQLite import)
  index/        SQLite implementation + rebuild + FTS5
  gosrc/        Go source analysis (symbols, imports, calls)
  ctxpack/      context compiler
  policy/       CUE: embedded kernel schema + project policies; policy hash
  constraints/  structured constraints; tier-1 CUE; tier-2 SMT orchestration
  smt/          z3 detection, SMT-LIB emission, result parsing
  mutation/     built-in AST mutator + go test -overlay runner
  view/         loopback web server + static export
  telos/        CLI, JSON envelope, guard, doctor, install
```

**Hard invariants** (Go code, not editable policy):

- **KERNEL-001 — Exact base.** A Change certifies only against the certified state it was built and verified on; the promotion CAS enforces it under races.
- **KERNEL-002 — No arbitrary certification.** Only a `VerifiedTransition` (kernel-constructible only) can be sealed.
- **KERNEL-003 — Corruption is terminal** for the current state: discard (`restore`) or salvage into a candidate; never silent adoption.
- **KERNEL-004 — Approval is digest-bound**: the approved digest must equal the folded target contract tree OID being promoted.
- **KERNEL-005 — Proof obligations are mandatory** for every active requirement under the applicable evidence policy.
- **KERNEL-006 — Open blocking findings forbid certification.**
- **KERNEL-007 — Verification is recomputed** for the exact candidate. Evidence whose content-addressed dependency closure is unchanged may be reused; unknown dependencies invalidate conservatively. No cached `PASS` is ever trusted on changed inputs.
- **KERNEL-008 — Project policy cannot weaken kernel invariants** — enforced structurally by CUE unification against an embedded, closed kernel floor (§20).
- **KERNEL-009 — Policy changes are privileged transitions** with elevated approval.
- **KERNEL-010 — Certified identity is Git's byte identity**: blob/tree OIDs after `.gitattributes` filters; no other canonicalization participates in integrity.

## 13. Certificate manifest

The certificate is a git note blob on the certified commit under `refs/notes/telos`. Canonical bytes are produced by exactly one function: a compact `json.Encoder` with `SetEscapeHTML(false)` over structs (map fields prohibited), so byte determinism follows from field order; the envelope stores the payload as `json.RawMessage`, and verification re-extracts the exact payload byte range — no re-canonicalization.

```json
{"telos_certificate":1,
 "payload":{
   "version":1,
   "project":{"id":"<project uuid>","genesis":"<oid>"},
   "commit":"<oid of this commit>","tree":"<oid>",
   "parent_certified":"<oid|empty>",
   "change":{"id":"CHG-104","category":"behavior_change","base":"<oid>"},
   "contract":{"tree":"<spec/ tree oid>","requirements":["REQ-001"],"delta_from":"<prev spec tree oid>"},
   "policy":{"blob":"<telos.toml blob oid>","hash":"<policy hash>"},
   "approvals":[{"kind":"contract|preserving_claim|adoption|policy|reset","digest":"<oid>","at":"RFC3339"}],
   "verification":{
     "evidence":[{"id":"EVD-…","record_blob":"<oid>","reused":false,"source_change":"CHG-104"}],
     "requirements_verified":["REQ-001"],
     "findings_open":[]},
   "toolchain":{"telos":"0.6.0","go":"go1.24.x"},
   "sealed_at":"RFC3339"},
 "seal":{"mode":"SEALED","algo":"HMAC-SHA256","mac":"<hex>"}}
```

`payload.commit` is self-referential (the commit exists before the note), so a note cannot be copied onto another commit. Categories: `behavior_change | behavior_preserving | policy_change | genesis`.

## 14. Committed Change artifacts

```text
changes/CHG-NNN/            # committed; retained after promotion as history/provenance
  change.json               # {change:1, id, category, title, base, branch:"telos/CHG-NNN",
                            #  status:"drafting|awaiting_approval|approved|ready|promoted|aborted",
                            #  approvals:[…], red_witnesses:{…}, privileged:bool,
                            #  created_at, promoted_commit}
  intent.md                 # free text; may declare INT-* sections (spec grammar)
  contract.delta.md         # behavior_change only (§8)
  decisions.md              # DEC-* sections, folded into spec/DECISIONS.md at promotion
  findings.json             # array of Finding (§10)
  evidence/EVD-*.json       # evidence records (§9)
  provenance.json           # written at promotion (§11)
```

`telos.toml` (tracked ⇒ protected): `project_id`, `agents`, `test_commands`, `test_files`, `closure = "go"|"tree"` (auto-detected: `go` when `go.mod` exists). The V1 `untraced` key is deleted — annotations are gone; protection is simply "git-tracked".

## 15. Human approval and the guard

Human approval is required for material semantic decisions, not for every operation. The V1 guard survives (PreToolUse hook, ask/deny JSON decisions, digest binding, deliberate fail-open on decode errors) and becomes **worktree-aware**:

- **In the certified root:** deny direct `Edit`/`Write`/`apply_patch` and any non-Telos shell command (V1 behavior).
- **In a candidate worktree:** direct edits are allowed **except** on protected paths — `spec/**`, `changes/*/change.json`, `changes/*/evidence/**`, `telos.toml`, `.claude/**`, `.codex/**`, `.agents/**`. Contract semantics go through the delta file; evidence goes through the broker. Protected-path writes are denied live *and* re-verified at `ready`.

Ask-gates (native permission prompts): `change approve` (digest re-checked for freshness before prompting, so the user is only asked when approval can succeed), `evidence adopt`, re-`init`, `restore`, `change abort` — with elevated wording when the Change is `privileged` (KERNEL-009: `telos.toml` or `policies/**` touched).

## 16. CLI surface

```text
telos init | status | verify | doctor | version
telos change start|show|diff|review|approve|ready|rebase|promote|abort
telos salvage [--into CHG-NNN] | restore
telos evidence list|show|red|green|adopt
telos findings list|add|resolve
telos index rebuild|status
telos search | show | related | impact | explain | context
telos view [--port N] [--open] [--static DIR]
telos guard
```

Every command keeps the stable JSON envelope: `{ok, command, result, next_actions, error{code,message,paths}}`.

**`telos status --json` result schema** (frozen at M0):

```json
{"context":"root|candidate",
 "state":"certified|corrupted|uninitialized",
 "certificate":{"commit":"<oid>","change":"CHG-104","sealed_at":"RFC3339"},
 "dirty":{"paths":["src/x.go"]},
 "salvage":{"proposal":"new_change|into","into":"CHG-041",
            "prompt":"Capture this edit as CHG-042?"},
 "changes":[{"id":"CHG-042","status":"drafting","category":"behavior_change",
             "base_stale":false,"worktree":"../telos-sdd-CHG-042"}],
 "change":{"id":"CHG-042","status":"drafting","category":"behavior_change",
           "base_stale":false,"review":"<pending digest|empty>",
           "obligations":{"met":3,"open":["REQ-007"]}},
 "contract":{"intents":4,"requirements":12,"decisions":3},
 "evidence":{"satisfied":11,"missing":1,"stale":0},
 "findings":{"open":2,"blocking":0}}
```

`certificate`, `dirty`, `salvage`, `changes` appear in `root` context; `change` in `candidate` context; counts appear in both. Agent orchestration routes on `context` + `state` + `change.status`.

**Error codes** — single-sourced in `internal/telos/codes.go`, generated into the agent protocol doc, cross-checked by CI:

| Code | Origin |
|---|---|
| `TELOS_COMMAND_FAILED` `TELOS_INPUT_INVALID` `TELOS_INPUT_REQUIRED` `TELOS_CONFIG_INVALID` `TELOS_GIT_UNAVAILABLE` `TELOS_GIT_REPOSITORY_REQUIRED` `TELOS_TESTS_FAILED` `TELOS_APPROVAL_STALE` `TELOS_NOTHING_PENDING` `TELOS_RED_EXPECTED` `TELOS_BASELINE_RED` | kept from V1 |
| `TELOS_STATE_CORRUPTED` (was CODE_CORRUPTED) · `TELOS_CONTRACT_INVALID` (was SPEC_INVALID) · `TELOS_CONTRACT_TAMPERED` (was SPEC_UNAPPROVED) · `TELOS_OBLIGATION_UNMET` (was RULE_NOT_IMPLEMENTED) · `TELOS_REQUIREMENT_UNKNOWN` (was TRACEABILITY_GAP) · `TELOS_NOT_INITIALIZED` (was STATE_MISSING) | renamed |
| `TELOS_TEST_FIRST` `TELOS_TEST_SEALED` `TELOS_RED_PENDING` `TELOS_RED_STALE` | kept, re-homed onto blob OIDs |
| `TELOS_ANNOTATION_MISSING` `TELOS_ANNOTATION_ORPHAN` `TELOS_ANNOTATION_MISMATCH` | **dropped** with annotations |
| `TELOS_BASE_STALE` `TELOS_CERTIFICATE_INVALID` `TELOS_APPROVAL_REQUIRED` `TELOS_FINDING_BLOCKING` `TELOS_CHANGE_UNKNOWN` `TELOS_CHANGE_STATE_INVALID` `TELOS_CANDIDATE_REQUIRED` `TELOS_ROOT_REQUIRED` `TELOS_WORKTREE_CONFLICT` `TELOS_INDEX_STALE` `TELOS_NODE_NOT_FOUND` `TELOS_SYMBOL_AMBIGUOUS` `TELOS_POLICY_INVALID` `TELOS_POLICY_WEAKENS_KERNEL` `TELOS_CONSTRAINT_UNSAT` `TELOS_BUDGET_TOO_SMALL` `TELOS_PORT_BUSY` | new |

## 17. Semantic graph and SQLite index

A derived graph over project knowledge lives in `.telos/cache/index.db` (pure-Go `modernc.org/sqlite`, FTS5). It is a **query model, never an authority model**, and it is disposable by definition:

```bash
rm .telos/cache/index.db && telos index rebuild   # must restore the complete graph
```

If deleting the database loses normative knowledge, the architecture is wrong.

Node kinds: Intent, Requirement, Decision, Constraint, Change, Finding, Scenario, Evidence, Test, File, Symbol, Package, Domain. Edge kinds: motivates, refines, supersedes, depends_on, constrains, conflicts_with, implements, verified_by, declared_in, changed_by, introduced_by, calls, imports, uses — every edge carries `authority`, `origin`, `change_id`. Code edges come from stdlib `go/ast` analysis and are always `derived` (or `candidate` when resolution is name-only).

The index is **root-bound**: it records the indexed commit and worktree fingerprint, auto-rebuilds by default on mismatch, and refuses to present a stale cache as current under `--no-rebuild` (`TELOS_INDEX_STALE`). Evidence freshness is computed at query time against current blob OIDs, never stored. The CLI (`search`, `show`, `related`, `impact`, `explain`, `evidence`, `findings`) and the web view consume the same `graph.Querier` interface; SQL never leaks out of `internal/index`.

## 18. Context compiler

`telos context CHG-104 --budget 16000 --json` compiles a bounded context pack: seeds (intent, delta IDs, touched symbols) → candidates (exact IDs, FTS, graph BFS depth 2, code dependency neighborhood — no embeddings in 0.6) → deterministic scoring (source, proximity, authority, class boost, recency; one tested `Weights` struct) → budget allocation.

Global invariants (every `invariant`-class REQ) are charged **before** budgeting and never truncated — a budget too small for them is `TELOS_BUDGET_TOO_SMALL`, stating the minimum. Remaining budget splits across categories (intent, requirements, decisions, findings, code, evidence, reserve) with greedy fill and overflow redistribution. Token estimation is `len/4 + 24` per item with a 10 % safety margin. **Pack content is canonical bytes only** — when a node is selected, its canonical source is loaded; LLM summaries may exist only as derived search aids. Each item records `why` (its retrieval path); the top omitted candidates are listed so an agent can pull more via `telos show`.

## 19. Web view

`telos view` runs a loopback-only web server (default `127.0.0.1:7343`; `--port`, `--open`; `--static DIR` renders every page once for export). Self-contained: `html/template` + vanilla JS + inline SVG via `embed.FS`; no external assets, no CDN, no framework. Read-only by construction: GET-only (405 otherwise), `Content-Security-Policy: default-src 'self'`, Host-header check against DNS rebinding, and **no mutating endpoints** — approving from the browser is explicitly rejected for 0.6. There is no `--bind` flag.

Sections: Overview (certification status, latest change, blocking findings, evidence health), Product, Contract (requirement pages: why / scenarios / evidence / implementation / history), Changes, Code (symbol → REQ), Evidence (coverage and gaps), Findings, Graph, Health (dimension table). The Graph page is an **ego-graph explorer**, never a global hairball: a focused node, deterministic radial ring layout, depth 1–3, edge-kind filters, click-to-refocus; the API caps ~120 nodes and collapses overflow into cluster nodes. The V1 markdown renderer and Gherkin highlighter (XSS-safe, tested) are reused. Evidence IDs never headline — the UI shows "green, 2h ago", with hashes behind a disclosure.

## 20. Policies, structured constraints, SMT, mutation

**Policies are CUE** (`cuelang.org/go`), split between an embedded kernel layer and the project:

- `policies/*.cue` (project, package `telospolicy`) — evidence classes per requirement class, architectural boundary rules, finding escalation rules, protected-path extensions.
- Kernel schema + floor ship **inside the binary** (`go:embed`): definitions are closed structs; kernel minima are concrete values (`red_green: true`), so a project writing `false` produces a unification *conflict*, not an override; sets are structs (`{path: true}`), additive by construction; escalation applies kernel ∪ project rules with a strictest-wins action lattice. KERNEL-008 is therefore structural. Numeric-bound weakening that CUE silently absorbs (project `>=0.3` vs kernel `>=0.5`) is detected by comparing the project-alone export against the unified export and reported as a non-blocking warning.

The **policy hash** — sha256 of the canonical JSON export of the unified value plus the kernel schema version — enters the certificate, and evidence records carry it in `depends_on`, so a policy change automatically stales the evidence its rules govern. Any Change touching `policies/**` or `telos.toml` is `privileged` (KERNEL-009).

**Structured constraints** attach to a requirement as a fenced ```` ```telos-constraint ```` block (typed `vars` + per-var CUE constraints + a `cross` list of relational expressions). Formalization is optional per requirement. Tier 1 (always available): CUE unification per scope detects empty ranges and concrete conflicts — provable unsatisfiability blocks certification (`TELOS_CONSTRAINT_UNSAT`, naming the REQ set). Tier 2 (optional): if a `z3` binary is on PATH, cross-variable systems in a conservative QF_LIA+Bool subset are checked via SMT-LIB with named assertions; `unsat` yields a blocking finding citing the unsat core; `unknown`/timeout is an explicit non-result that neither blocks nor satisfies anything. **z3 is never required**; its absence is reported explicitly (`doctor` shows `z3 (optional)`).

**Mutation testing** hardens test honesty as an evidence kind policy can require: a built-in AST mutator (stdlib; operator families: conditional boundary, negation, arithmetic, logical) scoped to functions intersecting the Change's diff, executed via `go test -overlay` so the tree is never touched, with hard cost caps (12 mutants/site, 100/run, timeout 2× baseline, stop-on-first-kill). Surviving mutants become candidate findings triaged through the normal taxonomy.

## 21. Agent architecture

Four specialized, untrusted agent roles orbit the deterministic kernel — single-sourced role definitions generated into provider formats (Claude `.md`, Codex `.toml`) with CI-enforced parity:

| Role | Does | Forbidden |
|---|---|---|
| **Challenger** | understands intent, asks material questions, searches the graph, drafts the minimal contract delta | approving, implementing, certifying |
| **Consistency critic** *(new)* | analyzes the target contract via `search`/`related`/`impact`, hunts contradictions and duplicates, files findings with `proposed_severity`+`confidence`+`rationale` | resolving contradictions, certifying |
| **Implementer** | works only in the candidate, satisfies obligations via the broker (witnessed red → green), smallest justified changes | editing the certified worktree, weakening contract, bypassing evidence, certifying |
| **Verifier** | independent read-only audit: test honesty, patch scope, provenance; emits findings | repairing its own findings, waiving failures, certifying |

The orchestrating skill routes on `telos status --json` (`context` + `state` + `change.status`) and obeys one presentation rule: never surface certificate IDs, evidence hashes, or worktree internals to the human — speak in intents, requirements, changes, and findings.

## 22. Canonical vs derived

**Canonical/certified:** contract (`spec/`), code, tests, `telos.toml`, `policies/`, `changes/CHG-*/**` (intents, deltas, decisions, findings, evidence records, provenance), certificates (notes). **Derived/disposable:** SQLite index, FTS, symbol cache, context packs, web assets, any LLM summaries. The architecture test: *all correctness-relevant state must be recoverable after deleting every derived artifact.*

## 23. Migration from V1 (v0.6 is a clean break)

No users exist; no migration tooling ships. Removed outright: the `spec_pending` asymmetric state (any protected edit is now symmetric corruption + salvage), `telos: RULE-*` annotations (→ provenance), OBJ/RULE vocabulary (→ INT/REQ/DEC/CHG), `.telos/state.json` (→ sealed certificates in git notes), the parallel hash universe (`normalize`/`rootHashMap`/`inventories` → git OIDs), the static-only `telos view` (→ localhost server with `--static` export), `telos trace` (→ `telos explain`), `telos spec put/review/approve` and `telos apply` (→ the Change lifecycle). Kept: the JSON envelope, the guard protocol, witnessed red/green (re-homed), digest-bound approval (digest becomes a tree OID), the bundle/install machinery, and the test discipline (error-code assertions, fake-suite probe).

## 24. Implementation milestones

| # | Branch | Delivers | Gate |
|---|---|---|---|
| M0 | `v2-contracts` | this document; `codes.go`; `graph.Querier` stub; frozen formats; the three loops as skipped e2e tests; dependency + cross-compile CI gate | contracts compile |
| M1 | `git-substrate` | `gitx`, certificates/seal/genesis, contract parser, `init/status/verify/doctor` | certified state on a toy repo |
| M2 | `change-lifecycle` | change verbs, candidate worktrees, delta+fold, worktree-aware guard | loop 1 through approval |
| M3 | `evidence-certify` | evidence records/closures/red-green/reuse, findings, `ready/promote` | `TestLoopFeature` green |
| M4 | `salvage-concurrency` | corruption, salvage/restore, rebase + selective revalidation, provenance | loops 2 & 3 green → **dogfooding starts** |
| M5 | `derived-graph` | SQLite index, query commands | search/impact on telos-sdd itself |
| M6 | `context-policies` | context compiler, CUE policies, constraints tier 1 | `telos context` usable |
| M7a/M7b | `bundle-tooling`/`bundle-v2` | role generator + validator; V2 skill/protocol/roles; version pinning | agent-driven loop 1 |
| M8 | `web-view` | the server | `telos view` on telos-sdd |
| M9 | `hardening-evidence` | mutation, SMT tier 2 | mutation score in Health |
| M10 | `docs-ci-release` | docs rewrite, release checklist, tag | **v0.6.0** |

## 25. Product statement

> **Telos makes every accepted repository state explicit and certifiable. Human intent becomes an approved contract; agents may change the implementation only inside verified transitions; evidence proves conformance; any out-of-band change invalidates the state immediately — and one gesture turns it into a Change.**

Shorter: **a certified-state kernel for AI-assisted software development**, whose operational guarantee is that **intent, contract, implementation, evidence, and policy evolve only through verified atomic transitions** — with Git as the substrate, evidence as content-addressed proof, and a deterministic kernel that no agent, however confident, can talk its way around.
