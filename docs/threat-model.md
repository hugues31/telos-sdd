# Threat model

## Protected against

- accidental edits to sealed intent, spec, test-plan, and generated feature paths through observed provider tools;
- silent artifact drift detected by normalized SHA-256 verification;
- downstream use of an invalid parent through transitive stale status;
- happy-path-only and tautological testing through a separate test-architect workflow and required traceability IDs;
- speculative implementation through an explicit change scope and generated context;
- provider lock-in through embedded, equivalent Codex and Claude Code adapters.

## Not protected against

- a malicious or privileged process that bypasses hooks and also rewrites the lock and Git history;
- an incorrect but internally consistent product decision;
- incomplete step definitions or external systems that behave differently from the test environment;
- compromised compilers, dependencies, CI runners, agent providers, or release infrastructure;
- semantic equivalence: SHA-256 proves byte-normalized identity, not behavioral correctness;
- direct source-code edits in the standard profile.

## Trust anchors

Reviewers must trust the Telos binary they execute, the Git history that contains sealed artifacts, and the configured verification commands. Release checksums protect download integrity only when fetched over an uncompromised channel and compared with the intended release.

