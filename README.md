# Telos SDD

**The spec lives in the repo. Git is the history. Code follows the approved spec — for Codex and Claude Code, without Tessl or a hosted runtime.**

In a Telos project, the versioned source of truth is `spec/`: a product intent (`spec/PRODUCT.md`) and behavioral rules with executable scenarios (`spec/<domain>.md`), in plain Markdown at the root of the repository. An agent challenges and sharpens your need in conversation, the resulting spec diff is approved by you through a native permission prompt, and only then does code change — through a broker that keeps both trees provably in sync:

- a spec change without implementation fails verification: `TELOS_RULE_NOT_IMPLEMENTED`;
- a code change without an approved spec fails verification: `TELOS_CODE_CORRUPTED`;
- an implementation before its witnessed failing test is refused: `TELOS_TEST_FIRST`.

## Founding principles

### Code is becoming a generated artifact

Modern coding models make source code progressively cheaper to produce and replace. The durable value of a software project therefore moves upstream: into its intent, behavioral rules, and executable examples. Telos gives that asset first-class status: the spec is not internal tool data in a hidden directory — it is readable, reviewable, versioned content at the repository root, and its diff appears in every pull request next to the code it justifies.

### A bug is evidence about the specification

Telos treats an accepted bug as evidence that the contract was incorrect, imprecise, or incomplete. Fixing a bug therefore starts in `spec/`: the rule or scenario that permitted the defect is corrected and re-approved first, then the implementation follows from the stronger contract. The codebase never accumulates fixes whose rationale exists only in commit history.

```text
Request
  → conversation: the agent challenges and sharpens the need
  → spec diff (objectives, rules, Gherkin scenarios)
  → one human approval, enforced by the harness
  → witnessed red: the rule's test fails before its implementation exists
  → broker-applied implementation; the sealed test turns green untouched
  → telos verify: spec == code, every rule proven
```

## Three mechanical invariants

1. **The spec matches its approved root.** Any difference — including a human editing `spec/` directly, which is legitimate — puts the project in `spec_pending`: the agent challenges and normalizes the diff, presents it, and the human approves. The spec is never silently restored.
2. **The code matches its declared root.** Any out-of-band code edit is `TELOS_CODE_CORRUPTED`. Recovery is Git (`git restore`, checkout of a green commit) or a deliberate, human-gated re-baseline. The write is never adopted.
3. **Every rule is proven by a witnessed red-green cycle.** A `RULE-NNN` is implemented only when a file matching the configured `test_files` references it, the broker witnessed that test fail before the implementation existed, the sealed test bytes reached green untouched, and the configured `test_commands` pass. A test the suite already passes is rejected as evidence — the test cannot move to satisfy the implementation; only the implementation may turn red into green. Until the cycle completes the spec is ahead and `telos verify` fails — which is exactly what blocks a merge in CI.

Every traced file carries a `telos: RULE-NNN` annotation in its first lines, validated on the post-image of every patch: code exists only because a rule demands it, and `telos trace RULE-NNN` lists the files and tests of any rule from the tree alone.

## Install

### Go

```bash
go install github.com/hugues31/telos-sdd/cmd/telos@latest
```

### Release binary — macOS or Linux

```bash
curl -fsSL https://raw.githubusercontent.com/hugues31/telos-sdd/main/install.sh | sh
```

### Release binary — PowerShell

```powershell
irm https://raw.githubusercontent.com/hugues31/telos-sdd/main/install.ps1 | iex
```

## Add Telos to a repository

```bash
cd your-repository
telos init --agent all --ci github
telos doctor
```

`init` requires a Git worktree. It creates `telos.toml` (human-owned configuration: test commands, test file patterns, untraced patterns), a `spec/PRODUCT.md` skeleton, and `.telos/state.json` (the only internal file, committed with your code), and installs the skill, agents, and guard hooks for the chosen providers. On an existing codebase, `init` adopts the current tree as the declared baseline; `telos verify` then lists every legacy file to classify — as untraced in `telos.toml`, or as implementation of rules you specify progressively.

## Use

After initialization, talk normally to your coding agent:

> I want to prevent a locked account from signing in without terminating existing sessions.

The repository instructions activate `$telos` in Codex or `/telos` in Claude Code. The orchestrator:

