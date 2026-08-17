# Threat model

Telos makes undeclared changes visible and forces human decisions through
native permission prompts. It is not an operating-system privilege boundary.

## SEALED vs ATTESTED

The default seal is `SEALED`: HMAC-SHA256 with a secret embedded in the
binary, over the exact certificate payload bytes. It detects every ordinary
out-of-protocol edit and makes a valid seal producible only through the
kernel's verified-transition path (`Seal(VerifiedTransition)` — there is no
sign-arbitrary-state API). It does NOT resist an adversary who can run the
same binary against a rewritten history: such an adversary can re-run the
whole protocol. A future `ATTESTED` mode (signed commits/tags, external CI
attestation, remote signing) can raise that bar; the certificate format
already carries the `seal.mode` field for it.

## Protected against

- Direct writes in the certified worktree (guard denies Edit/Write/apply_patch
  and non-broker shell); protected paths inside candidates (contract, config,
  policies, change record, evidence, findings).
- Silent adoption of out-of-band edits: corruption is terminal for the state;
  the only exits are salvage (work moves) or restore (human-gated discard).
- Approvals binding to different bytes than reviewed: the digest is the
  folded contract tree OID, re-checked by the guard before prompting and by
  the kernel at approve/ready/promote.
- Tests written after the fact or weakened to fit: the red witness requires
  failure on a green baseline, seals exact blob OIDs, and green requires the
  same bytes; mutation evidence reports tests that cannot tell a mutant from
  the real program.
- Stale proofs after a base change: evidence is content-addressed; a changed
  closure invalidates it, an unknown closure invalidates conservatively.
- Concurrent promotions: the branch and its certificate move in one ref
  transaction with a CAS on the base.
- Project policy weakening kernel invariants: the kernel floor is embedded
  and unified with project CUE — weakening is a compile-time conflict.
- Certificate notes copied between commits: the payload names its commit.

## Not protected against

- A same-privilege malicious process rewriting source, notes, and history
  consistently through the Telos binary itself.
- A test that discriminates for the wrong reason: witnessing proves the test
  distinguishes the states, not that it means what the requirement says —
  that is the verifier's audit and the human's review.
- An incorrect but internally consistent approved product decision.
- Compromised dependencies, compilers, test runners, providers, or release
  infrastructure; a user who approves prompts without reading; a provider
  that does not honor PreToolUse decisions.
- The web view leaks nothing off-host (loopback-only, GET-only, Host-checked,
  CSP, no external assets) but any local process can read it, like any local
  file.

## Trust anchors

The Telos binary, Git history, the hook installation, the configured test
commands, and the human at the prompts. Hashes prove byte identity, not who
controlled the OS account.
