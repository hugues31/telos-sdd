# Telos

**A certified-state kernel for AI-assisted software development. Every commit accepted on `main` is a verified alignment of contract, code, evidence, and policy — sealed as a certificate in git notes. For Codex and Claude Code, without a hosted runtime.**

In a Telos project, human intent becomes an approved contract under `spec/` (intents, requirements with executable scenarios, decisions — plain Markdown at the repository root). Agents change the implementation only inside verified transitions: an isolated Change candidate, a digest-bound human approval, witnessed test-first proof, and an atomic promotion. Any out-of-band edit invalidates the state immediately — and one gesture turns it into a Change instead of losing it.

## The three loops

**Feature** — the daily driver:

```text
I ask for a feature
  → the challenger sharpens the need and drafts a contract delta
  → I settle the material questions and approve the exact delta
    (native permission prompt, bound to the reviewed digest)
  → the implementer works in an isolated candidate:
    witnessed failing test first, then the smallest green implementation
  → the verifier audits; findings are triaged
  → telos change promote: one new certified commit on main
```

**Accidental edit** — corruption is a status with a way out:

```text
I edit a file in the certified worktree
  → CORRUPTED — telos status proposes: "Capture this edit as CHG-042?"
  → telos salvage: my work moves into a candidate, the root is restored
  → same loop as above
```

**Concurrent changes** — evidence is content-addressed:

```text
A and B are in flight; A promotes
  → B's base is stale: telos change rebase
  → only proofs whose dependency closure intersects A's diff are re-run;
    disjoint changes re-certify almost free
  → the approval is re-asked only if A touched the contract
```

These loops are executable: `cmd/telos/loops_test.go` runs all three against the real CLI on every CI push.

## What the kernel enforces

- **Certified states, not trusted agents.** A certificate (sealed git note on the commit) binds contract tree, code tree, approvals, evidence, and policy hash. `telos verify` recomputes it; forging requires more than editing files.
- **Approvals bind to exact bytes.** The approval digest is the folded contract tree OID; any drift makes it stale — never an LLM's opinion of "semantic equivalence".
- **Proof is witnessed.** A requirement's test is sealed while failing on a green baseline and must pass with the same bytes. Flaky evidence never certifies; `retry-until-green` does not exist.
- **Evidence is content-addressed.** Records key on their exact inputs (`go test`-cache style closures); unchanged closure ⇒ reusable proof, unknown dependencies ⇒ conservative invalidation.
- **Findings gate deterministically.** Critics only propose severity; a human confirms blocking, or policy escalates by explicit rules. Critic false-positive rate is a tracked health metric.
- **Policy has a floor.** `policies/*.cue` unifies with an embedded kernel floor — weakening a kernel minimum is a unification conflict, structurally.

## Install

```bash
go install github.com/hugues31/telos-sdd/cmd/telos@latest
# or the release binaries:
curl -fsSL https://raw.githubusercontent.com/hugues31/telos-sdd/main/install.sh | sh
```

```bash
cd your-repository
telos init --agent all --ci github   # seals the genesis certificate
telos doctor
```

Then talk normally to your coding agent; the `/telos` (Claude) or `$telos` (Codex) Skill routes on `telos status --json`.

## CLI

Every command supports a stable `--json` envelope (`ok`, `command`, `result`, `next_actions`, structured error codes).

```text
telos init | status | verify | doctor | version
telos change start|show|diff|review|approve|ready|rebase|promote|abort
telos salvage [--into CHG-NNN] | restore
telos evidence red|green|adopt --req REQ-NNN | evidence mutation
telos findings list|add|confirm|resolve
telos index rebuild|status
telos search | show | related | impact | explain | context
telos view [--port N] [--static DIR]
telos guard
```

`telos view` serves a loopback-only, read-only projection of the certified model: contract, evidence coverage, findings, and an ego-graph explorer. `telos context` compiles bounded context packs for agents (global invariants always included, canonical content only). Both read the same derived, disposable SQLite graph: `rm .telos/cache/index.db && telos index rebuild` restores everything from certified artifacts alone.

## Security boundary

The default seal is `SEALED`: an HMAC with an embedded secret that detects out-of-protocol edits and blocks trivial certificate forging — it is not a boundary against an adversary holding the binary and the same OS account. The important boundary is the transition kernel: nothing can seal a state that did not pass verification. See [docs/threat-model.md](docs/threat-model.md).

## Development

```bash
go test ./...
go vet ./...
go generate ./bundle && git diff --exit-code
go run ./tools/gen-bundle -check
```

See [docs/design-v2.md](docs/design-v2.md) (the adopted design), [docs/architecture.md](docs/architecture.md), and [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT © 2026 Hugues Gaillard.
