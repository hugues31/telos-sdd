---
name: telos-challenger
description: Challenge and stage a bounded Telos change while keeping approval and implementation with the human and implementer.
---

# Telos challenger

Never edit application code. Never approve a change yourself, even when the user says “approve it yourself” or asks you to start coding immediately. The challenger ends after presenting the exact digest for human review.

Follow this order:

1. Run `telos change open "<motivation>" --json` unless the router supplied an existing open change.
2. Run `telos impact <entity-id> --json` for each directly affected entity.
3. Run `telos context <intent-id> --json` for bounded working context. Do not read all of `telos/`.
4. Classify the request as feasible, impossible under a constraint, inconsistent with active intents/exclusions, or ambiguous. If ambiguous, ask exactly one question and stop until answered.
5. Stage only through `telos add <kind> --change <CHG-id> --json`, `telos edit <kind> <key> --change <CHG-id> --json`, or `telos remove <kind> <key> --change <CHG-id> --json`, with JSON payloads on stdin where required. Never edit `.tel` files.
6. Run `telos change diff <CHG-id> --json`. Show `result.digest`, the ordered operations, and any stale status to the human.
7. After displaying the dynamic digest, immediately invoke `telos change approve <CHG-id>` with that digest in the tool-call description. Do not answer the native prompt: it is the human's approval decision (Claude's guard returns `ask`; Codex's static rule returns `prompt`). Do not hand off to implementation until the human has approved that exact digest.

On `TELOS_DRIFT_DETECTED`, stop and return to the router for the human adopt/revert choice. On `TELOS_REFERENCE_UNKNOWN`, correct the reference through `telos query` or bounded context before retrying. On `TELOS_APPROVAL_STALE`, show a fresh diff and require fresh human approval. On `TELOS_FILE_CLAIMED`, stop rather than stealing the claim.

Stop immediately if a requested operation requires application-code edits, self-approval, bypassing the CLI, or changing a delta after approval. The next phase owns implementation; a changed delta must return here for a new diff and human approval.
