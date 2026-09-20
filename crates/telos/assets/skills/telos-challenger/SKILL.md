---
name: telos-challenger
description: Challenge and stage a bounded Telos change while keeping approval and implementation with the human and implementer.
---

# Telos challenger

Treat this skill as a behavioral contract, not an engine-enforced guarantee.

Work from the brainstormer's persisted brief and existing user authorization.
Do not edit application files. Prepare a plan definition with a goal, success
criteria, repository-wide scope, dependency-ordered tasks and final validation.
Each task needs a stable `TSK-NNN` identity, kind, allowed paths, acceptance
criteria, validations and an exact `spec_delta` (empty for purely technical work).
Every validation is a named argv command, scenario ID or explicit review.

1. Run `telos plan show <PLN-id> --json`; resolve blocking brief questions.
2. Run bounded `telos impact` and `telos pack` queries for affected entities.
3. Define tasks with `telos plan edit <PLN-id>` using JSON on stdin. To build a
   delta through existing staging commands, use `telos plan task prepare
   <PLN-id> <TSK-id>`, then stage into the returned change ID.
4. Before any classification or staging, perform the mandatory **Domain-language review**. Use only that bounded pack, published mappings, targeted Telos queries, and the user's request and answers. A mapped supplier notion is a published contract, not permission to read or change supplier internals; never read them.

   Build a **Language delta** that identifies:
   - newly introduced domain terms;
   - existing terms being reused;
   - renamed or replaced terms;
   - each term's owning context and capability;
   - each term's kind: actor, entity, value, event, or state;
   - each term's definition and difference from related concepts.

   Look explicitly for unjustified synonyms and overloaded terms (the same term used with multiple meanings), technical terms presented as domain concepts, confusion among a command, event, state, and entity, and creation of a new notion when an existing notion may already fit.

   Verify behavioral precision: the business trigger, the observable business outcome, affected invariants, one nominal case, at least one relevant edge, negative, or failure case, and the behaviors explicitly excluded. Separate the business outcome from the technical mechanism — “add an endpoint” is not an outcome; name the outcome it enables.

   Verify the domain boundary: the request belongs to the correct context and capability; it does not recreate locally a concept owned by a supplier; any new dependency or mapping is genuinely necessary; and any supplier modification genuinely requires a separate change.

   A **material ambiguity** is any unresolved question that could change notions or their meaning, the owning context, the observable outcome, an invariant, the scenarios, or the operations that would be staged.
5. While a material ambiguity exists, stage nothing. Ask the material question and persist it in the brief; continue independent read-only work while waiting. Then repeat the Domain-language review and loop until no material ambiguity remains. Never invent an answer, turn an assumption into a decision, or weaken the request to make it compatible with the engine.
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

   Staging may begin only when the final line explicitly states `Remaining material questions: none`. Persist the summary and decisions in the plan brief.
7. Perform the final request classification: feasible inside that owner, requiring an explicit context-map dependency, requiring a separately approved supplier change, impossible under a constraint, inconsistent with active intents/exclusions, or ambiguous. An ambiguous classification is a material ambiguity: return to step 4 instead of staging.
8. Stage only through the CLI, every call carrying `--change <CHG-id> --json`: `telos add <kind>`, `telos edit <kind> <key>`, `telos move <typed-selector> --to <owner>`, `telos map`, `telos remove <kind> <key>`. Include the explicit `owner` in add payloads. Use `CTX:<context>`, `CAP:<context>/<capability>`, and `NOT:<context>/<Notion>` selectors; never guess a bare notion or change ownership with an edit. Never edit `.tel` files.

   **Expression fields are a grammar, not prose.** A scenario's `then`, a `state-driven` `while`, an `unwanted` `if`, and a constraint's `rule.expr` are the Telos mini-language: `Notion.attr == literal` or `Notion.attr in (a, b)`, combined with `and`/`or`/`not`. Write `Invoice.state == settled`, never `the invoice is shown as settled`. Identifiers are ASCII whatever the domain's language — PascalCase notions, lower-kebab attributes — so transliterate them and keep the domain's own language in `title`, `def`, and `telos`. A `statement.action` starting with `set ` must parse as `set Notion.attr = literal`; any other action is a free clause. Each `given`/`when` step is `{"notion": "Name", "fields": {…}}` typed against that notion's declared attributes, so stage every notion and attribute a scenario asserts on in the same change. A rejected payload names the failing field in `error.message` (`payload.scenarios[0].then[1]: ...`) and the grammar in `error.hint`: correct that field, never loosen the scenario into prose the engine accepts.
9. For each prepared change, run `telos plan task import <PLN-id> <TSK-id>`.
   Inspect `telos plan diff <PLN-id> --json`: review the full scope, task graph,
   specification deltas, exclusions, validators and literal revision digest.
10. Invoke `telos plan approve <PLN-id> --expected-digest <digest>` under the
    user's approval. Native host rules prompt for this plan revision. Existing
    user decisions remain valid; do not ask again about resolved questions.
11. Load and invoke the implementer. Each covered change inherits plan approval;
    do not request another human `change approve`. A new goal, allowed path,
    behavior delta, dependency or validator requires a new approved revision.

Stop conditions, by code:

- `TELOS_DRIFT_DETECTED`: stop and return to the router for the human adopt/revert choice.
- `TELOS_REFERENCE_UNKNOWN`: correct the qualified reference through `telos query --context ... --capability ...` or the bounded pack before retrying.
- `TELOS_LAYOUT_VIOLATION` / `TELOS_CONTEXT_BOUNDARY_VIOLATION`: change the proposed boundary or dependency rather than weakening the request into something the engine accepts.
- `TELOS_APPROVAL_STALE`: show a fresh diff and require fresh human approval.
- `TELOS_FILE_CLAIMED`: stop rather than stealing the claim.

Stop immediately if a requested operation requires application-code edits, self-approval, bypassing the CLI, or changing a delta after approval. Never modify a change after approval: the next phase owns implementation, and a changed delta must return here for a new diff and human approval.
