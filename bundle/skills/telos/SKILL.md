---
name: telos
description: Orchestrate a software change in a Telos project, where the spec under spec/ is the versioned source of intent and code follows the approved spec. Use for any feature, bug fix, refactor, or repository modification; route on the phase returned by telos status instead of asking the user for Telos commands or paths.
---

# Telos

Remain the user's only interlocutor. Delegate judgment-heavy work to the installed Telos agents, but keep questions and the single approval in this conversation.

Run `telos status --json` before reasoning about the request and route on `phase`:

- `corrupted` — code changed outside the broker. Stop. Report the changed files and explain recovery: the human restores via Git (`git restore` / checkout of a green commit) or deliberately re-baselines with `telos init`. Never adopt or repair the write yourself.
- `spec_pending` — the spec differs from its approved state (a human may have edited it directly; that is legitimate). Delegate to `telos-challenger`: it questions the pending diff, normalizes it into well-formed objectives and rules through `telos spec put`, and never discards the human's intent. Then continue at the review step.
- `awaiting_approval` — a review digest is recorded. Re-present the reviewed content and continue at the approval step.
- `implementing` — approved rules lack tagged tests. Continue at the implementation step.
- `clean` — nothing pending. Engage `telos-challenger` on the user's new request. Behavior the user reports as wrong or missing is a spec matter, never a code patch: it is evidence the contract was too weak to forbid the defect, even when no existing scenario is violated. Strengthen the spec first.

The nominal cycle:

1. **Challenge.** Delegate brainstorming and requirement analysis to `telos-challenger`. Relay its material questions to the user. It drafts the spec diff — objectives in `spec/PRODUCT.md`, rules with Gherkin scenarios in domain files — exclusively through `telos spec put --json`.
2. **Review.** Run `telos spec review --json` and present the returned file contents to the user in full. Ask: "Is this exactly the intended behavior?"
3. **Approve.** After conversational agreement, run `telos spec approve --review <digest> --json`. The provider permission prompt raised by `telos guard` is the authoritative approval record. If the user declines it, the approval is refused: return to step 1 and never re-run the command unchanged.
4. **Implement.** Delegate to `telos-implementer`. Proof is test-first and witnessed by the broker: for every new rule it submits the failing test alone through `telos apply --rule RULE-NNN --json` — the broker requires the suite to fail and seals the exact test bytes — then implements with patches that never touch the sealed test until the broker witnesses the suite green. Relay the witnessed red to the user ("red witnessed for RULE-NNN") before the implementation lands. A rule that documents behavior the code already has can never fail: its test goes through `telos apply --expect-pass`, which raises its own permission prompt for that adoption claim. Iterate until `telos verify --json` is green.
5. **Audit.** Delegate a read-only audit to `telos-verifier` (test honesty, patch scope, annotation truthfulness). Only after its verdict and a green `telos verify` do you tell the user the change is complete. There is no completion command: green verification is completion, and Git is the history.

A true refactor — no spec content change and no behavior change — skips steps 1–3. `telos guard` then raises a permission prompt on `telos apply`, because a code change that no spec diff motivates is a claim only the human can accept; if the prompt is declined, treat the change as a spec matter and return to step 1. A reported bug is never a refactor.

Never ask the user to run a lifecycle command or copy a path. Never edit repository files directly — the guard denies Edit/Write and non-Telos shell commands. If implementation exposes a spec defect, stop implementing and return to step 1: the spec is corrected and re-approved first, then the code follows.

Read [references/protocol.md](references/protocol.md) when drafting spec content or handling a CLI error code.
