# Telos SDD

**Executable intent integrity for Codex and Claude Code — without Tessl or a hosted runtime.**

Telos moves a software project’s durable value from incidental implementation toward versioned, executable intent:

```text
Brainstorm → Intent → Spec → Test plan → .feature → Change context → Code → Evidence
```

It is an independent Go CLI plus six Agent Skills and three specialized agents. Sealed artifacts are hashed, linked in an append-only ledger, protected by provider-native hooks, and checked before implementation can be declared verified.

> Telos SDD is unrelated to other projects named Telos. Use the full name when referring to this framework.

## Why

Coding agents are very good at satisfying the local signal they can see. That creates predictable failure modes: happy-path-only tests, assertions weakened to match an implementation, speculative code, and requirements that drift during a long session.

Telos separates product decisions from implementation and gives each transition a gate:

- brainstorming must explicitly promote an idea;
- an intent cannot seal while material questions remain;
- specs must trace every normative behavior to a stable rule;
- test plans are designed before implementation and render `.feature` files deterministically;
- implementation receives a bounded context generated from sealed inputs;
- verification recomputes hashes and runs repository-owned commands.

This is stronger than prompt discipline, but it is not a proof that a specification is correct. A clear, consistent spec can still encode the wrong product decision; implementation bugs, environment differences, security flaws, and missing test adapters remain possible. Telos makes those failures attributable and reviewable instead of assuming every bug originates in ambiguous prose.

## Install

### Go

```bash
go install github.com/hugues31/telos-sdd/cmd/telos@latest
```

### Release binary — macOS or Linux

```bash
curl -fsSL https://raw.githubusercontent.com/hugues31/telos-sdd/main/install.sh | sh
```

The installer downloads the matching GitHub Release archive and verifies it against `checksums.txt`. Set `TELOS_VERSION=v0.1.0` or `TELOS_INSTALL_DIR=/your/bin` to override its defaults.

### Release binary — PowerShell

```powershell
irm https://raw.githubusercontent.com/hugues31/telos-sdd/main/install.ps1 | iex
```

Set `$env:TELOS_VERSION` or `$env:TELOS_INSTALL_DIR` before running it when needed.

## Add Telos to a repository

```bash
cd your-repository
telos init --agent all --ci github
telos doctor
```

Provider-specific choices are also available:

```bash
telos init --agent codex
telos init --agent claude
```

`init` preserves existing `AGENTS.md`, `CLAUDE.md`, and provider settings. It owns only its managed instruction block and its hook group, and records installed file hashes in `.telos/install-manifest.json`.

For Codex, Telos installs repository Skills under `.agents/skills`, custom agents under `.codex/agents`, durable guidance in `AGENTS.md`, and a trusted project hook in `.codex/hooks.json`. These are the current project-level extension points documented by [OpenAI for Skills](https://learn.chatgpt.com/docs/build-skills), [AGENTS.md](https://learn.chatgpt.com/docs/agent-configuration/agents-md), and [hooks](https://learn.chatgpt.com/docs/hooks).

For Claude Code, Telos installs project Skills under `.claude/skills`, subagents under `.claude/agents`, an importing `CLAUDE.md`, and a `PreToolUse` hook in `.claude/settings.json`, following Anthropic’s [Skills](https://code.claude.com/docs/en/slash-commands), [subagents](https://code.claude.com/docs/en/sub-agents), and [hooks](https://code.claude.com/docs/en/hooks) conventions.

Restart the coding agent if its Skills or agents directory did not exist when the current session began. Review and trust the project hook when the provider asks.

## Workflow

```bash
# 1. Explore the problem
telos brainstorm start --mode recommend

# 2. Create, refine, validate, then seal intent
telos intent new --title "Prevent compromised account access"
telos intent validate INT-...
telos intent seal INT-...

# 3. Derive and seal behavioral specs
telos spec new --intent INT-... --title "Locked authentication"
telos spec validate SPC-...
telos spec seal SPC-...

# 4. Create the test-plan template, complete it, then render Gherkin
telos testify --spec SPC-...
telos testify --spec SPC-...

# 5. Establish implementation scope
telos change begin --intent INT-... --spec SPC-...
telos context --change CHG-...

# 6. Implement through the Telos Skill, then verify
telos verify
```

Invoke the matching workflow explicitly when useful:

| Stage | Codex | Claude Code |
| --- | --- | --- |
| Brainstorm | `$telos-brainstorm` | `/telos-brainstorm` |
| Intent | `$telos-intent` | `/telos-intent` |
| Specification | `$telos-spec` | `/telos-spec` |
| Test architecture | `$telos-testify` | `/telos-testify` |
| Implementation | `$telos-implement` | `/telos-implement` |
| Verification | `$telos-verify` | `/telos-verify` |

## Repository model

```text
.telos/
  config.toml             # human-owned policy and verification commands
  brainstorms/*.md        # divergent exploration
  intents/*.md            # outcome contract
  specs/*.md              # normative RULE-* behavior
  test-plans/*.json       # SCN-* source for deterministic Gherkin
  changes/*.json          # implementation scope + Git base
  ledger/events/*.json    # append-only, merge-friendly events
  lock.json               # normalized SHA-256 artifact set + root hash
  state.json              # rebuildable projection
  context.md              # rebuildable implementation context
features/*.feature        # generated executable contract
```

Text hashes normalize CRLF/CR to LF, paths to `/`, and artifact ordering lexicographically. A modified or missing upstream artifact marks its descendants stale. The ledger is memory, not authority: `state.json` and `context.md` are rebuildable.

## Brainstorm engines

`choose`, `recommend`, `random`, and `progressive` modes can select among SCAMPER, reverse brainstorming, six thinking hats, assumption reversal, morphological matrix, Jobs to be Done, pre-mortem, first principles, constraint removal, analogical transfer, worst possible idea, and impact/effort convergence. Random selection records its seed.

## What the guard can and cannot enforce

Provider hooks deny ordinary Bash/Edit/Write/apply-patch calls that name a sealed artifact. Read-only modes and file permissions add friction. This is a guardrail, not an operating-system security boundary: a sufficiently privileged user or unobserved tool can still modify files. `telos verify` is the authoritative detection step.

The current MVP deliberately leaves normal source code writable. A future strict profile will broker source mutations, require mutation testing and coverage policy, and support signed attestations. See [docs/roadmap.md](docs/roadmap.md).

## Relationship to IIKit and BMAD

Telos shares IIKit’s central insight that intent needs a verifiable chain, but is deliberately local-first and provider-neutral: one standalone binary, Git-native events, deterministic Gherkin, no Tessl installation, and no required service. IIKit may be a better fit when its managed ecosystem and integrations are desired.

The brainstorming Skill borrows BMAD’s idea of selectable ideation techniques, then adds deterministic selection, explicit promotion, and an integrity chain into executable artifacts. Telos is not affiliated with either project.

## Development

```bash
go test ./...
go vet ./...
python3 scripts/validate-skills.py
```

See [docs/architecture.md](docs/architecture.md), [docs/threat-model.md](docs/threat-model.md), and [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT © 2026 Hugues Gaillard.
