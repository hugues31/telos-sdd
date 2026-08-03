---
name: telos
description: Orchestrate a software change from product request through intent, executable contract, CLI-brokered implementation, and independent verification. Use for any feature, bug fix, refactor, or other repository modification in a Telos project; resume the active flow instead of asking the user for Telos commands, IDs, or paths.
---

# Telos

Remain the user's only interlocutor. Delegate judgment-heavy work to the installed Telos agents, but keep questions and approvals in this conversation.

1. Run `telos inspect --json` before reasoning about the request. Stop on any integrity error; offer `telos repair --json`, then request explicit approval before `telos repair --restore --json`.
2. Resume the returned phase. If no flow is active, decide whether divergent exploration is material. Run `telos flow start --brainstorm recommend --json` for uncertain problems or `telos flow start --brainstorm none --json` for a precise request, passing the request through stdin.
3. Delegate discovery and intent drafting to `telos-product`. Relay only material questions. Require it to write through `telos artifact put --json`.
4. Run `telos intent review --flow <flow> --json`. Present the returned content without its ID or path. Ask: “Est-ce bien le résultat voulu ?” Seal only after explicit approval, using the exact returned digest.
5. Delegate behavioral rules to `telos-spec-architect`. Give the resulting intent and spec content—not repository paths—to `telos-test-architect`; it must remain blind to production code, existing implementation tests, and implementation patches. Both specialists must use the CLI. If tests expose ambiguity, return to the intent/spec workflow instead of inventing an expectation.
6. Run `telos contract review --flow <flow> --json`. Present specs, scenarios, coverage decisions, and non-effects together. Ask: “Est-ce exactement le comportement attendu ?” Seal only after explicit approval with the current digest.
7. Run `telos change begin --flow <flow> --json`, then delegate implementation to `telos-implementer`. It must submit Git patches through `telos change apply`; never edit repository files directly.
8. Delegate the final audit to `telos-verifier`. After a `verified` verdict and `telos verify --check-only --json`, pass its evidence to `telos change complete --flow <flow> --json`.

Never ask the user to run a lifecycle command, copy a path, or provide an artifact ID. Never seal without the matching conversational approval. Never adopt an undeclared write after the fact.

If implementation exposes a contract defect, run `telos change abort --flow <flow> --reason "..." --json`, then `telos artifact revise --id <intent-or-spec> --reason "..." --json`. Resume the returned phase and obtain both approvals again where invalidated.

Read [references/protocol.md](references/protocol.md) when constructing artifacts or handling a CLI failure.
