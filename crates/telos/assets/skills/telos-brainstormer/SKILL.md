---
name: telos-brainstormer
description: Clarify new Telos work, explore relevant alternatives, and persist a proportionate brief before preparing a repository-wide execution plan. Use for new requests or material scope changes, not routine resumption.
---

# Telos brainstorming

Begin with the user's request and existing decisions. Read `telos status --json`
and bounded `telos map`, `telos impact`, `telos pack`, or `telos show` results as
needed. A precise request may need only a short brief; an ambiguous request
deserves focused exploration. This skill has no dependency on BMAD.

1. State the intended outcome, its users, observable success, constraints and
   exclusions. Identify affected contexts, capabilities and repository paths,
   including tests, documentation, configuration and dependencies.
2. Offer useful alternatives when they affect the outcome, complexity, scope or
   recovery strategy. Name the tradeoff. Do not add an optional idea to the
   execution scope unless the user accepts it.
3. Ask only questions whose answers could materially change the plan. Prefer a
   small set of concrete choices. Reuse answers and permissions already given;
   never require a ceremonial question, fixed number of rounds or new approval
   for an unchanged decision. Continue independent read-only investigation while
   an answer is pending.
4. Persist the brief with `telos plan open` and `telos plan edit` (JSON on stdin).
   Use `brief.summary`, `users`, `exclusions`, `decisions`, `questions` and
   `brainstormed: true`. A decision has `id`, `text`, `state`, `source` and optional
   `alternatives`. States are `proposed`, `accepted`, `rejected`, `deferred`;
   sources are `user`, `existing_instruction`, `agent`. Agent proposals remain
   proposals. A question has `id`, `text`, `blocking` and nullable `answer`.
5. Preserve unanswered questions and rejected alternatives. On interruption,
   resume from the persisted brief; do not repeat answered questions or invent
   missing answers. Blocking questions prevent approval.
6. Load and invoke `telos-challenger` to translate the clarified brief into
   exact task contracts and the plan revision for review.

Do not edit application files during brainstorming. Preparing plan records
through the CLI is allowed before approval and needs no recursive outer plan.
