<p align="center">
  <img src="docs/assets/telos.png" alt="Telos" width="420">
</p>

[![CI](https://github.com/hugues31/telos-sdd/actions/workflows/ci.yml/badge.svg)](https://github.com/hugues31/telos-sdd/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org/)

**Keep software intent typed, reviewable, and testable.**

Telos is a local Rust CLI for specification-driven development. It stores
bounded contexts, capabilities, notions, intents, scenarios, constraints, and
per-context bindings as a typed, referentially checked model committed with
your project. Context ownership, dependency direction, vocabulary
qualification, and production-file boundaries are enforced without an LLM.
Changes pass through
reviewable transactions, and coherent states are sealed with Git blob IDs so
out-of-protocol edits are visible.

Telos is useful when people and coding agents need the same bounded source of
truth for what a system should do and why. It does not call an LLM, generate
application code, or require a hosted service.

## Install

Linux and macOS:

```console
curl -fsSL https://raw.githubusercontent.com/hugues31/telos-sdd/main/install.sh | sh
```

Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/hugues31/telos-sdd/main/install.ps1 | iex
```

From source (stable Rust toolchain and Node.js 22 required; build the embedded
frontend first):

```console
git clone https://github.com/hugues31/telos-sdd.git
cd telos-sdd
cd frontend
npm ci
npm run build
cd ..
cargo install --locked --path crates/telos
```

Prebuilt archives are also on the
[releases page](https://github.com/hugues31/telos-sdd/releases). Git must be
available on your PATH at runtime.

## Quick start

Run Telos inside an existing Git repository:

```console
telos init
telos status
telos check --sealed
```

At initialization time, you can instead install the optional Claude Code and
Codex integrations and create a GitHub sealed-state workflow:

```console
telos init --agents claude,codex --ci github
```

The generated workflow downloads the corresponding Telos release binary and
runs `telos check --sealed`. Publish that release with its binary assets
before relying on the workflow, and enable branch protection separately if the
check must be required before merge.

## See it in action on GitHub

[telos-tamagotchi](https://github.com/hugues31/telos-tamagotchi) is a
complete, replayable demo: a Tamagotchi raised spec-first through five sealed
change transactions — 12 intents, 17 scenarios each proven by red-then-green
witnesses, and two constraints with executable checks — with the
implementation in Python. Each version is a prompt you can read, hand to a
coding agent, or replay deterministically, and the demo's CI rebuilds the
whole story from an empty directory on every push. Browse the exported spec at
[hugues31.github.io/telos-tamagotchi](https://hugues31.github.io/telos-tamagotchi/).

<p align="center">
  <a href="https://hugues31.github.io/telos-tamagotchi/">
    <img src="https://raw.githubusercontent.com/hugues31/telos-tamagotchi/main/docs/demo.gif"
         alt="telos view browsing the sealed tamagotchi spec" width="840">
  </a>
</p>

## Core workflow

Never edit files under `telos/` directly. Use a change transaction:

```console
telos change open "settle an invoice after payment" --json

printf '%s\n' '{"status":"active"}' \
  | telos edit intent INT-0042 --change CHG-0001 --json

telos change diff CHG-0001 --json
telos change approve CHG-0001 --expected-digest '<digest from diff>' --json

telos pack INT-0042 --json
telos test SCN-0107 --json
# Implement SCN-0107 while keeping the witnessed test unchanged.
telos bind src/billing/invoice.rs INT-0042 --json
telos test SCN-0107 --json

telos change reconcile CHG-0001 --json
telos check --sealed --json
```

In strict TDD mode, run the scenario test while it still fails to record a red
witness. Implement the scenario without changing that test, then rerun it
successfully to record green evidence for the same test bytes. If linked code
or specification files change outside the protocol, `telos status` reports
drift; use `telos adopt` to capture it or `telos revert` to restore the seal.

## Explore and rebuild

```console
# Browse the current model on a loopback-only server.
telos view --port 3000

# Export a coherent sealed model as a self-contained static site.
telos view --export site

# Inspect prerequisite order and measure scenario progress.
telos rebuild plan --json
telos rebuild status --json
```

`telos view` serves the embedded SPA with hash routes such as `#/intents` and
`#/graph`. An exported site is ready for static hosting, including GitHub
Pages, and `site/index.html` also opens directly with `file://`.

`rebuild` plans and measures a reconstruction; it does not write application
code. The [Billing demo](demo/billing) is a spec-only project that exercises
the complete reconstruction protocol.

## Command map

| Task | Commands |
|---|---|
| Inspect | `status`, `check`, `show`, `list`, `query`, `impact`, `pack`, `map` |
| Change the model | `change open|list|diff|approve|reconcile|abandon`, `add`, `edit`, `move`, `map --change`, `remove` |
| Record evidence | `test`, `bind` |
| Resolve drift | `adopt`, `revert` |
| Present | `view`, `view --export` |
| Reconstruct | `rebuild plan`, `rebuild status` |
| Configure | `config`, `init --agents`, `init --ci github` |

Every public command supports a stable five-key `--json` envelope. See the
[CLI contracts](docs/contracts.md) for schemas, error codes, safety boundaries,
and exact command behavior.

## Development

```console
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p telos --test rebuild_demo
```

CI runs the workspace on Linux, macOS, and Windows.

## License

[MIT](LICENSE) © 2026 Hugues Gaillard.
