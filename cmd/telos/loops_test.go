package main

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// The three V2 acceptance loops (docs/design-v2.md §0). They are the
// executable arbitration record for the v0.6 rewrite: committed skipped at
// M0 and un-skipped milestone by milestone — loop 1 at M3, loops 2 and 3 at
// M4. They exercise only the target CLI surface plus the JSON envelope, so
// they survive every internal rewrite underneath.

func gitOut(t *testing.T, root string, args ...string) string {
	t.Helper()
	cmd := exec.Command("git", append([]string{"-C", root}, args...)...)
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %v: %v\n%s", args, err, out)
	}
	return string(out)
}

func readFile(t *testing.T, root, rel string) string {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(root, filepath.FromSlash(rel)))
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

const v2Config = `project_id = "e2e-loops"
agents = ["claude"]
test_commands = ["go run tools/probe.go"]
test_files = ["tests/**"]
closure = "tree"
`

const v2Product = `# Product

### INT-001 — Application greets reliably

The application always produces its greeting.
`

const v2GreetingDelta = "<!-- telos:op add file: spec/core.md -->\n" +
	"### REQ-001 — Emit the greeting\n" +
	"Class: behavior\n" +
	"Motivated by: INT-001\n\n" +
	"The application emits the greeting exactly once.\n\n" +
	"```gherkin\nScenario: greeting is emitted\n  Given the application runs\n  Then the greeting is produced once\n```\n"

// setupCertified creates a git repo holding the content genesis adopts
// (config, product intent, probe suite) and initializes it into a certified
// state. telos.toml is written before init: it is tracked and therefore
// protected, so the genesis commit is the moment it enters certification.
func setupCertified(t *testing.T, bin string) string {
	t.Helper()
	root := t.TempDir()
	git(t, root, "init", "--quiet", "-b", "main")
	git(t, root, "config", "user.email", "telos@e2e")
	git(t, root, "config", "user.name", "telos e2e")
	write(t, root, "app.txt", "hello\n")
	write(t, root, "tools/probe.go", probeProgram)
	write(t, root, "telos.toml", v2Config)
	write(t, root, "spec/PRODUCT.md", v2Product)
	expectOK(t, runCLI(t, bin, root, "", "init", "--agent", "claude"), "init (genesis)")
	expectOK(t, runCLI(t, bin, root, "", "verify"), "verify (genesis)")
	return root
}

func startChange(t *testing.T, bin, root, category, title string) (id, worktree string) {
	t.Helper()
	result := expectOK(t, runCLI(t, bin, root, "", "change", "start", "--category", category, "--title", title), "change start")
	id, _ = result["id"].(string)
	worktree, _ = result["worktree"].(string)
	if id == "" || worktree == "" {
		t.Fatalf("change start returned %v", result)
	}
	return id, worktree
}

func approveChange(t *testing.T, bin, worktree string) {
	t.Helper()
	review := expectOK(t, runCLI(t, bin, worktree, "", "change", "review"), "change review")
	digest, _ := review["digest"].(string)
	if digest == "" {
		t.Fatalf("change review returned no digest: %v", review)
	}
	expectOK(t, runCLI(t, bin, worktree, "", "change", "approve", "--digest", digest), "change approve")
}

func certificateJSON(t *testing.T, root string) map[string]any {
	t.Helper()
	raw := gitOut(t, root, "notes", "--ref=refs/notes/telos", "show", "HEAD")
	var cert map[string]any
	if err := json.Unmarshal([]byte(raw), &cert); err != nil {
		t.Fatalf("certificate note is not valid JSON: %v\n%s", err, raw)
	}
	return cert
}

