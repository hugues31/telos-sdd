# Architecture

Telos has four layers:

1. One public `telos` Skill owns the conversation, phase resumption and two user approvals.
2. Five specialized agents own product analysis, specification, adversarial tests, implementation and independent verification.
3. The dependency-free Go CLI is the only mutation authority. It owns IDs, state, validation, rendering, patch application, hashes and recovery.
4. Git and provider hooks expose history and block ordinary direct-write paths.

## Flow state machine

Each worktree permits one active `FLW-*` object:

```text
discovery
  → intent_draft
  → intent_review
  → contract_draft
  → contract_review
  → ready_to_implement
  → implementing
  → complete
```

`telos inspect --json` is the entry point for every agent turn. It audits repository content and draft hashes before returning the active phase and legal next actions.

## Authority chain

```text
reviewed + sealed intent (CRIT-*)
  └── reviewed + atomically sealed contract
        ├── specs (RULE-* → CRIT-*)
        ├── test plans (SCN-* → RULE-*)
        └── deterministic .feature files
              └── active change + generated context
                    └── traced patch transactions
                          └── independent evidence + completion
```

Review digests bind approval to exact draft bytes. Contract sealing stages and validates every spec, plan and feature before updating the artifact lock once.

Sealing, completion and restore are additionally human-gated at the harness: `telos guard` answers `intent seal`, `contract seal`, `change complete` and `repair --restore` with an `ask` permission decision naming the flow, artifacts and review digest, and denies a seal whose digest no longer matches the recorded review. The approval record is therefore a provider permission prompt, not orchestrator-reported conversation.

## Repository integrity

The artifact lock and repository lock are independent:

- `lock.json` covers sealed product and executable-contract artifacts;
- `repository-lock.json` covers every Git-tracked or non-ignored regular file outside `.telos`;
- mutation records connect before/after repository roots to immutable patch bytes and traceability IDs;
- content-addressed blobs reconstruct the last declared repository state.

Roots hash sorted `normalized/path + NUL + normalized-content-hash + LF` records. Text line endings are normalized to LF.

The CLI checks the repository root before every operation. `change apply` checks a Git patch, applies it, stores its evidence and advances the root as one brokered transaction. A failure reverses the patch and restores the prior lock.

## Contract validation

Intent criteria, spec rules and scenarios are machine-linked. Every rule/category pair has a coverage decision. `covered` requires a tagged scenario; `not_applicable` requires a rationale. IDs are unique across the flow.

The JSON plan is the reviewed source and `.feature` is a deterministic projection. Language-specific step definitions remain traced implementation code and therefore must be submitted through `change apply`.

## Provider boundary

Codex and Claude Code receive the same public Skill content and equivalent custom-agent roles through their native repository layouts. Provider hooks call `telos guard`; no provider adapter contains integrity logic. Guard denies non-broker mutations and never lets a human-gate command pass silently: it answers with `ask` or `deny`, so every seal, completion and restore surfaces a native permission prompt.
