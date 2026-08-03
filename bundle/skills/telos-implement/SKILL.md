---
name: telos-implement
description: Implement an active Telos change strictly from sealed intent, specs, and executable scenarios. Use only after test artifacts exist and a `telos change begin` ID has established scope; do not use to bypass or edit sealed requirements.
---

# Telos Implement

Implement the smallest change that satisfies the sealed contract.

1. Require an active change ID. If absent, run `telos change begin --intent <id> --spec <id>` only after confirming all referenced artifacts and generated features are sealed.
2. Run `telos context --change <change-id>` and use `.telos/context.md` as the implementation boundary.
3. Inspect the repository and map each required code/test change to a `RULE-NNN` and `SCN-NNN`.
4. Implement thin vertical slices. Run the relevant executable scenarios after each slice.
5. Add adapter or step-definition code needed to execute Gherkin, but do not alter generated `.feature` files or sealed test plans.
6. Do not add speculative abstractions, adjacent features, unrelated refactors, silent fallbacks, or behavior without a traced rule.
7. Never weaken an assertion, skip a scenario, replace the subject under test with a mock, or special-case test inputs. If the contract is impossible or contradictory, stop and return to the spec workflow.
8. Run all configured verification commands and then `$telos-verify`.

Return the rule-to-code/test mapping, commands run, and any contract gap. Do not claim completion without verifier evidence.

