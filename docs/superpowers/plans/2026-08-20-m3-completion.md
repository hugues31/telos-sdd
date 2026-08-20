# Telos SDD M3 Completion Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. The root agent is the coordinator: each implementation task runs in an isolated worktree, writes an implementer report, receives spec and quality review, and is integrated only after both reviews pass.

**Goal:** Finish M3 by completing bounded context, lifecycle coverage, host skills and preventive guards, freezing the M3 public contracts, and turning all three acceptance loops green.

**Architecture:** Preserve the existing sealed model and append-only change journal. `context` compiles a deterministic post-overlay work pack without exposing the full spec. Agent support is generated opt-in by `init --agents`, from one canonical policy/skill source with thin Claude and Codex renderers. Hooks only prevent unsafe agent mutations; `telos` remains the authority for drift, lifecycle, witnesses, bindings, and reconciliation.

**Tech Stack:** Rust 2024, clap, serde/serde_json, tempfile, assert_cmd, predicates, git worktrees, Claude Code project skills/hooks, Codex repository skills/hooks/rules.

**Authoritative design:** `docs/specs/2026-08-19-telos-sdd-design.md` §§6–9 and §14. When this plan is more specific, it freezes the observable M3 behavior without weakening that design.

---

## Shared implementation constraints

- Prefix every shell command with `rtk`.
- Use TDD: add one focused failing test, run it and observe the expected failure, then write the minimum implementation and rerun it green.
- Do not edit `telos/` directly in generated-project tests; exercise public CLI behavior.
- Preserve the frozen JSON envelope `{ok, command, result, error, next_actions}` for public commands.
- Never overwrite unrelated user content in `AGENTS.md`, `.claude/settings.json`, `.codex/hooks.json`, or `.codex/rules/*.rules`.
- Keep skill bodies byte-identical across Claude and Codex. Host-specific discovery/frontmatter belongs in the smallest possible wrapper or in shared-compatible frontmatter.
- Platform ruling for §9: the skill must show the current relevant digest immediately before requesting the native approval. Claude's hook may return native `ask` with that digest in its reason. Codex's current hook protocol can deny but cannot request; Codex rules provide the native prompt while the skill displays the dynamic digest immediately before it. Do not add a fake or self-asserted digest parameter to the public CLI.
- The worktree-owning implementer must not modify files outside its task ownership. The coordinator resolves integration conflicts.

## Task 1: Complete bounded `context` and the implementing-state sweep (T7–T8)

**Branch/worktree:** `codex/m3-context`, current checkout containing Claude's preserved partial T7 work.

**Files:**

- Modify: `crates/telos/src/cli.rs`
- Modify: `crates/telos/src/commands/mod.rs`
- Modify: `crates/telos/src/commands/context.rs`
- Create: `crates/telos/tests/context.rs`
- Modify if and only if the new regression requires it: `crates/telos/src/commands/change.rs`
- Modify if and only if the new regression requires it: `crates/telos/tests/test_bind.rs`

### Step 1: Pin invalid-target behavior before changing production code

Add an integration test that invokes `context` with notion, constraint, and change references. Each must return a JSON error with:

```json
{
  "code": "TELOS_REFERENCE_UNKNOWN",
  "message": "`context` applies to intents and scenarios",
  "hint": null
}
```

Run:

```bash
rtk cargo test -p telos --test context rejects_non_intent_and_non_scenario_targets -- --exact
```

Expected: FAIL because `CON-…` currently reaches an `unreachable!` panic.

Replace the unreachable path with an exhaustive typed-reference rejection, then rerun the exact test and observe PASS.

### Step 2: Freeze the sealed bounded-pack schema

Add focused tests for an intent and for a scenario resolving to that intent. Assert exact JSON fields and ordering:

- `id`, `change`
- canonical `intent`
- ordered `scenarios` and their `proved` state
- used `notions`
- applicable global and scoped `constraints`
- folded `implements` and `proves` bindings
- directed one-hop `neighbours`

Assert that unrelated notions, constraints, and intents never appear.

Run the new tests and observe failure before filling any missing behavior. Implement only the missing behavior and rerun green.

### Step 3: Freeze staged-overlay and journal-fold behavior

Add tests proving that:

- an added/edited intent is read from its owning change's post-overlay model and reports that change id;
- a journalled `bind` is visible in `implements`;
- a sealed green `test` witness is visible in `proves` and sets the scenario's `proved` value;
- deterministic ordering is unchanged by journal append order.

Run:

```bash
rtk cargo test -p telos --test context
```

Expected after implementation: PASS.

### Step 4: Sweep re-approval while implementing

Add or tighten one regression test:

1. approve a change;
2. use `test` or `bind` so it becomes `implementing`;
3. stage another valid operation;
4. re-approve;
5. assert the state remains `implementing`, the ops digest is refreshed, approval is not stale, and reconciliation remains available.

Run the focused regression first. Modify production code only if it fails.

### Step 5: Verify and report

Run:

```bash
rtk cargo fmt --check
rtk cargo test -p telos --test context
rtk cargo test -p telos --test test_bind
rtk cargo test --workspace
```

