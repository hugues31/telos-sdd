---
name: telos-spec
description: Derive one or more complete behavioral specifications from a sealed Telos intent. Use when translating an approved outcome into normative rules, examples, boundaries, failure behavior, and observability without choosing unnecessary implementation details.
---

# Telos Spec

Derive the smallest set of independently coherent specs that fully covers the sealed intent.

1. Read the sealed intent and confirm its hash with `telos status`.
2. Split specs by independently changeable behavior, not by code layer or file.
3. Run `telos spec new --intent <intent-id> --title "<behavior>"` for each behavior.
4. Give every normative rule a stable `RULE-NNN` identifier. Write rules as observable invariants with explicit preconditions and effects.
5. Cover representative examples, empty and limit values, permissions, invalid transitions, duplicate requests, retries, concurrency, partial failures, compatibility, and recovery where relevant.
6. State non-effects: what must not happen. Define externally visible errors, events, metrics, and audit evidence without prescribing internal architecture unless the intent constrains it.
7. Cross-check every intent success criterion against at least one rule and remove rules that do not trace back to the intent.
8. Run `telos spec validate <id>`, resolve every failure, show the spec for approval, then run `telos spec seal <id>`.

Do not inspect existing implementation to weaken the desired contract. Existing technical constraints may inform feasibility but cannot silently redefine intent.

Return a traceability summary from intent criteria to rule IDs and the sealed spec hashes.

