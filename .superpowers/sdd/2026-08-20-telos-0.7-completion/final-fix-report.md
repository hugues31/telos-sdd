# Telos 0.7 final fix report

Date: 2026-08-21

Reviewed base: `c5de40976a17096d8a37c52e378f794fdd842354`

Implementation commits before the documentation/report checkpoint:

- `f53c90a8b6e174faa99f26c973712c00038a3636` — proof, path, init, runner,
  agent and export publication boundaries;
- `80b13831f3caf08f2d1ddf01a9c161d6192110e9` — solution-free Billing demo;
- `93256216b8684e6e6a426821b163d41e85c726ca` — decision tokens, global proof
  de-duplication and the five minor findings.

No finding required weakening the approved design. The five-key envelope stays
unchanged. The only public nested-schema addition is `status.result.drift.token`;
drift and diff `next_actions` now carry the exact corresponding token/digest.

## Critical findings

### 1. Repository path escape and outside-repo restore

- **RED:** `cargo test -p telos-core --test path_safety` exposed both failures:
  `git_hashing_rejects_an_in_repo_symlink_to_outside` accepted/hash-read an
  outside owner and `revert_never_changes_an_outside_owner_through_a_parent_symlink`
  could reach that owner. Unit/parser cases also accepted `..`, roots, Windows
  prefixes/backslashes, controls, and `telos/` journal paths.
- **Root cause:** `RepoPath::new` was used at untrusted boundaries and downstream
  I/O joined path strings to the repository pathname. Neither typed validation
  nor pathname joining was a containment capability.
- **Fix:** added fallible `RepoPath::parse`/`parse_outside_telos`, applied them to
  CLI, parser, binding, journal and lock boundaries, and repeated validation at
  Git/I/O boundaries. `RepoFs` anchors read/write/delete below an opened root and
  opens every parent/final entry no-follow. Revert uses that capability and Git
  hashing rejects symlink redirection.
- **GREEN:** `path_safety` 2/2; RepoPath/parser/binding unit cases; affected
  `test_bind`, `adopt_revert`, status/seal and Git suites; full workspace.
- **Concerns:** Unix symlink behavior ran dynamically. Windows code is covered by
  cfg-specific implementation and spelling tests, but no Windows sysroot is
  installed on this host (see platform verification).

### 2. Test verdict and seal not tied to executed bytes

- **RED:** `test_refuses_when_the_runner_rewrites_its_own_test` journalled the
  post-run proof OID; `ordinary_reconcile_refuses_code_bytes_changed_by_its_runner`
  sealed/closed after bound code changed; and
  `full_reconcile_refuses_spec_bytes_changed_by_its_runner` could publish a lock
  for spec bytes changed during the run.
- **Root cause:** proof OIDs were captured after runner execution, and reconcile
  constructed the lock from a fresh late hash rather than the pre-execution
  snapshot.
