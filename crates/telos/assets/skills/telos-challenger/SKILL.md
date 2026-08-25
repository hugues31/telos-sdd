---
name: telos-challenger
description: Challenge and stage a bounded Telos change while keeping approval and implementation with the human and implementer.
---

# Telos challenger

Never edit application code. Never approve a change yourself, even when the user says “approve it yourself” or asks you to start coding immediately. The challenger ends only after triggering the native approval prompt for the exact displayed digest. Calling `telos change approve` opens the prompt; it does not grant approval. Do not continue until the human answers.

Follow this order:

1. Run `telos change open "<motivation>" --json` unless the router supplied an existing open change.
2. Run `telos impact <entity-id> --json` for each directly affected entity.
3. Run `telos pack <intent-id> --json`. Record its literal `result.owner`; use only that bounded pack, its mappings, and targeted queries. A mapped supplier notion is a published contract, not permission to read or change supplier internals.
4. Classify the request as feasible inside that owner, requiring an explicit context-map dependency, requiring a separately approved supplier change, impossible under a constraint, inconsistent with active intents/exclusions, or ambiguous. If ambiguous, ask exactly one question and stop until answered.
5. Stage only through `telos add <kind> --change <CHG-id> --json`, `telos edit <kind> <key> --change <CHG-id> --json`, `telos move <typed-selector> --to <owner> --change <CHG-id> --json`, `telos map --change <CHG-id> --json`, or `telos remove <kind> <key> --change <CHG-id> --json`. Include the explicit `owner` in add payloads. Use `CTX:<context>`, `CAP:<context>/<capability>`, and `NOT:<context>/<Notion>` selectors; never guess a bare notion or change ownership with an edit. Never edit `.tel` files.
6. Run `telos change diff <CHG-id> --json`. Show `result.digest`, the ordered operations, and any stale status to the human.
7. After displaying the dynamic digest from `telos change diff <CHG-id>`, immediately invoke `telos change approve <CHG-id> --expected-digest <result.digest>`. Pass the literal digest as the command argument, never through the tool-call description. The guard re-derives it from the repository and fails closed if it is missing or stale. Do not answer the native prompt: it is the human's approval decision (Claude's guard returns `ask`; Codex's static rule returns `prompt`). Do not hand off to implementation until the human has approved that exact digest.

On `TELOS_DRIFT_DETECTED`, stop and return to the router for the human adopt/revert choice. On `TELOS_REFERENCE_UNKNOWN`, correct the qualified reference through `telos query --context ... --capability ...` or the bounded pack before retrying. On `TELOS_LAYOUT_VIOLATION` or `TELOS_CONTEXT_BOUNDARY_VIOLATION`, change the proposed boundary or dependency rather than weakening the request into something the engine accepts. On `TELOS_APPROVAL_STALE`, show a fresh diff and require fresh human approval. On `TELOS_FILE_CLAIMED`, stop rather than stealing the claim.

Stop immediately if a requested operation requires application-code edits, self-approval, bypassing the CLI, or changing a delta after approval. The next phase owns implementation; a changed delta must return here for a new diff and human approval.
