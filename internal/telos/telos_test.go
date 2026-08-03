package telos

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestNormalizeAndRootHashAreDeterministic(t *testing.T) {
	if got := string(normalize([]byte("a\r\nb\rc\n"))); got != "a\nb\nc\n" {
		t.Fatalf("normalize = %q", got)
	}
	a := []LockedFile{{Path: "z/file", Hash: "2"}, {Path: "a/file", Hash: "1"}}
	b := []LockedFile{{Path: "a/file", Hash: "1"}, {Path: "z/file", Hash: "2"}}
	if rootHash(a) != rootHash(b) {
		t.Fatal("root hash depends on input ordering")
	}
}

func TestAtomicWriteReplacesReadOnlyFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "sealed.txt")
	if err := os.WriteFile(path, []byte("before\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(path, 0o444); err != nil {
		t.Fatal(err)
	}
	if err := atomicWrite(path, []byte("after\n"), 0o444); err != nil {
		t.Fatalf("replace read-only file: %v", err)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "after\n" {
		t.Fatalf("atomic write content = %q, want %q", data, []byte("after\n"))
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm()&0o222 != 0 {
		t.Fatalf("atomic write mode = %o, want read-only", info.Mode().Perm())
	}
}

func TestRunInitRequiresGitWorktree(t *testing.T) {
	root := t.TempDir()
	var stdout, stderr bytes.Buffer
	err := runInit(root, []string{"--agent", "codex"}, &stdout, &stderr)
	var commandErr *commandError
	if !errors.As(err, &commandErr) {
		t.Fatalf("init error = %v, want structured Git repository error", err)
	}
	if commandErr.Code != "TELOS_GIT_REPOSITORY_REQUIRED" {
		t.Fatalf("init error code = %q, want TELOS_GIT_REPOSITORY_REQUIRED", commandErr.Code)
	}
	want := "not a Git repository; run `git init` before `telos init`"
	if commandErr.Message != want {
		t.Fatalf("init error message = %q, want %q", commandErr.Message, want)
	}
	if _, err := os.Stat(filepath.Join(root, ".telos")); !os.IsNotExist(err) {
		t.Fatalf("init created Telos state outside a Git worktree: %v", err)
	}
}

func TestRunInitAcceptsGitWorktree(t *testing.T) {
	root := t.TempDir()
	cmd := exec.Command("git", "-C", root, "init", "--quiet")
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git init: %v\n%s", err, out)
	}
	var stdout, stderr bytes.Buffer
	if err := runInit(root, []string{"--agent", "codex"}, &stdout, &stderr); err != nil {
		t.Fatalf("telos init in Git worktree: %v", err)
	}
	if _, err := os.Stat(filepath.Join(root, ".telos", "config.toml")); err != nil {
		t.Fatalf("Telos config not created: %v", err)
	}
}

func TestFeatureRenderingIsDeterministic(t *testing.T) {
	plan := TestPlan{Spec: "SPC-1", Feature: "Account lockout", Scenarios: []Scenario{{
		ID: "SCN-002", Rule: "RULE-003", Name: "Reject a locked account", Tags: []string{"negative", "authorization"},
		Given: []string{"an account is locked"}, When: []string{"the owner authenticates"}, Then: []string{"access is denied", "an audit event is emitted"},
	}}}
	first := renderFeature(plan)
	second := renderFeature(plan)
	if first != second {
		t.Fatal("feature rendering changed between calls")
	}
	if !strings.Contains(first, "@rule_003") || !strings.Contains(first, "Scenario: Reject a locked account") {
		t.Fatalf("missing traceability in feature:\n%s", first)
	}
}

func TestRandomBrainstormSelectionIsReproducible(t *testing.T) {
	first := selectBrainstormEngine("random", 8675309)
	second := selectBrainstormEngine("random", 8675309)
	if first != second {
		t.Fatalf("same seed selected %q then %q", first, second)
	}
}

func TestInitPreservesUserFilesAndRefreshesIdempotently(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "AGENTS.md"), []byte("# Existing\n\nKeep me.\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(root, ".claude"), 0o755); err != nil {
		t.Fatal(err)
	}
	settings := []byte("{\n  \"permissions\": {\"deny\": [\"Read(./secret)\"]}\n}\n")
	if err := os.WriteFile(filepath.Join(root, ".claude", "settings.json"), settings, 0o644); err != nil {
		t.Fatal(err)
	}
	if err := initProject(root, "all", true); err != nil {
		t.Fatal(err)
	}
	if err := initProject(root, "all", true); err != nil {
		t.Fatal(err)
	}
	agents, err := os.ReadFile(filepath.Join(root, "AGENTS.md"))
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(agents, []byte("Keep me.")) || bytes.Count(agents, []byte(managedStart)) != 1 {
		t.Fatalf("AGENTS.md was not merged idempotently:\n%s", agents)
	}
	claude, err := os.ReadFile(filepath.Join(root, ".claude", "settings.json"))
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(claude, []byte("Read(./secret)")) || bytes.Count(claude, []byte("telos guard")) != 1 {
		t.Fatalf("Claude settings were not merged idempotently:\n%s", claude)
	}
	for _, path := range []string{
		".agents/skills/telos/SKILL.md",
		".claude/skills/telos/SKILL.md",
		".codex/agents/telos-product.toml",
		".codex/agents/telos-spec-architect.toml",
		".codex/agents/telos-test-architect.toml",
		".codex/agents/telos-implementer.toml",
		".codex/agents/telos-verifier.toml",
		".claude/agents/telos-product.md",
		".claude/agents/telos-verifier.md",
		".github/workflows/telos-verify.yml",
	} {
		if _, err := os.Stat(filepath.Join(root, filepath.FromSlash(path))); err != nil {
			t.Errorf("missing installed file %s: %v", path, err)
		}
	}
}