- **Fix:** `telos test` hashes before spawn and requires the same OID afterward
  before journalling. Ordinary/full reconcile capture complete spec and bound
  code/proof OID maps before checks/tests, compare path sets and OIDs after the
  run, build the code portion of the lock from the proven snapshot, and recheck
  spec/code at lock publication. A mismatch leaves journal/change/lock state
  untouched (apart from the external runner's own mutation).
- **GREEN:** the three mutation tests above, plus reconcile/test-bind suites and
  full workspace.
- **Concerns:** an edit after a successful publication is normal later drift;
  resistance to an actively malicious same-UID process is not claimed.

### 3. Fresh init adopts foreign core and bypasses seal gates

- **RED:** `fresh_init_refuses_every_foreign_telos_owner_without_a_marker`
  demonstrated foreign core files/directories being treated as init output;
  `fresh_init_refuses_an_active_unproved_prepopulation_without_sealing_it` and
  `fresh_init_refuses_live_and_dangling_telos_symlinks_without_touching_owner`
  exposed prepopulation/symlink adoption.
- **Root cause:** fresh admission checked only `telos.toml`, captured other
  existing core bytes into the transaction, and created the initial lock without
  the full config/sealability admission used by full reconcile.
- **Fix:** before marker creation, fresh init admits only an absent `telos/` or
  empty canonical subdirectories. Any other file, directory, live/dangling
  symlink or active tree is foreign. Resume requires an exact authenticated v2
  marker/options/phase/core/directory state. Initial lock generation validates
  config, loads the generated empty model and applies `require_sealable_structure`.
- **GREEN:** the fresh-owner table, empty-directory acceptance, active-unproved,
  live/dangling symlink, exact-resume and marker-boundary unit tests; agent/init-CI
  integration suites; full workspace.
- **Concerns:** a foreign partial tree without a valid marker requires explicit
  owner recovery/move; init intentionally will not guess that it owns it.

## Important findings

### 4. Shell filter re-injection

- **RED:** `arithmetic_and_quote_payloads_remain_data_in_a_real_process`,
  `nested_shell_and_eval_templates_fail_before_real_injection`,
  `nested_cmd_and_call_templates_fail_before_real_injection`, and
  `control_byte_filters_fail_closed_before_spawn` showed arithmetic,
  substitution, `eval`, `sh -c`, `cmd /C`, `call`, CR/LF/NUL and quote hazards.
- **Root cause:** quote-state rewriting still passed the resulting string to a
  shell, where nested/second interpretation defeated the first quoting layer.
- **Fix:** kept D10 literal substitution only for display. Execution now parses a
  restricted simple-word template and spawns one executable with an argv vector;
  `{filter}` is inserted as data (including embedded `module::{filter}`). Shell
  operators/substitutions, eval/call/env and nested interpreters fail closed.
- **GREEN:** core `exec` unit tests, real `globs_exec` injection tests, legitimate
  metacharacter/quote tests, reconcile embedded-filter tests and full workspace.
- **Concerns:** projects that need shell syntax in `[test] cmd` must move it to a
  dedicated script and configure that script as the direct executable. D10
  display remains diagnostic and is not promised to be shell-replayable.

### 5. Export state/model torn read

- **RED:** `commands::view::tests::export_refuses_a_normal_save_between_model_read_and_authentication`
  used a deterministic post-model-read hook; old export could publish the newly
  read model under the earlier coherent state.
- **Root cause:** state/open-change admission happened before model load with no
  authentication pass tying those reads together.
- **Fix:** export loads the model, then freshly scans changes and recomputes OID
  state before building the immutable `ViewSnapshot`. Only that authenticated
  model is rendered.
- **GREEN:** the barrier test, view-export suite and full workspace.
- **Concerns:** a later save may coexist with publication of the exact earlier
  sealed snapshot; it cannot change the already-owned projection bytes.

### 6. Export destination parent identity

- **RED:** `view::export::tests::a_rotated_destination_parent_is_refused_and_its_new_owner_is_preserved`
  renamed/replaced the parent immediately before publish; old export reported
  success while the advertised pathname had no destination.
- **Root cause:** export held a directory capability but did not prove that the
  announced parent pathname still resolved to that same directory.
- **Fix:** staging captures the no-follow identity of every component in the
  announced parent chain, reopens the chain at the final boundary, compares
  identities and refuses before promotion on rotation.
- **GREEN:** rotated-parent owner-preservation test plus the full staging,
  reservation, substitution, collision and export suites.
- **Concerns:** malicious same-UID inter-syscall substitution remains outside §5;
  ordinary rename/replacement is covered.

### 7. Agent merge marker corruption on resume

- **RED:** `agents::tests::malformed_owned_block_markers_are_rejected_before_any_agent_write`
  accepted orphaned, duplicate and reversed markers as an absent block;
  `commands::init::tests::retry_after_partial_agent_publication_refuses_corrupted_owned_markers`
  reproduced the partial-publication retry loss path.
- **Root cause:** `merge_owned_block` returned a string and treated every shape
  other than one apparent pair as “no owned block”.
- **Fix:** merge planning is fallible and admits exactly 0/0 or one ordered 1/1
  marker pair. Every consumer preflights it before publication, including exact
  init resume.
- **GREEN:** both tests above and all agent/init suites.
- **Concerns:** malformed owner files are preserved and require manual marker
  repair; Telos never guesses which user span to delete.

### 8. Existing agent owner save overwritten after validation

- **RED:** `agents::tests::existing_merge_target_save_after_validation_is_restored_by_publication_cas`
  placed an IDE save in the deterministic validation-to-publish hook; the old
  rename replaced it.
- **Root cause:** byte comparison and final pathname replacement were separate
  operations without compare-and-swap ownership.
- **Fix:** existing merges stage privately, atomically displace the current
  target to a CSPRNG backup, validate the displaced bytes/identity, and publish
  create-only. On mismatch the late owner wins and displaced bytes are restored
  or retained privately so neither version is destroyed.
- **GREEN:** late-save CAS test, preflight-change/no-op/create-only/staging
  substitution tests and all agent/init suites.
- **Concerns:** if another writer wins the tiny restoration slot, retained backup
  bytes are intentionally not deleted; the command fails rather than losing an
  owner. Adversarial same-UID substitution is excluded by §5.

### 9. Human decision not bound to displayed digest/scope

- **RED:** `change_approve_refuses_a_digest_that_changed_during_review`,
  `adopt_refuses_a_drift_token_whose_scope_changed_during_review`,
  `revert_refuses_a_drift_token_whose_scope_changed_during_review`, and
  `guard_denies_tokens_made_stale_while_the_native_prompt_is_open` all crossed a
  deterministic review/prompt mutation boundary; old commands had no expected
  value to reject.
- **Root cause:** the guard displayed repository context, but the authorized
  command re-read and mutated whichever digest/scope existed afterward.
- **Fix:** added optional public `--expected-digest` and `--expected-state`.
  Drift tokens hash the full sealed spec/code tables plus exact sorted path/kind
  scope. Commands re-read at the mutation boundary. Status/diff next actions and
  generated skills pass exact values; guard parsing allows only canonical direct
  token-bearing actions and independently recomputes the value.
- **GREEN:** change-flow, adopt/revert, status and agent-guard token suites;
  generated-skill byte assertions; contract suite; full workspace.
- **Concerns:** direct humans may deliberately omit the flags for compatibility;
  that route binds to its first observation and still rechecks. Agent automation
  fails closed without a token. `drift.token` is the explicit schema addition.

### 10. Billing demo embeds the solution

- **RED:** `public_billing_demo_contains_no_extractable_solution` initially found
  Cargo manifests, Rust source/tests and extractable heredoc solution fragments
  under the copied public demo.
- **Root cause:** the e2e used README-embedded solution bytes as its fixture and
  the README described that as a spec-only reconstruction.
- **Fix:** public `demo/billing` now contains exactly README plus nine Telos owners;
  all possible implementation/checker bytes live as private constants in
  `crates/telos/tests/rebuild_demo.rs`. README documents only an external
  implementer's bounded lifecycle and calls automation a protocol/conformance
  harness.
- **GREEN:** solution-free exact-file/forbidden-fragment test and two fresh real
  CLI reconstructions with identical observations and `0/2 → 1/2 → 2/2`.
- **Concerns:** the harness proves deterministic protocol conformity, not
  independent LLM creativity or source-byte identity.

### 11. Git batch hashing pipe deadlock

- **RED:** `blob_oids_drains_a_large_real_git_batch_without_pipe_deadlock`
  submits 4,096 real paths (>64 KiB stdout) and bounds completion at 10 seconds;
  the write-all-before-read implementation timed out with a full pipe.
- **Root cause:** parent wrote every `--stdin-paths` input before draining the
  child's stdout/stderr.
- **Fix:** a scoped writer thread feeds stdin while `wait_with_output` drains
  stdout/stderr; spawn, writer, wait, exit and row-count errors all propagate.
- **GREEN:** 4,096-path test completes and returns every OID; Git suite and full
  workspace pass.
- **Concerns:** none beyond the existing <10k target; the regression deliberately
  crosses normal pipe capacity.

### 12. Rebuild status global proof de-duplication

- **RED:** `one_proof_shared_by_two_scenarios_runs_once_and_projects_one_outcome`
  bound one whole-file proof to `SCN-0091` and `SCN-0107`; the invocation counter
  was two and could yield divergent rows.
- **Root cause:** proof execution/cache lifetime was inside each scenario loop.
- **Fix:** collect `TestRef -> first scenario` globally in a `BTreeMap`, execute
  each structural target once, then project the cloned result into every owning
  scenario.
- **GREEN:** counter exactly one, rows identical; `rebuild` 22/22 and full
  workspace.
- **Concerns:** execution order is global structural `TestRef` order, independent
  of scenario declaration order, as frozen by contract.

## Minor findings

### M1. Spec-only rebuild configuration validation

- **RED:** `spec_only_rebuild_validates_runtime_glob_syntax` showed both `plan`
  and `status` accepting `globs = ["["]` when no lock existed.
- **Fix:** spec-only load calls `config.validate_self()` before the model.
- **GREEN:** both subcommands return `TELOS_PARSE_ERROR`; rebuild suite green.
- **Concerns:** runner-template grammar is validated only by execution surfaces;
  `validate_self` owns runtime glob semantics.

### M2. Config write double discovery

- **RED:** source-level reproducer: write-mode `run()` called
  `Workspace::discover`, then `stage()` called `project()` and discovered/read
  the workspace again, creating two independently observable snapshots.
- **Fix:** branch to `stage()` before read-mode discovery; write mode now has only
  `project()`'s single discovery.
- **GREEN:** source audit plus config/change/full-workspace suites.
- **Concerns:** none.

### M3. README collision wording

- **RED:** published sentence said export “publishes no final destination”, which
  contradicted the case where a pre-existing non-Telos owner remains at that
  pathname.
- **Fix:** exact wording is “publishes no Telos-owned destination and preserves
  the existing owner.”
- **GREEN:** `published_contract_closes_the_final_review_boundaries` freezes the
  sentence.
- **Concerns:** none.

### M4. Predictable init/agent staging names

- **RED:** source-level reproducer found process-local counters/time-derived
  private names despite the documented CSPRNG claim.
- **Fix:** safe staging and backup names use 16 bytes from `getrandom` (128-bit),
  hex encoded, with create-new collision retries and identity-safe cleanup.
- **GREEN:** agent staging/CAS/substitution suites, export staging suite, Clippy
  and full workspace.
- **Concerns:** randomness prevents accidental collision; it is explicitly not
  an authorization secret against an observing same-UID adversary.

### M5. Watcher ignores nested `target`

- **RED:** `watcher_ignores_only_the_repository_root_target_directory` showed
  `examples/target/relevant.rs` being filtered along with root `target/`.
- **Fix:** ignored-name matching is restricted to the first path component.
- **GREEN:** root target stays ignored, nested target triggers reload; view-server
  and full-workspace suites pass.
- **Concerns:** none.

## Documentation and contract result

- Design §5 now states repository capability/no-follow containment, executed
  snapshot binding, CSPRNG/CAS publication guarantees and the exact same-UID
  threat boundary.
- `docs/contracts.md` distinguishes frozen D10 display from direct argv
  execution, documents exact token flags/errors/next actions, export
  authentication/parent identity, init eligibility/marker/CAS rules, global
  proof de-duplication and root-only watcher ignores.
- README no longer claims demo solution heredocs and uses the exact collision
  wording.
- `published_contract_closes_the_final_review_boundaries` was RED on the old
  public text and is GREEN with these contracts; the full contract suite is
  27/27.

## Final verification

Final fresh gate and smoke outputs are recorded after the documentation/report
checkpoint and in the final handoff. Cross-platform cfg/API verification is
reported separately because this host has only the Arch Linux
`x86_64-unknown-linux-gnu` standard library; `rustup` is not installed.

## Scoped re-review follow-up

The independent scoped re-review passed 13/17 items and identified four
follow-ups. They were resolved locally without another delegated fix wave:

- init's contract now states the implemented authenticated canonical directory
  **shape** guarantee rather than claiming directory inode identities; the
  original foreign-owner and seal-gate finding remains covered;
- environment/assignment-wrapped human actions were verified to already fail
  closed through the direct-mutation classifier, and now have an explicit
  two-host regression test;
- drift tokens now include the live blob OID of every present drift entry, so
  changed bytes under the same `(path, kind)` scope invalidate adopt/revert;
- sealed rebuild validates drift before runtime globs, and watcher root ignores
  for `.git`, `target`, and `.superpowers` no longer suppress same-named nested
  project paths.

Focused GREEN after these corrections: core state 17/17, agent init 41/41,
rebuild 23/23, adopt/revert 22/22, Telos binary units 68/68, and contracts
27/27 outside the loopback-restricted sandbox.