// TestLoopFeature — acceptance loop 1: request → contract delta → digest-bound
// approval → witnessed red/green in the candidate → certification → atomic
// promotion of exactly one new certified commit on main.
func TestLoopFeature(t *testing.T) {
	t.Skip("V2 acceptance loop 1 — un-skipped at M3 (docs/design-v2.md §0)")

	bin := buildCLI(t)
	root := setupCertified(t, bin)

	// A behavior change starts from the certified state in an isolated candidate.
	id, wt := startChange(t, bin, root, "behavior_change", "Emit the greeting")

	// The contract delta is drafted in the candidate; spec/ itself stays
	// untouched there (a direct spec edit is TELOS_CONTRACT_TAMPERED).
	write(t, wt, "changes/"+id+"/contract.delta.md", v2GreetingDelta)

	// Approval binds to the exact folded contract: drift after review
	// invalidates the presented digest.
	review := expectOK(t, runCLI(t, bin, wt, "", "change", "review"), "review")
	digest, _ := review["digest"].(string)
	write(t, wt, "changes/"+id+"/contract.delta.md", v2GreetingDelta+"\nLate drift.\n")
	expectCode(t, runCLI(t, bin, wt, "", "change", "approve", "--digest", digest), "TELOS_APPROVAL_STALE", "stale approve")
	write(t, wt, "changes/"+id+"/contract.delta.md", v2GreetingDelta)
	approveChange(t, bin, wt)

	// A test the suite already passes proves nothing.
	write(t, wt, "tests/core_test.txt", "asserts REQ-001\nexpect app.txt\n")
	expectCode(t, runCLI(t, bin, wt, "", "evidence", "red", "--req", "REQ-001"), "TELOS_RED_EXPECTED", "red on passing test")

	// The witnessed failing test is sealed as red evidence (blob OIDs).
	write(t, wt, "tests/core_test.txt", "asserts REQ-001\nexpect out/greeting.txt\n")
	expectOK(t, runCLI(t, bin, wt, "", "evidence", "red", "--req", "REQ-001"), "witnessed red")

	// Sealed bytes may not move to satisfy the implementation.
	write(t, wt, "tests/core_test.txt", "asserts REQ-001\nexpect app.txt\n")
	expectCode(t, runCLI(t, bin, wt, "", "evidence", "green", "--req", "REQ-001"), "TELOS_RED_STALE", "green on rewritten test")
	write(t, wt, "tests/core_test.txt", "asserts REQ-001\nexpect out/greeting.txt\n")

	// Only the implementation turns the witnessed red into green.
	expectCode(t, runCLI(t, bin, wt, "", "change", "ready"), "TELOS_RED_PENDING", "ready while red")
	write(t, wt, "out/greeting.txt", "greeting\n")
	expectOK(t, runCLI(t, bin, wt, "", "evidence", "green", "--req", "REQ-001"), "witnessed green")

	// Certification gates pass; promotion is atomic.
	expectOK(t, runCLI(t, bin, wt, "", "change", "ready"), "ready")
	expectOK(t, runCLI(t, bin, wt, "", "change", "promote"), "promote")

	// The root advanced to a new certified commit: folded contract, retained
	// Change record, sealed certificate note naming the change.
	expectOK(t, runCLI(t, bin, root, "", "verify"), "verify (promoted)")
	if !strings.Contains(readFile(t, root, "spec/core.md"), "REQ-001") {
		t.Fatal("promotion did not fold the contract delta into spec/")
	}
	if _, err := os.Stat(filepath.Join(root, "changes", id, "change.json")); err != nil {
		t.Fatalf("promotion did not retain the Change record: %v", err)
	}
	cert := certificateJSON(t, root)
	payload, _ := cert["payload"].(map[string]any)
	change, _ := payload["change"].(map[string]any)
	if change["id"] != id {
		t.Fatalf("certificate change = %v, want %s", change["id"], id)
	}
	status := expectOK(t, runCLI(t, bin, root, "", "status"), "status")
	if status["state"] != "certified" {
		t.Fatalf("state = %v, want certified", status["state"])
	}
}

