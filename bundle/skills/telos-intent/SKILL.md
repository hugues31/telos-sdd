---
name: telos-intent
description: Create, refine, validate, and seal a precise Telos development intent from a request or promoted brainstorm. Use before writing specs whenever desired outcomes, actors, scope, exclusions, constraints, success criteria, or open questions are not yet sealed.
---

# Telos Intent

Treat the intent as a contract about why and what must become observable, not how to implement it.

1. Run `telos intent new --title "<outcome>"` and add `--from <brainstorm-id>` when promoting a brainstorm.
2. Work with the `telos-intent-refiner` agent when available.
3. Fill every required section. Name actors and permissions, included behavior, non-goals, measurable success criteria, constraints, and all material questions.
4. Challenge vague terms such as fast, intuitive, secure, robust, normal, and should. Replace them with observable thresholds or decisions.
5. Ask the user focused questions in small rounds. After each answer, reformulate the artifact and check for contradictions or hidden implementation choices.
6. Run `telos intent validate <id>`. Repeat the refinement loop until it succeeds. `Open questions` must contain exactly `None.` only when every material ambiguity is resolved.
7. Show the final intent and get explicit user approval.
8. Run `telos intent seal <id>` only after approval.

Do not write a spec, feature file, test, or production code while the intent is draft. Do not invent product decisions to make validation pass.

Return the intent ID, validation result, remaining risks, and sealed hash when sealed.