const fixtureIntentBody = `# Lock compromised accounts

## Outcome

Compromised accounts cannot authenticate within one second of a confirmed lock.

## Actors

Account owner and security operator.

## Scope

Lock authentication and emit an audit event.

## Non-goals

Credential recovery is excluded.

## Success criteria

### CRIT-001 — Locked authentication

Every authentication attempt after lock is denied and audited.

## Constraints

Existing sessions are outside this change.

## Open questions

None.
`

const fixtureSpecBody = `# Locked authentication

## Context

An account has an authoritative locked state.

## Rules

### RULE-001 — Deny authentication

Traces: CRIT-001

Every authentication attempt for a locked account is denied without creating a session.

## Examples

A correct password remains denied after lock.

## Boundaries

Repeated attempts remain denied and idempotent.

## Non-effects

Existing sessions remain unchanged.

## Failure modes

An audit failure must not allow authentication.

## Observability

Each denial emits one account-lock audit event.
`

func fixturePlan(specID string) TestPlan {
	plan := TestPlan{Spec: specID, Feature: slug(specID), Scenarios: []Scenario{{
		ID: "SCN-001", Rule: "RULE-001", Name: "Deny a correct password after lock", Tags: append([]string(nil), coverageCategories...),
		Given: []string{"an account is locked"}, When: []string{"the correct password is submitted"}, Then: []string{"authentication is denied", "no session is created"},
	}}}
	for _, category := range coverageCategories {
		plan.Coverage = append(plan.Coverage, Coverage{Rule: "RULE-001", Category: category, Status: "covered"})
	}
	return plan
}

