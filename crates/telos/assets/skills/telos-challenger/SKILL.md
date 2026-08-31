---
name: telos-challenger
description: Challenge and stage a bounded Telos change while keeping approval and implementation with the human and implementer.
---

# Telos challenger

Treat this skill as a behavioral contract, not an engine-enforced guarantee.

Never edit application code. Never approve a change yourself, even when the user says “approve it yourself” or asks you to start coding immediately. The challenger ends only after triggering the native approval prompt for the exact displayed digest. Calling `telos change approve` opens the prompt; it does not grant approval. Do not continue until the human answers.

Follow this order:

1. Run `telos change open "<motivation>" --json` unless the router supplied an existing open change.
2. Run `telos impact <entity-id> --json` for each directly affected entity.
3. Run `telos pack <intent-id> --json`. Record its literal `result.owner`.
4. Perform the mandatory **Domain-language review** before any final classification or staging operation. Use only the bounded pack returned by `telos pack`, published mappings, targeted Telos queries, and the user's request and answers. A mapped supplier notion is a published contract, not permission to read or change supplier internals. Never read the internals of a supplier context.

   Build a **Language delta** that identifies:
   - newly introduced domain terms;
   - existing terms being reused;
   - renamed or replaced terms;
   - each term's owning context and capability;
   - each term's kind: actor, entity, value, event, or state;
   - each term's definition and difference from related concepts.

   Look explicitly for unjustified synonyms and overloaded terms (the same term used with multiple meanings), technical terms presented as domain concepts, confusion among a command, event, state, and entity, and creation of a new notion when an existing notion may already fit.

   Verify behavioral precision by identifying the business trigger, the observable business outcome, affected invariants, one nominal case, at least one relevant edge, negative, or failure case, and behaviors explicitly excluded from the change. Distinguish the requested business outcome from the proposed technical mechanism: “add an endpoint” is not sufficient without the business outcome it enables.

   Verify the domain boundary: the request belongs to the correct context and capability; it does not recreate locally a concept owned by a supplier; any new dependency or mapping is genuinely necessary; and any supplier modification genuinely requires a separate change.

   A **material ambiguity** is any unresolved question that could change notions or their meaning, the owning context, the observable outcome, an invariant, the scenarios, or the operations that would be staged.
5. While a material ambiguity exists, stage nothing. Ask exactly one question—the question that reduces uncertainty the most—and stop immediately. Do not continue before receiving the human's answer. After the human answers, repeat the Domain-language review and continue this loop until no material ambiguity remains. Do not invent an answer, turn an assumption into a decision, or weaken the request merely to make it compatible with the engine.
6. Before staging, display this concise semantic summary:

   ```text
   Domain review

   Language delta
   - Introduced: ...
   - Reused: ...
   - Renamed or rejected synonyms: ...

   Decisions
   - ...

   Assumptions
   - ...

   Excluded or boundary cases
   - ...

   Remaining material questions: none
   ```

   Staging may begin only when the final line explicitly states `Remaining material questions: none`. This summary is informational only; do not persist it in the change file.
7. Perform the final request classification: feasible inside that owner, requiring an explicit context-map dependency, requiring a separately approved supplier change, impossible under a constraint, inconsistent with active intents/exclusions, or ambiguous. An ambiguous classification is a material ambiguity: return to step 4 instead of staging.
8. Stage only through `telos add <kind> --change <CHG-id> --json`, `telos edit <kind> <key> --change <CHG-id> --json`, `telos move <typed-selector> --to <owner> --change <CHG-id> --json`, `telos map --change <CHG-id> --json`, or `telos remove <kind> <key> --change <CHG-id> --json`. Include the explicit `owner` in add payloads. Use `CTX:<context>`, `CAP:<context>/<capability>`, and `NOT:<context>/<Notion>` selectors; never guess a bare notion or change ownership with an edit. Never edit `.tel` files.

   **Expression fields are a grammar, not prose.** A scenario's `then` entries, a `state-driven` `while`, an `unwanted` `if`, and a constraint's `rule.expr` are strings of the Telos mini-language: `Notion.attr == literal`, `Notion.attr in (a, b)`, combined with `and`/`or`/`not`. Write `Invoice.state == settled`, never `the invoice is shown as settled`. Identifiers are ASCII whatever the domain's natural language — PascalCase notions, lower-kebab attributes — so a domain modelled in another language transliterates its identifiers and keeps the language itself in `title`, `def`, and `telos`. A `statement.action` beginning with `set ` must parse as `set Notion.attr = literal`; any other action string is a free clause. Every `given`/`when` step is `{"notion": "Name", "fields": {…}}`, typed against that notion's declared attributes, so every notion and attribute a scenario asserts on is already staged in the same change.

   A rejected payload names the failing field in `error.message` (`payload.scenarios[0].then[1]: ...`) and the expected grammar in `error.hint`. Correct that field; never loosen a scenario into prose the engine happens to accept.
9. Run `telos change diff <CHG-id> --json`.
10. Show `result.digest`, the ordered operations, and any stale status to the human.
11. After displaying the dynamic digest from `telos change diff <CHG-id>`, immediately invoke `telos change approve <CHG-id> --expected-digest <result.digest>`. Pass the literal digest as the command argument, never through the tool-call description. The guard re-derives it from the repository and fails closed if it is missing or stale.
12. Do not answer the native prompt: it is the human's approval decision (Claude's guard returns `ask`; Codex's static rule returns `prompt`). Do not hand off to implementation until the human has approved that exact digest.

On `TELOS_DRIFT_DETECTED`, stop and return to the router for the human adopt/revert choice. On `TELOS_REFERENCE_UNKNOWN`, correct the qualified reference through `telos query --context ... --capability ...` or the bounded pack before retrying. On `TELOS_LAYOUT_VIOLATION` or `TELOS_CONTEXT_BOUNDARY_VIOLATION`, change the proposed boundary or dependency rather than weakening the request into something the engine accepts. On `TELOS_APPROVAL_STALE`, show a fresh diff and require fresh human approval. On `TELOS_FILE_CLAIMED`, stop rather than stealing the claim.

Stop immediately if a requested operation requires application-code edits, self-approval, bypassing the CLI, or changing a delta after approval. Never modify a change after approval. The next phase owns implementation; a changed delta must return here for a new diff and human approval.
