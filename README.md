<h1 align="center">
  <img src="docs/assets/telos.png" alt="Telos" width="420">
</h1>

<p align="center"><strong>Give people and coding agents a shared contract for what your software should do.</strong></p>

<p align="center">
  Local-first&nbsp;&nbsp;·&nbsp;&nbsp;Model-agnostic&nbsp;&nbsp;·&nbsp;&nbsp;Git-native&nbsp;&nbsp;·&nbsp;&nbsp;Any tech stack
</p>

<p align="center">
  <a href="https://github.com/hugues31/telos-sdd/actions/workflows/ci.yml"><img src="https://github.com/hugues31/telos-sdd/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/built_with-Rust-orange.svg" alt="Built with Rust"></a>
</p>

Telos is a local CLI that keeps requirements, scenarios, code links, and test
evidence together in Git. Approve resumable plans before implementation, give coding
agents focused context, and let configured CI detect drift from the approved
state.

Telos is written in Rust but works with any language. It makes no LLM calls,
generates no application code, and requires no hosted service.

## Why Telos?

**Tired of coding agents breaking existing behavior every time you ask for a
new feature?**

Telos is built on a simple spec-first principle: if your intent, scenarios,
constraints, and test evidence are detailed enough, a fresh agent—with no chat
history or hidden project knowledge—should be able to rebuild the software.

The specification is the durable source of truth. The current code is one
verified implementation of it.

- **Durable intent** versioned with the repository.
- **Focused context** for developers and coding agents.
- **Resumable plans** with one approval for their scope and durable task progress.
- **Traceable changes** linking files and domain entities to dated plan receipts.
- **Test evidence** linked to the same test failing, then passing in strict TDD
  mode.
- **Repository governance** covering code, tests, documentation and configuration.

## How it works

<p align="center">
  <img src="docs/assets/workflow.svg" alt="Telos workflow: define intent, review the change, implement with focused context, record test evidence, then seal and verify the state locally or in optional CI" width="1100">
</p>

Each approved cycle is sealed against the exact Git contents, making later
drift visible.

## Quick start

Install the latest release on Linux or macOS:

```console
curl -fsSL https://raw.githubusercontent.com/hugues31/telos-sdd/main/install.sh | sh
```

On Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/hugues31/telos-sdd/main/install.ps1 | iex
```

Then initialize Telos inside an existing Git repository:

```console
cd my-project
telos init --agents claude,codex --ci github
telos status
telos check --sealed --planned
telos view --port 3000 --open
```

Omit either initialization flag if you do not need it. `telos view --open`
launches the loopback-only view in your default web browser.

## See it in action

[Telos Tic-tac-toe](https://github.com/hugues31/telos-tictactoe) is a
tic-tac-toe game in Python, playable in the terminal and in a window, built
from six plain-text prompts given to a coding agent: four bounded contexts,
ten intents, a context map, an architecture constraint with an executable
check, and a hand edit of sealed code caught as drift. Browse its
[exported specification](https://hugues31.github.io/telos-tictactoe/),
check out the tag of any prompt, or replay the prompts against a Telos
release with its agent runner.

<p align="center">
  <a href="https://hugues31.github.io/telos-tictactoe/">
    <img src="https://raw.githubusercontent.com/hugues31/telos-tictactoe/main/docs/demo.gif" alt="Browsing the sealed Telos Tic-tac-toe specification" width="840">
  </a>
</p>

## A typical development loop

Every change starts with a native plan. The generated router calls the dedicated
brainstorming skill to clarify the request, then prepares a reviewable plan.
Approve its scope once and let the agent execute the covered tasks.

```console
telos plan open "settle an invoice after payment" --json
telos plan edit "$PLAN" < /tmp/plan-definition.json
telos plan diff "$PLAN" --json
telos plan approve "$PLAN" --expected-digest '<digest from plan diff>'
telos plan task start "$PLAN" TSK-001 --json

# Implement and prove the approved task, then save the next action.
telos plan checkpoint "$PLAN" --summary "Implementation ready" --next-action "Run validation"
telos change reconcile "$CHANGE"
telos plan verify "$PLAN" --task TSK-001 tests
telos plan task finish "$PLAN" TSK-001
telos plan verify "$PLAN" final
telos plan complete "$PLAN"
telos check --sealed --planned
```

`$PLAN` and `$CHANGE` are returned UUID identities. Validators are defined in the
plan. The [native plan guide](docs/plans.md) includes a complete definition,
behavioral delta preparation, recovery, branch integration and CI setup. The
[Billing demo](demo/billing) exercises the public reconstruction protocol.

Plans and progress are versioned `.tel` records. After an interruption, run
`telos plan resume "$PLAN" --json`; another agent can continue without the old
chat. The dashboard exposes **View plan**, task completion percentages and
history links. A scope change requires a new approved revision.

Set `[test] report` in `telos/telos.toml` to the JUnit XML file your runner
writes (and `{report}` in `[test] cmd` to tell it where): every green then
means a test named after the scenario executed and passed, a run that
executed nothing is refused, and `telos status` reports `proof_evidence`.

If `telos status` reports later drift, prepare an explicit recovery plan before
adopting or restoring it. Initialization observes existing files; it does not
claim they were implemented at the initialization date.

## A small mental model

| Term | Meaning |
|---|---|
| **Plan** | An approved scope, dependency graph, validations and durable progress. |
| **Receipt** | The dated record connecting applied changes to a plan and task. |
| **Context** | A domain boundary that owns vocabulary and behavior. |
| **Capability** | A responsibility the context provides. |
| **Notion** | A named domain concept and its attributes. |
| **Intent** | A behavior or outcome the software must support. |
| **Scenario** | An executable example that proves an intent. |
| **Constraint** | A rule that behavior or architecture must respect. |
| **Binding** | A link from an intent to the code that implements it. |

Telos validates references, ownership, dependency direction, vocabulary, and
production-file boundaries deterministically.

## Explore and reconstruct

- `telos view --port 3000 --open` browses the current model locally and opens it
  in the default web browser.
- `telos view --export site --open` creates a self-contained static site and
  opens its index page.
- `telos rebuild plan` and `telos rebuild status` show implementation order and
  scenario progress. They do not write code.

## Reference

- [Native plans](docs/plans.md): approval, resumption, history and CI.
- [CLI contracts](docs/contracts.md): schemas, errors, and safety boundaries.
- [Billing demo](demo/billing): the complete reconstruction protocol.
- [Telos Tic-tac-toe](https://github.com/hugues31/telos-tictactoe): a Python
  example built from prompts, replayable against any Telos release.
- [Releases](https://github.com/hugues31/telos-sdd/releases): prebuilt archives.

Git must be available on `PATH`. Generated CI also requires a published Telos
binary release and separately configured branch protection. Establish the initial
bootstrap as the trusted baseline before enabling the generated required check.

<details>
<summary><strong>Build from source</strong></summary>

Requires stable Rust and Node.js 22:

```console
git clone https://github.com/hugues31/telos-sdd.git
cd telos-sdd/frontend
npm ci
npm run build
cd ..
cargo install --locked --path crates/telos
```

</details>

## Developing Telos

```console
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p telos --test rebuild_demo
```

CI runs on Linux, macOS, and Windows.

## License

[MIT](LICENSE) © 2026 Hugues Gaillard.