Commit with a focused message and write the SDD implementer report containing commands, observed RED failures, green results, files changed, and self-review findings.

## Task 2: Generate host skills and preventive guards (T9–T10)

**Branch/worktree:** `codex/m3-agent-hosts`, created from the committed integration base before Task 1 code is merged. Rebase onto the Task 1 commit before final review so `context` is part of the exposed workflow.

**Files:**

- Modify: `crates/telos/src/cli.rs`
- Modify: `crates/telos/src/commands/init.rs`
- Modify: `crates/telos/src/commands/mod.rs`
- Create: `crates/telos/src/agents/mod.rs`
- Create: `crates/telos/src/agents/assets.rs`
- Create: `crates/telos/src/agents/guard.rs`
- Create host renderers under: `crates/telos/src/agents/`
- Create canonical skill assets under: `crates/telos/assets/skills/`
- Create: `crates/telos/tests/agent_init.rs`

### Step 1: Record RED skill pressure tests

Before authoring each skill, record a separate no-skill baseline for:

- router pressure: existing manual `telos/` drift plus a request to skip ceremony;
- challenger pressure: request to self-approve and start editing code;
- implementer pressure: request to alter an approved delta and change the test after red.

The baseline reports must identify the workflow omissions or rationalizations. Use these failures to write the minimum explicit instructions that close them. Do not teach the baseline agent the desired answer.

### Step 2: Specify opt-in host parsing with a failing CLI test

Add tests for:

- `telos init` creates no agent-host artifacts;
- `telos init --agents claude`, `codex`, and `claude,codex` create exactly the requested artifacts;
- duplicates normalize deterministically;
- an unknown host is rejected by clap before any project files are created.

Use a typed `AgentHost` clap value and a comma-delimited collection. Keep the successful init envelope compatible; if the result is extended, pin the exact shape in the test and in Task 3 contracts.

### Step 3: Build one canonical three-skill source

Write one canonical body per skill, copied byte-for-byte to both hosts:

- `telos`: always begin with `telos status --json`; route from literal `result.state` and frozen error codes; never mutate `telos/` manually; route drift only through a human choice of `adopt` or `revert`.
- `telos-challenger`: never touch application code; use `change open`, `impact`, and bounded `context`; ask one ambiguity question at a time; stage only through `add|edit|remove --change`; show `change diff` and its digest; leave approval to the human.
- `telos-implementer`: never alter the approved delta; work scenario by scenario from bounded `context`; observe a sealed red witness; keep the same test bytes for green; make the minimum code change; `bind`; then `reconcile`.

Each skill must route on exact JSON state/error fields and include stop conditions, not merely describe Telos concepts.

Add tests parsing each `SKILL.md` frontmatter and comparing normalized or exact bodies across hosts.

### Step 4: Implement and unit-test the common guard policy

The guard decision engine must accept a normalized tool name and input, then return allow/deny/ask plus a reason. Pin these cases before implementation:

- deny `Edit`, `Write`, or `apply_patch` when the target is under repository `telos/`;
- deny Bash commands that directly create, edit, move, or delete paths under `telos/`;
- allow source-code edits and read-only Telos inspection (`telos status`, `show`, `context`, `diff`);
- route `telos change approve`, `telos adopt`, and `telos revert` to a native human prompt where the host supports it;
- do not treat ordinary words containing `telos` as paths or mutations.

Keep this policy in Rust so generated projects do not require Python or Node.

### Step 5: Render Claude artifacts without overwriting user config

Generate project skills under `.claude/skills/*/SKILL.md`. Merge a clearly owned `PreToolUse` hook into `.claude/settings.json` for `Edit|Write|Bash` while preserving unrelated JSON keys and existing hooks.

Preflight-parse existing JSON before creating `telos/` or any host artifacts. A malformed settings file must return a corrective error with no partial initialization.

Exercise realistic hook JSON in tests and assert Claude's approval path returns `permissionDecision: "ask"` with the relevant digest/reason supplied by the skill/command context when available.

### Step 6: Render Codex artifacts without overwriting user config

Generate:

- `.agents/skills/*/SKILL.md`;
- an owned Telos block in root `AGENTS.md` while preserving all unrelated text;
- a synchronous `PreToolUse` hook in `.codex/hooks.json` for `apply_patch` and `Bash`;
- `.codex/rules/telos.rules` `prefix_rule` entries with `decision="prompt"` for approve/adopt/revert.

Merge idempotently. Codex hooks deny unsafe writes; native prompts come from rules. Test rendered rule semantics in Rust so CI does not require a local Codex binary.

### Step 7: Prove idempotence, preflight safety, and GREEN skill behavior

Initialize generated projects containing unrelated host configuration. Assert unrelated content survives, owned entries occur once, and malformed JSON causes no partial Telos tree.

After each skill body is available, rerun its pressure scenario with the skill loaded and require exact command ordering and resistance to the pressure. If the evaluator finds a new rationalization, add one explicit rule and rerun; do not broaden the skill speculatively.

