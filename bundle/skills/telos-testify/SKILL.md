---
name: telos-testify
description: Design an adversarial Telos JSON test plan and deterministically generate executable Gherkin feature files from sealed specs. Use after specification sealing and before implementation, especially to prevent happy-path-only, tautological, skipped, or implementation-coupled tests.
---

# Telos Testify

Act independently from the implementer. Test the contract, not the current code.

1. Read the sealed intent and spec. Do not inspect production implementation first.
2. Run `telos testify --spec <spec-id>` once to create `.telos/test-plans/<spec-id>.json`; the command intentionally stops on the initial TODO template.
3. Replace the template with scenarios that map to real `RULE-NNN` identifiers. Keep stable `SCN-NNN` identifiers.
4. For each applicable rule include positive, negative, boundary, authorization, state-transition, retry/idempotency, concurrency, and failure/recovery cases. Tag each scenario by category.
5. Make Given/When/Then steps externally observable. Assert outputs, state, emitted signals, and prohibited side effects. Avoid assertions that merely repeat mocks or implementation calls.
6. Reject conditional skips, empty assertions, tests of fixtures instead of behavior, mocks of the subject under test, and scenarios that cannot fail for a broken implementation.
7. Ask for a product decision if the sealed spec cannot determine an expected outcome. Do not patch ambiguity in the test.
8. Run `telos testify --spec <spec-id>` again. Review the deterministic `features/*.feature` output; never edit it directly.

Return coverage by rule and category, deliberate omissions with rationale, and generated artifact hashes.