1. runs `telos status --json` and routes on the phase;
2. delegates challenging, implementation, and audit to three separate agents;
3. presents the exact spec diff and asks for the one product decision that matters:

**"Is this exactly the intended behavior?"**

That approval is enforced by the harness, not by orchestrator discipline: `telos guard` answers `telos spec approve` with an `ask` permission decision naming the digest and files, so the provider surfaces a native prompt. A digest that no longer matches the reviewed content is denied outright. Declining the prompt refuses the approval.

The guard also gates the two code paths a spec approval does not cover. `telos apply` on a clean project raises its own prompt naming the cited rules and patched files, because such a patch claims to preserve behavior (a refactor, a hardened test) and only you can accept that claim — a reported bug never qualifies; it is evidence the spec was too weak, and the fix starts there. And `telos apply --expect-pass` prompts for the adoption claim: a rule documenting behavior the code already has can never be witnessed failing, so a test that passes immediately counts as proof only once you accept it. No code ever changes without a named human decision.

There is no completion ceremony: green `telos verify` is completion, your Git commit is the record, and the PR diff shows the spec change next to the code that implements it.

## Internal agents

| Agent | Responsibility | Forbidden |
| --- | --- | --- |
| `telos-challenger` | Brainstorming, challenging the need, drafting the spec diff | Approving, touching code |
| `telos-implementer` | Witnessed failing test first, then the smallest annotated patches via the broker | Direct writes, weakening assertions, touching sealed tests |
| `telos-verifier` | Read-only audit: test honesty, patch scope, annotation truth | Repairing or waiving failures |

## Repository model

```text
telos.toml               # human-owned config: test_commands, test_files, untraced patterns
spec/
  PRODUCT.md             # vision, OBJ-* objectives, constraints, non-goals
  <domain>.md            # RULE-* rules, Traces: OBJ-*, ```gherkin scenarios
.telos/
  state.json             # approved spec root + declared code root (committed)
<code>                   # every traced file annotated `telos: RULE-*`
<tests>                  # every rule referenced by a tagged test
```

OBJ and RULE ids are unique across the repository. Rules trace to objectives; annotations trace files to rules; tags trace tests to rules. The whole chain is recomputable from the working tree — there is no ledger, no blob store, no internal history. Git already does that.

## CLI

Every command supports a stable `--json` envelope with `ok`, `command`, `result`, `next_actions`, and structured error codes.

```text
telos init [--agent codex|claude|all] [--ci github]
telos doctor | telos status | telos version
telos spec put --file spec/<name>.md [--delete]   # brokered spec writes (stdin)
telos spec review                                 # digest + exact content to present
telos spec approve --review <digest>              # the single human gate
telos apply --rule RULE-NNN [--rule ...]          # brokered Git patch (stdin)
telos apply ... --expect-pass                     # adopt existing behavior (human-gated)
telos verify                                      # recompute every invariant
telos trace [RULE-NNN]                            # rule → files → tests
telos view [--out <path>] [--open]                # self-contained HTML spec view
telos guard                                       # provider hook endpoint
```

`telos view` renders the whole contract as one static HTML page — product intent, rules with their scenarios, per-rule implementation status, files and tests from the annotations, the objective→rule traceability matrix, and the verification setup from `telos.toml` (so a reader knows what "proven" means; keep secrets out of `test_commands`) — with no external assets. It writes to the system temp directory by default; an `--out` inside the repository must be git-ignored, since a generated page is not part of the declared code tree.

Humans normally need only `init` and `doctor` — and their IDE for `spec/`, since a direct spec edit is simply a pending diff the agent will pick up, normalize, and bring to approval.

## Security boundary

Telos makes undeclared changes visible and forces human decisions through native permission prompts. It does not create an operating-system privilege boundary: a malicious process running with the same user privileges can rewrite state and Git history consistently. Hashes prove byte identity, not semantic equivalence between prose and code; the file-level annotation guarantee is mechanical, while sub-file honesty (a decorative test tag, an unrelated hunk) is audited by the independent verifier and reviewed by the human on the spec diff. See [docs/threat-model.md](docs/threat-model.md).

## Development

```bash
go test ./...
go vet ./...
python3 scripts/validate-skills.py
```

See [docs/architecture.md](docs/architecture.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT © 2026 Hugues Gaillard.