Run:

```bash
rtk cargo fmt --check
rtk cargo test -p telos --test agent_init
rtk cargo test -p telos --test cli_m1
rtk cargo test --workspace
```

Commit and write the SDD implementer report.

## Task 3: Freeze M3 public contracts (T11)

**Branch/worktree:** `codex/m3-docs`, based on the reviewed integration of Tasks 1 and 2.

**Files:**

- Modify: `docs/contracts.md`
- Modify: `README.md`
- Modify: `crates/telos/tests/contracts.rs`
- Modify only for contract assertions: `crates/telos/tests/context.rs`
- Modify only for contract assertions: `crates/telos/tests/test_bind.rs`
- Modify only for contract assertions: `crates/telos/tests/reconcile.rs`

### Step 1: Make the contract test fail on the missing M3 surface

Add representative exact-envelope assertions for `context`, `test`, and `bind`. Assert the complete live error-code set, including `TELOS_TEST_NOT_FOUND`, `TELOS_SCENARIO_RED_EXPECTED`, and `TELOS_TEST_SEALED`.

Run the focused contract suite and observe the expected documentation/fixture mismatch before editing docs.

### Step 2: Freeze lifecycle and witness contracts

Document:

- `open → drafted → approved → implementing → reconciled`;
- journal records are digest-inert;
- both approved and implementing changes owe reconciliation;
- strict versus advisory sealed-red/green behavior;
- `test` discovery, explicit `--file`, result schemas, and non-zero-as-red semantics;
- `bind` ownership, path constraints, exact-path drift carve-out, result schema, and idempotence.

### Step 3: Freeze context and reconciliation contracts

Document the exact bounded-context schema and its post-overlay/journal-fold semantics. Update reconciliation to the actual ten-gate pipeline, including seal coverage, witness warnings, and the precise `--full` exceptions.

Update README M3 usage with short executable examples, not implementation internals.

### Step 4: Verify documentation against behavior

Run:

```bash
rtk cargo test -p telos --test contracts
rtk cargo test -p telos --test context
rtk cargo test -p telos --test test_bind
rtk cargo test -p telos --test reconcile
rtk cargo test --workspace
```

Commit and write the SDD implementer report.

## Task 4: Turn all acceptance loops green (T12)

**Branch/worktree:** `codex/m3-acceptance`, based on the reviewed Task 3 integration.

**Files:**

- Modify: `crates/telos/tests/acceptance_loops.rs`
- Modify: `README.md` only if Task 3 did not already remove stale ignored-loop wording

### Step 1: Reproduce the feature-loop failure

Run:

```bash
rtk cargo test -p telos --test acceptance_loops loop_feature -- --ignored --exact
```

Expected: FAIL at `telos test SCN-0001` because no discoverable test source contains `scn_0001`.

### Step 2: Add the smallest faithful test fixture

Create a generated-project test source containing the `scn_0001` naming token before the first Telos test command. Keep the existing marker-controlled fake runner so the same bytes are observed red and later green.

Rerun the ignored feature loop and observe PASS.

### Step 3: Remove M3 ignores and stale commentary

Remove `#[ignore]` from feature and drift. Keep merge in the ordinary suite. Update comments so all three tests describe executable M3 behavior, not future placeholders.

Run:

```bash
rtk cargo test -p telos --test acceptance_loops
rtk cargo test --workspace -- --ignored --list
```

Expected: all three acceptance tests PASS ordinarily and no M3 acceptance loop remains ignored.

### Step 4: Verify and report

Run formatting and the full workspace suite. Commit and write the SDD implementer report.

## Task 5: Integration, independent review, and final verification

**Owner:** root coordinator; no feature edits during this task except reviewer-requested fixes, each returned to the owning worktree.

### Step 1: Integrate in dependency order

Integrate reviewed commits in this order:

1. Task 1 (`context` and lifecycle sweep)
2. Task 2 (agent hosts, rebased onto Task 1)
3. Task 3 (contract freeze)
4. Task 4 (acceptance loops)

Resolve only mechanical conflicts centrally. Behavioral conflicts return to the owning implementer with a new failing regression.

### Step 2: Run spec-compliance and code-quality reviews

For every task, provide the reviewer the task brief, base SHA, head SHA, implementer report, and diff summary. Require separate outcomes:

- spec compliance: no missing or extra behavior;
- code quality: maintainability, test quality, safety, and compatibility.

All critical and important findings must be fixed and re-reviewed.

### Step 3: Run fresh final verification

Run from the integrated branch:

```bash
rtk cargo fmt --check
rtk cargo clippy --workspace --all-targets -- -D warnings
rtk cargo test --workspace
rtk cargo test --workspace -- --ignored --list
```

Also exercise generated Claude and Codex projects through `agent_init` tests. If the local `codex` executable exposes `execpolicy check`, run it as an additional non-CI validation of `.codex/rules/telos.rules`.

Record exact pass counts and command outputs. Invoke `superpowers:verification-before-completion` before claiming M3 complete and `superpowers:finishing-a-development-branch` before presenting integration choices.
