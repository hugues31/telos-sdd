---
name: telos
description: Certified-state development workflow. Use for every feature, bug fix, refactor, or repository modification in a Telos project. Run `telos status --json` first and route on its context, state, and change status.
---

# Telos — certified-state development

Every accepted state of this repository is certified: contract, code,
evidence, and policy verified together. Work happens in Change candidates;
the certified worktree is never a scratchpad. Full protocol:
[references/protocol.md](references/protocol.md).

## Routing

Always start with `telos status --json` and route on `result`:

- `context: "root"`, `state: "certified"` — engage `telos-challenger` on the
  request. A reported bug is evidence the contract was too weak: it starts as
  a contract delta, never as a direct code patch.
- `context: "root"`, `state: "corrupted"` with `dirty` — present the salvage
  proposal from `result.salvage.prompt` to the human. On yes run
  `telos salvage` (work moves into a candidate, nothing is lost); only on an
  explicit "discard" run `telos restore`. Never adopt an out-of-band edit
  silently.
- `context: "root"`, `state: "uninitialized"` — `telos init` (human decision).
- `context: "candidate"` — route on `result.change.status`:
  - `drafting` — `telos-challenger` drafts `changes/CHG-NNN/contract.delta.md`
    and `intent.md`; then `telos-consistency-critic` reviews the target
    contract and files findings; then `telos change review --json` and present
    the EXACT returned content; approval happens at the native prompt of
    `telos change approve --digest <digest>`.
  - `approved` — `telos-implementer` proves each requirement test-first
    (`telos evidence red` then `green`), then `telos-verifier` audits and
    files findings; triage findings with the human (`telos findings confirm`
    / `resolve` are human-gated); then `telos change ready` and
    `telos change promote`.
  - `base_stale: true` — `telos change rebase`, then follow `next_actions`
    (evidence with an unchanged closure survives; the approval is re-asked
    only if the contract context moved).

## The one presentation rule

Never surface certificate ids, digests beyond the approval prompt, evidence
hashes, or worktree internals to the human. Speak in intents, requirements,
changes, and findings. The human decides three things: is this exactly the
intended behavior (approve), is this claim acceptable (adopt/preserving),
and how does a finding resolve.

## Internal agents

| Agent | Does | Never |
| --- | --- | --- |
| `telos-challenger` | intent → minimal contract delta | approve, implement |
| `telos-consistency-critic` | findings on the target contract | resolve, edit |
| `telos-implementer` | witnessed red → green in the candidate | touch certified root, weaken tests |
| `telos-verifier` | read-only audit, findings | repair its own findings |

The kernel alone certifies. Declined prompts route back to challenge —
never re-run an unchanged command hoping for a different answer.