func TestFlowContractAndTamperInvalidation(t *testing.T) {
	root := t.TempDir()
	if err := initProject(root, "all", false); err != nil {
		t.Fatal(err)
	}
	flow, err := startFlow(root, "Lock compromised accounts", "none")
	if err != nil {
		t.Fatal(err)
	}
	intentPath, _, _, err := findArtifact(root, "intent", flow.Intent)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := putArtifact(root, flow.Intent, fixtureIntentBody); err != nil {
		t.Fatal(err)
	}
	flow, digest, _, err := reviewIntent(root, flow.ID)
	if err != nil {
		t.Fatal(err)
	}
	flow, err = sealReviewedIntent(root, flow.ID, digest)
	if err != nil {
		t.Fatal(err)
	}
	flow, _, err = attachSpec(root, flow.ID, "Locked authentication")
	if err != nil {
		t.Fatal(err)
	}
	specID := flow.Specs[0]
	specPath, _, _, err := findArtifact(root, "spec", specID)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := putArtifact(root, specID, fixtureSpecBody); err != nil {
		t.Fatal(err)
	}
	planData, _ := json.Marshal(fixturePlan(specID))
	if _, err := putTestPlan(root, specID, planData); err != nil {
		t.Fatal(err)
	}
	flow, contractDigest, _, err := reviewContract(root, flow.ID)
	if err != nil {
		t.Fatal(err)
	}
	flow, err = sealReviewedContract(root, flow.ID, contractDigest)
	if err != nil {
		t.Fatal(err)
	}
	featurePath := "features/" + slug(specID) + ".feature"
	if _, err := os.Stat(filepath.Join(root, filepath.FromSlash(featurePath))); err != nil {
		t.Fatal(err)
	}
	flow, change, err := beginFlowChange(root, flow.ID)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := buildContext(root, change.ID); err != nil {
		t.Fatal(err)
	}
	if err := requireCleanAudit(root); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(intentPath, 0o644); err != nil {
		t.Fatal(err)
	}
	if err := atomicWrite(intentPath, []byte("tampered\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	results, err := audit(root)
	if err != nil {
		t.Fatal(err)
	}
	status := map[string]string{}
	for _, result := range results {
		status[result.Path] = result.Status
	}
	if status[relative(root, intentPath)] != "tampered" || status[relative(root, specPath)] != "stale" || status[featurePath] != "stale" || flow.Phase != "implementing" {
		t.Fatalf("unexpected invalidation statuses: %#v", status)
	}
}

func TestGuardDeniesLockedArtifactWrite(t *testing.T) {
	root := t.TempDir()
	if err := initProject(root, "codex", false); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(root, ".telos", "specs", "sealed.md")
	if err := os.WriteFile(path, []byte("sealed\n"), 0o444); err != nil {
		t.Fatal(err)
	}
	h, _ := fileHash(path)
	if _, err := lockFile(root, LockedFile{ID: "SPC-1", Kind: "spec", Path: relative(root, path), Hash: h}); err != nil {
		t.Fatal(err)
	}
	input := map[string]any{"cwd": root, "tool_input": map[string]any{"command": "apply patch to .telos/specs/sealed.md"}}
	b, _ := json.Marshal(input)
	var out bytes.Buffer
	if err := runGuard(bytes.NewReader(b), &out); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out.String(), `"permissionDecision":"deny"`) {
		t.Fatalf("guard did not deny write: %s", out.String())
	}
}

func TestGuardRequiresCLIBrokerForDirectWrites(t *testing.T) {
	root := t.TempDir()
	if err := initProject(root, "codex", false); err != nil {
		t.Fatal(err)
	}
	input := map[string]any{"cwd": root, "tool_name": "Write", "tool_input": map[string]any{"file_path": filepath.Join(root, "source.go"), "content": "package source"}}
	b, _ := json.Marshal(input)
	var out bytes.Buffer
	if err := runGuard(bytes.NewReader(b), &out); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out.String(), `"permissionDecision":"deny"`) {
		t.Fatalf("guard accepted a direct source write: %s", out.String())
	}
	allowed := map[string]any{"cwd": root, "tool_name": "Bash", "tool_input": map[string]any{"command": "telos change apply --flow FLW-X"}}
	b, _ = json.Marshal(allowed)
	out.Reset()
	if err := runGuard(bytes.NewReader(b), &out); err != nil {
		t.Fatal(err)
	}
	if out.Len() != 0 {
		t.Fatalf("guard denied the CLI broker: %s", out.String())
	}
	for _, command := range []string{
		"printf tampered > source.go",
		"telos inspect --json; rm source.go",
		"/usr/local/bin/telos inspect --json && cp other source.go",
	} {
		denied := map[string]any{"cwd": root, "tool_name": "Bash", "tool_input": map[string]any{"command": command}}
		b, _ = json.Marshal(denied)
		out.Reset()
		if err := runGuard(bytes.NewReader(b), &out); err != nil {
			t.Fatal(err)
		}
		if !strings.Contains(out.String(), `"permissionDecision":"deny"`) {
			t.Fatalf("guard accepted non-broker shell command %q: %s", command, out.String())
		}
	}
	heredoc := map[string]any{"cwd": root, "tool_name": "Bash", "tool_input": map[string]any{"command": "telos artifact put --id INT-X --json <<'TELOS_BODY'\n# Intent\nTELOS_BODY"}}
	b, _ = json.Marshal(heredoc)
	out.Reset()
	if err := runGuard(bytes.NewReader(b), &out); err != nil {
		t.Fatal(err)
	}
	if out.Len() != 0 {
		t.Fatalf("guard denied CLI stdin streaming: %s", out.String())
	}
}

