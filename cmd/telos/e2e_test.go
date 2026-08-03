package main

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestCLIEndToEnd(t *testing.T) {
	binName := "telos"
	if runtime.GOOS == "windows" {
		binName += ".exe"
	}
	bin := filepath.Join(t.TempDir(), binName)
	build := exec.Command("go", "build", "-o", bin, ".")
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("build CLI: %v\n%s", err, out)
	}
	root := t.TempDir()
	runE2E(t, root, bin, "init", "--agent", "all")
	runE2E(t, root, bin, "doctor")

	intentOut := runE2E(t, root, bin, "intent", "new", "--title", "Deny locked accounts")
	intentID := strings.Fields(intentOut)[0]
	intentPath := onlyMatch(t, filepath.Join(root, ".telos", "intents", "*.md"))
	intent := "+++\nid = \"" + intentID + "\"\ntype = \"intent\"\nstatus = \"draft\"\nrevision = 1\n+++\n\n" + validE2EIntent
	if err := os.WriteFile(intentPath, []byte(intent), 0o644); err != nil {
		t.Fatal(err)
	}
	runE2E(t, root, bin, "intent", "validate", intentID)
	runE2E(t, root, bin, "intent", "seal", intentID)

	specOut := runE2E(t, root, bin, "spec", "new", "--intent", intentID, "--title", "Locked authentication")
	specID := strings.Fields(specOut)[0]
	specPath := onlyMatch(t, filepath.Join(root, ".telos", "specs", "*.md"))
	spec := "+++\nid = \"" + specID + "\"\ntype = \"spec\"\nstatus = \"draft\"\nrevision = 1\nintent = \"" + intentID + "\"\nparents = [\"" + intentID + "\"]\n+++\n\n" + validE2ESpec
	if err := os.WriteFile(specPath, []byte(spec), 0o644); err != nil {
		t.Fatal(err)
	}
	runE2E(t, root, bin, "spec", "validate", specID)
	runE2E(t, root, bin, "spec", "seal", specID)

	planPath := filepath.Join(root, ".telos", "test-plans", strings.ToLower(specID)+".json")
	plan := map[string]any{
		"version": 1, "spec": specID, "feature": strings.ToLower(specID),
		"scenarios": []any{map[string]any{
			"id": "SCN-001", "rule": "RULE-001", "name": "Deny login after lock", "tags": []string{"negative"},
			"given": []string{"an account is locked"}, "when": []string{"valid credentials are submitted"}, "then": []string{"authentication is denied", "no session is created"},
		}},
	}
	planData, _ := json.MarshalIndent(plan, "", "  ")
	if err := os.WriteFile(planPath, append(planData, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
	runE2E(t, root, bin, "testify", "--spec", specID)
	changeOut := runE2E(t, root, bin, "change", "begin", "--intent", intentID, "--spec", specID)
	changeID := strings.TrimSpace(changeOut)
	runE2E(t, root, bin, "context", "--change", changeID)
	runE2E(t, root, bin, "verify")

	if err := os.Chmod(specPath, 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(specPath, []byte("tampered\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(bin, "verify")
	cmd.Dir = root
	if err := cmd.Run(); err == nil {
		t.Fatal("verify accepted a tampered sealed specification")
	}
}

func runE2E(t *testing.T, root, bin string, args ...string) string {
	t.Helper()
	cmd := exec.Command(bin, args...)
	cmd.Dir = root
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("telos %s: %v\n%s", strings.Join(args, " "), err, out)
	}
	return string(out)
}

func onlyMatch(t *testing.T, pattern string) string {
	t.Helper()
	matches, err := filepath.Glob(pattern)
	if err != nil || len(matches) != 1 {
		t.Fatalf("glob %s: %v (%v)", pattern, err, matches)
	}
	return matches[0]
}

const validE2EIntent = `# Deny locked accounts

## Outcome

Locked accounts cannot authenticate.

## Actors

Account owner and security operator.

## Scope

Authentication attempts after lock.

## Non-goals

Recovery is excluded.

## Success criteria

Every attempt is denied without a session.

## Constraints

Existing sessions are unchanged.

## Open questions

None.
`

const validE2ESpec = `# Locked authentication

## Context

An account is locked.

## Rules

### RULE-001 — Deny authentication

A locked account is denied without creating a session.

## Examples

Valid credentials remain denied.

## Boundaries

Repeated attempts remain denied.

## Failure modes

Audit failure never permits access.

## Observability

Each denial is auditable.
`