// TestLoopSalvage — acceptance loop 2: an out-of-band edit corrupts the
// certified worktree; status proposes the one-gesture capture into a Change;
// the diff rides the normal loop; restore discards; an out-of-band commit is
// corruption of the certified chain.
func TestLoopSalvage(t *testing.T) {
	t.Skip("V2 acceptance loop 2 — un-skipped at M4 (docs/design-v2.md §0)")

	bin := buildCLI(t)
	root := setupCertified(t, bin)

	// Corruption is symmetric: any protected edit, code or contract alike.
	write(t, root, "app.txt", "tampered\n")
	status := expectOK(t, runCLI(t, bin, root, "", "status"), "status (corrupted)")
	if status["state"] != "corrupted" {
		t.Fatalf("state = %v, want corrupted", status["state"])
	}
	salvage, _ := status["salvage"].(map[string]any)
	if salvage == nil || salvage["prompt"] == "" {
		t.Fatalf("status did not propose salvage: %v", status)
	}
	expectCode(t, runCLI(t, bin, root, "", "verify"), "TELOS_STATE_CORRUPTED", "verify (corrupted)")

	// Salvage captures the diff into a candidate and restores the root; the
	// result names where the work went.
	result := expectOK(t, runCLI(t, bin, root, "", "salvage"), "salvage")
	wt, _ := result["worktree"].(string)
	id, _ := result["change"].(string)
	if wt == "" || id == "" {
		t.Fatalf("salvage returned %v", result)
	}
	expectOK(t, runCLI(t, bin, root, "", "verify"), "verify (restored)")
	if readFile(t, root, "app.txt") != "hello\n" {
		t.Fatal("salvage did not restore the certified bytes")
	}
	if readFile(t, wt, "app.txt") != "tampered\n" {
		t.Fatal("salvage did not carry the edit into the candidate")
	}

	// The salvaged diff rides the normal loop as a behavior-preserving change.
	approveChange(t, bin, wt)
	expectOK(t, runCLI(t, bin, wt, "", "change", "ready"), "ready")
	expectOK(t, runCLI(t, bin, wt, "", "change", "promote"), "promote")
	expectOK(t, runCLI(t, bin, root, "", "verify"), "verify (promoted)")
	if readFile(t, root, "app.txt") != "tampered\n" {
		t.Fatal("promotion did not land the salvaged edit")
	}

	// restore discards instead of preserving.
	write(t, root, "app.txt", "again\n")
	expectOK(t, runCLI(t, bin, root, "", "restore"), "restore")
	if readFile(t, root, "app.txt") != "tampered\n" {
		t.Fatal("restore did not return to certified bytes")
	}

	// An out-of-band commit leaves HEAD without a valid certificate.
	write(t, root, "app.txt", "rogue\n")
	git(t, root, "add", "-A")
	git(t, root, "commit", "--quiet", "-m", "rogue")
	expectCode(t, runCLI(t, bin, root, "", "verify"), "TELOS_CERTIFICATE_INVALID", "verify (uncertified tip)")
}

