---
name: telos-verify
description: Independently audit a Telos change for hash integrity, traceability, scope discipline, executable coverage, test cheating, and configured verification results. Use before claiming completion, committing a change, or merging CI.
---

# Telos Verify

Verify evidence; do not infer success from an implementation narrative.

1. Prefer the read-only `telos-verifier` agent when available.
2. Run `telos status`. Stop on a missing, stale, or tampered sealed artifact.
3. Read the active change, `.telos/context.md`, and the Git diff since its recorded base.
4. Map every production change to a sealed `RULE-NNN`; flag untraced behavior and unnecessary code.
5. Map every rule to executable scenarios. Inspect step definitions and assertions for skipped cases, unconditional passes, swallowed failures, tests of mocks rather than behavior, and special cases keyed to fixtures.
6. Check that negative, boundary, authorization, retry, concurrency, and recovery coverage exists where the spec makes each category relevant.
7. Run `telos verify`. This checks sealed hashes and all configured verification commands.
8. Report failures without repairing or waiving them. Send requirement defects back through a new intent/spec revision rather than editing sealed artifacts.

Return a verdict of `verified` or `rejected`, the root hash, commands and results, traceability gaps, scope violations, and test-integrity findings.