func TestGuardForcesHumanGateOnSealCompleteRestore(t *testing.T) {
	root := t.TempDir()
	if err := initProject(root, "codex", false); err != nil {
		t.Fatal(err)
	}
	guard := func(command string) string {
		input := map[string]any{"cwd": root, "tool_name": "Bash", "tool_input": map[string]any{"command": command}}
		b, _ := json.Marshal(input)
		var out bytes.Buffer
		if err := runGuard(bytes.NewReader(b), &out); err != nil {
			t.Fatalf("guard failed on %q: %v", command, err)
		}
		return out.String()
	}
	flow, err := startFlow(root, "Lock compromised accounts", "none")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := putArtifact(root, flow.Intent, fixtureIntentBody); err != nil {
		t.Fatal(err)
	}
	if got := guard("telos intent seal --flow " + flow.ID + " --review deadbeef --json"); !strings.Contains(got, `"permissionDecision":"deny"`) {
		t.Fatalf("unreviewed intent seal was not denied: %s", got)
	}
	flow, digest, _, err := reviewIntent(root, flow.ID)
	if err != nil {
		t.Fatal(err)
	}
	for _, command := range []string{
		"telos intent seal --flow " + flow.ID + " --review " + digest + " --json",
		"telos intent seal --flow=" + flow.ID + " --review=" + digest + " --json",
	} {
		got := guard(command)
		if !strings.Contains(got, `"permissionDecision":"ask"`) || !strings.Contains(got, flow.Intent) {
			t.Fatalf("reviewed intent seal was not gated with ask (%q): %s", command, got)
		}
	}
	if got := guard("telos intent seal --flow " + flow.ID + " --review deadbeef --json"); !strings.Contains(got, `"permissionDecision":"deny"`) {
		t.Fatalf("stale intent seal digest was not denied: %s", got)
	}
	if got := guard("telos change complete --flow " + flow.ID + " --json"); !strings.Contains(got, `"permissionDecision":"deny"`) {
		t.Fatalf("unresolvable change completion was not denied: %s", got)
	}
	if flow, err = sealReviewedIntent(root, flow.ID, digest); err != nil {
		t.Fatal(err)
	}
	if flow, _, err = attachSpec(root, flow.ID, "Locked authentication"); err != nil {
		t.Fatal(err)
	}
	specID := flow.Specs[0]
	if _, err := putArtifact(root, specID, fixtureSpecBody); err != nil {
		t.Fatal(err)
	}
	planData, _ := json.Marshal(fixturePlan(specID))
	if _, err := putTestPlan(root, specID, planData); err != nil {
		t.Fatal(err)
	}
	if got := guard("telos contract seal --flow " + flow.ID + " --json"); !strings.Contains(got, `"permissionDecision":"deny"`) {
		t.Fatalf("unreviewed contract seal was not denied: %s", got)
	}
	flow, contractDigest, _, err := reviewContract(root, flow.ID)
	if err != nil {
		t.Fatal(err)
	}
	if got := guard("telos contract seal --flow " + flow.ID + " --review " + contractDigest + " --json"); !strings.Contains(got, `"permissionDecision":"ask"`) || !strings.Contains(got, specID) {
		t.Fatalf("reviewed contract seal was not gated with ask: %s", got)
	}
	if flow, err = sealReviewedContract(root, flow.ID, contractDigest); err != nil {
		t.Fatal(err)
	}
	flow, change, err := beginFlowChange(root, flow.ID)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := buildContext(root, change.ID); err != nil {
		t.Fatal(err)
	}
	if got := guard("telos change complete --flow " + flow.ID + " --json"); !strings.Contains(got, `"permissionDecision":"ask"`) || !strings.Contains(got, change.ID) {
		t.Fatalf("change completion was not gated with ask: %s", got)
	}
	if got := guard("telos repair --restore --json"); !strings.Contains(got, `"permissionDecision":"ask"`) {
		t.Fatalf("repair --restore was not gated with ask: %s", got)
	}
	if got := guard("telos repair --json"); got != "" {
		t.Fatalf("read-only repair should pass silently: %s", got)
	}
	if got := guard("telos inspect --json"); got != "" {
		t.Fatalf("inspect should pass silently: %s", got)
	}
}

