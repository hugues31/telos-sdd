# Telos SDD

**Agent-first, executable intent integrity for Codex and Claude Code — without Tessl or a hosted runtime.**

Telos turns a product request into an approved, executable contract before allowing code to change. The user speaks to one orchestrator; five isolated specialists and a deterministic Go CLI carry out the workflow.

## Founding principles

### Code is becoming a generated artifact

Modern coding models make source code progressively cheaper to produce and replace. The durable value of a software project therefore moves upstream: into its intent, behavioral specifications, constraints and executable examples.

Telos treats code as one implementation of that contract, not as the primary source of truth. Given the same external context—dependencies, toolchain, runtime services and platform—a sufficiently complete specification should allow a modern coding model to reconstruct a functionally equivalent project with little or no human rewriting. The objective is not byte-for-byte reproduction, but reproduction of every specified behavior, boundary and prohibited side effect.

### A bug is evidence about the specification

Within a fixed external context, Telos treats an accepted bug as evidence that the executable contract was incorrect, imprecise, incomplete or inconsistent with another rule. If an implementation simply violates an already precise contract, verification should reject it; if it reaches users anyway, the contract's executable checks or verification process were themselves insufficient.

Fixing a bug therefore means more than patching code. The intent, rule or scenario that permitted the defect must first be corrected or completed, then the implementation is regenerated or revised from that stronger contract. This prevents the codebase from accumulating fixes whose rationale exists only in source code or commit history.

```text
Request
  → Intent approval
  → Specs + scenarios approval
  → CLI-brokered implementation
  → Independent verification
```

The CLI owns state, validation, rendering, hashes and writes. Agents own judgment. The user never copies a Telos path, ID, or lifecycle command.

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

Use `--agent codex` or `--agent claude` to install only one provider adapter. `init` preserves existing project instructions and provider settings while owning its managed block and hook group.

## Use

After initialization, talk normally to your coding agent:

> I want to prevent a locked account from signing in without terminating existing sessions.

The repository instructions activate `$telos` in Codex or `/telos` in Claude Code. Explicit invocation remains available, but is not required.

The orchestrator:

1. runs `telos inspect --json` and resumes the active flow;
2. uses brainstorming only when the problem or solution space is uncertain;
3. delegates intent, spec, tests, implementation and verification to separate agents;
4. asks for two product decisions in the main conversation;
5. applies every repository mutation through the CLI.

The two approvals are deliberately human:

- **Intent:** “Is this the desired outcome?”
- **Executable contract:** “Is this exactly the expected behavior?”

Each review returns a digest of the exact content shown. Any later change invalidates that approval.

Both approvals are enforced by the harness, not by orchestrator discipline. `telos guard` answers every `intent seal`, `contract seal`, `change complete`, and `repair --restore` with an `ask` permission decision, so the provider surfaces a native permission prompt naming the flow, artifacts, and review digest. An agent cannot seal, complete, or restore silently; declining the prompt refuses the approval. A seal whose digest no longer matches the recorded review is denied outright, so the user is only prompted for seals that can succeed.

## Internal agents

| Agent | Responsibility | Forbidden |
| --- | --- | --- |
| `telos-product` | Brainstorming and measurable intent | Specs, tests, code |
| `telos-spec-architect` | Observable rules, boundaries and non-effects | Weakening intent from existing code |
| `telos-test-architect` | Adversarial scenarios and coverage decisions | Inspecting implementation first |
| `telos-implementer` | Smallest traced Git patch | Direct repository writes |
| `telos-verifier` | Read-only integrity and test-honesty audit | Repairing or waiving failures |

These are delegated specialists, not extra user-facing commands.

## Strict integrity

Telos inventories every Git-tracked or non-ignored regular file, excluding `.git/**` and internal `.telos/**` data. It stores normalized SHA-256 hashes and content-addressed recovery blobs.

Every implementation patch records:

- its SHA-256 digest;
- its before and after repository roots;
- every touched path;
- the `RULE-NNN` and `SCN-NNN` identifiers that authorize it;
- append-only ledger evidence.

Provider hooks deny ordinary Edit, Write, apply-patch and obvious shell mutation paths, and force a native permission prompt on the four human-gate commands. The authoritative control is recomputation: if any byte differs from the last CLI-declared state, commands fail with:

```text
TELOS_INTEGRITY_UNDECLARED_CHANGE: project corrupted
```

The write cannot be adopted after the fact. The orchestrator may inspect the repair plan, request explicit approval, then run `telos repair --restore --json` to reconstruct the last declared state.

Git-ignored build outputs remain outside the inventory. A test or generator that changes a non-ignored file makes verification fail.

## Executable contract

Intent success criteria use stable headings:

```markdown
### CRIT-001 — Locked authentication is denied
```

Every normative rule traces one or more criteria:

```markdown
### RULE-001 — Deny authentication

Traces: CRIT-001
```

Every rule receives an explicit decision for nine coverage categories: positive, negative, boundary, authorization, state transition, retry/idempotency, concurrency, failure/recovery and prohibited side effects. A category is either backed by a tagged `SCN-NNN` scenario or marked `not_applicable` with a concrete rationale.

Contract sealing is atomic: specs, JSON plans and deterministic `features/*.feature` files are all sealed, or none are.

If implementation exposes a wrong contract, Telos reverses declared patches with `change abort`, then creates an immutable intent or spec successor with `artifact revise`. It never edits a sealed artifact.

## CLI primitives for agents and CI

The workflow API is intentionally machine-oriented. Every command supports a stable `--json` envelope with `ok`, `command`, `result`, `next_actions`, and structured error codes.

```text
telos inspect --json
telos flow start --brainstorm none|recommend --json
telos artifact put --id ... --json
telos intent new|review|seal --flow ... --json
telos spec new --flow ... --json
telos test-plan put --spec ... --json
telos contract validate|review|seal --flow ... --json
telos change begin|apply|abort|complete --flow ... --json
telos artifact revise --id ... --json
telos verify --flow ... --check-only --json
telos repair --restore --json
```

Humans normally need only `init` and `doctor`.

## Repository model

```text
.telos/
  config.toml
  flows/*.json             # FLW-* state machine
  brainstorms/*.md
  intents/*.md             # CRIT-* outcome contract
  specs/*.md               # RULE-* behavioral contract
  test-plans/*.json        # SCN-* plus coverage matrix
  changes/*.json           # implementation scope and source roots
  mutations/*.json         # traced patch transactions
  patches/*.patch          # immutable patch evidence
  blobs/*                  # content-addressed recovery data
  ledger/events/*.json     # append-only events
  lock.json                # sealed artifact root
  repository-lock.json     # declared repository root
  context.md               # generated implementation boundary
features/*.feature         # deterministic executable contract
```

## Security boundary

Telos detects undeclared content and makes ordinary agent bypasses visible. It does not create an operating-system privilege boundary: a malicious process running with the same user privileges can attempt to bypass local hooks and rewrite metadata. Hostile guarantees require an external signer, protected key material or privilege-separated broker.

Hashes also cannot prove semantic equivalence between prose and code. Telos combines structural traceability, executable scenarios and an independent verifier to make drift reviewable rather than claiming a formal proof.

## Development

```bash
go test ./...
go vet ./...
python3 scripts/validate-skills.py
```

See [docs/architecture.md](docs/architecture.md), [docs/threat-model.md](docs/threat-model.md), and [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT © 2026 Hugues Gaillard.
