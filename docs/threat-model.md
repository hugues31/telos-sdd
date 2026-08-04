# Threat model

## Protected against

- ordinary direct Edit, Write, apply-patch and obvious shell mutation attempts observed by provider hooks;
- undeclared code changes, through code-root recomputation before and after every broker operation and in `telos verify`;
- silent spec changes, through spec-root recomputation: any pending diff must pass review and a native human approval prompt before implementation resumes;
- an approval binding to different bytes than were reviewed: the digest is recomputed at the gate by both the guard and the command, and a stale digest is denied outright;
- unjustified code, through mandatory file-level `telos:` annotations validated on every patch post-image against the cited rules;
- specification without proof, through per-rule tagged tests and real execution of the configured test commands;
- tautological or decorative tests, through the witnessed red-green cycle: a rule's test must be seen failing by the broker on a green baseline before any implementation, and the sealed test bytes may not change until the suite is witnessed green — a test the suite already passes is rejected unless the human accepts the explicit `--expect-pass` adoption claim at a permission prompt;
- a test rewritten to satisfy the implementation: sealed files change only through another witnessed-red test-only patch, and test references cannot be smuggled in outside a rule's own cycle;
- test commands that mutate tracked files, through post-run root recomputation;
- an agent re-baselining away a corruption: `telos init` inside an initialized project raises an `ask` permission prompt;
- the broker's own hooks being patched: `telos apply` refuses paths under the provider directories and managed instruction files.

## Not protected against

- a malicious process with the same OS privileges that bypasses hooks and rewrites source, state, and Git history consistently;
- a test that discriminates for the wrong reason: the witnessed red proves the test distinguishes the pre- and post-implementation trees, not that it asserts the rule's meaning — an over-specified or misdirected assertion passes the cycle and remains the independent verifier's and the PR reviewer's to catch;
- a flaky or time-dependent suite, which can produce false red and false green witnesses — the reliability of `test_commands` is a human responsibility in `telos.toml`;
- a compliance annotation on unrelated code that also fools the independent verifier and the human reviewing the spec diff — the file-level guarantee is mechanical, sub-file honesty is audited, not proven;
- an incorrect but internally consistent product decision approved by the user;
- compromised dependencies, compilers, test runners, agent providers or release infrastructure;
- a user who approves harness permission prompts without reading them, or a provider that does not honor `PreToolUse` permission decisions;
- behavior outside the exercised environment;
- formal semantic equivalence between prose, scenarios and code.

## Trust anchors

Reviewers trust the Telos binary, the Git history, provider hook installation, the configured test commands, and the independent-verifier process. Local hashes prove byte identity, not who controlled the operating-system account.

Hostile guarantees require a privilege-separated mutation service or external signer whose key is unavailable to coding agents. Signed CI attestations remain the intended next trust layer.