func TestLedgerDetectsRewrittenArtifactAndLock(t *testing.T) {
	root := t.TempDir()
	if err := initProject(root, "codex", false); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(root, ".telos", "specs", "sealed.md")
	if err := os.WriteFile(path, []byte("original\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	h, _ := fileHash(path)
	lock, err := lockFile(root, LockedFile{ID: "SPC-1", Kind: "spec", Path: relative(root, path), Hash: h})
	if err != nil {
		t.Fatal(err)
	}
	if err := appendEvent(root, "spec.sealed", "SPC-1", nil, lock.RootHash); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("rewritten\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	newHash, _ := fileHash(path)
	lock.Artifacts[0].Hash = newHash
	if err := saveLock(root, lock); err != nil {
		t.Fatal(err)
	}
	results, err := audit(root)
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, result := range results {
		if result.Path == ".telos/lock.json" && result.Status == "tampered" {
			found = true
		}
	}
	if !found {
		t.Fatalf("rewritten lock was not detected: %#v", results)
	}
}

func TestIntentReviewDigestBecomesStaleAfterCLIMutation(t *testing.T) {
	root := t.TempDir()
	if err := initProject(root, "codex", false); err != nil {
		t.Fatal(err)
	}
	flow, err := startFlow(root, "Define locked authentication", "none")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := putArtifact(root, flow.Intent, validReviewIntent); err != nil {
		t.Fatal(err)
	}
	flow, digest, _, err := reviewIntent(root, flow.ID)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := putArtifact(root, flow.Intent, strings.Replace(validReviewIntent, "cannot authenticate", "never authenticates", 1)); err != nil {
		t.Fatal(err)
	}
	_, err = sealReviewedIntent(root, flow.ID, digest)
	var commandErr *commandError
	if !errors.As(err, &commandErr) || commandErr.Code != "TELOS_APPROVAL_STALE" {
		t.Fatalf("expected stale approval, got %v", err)
	}
}

func TestDirectDraftEditIsDetectedAndRestored(t *testing.T) {
	root := t.TempDir()
	if err := initProject(root, "codex", false); err != nil {
		t.Fatal(err)
	}
	flow, err := startFlow(root, "Define locked authentication", "none")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := putArtifact(root, flow.Intent, validReviewIntent); err != nil {
		t.Fatal(err)
	}
	path, _, _, _ := findArtifact(root, "intent", flow.Intent)
	if err := os.WriteFile(path, []byte("tampered\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	err = auditFlowDrafts(root, flow)
	var commandErr *commandError
	if !errors.As(err, &commandErr) || commandErr.Code != "TELOS_INTEGRITY_UNDECLARED_CHANGE" {
		t.Fatalf("expected draft integrity error, got %v", err)
	}
	if _, err := repairManagedArtifacts(root); err != nil {
		t.Fatal(err)
	}
	flow, err = loadFlow(root, flow.ID)
	if err != nil {
		t.Fatal(err)
	}
	if err := auditFlowDrafts(root, flow); err != nil {
		t.Fatalf("restored draft is still invalid: %v", err)
	}
}

func TestSealedIntentRevisionCreatesImmutableSuccessor(t *testing.T) {
	root := t.TempDir()
	if err := initProject(root, "codex", false); err != nil {
		t.Fatal(err)
	}
	flow, err := startFlow(root, "Define locked authentication", "none")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := putArtifact(root, flow.Intent, validReviewIntent); err != nil {
		t.Fatal(err)
	}
	flow, digest, _, err := reviewIntent(root, flow.ID)
	if err != nil {
		t.Fatal(err)
	}
	flow, err = sealReviewedIntent(root, flow.ID, digest)
	if err != nil {
		t.Fatal(err)
	}
	originalID := flow.Intent
	flow, successorID, _, err := reviseArtifact(root, originalID, "Clarify the approved outcome")
	if err != nil {
		t.Fatal(err)
	}
	_, original, _, _ := findArtifact(root, "intent", originalID)
	_, successor, _, _ := findArtifact(root, "intent", successorID)
	if original.Status != "sealed" || successor.Status != "draft" || successor.Supersedes != originalID || flow.Intent != successorID {
		t.Fatalf("unexpected revision state: original=%#v successor=%#v flow=%#v", original, successor, flow)
	}
}

func TestMutationJournalTamperingIsDetected(t *testing.T) {
	root := t.TempDir()
	if err := initProject(root, "codex", false); err != nil {
		t.Fatal(err)
	}
	repository, err := loadRepositoryLock(root)
	if err != nil {
		t.Fatal(err)
	}
	patch := []byte("immutable patch evidence\n")
	patchPath := filepath.Join(root, ".telos", "patches", "mut-test.patch")
	if err := atomicWrite(patchPath, patch, 0o444); err != nil {
		t.Fatal(err)
	}
	mutation := Mutation{
		ID: "MUT-TEST", Change: "CHG-TEST", PatchHash: patchHash(patch),
		PatchPath: relative(root, patchPath), BeforeRoot: repository.RootHash, AfterRoot: repository.RootHash,
	}
	mutationPath := filepath.Join(root, ".telos", "mutations", "mut-test.json")
	if err := writeJSON(mutationPath, mutation); err != nil {
		t.Fatal(err)
	}
	change := Change{ID: "CHG-TEST", Status: "complete", SourceBaseRoot: repository.RootHash, SourceCurrentRoot: repository.RootHash, Transactions: []string{mutation.ID}}
	if err := appendEvent(root, "change.patch-applied", change.ID, map[string]any{"mutation": mutation.ID, "patch_hash": mutation.PatchHash, "repository_root": repository.RootHash}, ""); err != nil {
		t.Fatal(err)
	}
	if err := auditChangeTransactions(root, change); err != nil {
		t.Fatal(err)
	}
	mutation.PatchHash = "rewritten"
	if err := writeJSON(mutationPath, mutation); err != nil {
		t.Fatal(err)
	}
	err = auditChangeTransactions(root, change)
	var commandErr *commandError
	if !errors.As(err, &commandErr) || commandErr.Code != "TELOS_INTEGRITY_JOURNAL" {
		t.Fatalf("expected journal tamper error, got %v", err)
	}
}

func TestCoverageMatrixCannotOmitACategory(t *testing.T) {
	plan := TestPlan{Spec: "SPC-TEST", Feature: "coverage", Scenarios: []Scenario{{
		ID: "SCN-001", Rule: "RULE-001", Name: "Observable behavior", Tags: []string{"positive"},
		Given: []string{"a valid state"}, When: []string{"the action occurs"}, Then: []string{"the outcome is visible"},
	}}}
	for _, category := range coverageCategories[:len(coverageCategories)-1] {
		status := "not_applicable"
		rationale := "The rule has no behavior in this category."
		if category == "positive" {
			status, rationale = "covered", ""
		}
		plan.Coverage = append(plan.Coverage, Coverage{Rule: "RULE-001", Category: category, Status: status, Rationale: rationale})
	}
	err := validatePlan(plan, map[string][]string{"RULE-001": {"CRIT-001"}}, map[string]bool{})
	var commandErr *commandError
	if !errors.As(err, &commandErr) || commandErr.Code != "TELOS_CONTRACT_INVALID" {
		t.Fatalf("expected missing coverage rejection, got %v", err)
	}
}

func TestVerificationCommandChangingSourceCorruptsProject(t *testing.T) {
	root := t.TempDir()
	sourcePath := filepath.Join(root, "source.txt")
	if err := os.WriteFile(sourcePath, []byte("clean\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := initProject(root, "codex", false); err != nil {
		t.Fatal(err)
	}
	cfg, err := readConfig(root)
	if err != nil {
		t.Fatal(err)
	}
	if runtime.GOOS == "windows" {
		cfg.VerificationCommands = []string{"echo tampered>source.txt"}
	} else {
		cfg.VerificationCommands = []string{"printf tampered > source.txt"}
	}
	if err := atomicWrite(filepath.Join(root, ".telos", "config.toml"), []byte(configText(cfg)), 0o644); err != nil {
		t.Fatal(err)
	}

	_, err = verifyProject(root, &bytes.Buffer{}, &bytes.Buffer{}, true)
	var commandErr *commandError
	if !errors.As(err, &commandErr) || commandErr.Code != "TELOS_INTEGRITY_UNDECLARED_CHANGE" {
		t.Fatalf("expected verification-side source mutation to corrupt the project, got %v", err)
	}
}

func TestContractSealRollsBackEveryWriteOnFailure(t *testing.T) {
	root := t.TempDir()
	flow, digest, specID := prepareReviewedContract(t, root)
	specPath, _, _, err := findArtifact(root, "spec", specID)
	if err != nil {
		t.Fatal(err)
	}
	planPath := filepath.Join(root, ".telos", "test-plans", strings.ToLower(specID)+".json")
	specBefore, err := os.ReadFile(specPath)
	if err != nil {
		t.Fatal(err)
	}
	planBefore, err := os.ReadFile(planPath)
	if err != nil {
		t.Fatal(err)
	}
	featureTarget := filepath.Join(root, "features", slug(specID)+".feature")
	if err := os.Mkdir(featureTarget, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := requireRepositoryClean(root); err != nil {
		t.Fatalf("empty directories must not alter the repository inventory: %v", err)
	}

	if _, err := sealReviewedContract(root, flow.ID, digest); err == nil {
		t.Fatal("expected contract seal to fail on an unusable feature target")
	}
	specAfter, err := os.ReadFile(specPath)
	if err != nil {
		t.Fatal(err)
	}
	planAfter, err := os.ReadFile(planPath)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(specBefore, specAfter) || !bytes.Equal(planBefore, planAfter) {
		t.Fatal("failed contract seal left partially sealed artifacts")
	}
	_, meta, _, err := findArtifact(root, "spec", specID)
	if err != nil {
		t.Fatal(err)
	}
	if meta.Status != "draft" {
		t.Fatalf("spec status after rollback = %q, want draft", meta.Status)
	}
}

func prepareReviewedContract(t *testing.T, root string) (Flow, string, string) {
	t.Helper()
	if err := initProject(root, "codex", false); err != nil {
		t.Fatal(err)
	}
	flow, err := startFlow(root, "Define locked authentication", "none")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := putArtifact(root, flow.Intent, validReviewIntent); err != nil {
		t.Fatal(err)
	}
	flow, intentDigest, _, err := reviewIntent(root, flow.ID)
	if err != nil {
		t.Fatal(err)
	}
	flow, err = sealReviewedIntent(root, flow.ID, intentDigest)
	if err != nil {
		t.Fatal(err)
	}
	flow, _, err = attachSpec(root, flow.ID, "Locked authentication")
	if err != nil {
		t.Fatal(err)
	}
	specID := flow.Specs[0]
	if _, err := putArtifact(root, specID, validReviewSpec); err != nil {
		t.Fatal(err)
	}
	plan := TestPlan{Spec: specID, Feature: slug(specID), Scenarios: []Scenario{{
		ID: "SCN-001", Rule: "RULE-001", Name: "Deny authentication after lock", Tags: append([]string(nil), coverageCategories...),
		Given: []string{"an account is locked"}, When: []string{"authentication is attempted"}, Then: []string{"access is denied"},
	}}}
	for _, category := range coverageCategories {
		plan.Coverage = append(plan.Coverage, Coverage{Rule: "RULE-001", Category: category, Status: "covered"})
	}
	data, err := json.Marshal(plan)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := putTestPlan(root, specID, data); err != nil {
		t.Fatal(err)
	}
	flow, digest, _, err := reviewContract(root, flow.ID)
	if err != nil {
		t.Fatal(err)
	}
	return flow, digest, specID
}

const validReviewIntent = `# Locked authentication

## Outcome

A locked account cannot authenticate.

## Actors

Account owner and security operator.

## Scope

Authentication after lock.

## Non-goals

Recovery is excluded.

## Success criteria

### CRIT-001 — Access denied

Every attempt is denied without a session.

## Constraints

Existing sessions are unchanged.

## Open questions

None.
`

const validReviewSpec = `# Locked authentication

## Context

An account has an authoritative locked state.

## Rules

### RULE-001 — Deny authentication

Traces: CRIT-001

Every authentication attempt for a locked account is denied.

## Examples

A correct password remains denied after lock.

## Boundaries

Repeated attempts remain denied.

## Non-effects

Existing sessions remain unchanged.

## Failure modes

An audit failure must not allow authentication.

## Observability

Each denial is observable.
`