// TestLoopConcurrent — acceptance loop 3: two changes in flight; the first
// promotion stales the second's base; rebase triggers selective revalidation
// where only evidence whose dependency closure intersects the rebased diff is
// recomputed — disjoint changes re-certify almost free.
func TestLoopConcurrent(t *testing.T) {
	t.Skip("V2 acceptance loop 3 — un-skipped at M4 (docs/design-v2.md §0)")

	bin := buildCLI(t)
	root := t.TempDir()
	git(t, root, "init", "--quiet", "-b", "main")
	git(t, root, "config", "user.email", "telos@e2e")
	git(t, root, "config", "user.name", "telos e2e")
	// A tiny two-package Go module so the evidence closures of the two
	// changes are disjoint: pkga and pkgb do not import each other.
	write(t, root, "go.mod", "module example.com/toy\n\ngo 1.24\n")
	write(t, root, "pkga/pkga.go", "package pkga\n\nfunc A() int { return 0 }\n")
	write(t, root, "pkgb/pkgb.go", "package pkgb\n\nfunc B() int { return 0 }\n")
	write(t, root, "telos.toml", "project_id = \"e2e-concurrent\"\nagents = [\"claude\"]\ntest_commands = [\"go test ./...\"]\ntest_files = [\"**/*_test.go\"]\nclosure = \"go\"\n")
	write(t, root, "spec/PRODUCT.md", v2Product)
	expectOK(t, runCLI(t, bin, root, "", "init", "--agent", "claude"), "init (genesis)")

	deltaA := "<!-- telos:op add file: spec/core.md -->\n" +
		"### REQ-101 — pkga computes A\nClass: behavior\nMotivated by: INT-001\n\n" +
		"```gherkin\nScenario: A\n  Given pkga\n  Then A returns 1\n```\n"
	deltaB := "<!-- telos:op add file: spec/core.md -->\n" +
		"### REQ-102 — pkgb computes B\nClass: behavior\nMotivated by: INT-001\n\n" +
		"```gherkin\nScenario: B\n  Given pkgb\n  Then B returns 2\n```\n"

	idA, wtA := startChange(t, bin, root, "behavior_change", "pkga computes A")
	idB, wtB := startChange(t, bin, root, "behavior_change", "pkgb computes B")

	// Both changes reach witnessed green against the same base.
	write(t, wtA, "changes/"+idA+"/contract.delta.md", deltaA)
	approveChange(t, bin, wtA)
	write(t, wtA, "pkga/pkga_test.go", "package pkga\n\nimport \"testing\"\n\n// asserts REQ-101\nfunc TestA(t *testing.T) {\n\tif A() != 1 {\n\t\tt.Fatal(\"REQ-101\")\n\t}\n}\n")
	expectOK(t, runCLI(t, bin, wtA, "", "evidence", "red", "--req", "REQ-101"), "red A")
	write(t, wtA, "pkga/pkga.go", "package pkga\n\nfunc A() int { return 1 }\n")
	expectOK(t, runCLI(t, bin, wtA, "", "evidence", "green", "--req", "REQ-101"), "green A")

	write(t, wtB, "changes/"+idB+"/contract.delta.md", deltaB)
	approveChange(t, bin, wtB)
	write(t, wtB, "pkgb/pkgb_test.go", "package pkgb\n\nimport \"testing\"\n\n// asserts REQ-102\nfunc TestB(t *testing.T) {\n\tif B() != 2 {\n\t\tt.Fatal(\"REQ-102\")\n\t}\n}\n")
	expectOK(t, runCLI(t, bin, wtB, "", "evidence", "red", "--req", "REQ-102"), "red B")
	write(t, wtB, "pkgb/pkgb.go", "package pkgb\n\nfunc B() int { return 2 }\n")
	expectOK(t, runCLI(t, bin, wtB, "", "evidence", "green", "--req", "REQ-102"), "green B")

	// A promotes first.
	expectOK(t, runCLI(t, bin, wtA, "", "change", "ready"), "ready A")
	expectOK(t, runCLI(t, bin, wtA, "", "change", "promote"), "promote A")

	// B's base is stale; green(A) and green(B) do not imply green(A+B).
	expectCode(t, runCLI(t, bin, wtB, "", "change", "ready"), "TELOS_BASE_STALE", "ready B before rebase")
	expectOK(t, runCLI(t, bin, wtB, "", "change", "rebase"), "rebase B")

	// Selective revalidation: A never touched pkgb's closure, so B's witnessed
	// red/green survives the rebase; the whole-module suite is recomputed.
	expectOK(t, runCLI(t, bin, wtB, "", "change", "ready"), "ready B")
	expectOK(t, runCLI(t, bin, wtB, "", "change", "promote"), "promote B")
	expectOK(t, runCLI(t, bin, root, "", "verify"), "verify (both promoted)")

	cert := certificateJSON(t, root)
	payload, _ := cert["payload"].(map[string]any)
	verification, _ := payload["verification"].(map[string]any)
	entries, _ := verification["evidence"].([]any)
	var reused, recomputed bool
	for _, e := range entries {
		entry, _ := e.(map[string]any)
		if entry["reused"] == true {
			reused = true
		}
		if entry["reused"] == false {
			recomputed = true
		}
	}
	if !reused || !recomputed {
		t.Fatalf("expected both reused and recomputed evidence in B's certificate: %v", entries)
	}
}
