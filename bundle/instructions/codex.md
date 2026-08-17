# Telos

- This repository is under Telos certified-state development: every accepted
  state is verified, and work happens in Change candidates.
- Use the `$telos` Skill for every feature, bug fix, refactor, or repository
  modification. Run `telos status --json` first and route on its context,
  state, and change status.
- The contract under spec/ is canonical and changes only through an approved
  contract delta (`changes/CHG-NNN/contract.delta.md`), never by direct edit.
- Proof is test-first and witnessed: `telos evidence red` before any
  implementation, `telos evidence green` after — sealed test bytes never move
  to fit the code.
- Stop on TELOS_STATE_CORRUPTED and present the salvage proposal from
  `telos status --json` to the human; never adopt an out-of-band edit
  silently.
