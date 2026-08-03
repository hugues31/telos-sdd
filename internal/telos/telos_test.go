package telos

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
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

func TestFeatureRenderingIsDeterministic(t *testing.T) {
	plan := TestPlan{Version: 1, Spec: "SPC-1", Feature: "Account lockout", Scenarios: []Scenario{{
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
		".agents/skills/telos-intent/SKILL.md",
		".claude/skills/telos-intent/SKILL.md",
		".codex/agents/telos-verifier.toml",
		".claude/agents/telos-verifier.md",
		".github/workflows/telos-verify.yml",
	} {
		if _, err := os.Stat(filepath.Join(root, filepath.FromSlash(path))); err != nil {
			t.Errorf("missing installed file %s: %v", path, err)
		}
	}
}

func TestLifecycleAndTamperInvalidation(t *testing.T) {
	root := t.TempDir()
	if err := initProject(root, "all", false); err != nil {
		t.Fatal(err)
	}
	intentID, _, err := newIntent(root, "Lock compromised accounts", "")
	if err != nil {
		t.Fatal(err)
	}
	intentPath, intentMeta, _, err := findArtifact(root, "intent", intentID)
	if err != nil {
		t.Fatal(err)
	}
	intentBody := `# Lock compromised accounts

## Outcome

Compromised accounts cannot authenticate within one second of a confirmed lock.

## Actors

Account owner and security operator.

## Scope

Lock authentication and emit an audit event.

## Non-goals

Credential recovery is excluded.

## Success criteria

Every authentication attempt after lock is denied and audited.

## Constraints

Existing sessions are outside this change.

## Open questions

None.
`
	if err := atomicWrite(intentPath, renderArtifact(intentMeta, intentBody), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := sealArtifact(root, "intent", intentID); err != nil {
		t.Fatal(err)
	}
	specID, _, err := newSpec(root, intentID, "Locked authentication")
	if err != nil {
		t.Fatal(err)
	}
	specPath, specMeta, _, err := findArtifact(root, "spec", specID)
	if err != nil {
		t.Fatal(err)
	}
	specBody := `# Locked authentication

## Context

An account has an authoritative locked state.

## Rules

### RULE-001 — Deny authentication

Every authentication attempt for a locked account is denied without creating a session.

## Examples

A correct password remains denied after lock.

## Boundaries

Repeated attempts remain denied and idempotent.

## Failure modes

An audit failure must not allow authentication.

## Observability

Each denial emits one account-lock audit event.
`
	if err := atomicWrite(specPath, renderArtifact(specMeta, specBody), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := sealArtifact(root, "spec", specID); err != nil {
		t.Fatal(err)
	}
	planPath := filepath.Join(root, ".telos", "test-plans", strings.ToLower(specID)+".json")
	plan := TestPlan{Version: 1, Spec: specID, Feature: slug(specID), Scenarios: []Scenario{{
		ID: "SCN-001", Rule: "RULE-001", Name: "Deny a correct password after lock", Tags: []string{"negative"},
		Given: []string{"an account is locked"}, When: []string{"the correct password is submitted"}, Then: []string{"authentication is denied", "no session is created"},
	}}}
	if err := writeJSON(planPath, plan); err != nil {
		t.Fatal(err)
	}
	featurePath, err := testify(root, specID, planPath)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(root, filepath.FromSlash(featurePath))); err != nil {
		t.Fatal(err)
	}
	changeID, err := beginChange(root, intentID, []string{specID})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := buildContext(root, changeID); err != nil {
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
	if status[relative(root, intentPath)] != "tampered" || status[relative(root, specPath)] != "stale" || status[featurePath] != "stale" {
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
