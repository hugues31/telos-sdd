package main

import (
	"bytes"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

type cliEnvelope struct {
	OK     bool            `json:"ok"`
	Result json.RawMessage `json:"result"`
	Error  struct {
		Code string `json:"code"`
	} `json:"error"`
}

type flowResult struct {
	ID         string   `json:"id"`
	Intent     string   `json:"intent"`
	Specs      []string `json:"specs"`
	Phase      string   `json:"phase"`
	Brainstorm string   `json:"brainstorm"`
}

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
	if err := os.WriteFile(filepath.Join(root, "app.txt"), []byte("open\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	runGit(t, root, "init")
	runGit(t, root, "config", "user.email", "telos@example.test")
	runGit(t, root, "config", "user.name", "Telos Test")
	runGit(t, root, "add", "app.txt")
	runGit(t, root, "commit", "-m", "baseline")

	runE2EJSON(t, root, bin, "", "init", "--agent", "all", "--json")
	flowEnvelope := runE2EJSON(t, root, bin, "", "flow", "start", "--brainstorm", "none", "--request", "Deny locked accounts", "--json")
	var flow flowResult
	decodeResult(t, flowEnvelope, &flow)
	if flow.ID == "" || flow.Intent == "" || flow.Phase != "intent_draft" {
		t.Fatalf("unexpected flow: %#v", flow)
	}

	runE2EJSON(t, root, bin, validE2EIntentV2, "artifact", "put", "--id", flow.Intent, "--json")
	reviewEnvelope := runE2EJSON(t, root, bin, "", "intent", "review", "--flow", flow.ID, "--json")
	var review struct {
		Digest string `json:"digest"`
	}
	decodeResult(t, reviewEnvelope, &review)
	if review.Digest == "" {
		t.Fatal("intent review returned no digest")
	}
	runE2EJSON(t, root, bin, "", "intent", "seal", "--flow", flow.ID, "--review", review.Digest, "--json")

	specEnvelope := runE2EJSON(t, root, bin, "", "spec", "new", "--flow", flow.ID, "--title", "Locked authentication", "--json")
	var specResult struct {
		Flow flowResult `json:"flow"`
	}
	decodeResult(t, specEnvelope, &specResult)
	if len(specResult.Flow.Specs) != 1 {
		t.Fatalf("spec not attached: %#v", specResult)
	}
	specID := specResult.Flow.Specs[0]
	runE2EJSON(t, root, bin, validE2ESpecV2, "artifact", "put", "--id", specID, "--json")
	plan := completePlan(specID)
	planData, _ := json.Marshal(plan)
	runE2EJSON(t, root, bin, string(planData), "test-plan", "put", "--spec", specID, "--json")

	contractEnvelope := runE2EJSON(t, root, bin, "", "contract", "review", "--flow", flow.ID, "--json")
	var contractReview struct {
		Digest string `json:"digest"`
	}
	decodeResult(t, contractEnvelope, &contractReview)
	runE2EJSON(t, root, bin, "", "contract", "seal", "--flow", flow.ID, "--review", contractReview.Digest, "--json")
	runE2EJSON(t, root, bin, "", "change", "begin", "--flow", flow.ID, "--json")

	patch := "diff --git a/app.txt b/app.txt\n--- a/app.txt\n+++ b/app.txt\n@@ -1 +1 @@\n-open\n+secured\n"
	traceFailure := runE2EJSONFailureInput(t, root, bin, patch, "change", "apply", "--flow", flow.ID, "--rule", "RULE-999", "--scenario", "SCN-001", "--json")
	if traceFailure.Error.Code != "TELOS_TRACEABILITY_GAP" {
		t.Fatalf("unexpected traceability error: %#v", traceFailure)
	}
	if data, _ := os.ReadFile(filepath.Join(root, "app.txt")); string(data) != "open\n" {
		t.Fatalf("rejected patch changed the repository: %q", data)
	}
	runE2EJSON(t, root, bin, patch, "change", "apply", "--flow", flow.ID, "--rule", "RULE-001", "--scenario", "SCN-001", "--json")
	runE2EJSON(t, root, bin, "", "verify", "--flow", flow.ID, "--check-only", "--json")
	complete := runE2EJSON(t, root, bin, "verified: hashes, traceability, and assertions are valid", "change", "complete", "--flow", flow.ID, "--json")
	if !complete.OK {
		t.Fatal("change did not complete")
	}
	declaredContent, err := os.ReadFile(filepath.Join(root, "app.txt"))
	if err != nil {
		t.Fatal(err)
	}
	if strings.TrimSpace(string(declaredContent)) != "secured" {
		t.Fatalf("declared content = %q, want secured with platform-native line endings", declaredContent)
	}

	if err := os.WriteFile(filepath.Join(root, "app.txt"), []byte("tampered\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	failure := runE2EJSONFailure(t, root, bin, "inspect", "--json")
	if failure.Error.Code != "TELOS_INTEGRITY_UNDECLARED_CHANGE" {
		t.Fatalf("unexpected integrity error: %#v", failure)
	}
	runE2EJSON(t, root, bin, "", "repair", "--restore", "--json")
	if data, _ := os.ReadFile(filepath.Join(root, "app.txt")); !bytes.Equal(data, declaredContent) {
		t.Fatalf("repair did not restore declared content: %q", data)
	}
}

func completePlan(specID string) map[string]any {
	categories := []string{"positive", "negative", "boundary", "authorization", "state-transition", "retry-idempotency", "concurrency", "failure-recovery", "prohibited-side-effect"}
	coverage := make([]any, 0, len(categories))
	for _, category := range categories {
		coverage = append(coverage, map[string]any{"rule": "RULE-001", "category": category, "status": "covered"})
	}
	return map[string]any{
		"spec": specID, "feature": strings.ToLower(specID),
		"scenarios": []any{map[string]any{
			"id": "SCN-001", "rule": "RULE-001", "name": "Deny login after lock", "tags": categories,
			"given": []string{"an account is locked"}, "when": []string{"valid credentials are submitted"}, "then": []string{"authentication is denied", "no session is created"},
		}},
		"coverage": coverage,
	}
}

func runE2EJSON(t *testing.T, root, bin, input string, args ...string) cliEnvelope {
	t.Helper()
	cmd := exec.Command(bin, args...)
	cmd.Dir = root
	cmd.Stdin = strings.NewReader(input)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("telos %s: %v\n%s", strings.Join(args, " "), err, out)
	}
	var envelope cliEnvelope
	if err := json.Unmarshal(out, &envelope); err != nil || !envelope.OK {
		t.Fatalf("invalid success envelope for %s: %v\n%s", strings.Join(args, " "), err, out)
	}
	return envelope
}

func runE2EJSONFailure(t *testing.T, root, bin string, args ...string) cliEnvelope {
	return runE2EJSONFailureInput(t, root, bin, "", args...)
}

func runE2EJSONFailureInput(t *testing.T, root, bin, input string, args ...string) cliEnvelope {
	t.Helper()
	cmd := exec.Command(bin, args...)
	cmd.Dir = root
	cmd.Stdin = strings.NewReader(input)
	out, err := cmd.CombinedOutput()
	if err == nil {
		t.Fatalf("telos %s unexpectedly succeeded: %s", strings.Join(args, " "), out)
	}
	var envelope cliEnvelope
	if json.Unmarshal(out, &envelope) != nil || envelope.OK {
		t.Fatalf("invalid failure envelope for %s: %s", strings.Join(args, " "), out)
	}
	return envelope
}

func decodeResult(t *testing.T, envelope cliEnvelope, target any) {
	t.Helper()
	if err := json.Unmarshal(envelope.Result, target); err != nil {
		t.Fatal(err)
	}
}

func runGit(t *testing.T, root string, args ...string) {
	t.Helper()
	cmd := exec.Command("git", append([]string{"-C", root}, args...)...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git %s: %v\n%s", strings.Join(args, " "), err, out)
	}
}

const validE2EIntentV2 = `# Deny locked accounts

## Outcome

Locked accounts cannot authenticate.

## Actors

Account owner and security operator.

## Scope

Authentication attempts after lock.

## Non-goals

Recovery is excluded.

## Success criteria

### CRIT-001 — Denied authentication

Every attempt is denied without a session.

## Constraints

Existing sessions are unchanged.

## Open questions

None.
`

const validE2ESpecV2 = `# Locked authentication

## Context

An account is locked.

## Rules

### RULE-001 — Deny authentication

Traces: CRIT-001

A locked account is denied without creating a session.

## Examples

Valid credentials remain denied.

## Boundaries

Repeated attempts remain denied.

## Non-effects

Existing sessions remain unchanged.

## Failure modes

Audit failure never permits access.

## Observability

Each denial is auditable.
`
