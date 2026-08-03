# Threat model

## Protected against

- ordinary direct Edit, Write, apply-patch and obvious shell mutation attempts observed by provider hooks;
- undeclared tracked or non-ignored file changes through repository-root recomputation;
- direct draft edits through flow-owned draft hashes;
- sealed artifact drift and stale descendants;
- rewritten mutation JSON when it differs from stored patch bytes or append-only ledger evidence;
- happy-path-only coverage through a mandatory per-rule category matrix;
- speculative code through RULE/SCN references on every patch;
- partial contract sealing through staged validation and rollback;
- contract defects being hidden by implementation changes, through abort and immutable revision flows.

## Not protected against

- a malicious process with the same OS privileges that bypasses hooks and rewrites source, locks, patches, ledger and Git history consistently;
- a false RULE/SCN label on unrelated code that also fools the independent verifier;
- an incorrect but internally consistent product decision approved by the user;
- compromised dependencies, compilers, test runners, agent providers or release infrastructure;
- behavior outside the exercised environment;
- formal semantic equivalence between prose, scenarios and code.

## Trust anchors

Reviewers trust the Telos binary, the Git history, provider hook installation, configured verification commands and the independent-verifier process. Local hashes prove byte identity and declaration history, not who controlled the operating-system account.

Hostile guarantees require a privilege-separated mutation service or external signer whose key is unavailable to coding agents. Signed CI attestations remain the intended next trust layer.
