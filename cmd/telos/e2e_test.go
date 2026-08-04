package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

type cliEnvelope struct {
	OK     bool           `json:"ok"`
	Result map[string]any `json:"result"`
	Error  struct {
		Code string `json:"code"`
	} `json:"error"`
}

func buildCLI(t *testing.T) string {
	t.Helper()
	binName := "telos"
	if runtime.GOOS == "windows" {
		binName += ".exe"
	}
	bin := filepath.Join(t.TempDir(), binName)
	build := exec.Command("go", "build", "-o", bin, ".")
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("build CLI: %v\n%s", err, out)
	}
	return bin
}

func runCLI(t *testing.T, bin, root, stdin string, args ...string) cliEnvelope {
	t.Helper()
	cmd := exec.Command(bin, append(args, "--json")...)
	cmd.Dir = root
	cmd.Stdin = strings.NewReader(stdin)
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	_ = cmd.Run()
	var envelope cliEnvelope
	if err := json.Unmarshal(stdout.Bytes(), &envelope); err != nil {
		t.Fatalf("telos %v produced invalid JSON: %v\nstdout: %s\nstderr: %s", args, err, stdout.String(), stderr.String())
	}
	return envelope
}

func expectOK(t *testing.T, envelope cliEnvelope, label string) map[string]any {
	t.Helper()
	if !envelope.OK {
		t.Fatalf("%s failed with %s", label, envelope.Error.Code)
	}
	return envelope.Result
}

func expectCode(t *testing.T, envelope cliEnvelope, code, label string) {
	t.Helper()
	if envelope.OK {
		t.Fatalf("%s unexpectedly succeeded", label)
	}
	if envelope.Error.Code != code {
		t.Fatalf("%s error = %s, want %s", label, envelope.Error.Code, code)
	}
}

func git(t *testing.T, root string, args ...string) {
	t.Helper()
	cmd := exec.Command("git", append([]string{"-C", root}, args...)...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git %v: %v\n%s", args, err, out)
	}
}

func write(t *testing.T, root, rel, content string) {
	t.Helper()
	path := filepath.Join(root, filepath.FromSlash(rel))
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

const productSpec = `# Product

## Objectives

### OBJ-001 — Application greets reliably

The application always produces its greeting.
`

const coreSpec = "# Core\n\n### RULE-001 — Emit the greeting\n\nTraces: OBJ-001\n\nThe application emits the greeting exactly once.\n\n```gherkin\nScenario: greeting is emitted\n  Given the application runs\n  Then the greeting is produced once\n```\n"

const legacySpec = "# Legacy\n\n### RULE-002 — Greeting source persists\n\nTraces: OBJ-001\n\nThe greeting source file remains present.\n\n```gherkin\nScenario: source persists\n  Given the application tree\n  Then app.txt exists\n```\n"

// probeProgram is the e2e suite: every `expect <path>` line found under
// tests/ must name an existing file, so a test stays red exactly until its
// implementation exists.
const probeProgram = `package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

func main() {
	entries, err := os.ReadDir("tests")
	if err != nil {
		return
	}
	var missing []string
	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		data, err := os.ReadFile(filepath.Join("tests", entry.Name()))
		if err != nil {
			fmt.Println(err)
			os.Exit(1)
		}
		for _, line := range strings.Split(string(data), "\n") {
			rest, ok := strings.CutPrefix(strings.TrimSpace(line), "expect ")
			if !ok {
				continue
			}
			rest = strings.TrimSpace(rest)
			if _, err := os.Stat(filepath.FromSlash(rest)); err != nil {
				missing = append(missing, rest)
			}
		}
	}
	if len(missing) > 0 {
		fmt.Println("missing:", strings.Join(missing, ", "))
		os.Exit(1)
	}
}
`

func addPatch(path string, lines []string) string {
	var b strings.Builder
	fmt.Fprintf(&b, "diff --git a/%s b/%s\n", path, path)
	b.WriteString("new file mode 100644\n--- /dev/null\n")
	fmt.Fprintf(&b, "+++ b/%s\n@@ -0,0 +1,%d @@\n", path, len(lines))
	for _, line := range lines {
		b.WriteString("+" + line + "\n")
	}
	return b.String()
}

func TestCLIEndToEnd(t *testing.T) {
	bin := buildCLI(t)
	root := t.TempDir()
	git(t, root, "init", "--quiet")
	git(t, root, "config", "user.email", "telos@e2e")
	git(t, root, "config", "user.name", "telos e2e")
	write(t, root, "app.txt", "hello\n")
	write(t, root, "tools/probe.go", probeProgram)

	// Bootstrap an existing project: init adopts the current tree as baseline.
	expectOK(t, runCLI(t, bin, root, "", "init", "--agent", "all"), "init")
	for _, rel := range []string{"telos.toml", "spec/PRODUCT.md", ".telos/state.json", ".claude/skills/telos/SKILL.md", ".agents/skills/telos/SKILL.md", ".claude/settings.json", ".codex/hooks.json"} {
		if _, err := os.Stat(filepath.Join(root, filepath.FromSlash(rel))); err != nil {
			t.Fatalf("init did not create %s: %v", rel, err)
		}
	}
	expectOK(t, runCLI(t, bin, root, "", "doctor"), "doctor")

	// The legacy file is neither untraced nor annotated: verify demands adoption.
	expectCode(t, runCLI(t, bin, root, "", "verify"), "TELOS_ANNOTATION_MISSING", "verify (bootstrap)")

	// The human configures the project (telos.toml is human-owned).
	write(t, root, "telos.toml", `agents = ["claude", "codex"]
test_commands = ["go run tools/probe.go"]
test_files = ["tests/**"]
untraced = ["README.md", ".github/**", ".claude/**", ".codex/**", ".agents/**", "CLAUDE.md", "AGENTS.md", "tools/**"]
`)

	// Draft the spec through the broker.
	expectOK(t, runCLI(t, bin, root, productSpec, "spec", "put", "--file", "spec/PRODUCT.md"), "spec put PRODUCT")
	expectOK(t, runCLI(t, bin, root, coreSpec, "spec", "put", "--file", "spec/core.md"), "spec put core")
	review := expectOK(t, runCLI(t, bin, root, "", "spec", "review"), "spec review")
	digest, _ := review["digest"].(string)
	if digest == "" {
		t.Fatal("spec review returned no digest")
	}

	// Any later spec change invalidates the presented digest.
	expectOK(t, runCLI(t, bin, root, coreSpec+"\nLate drift.\n", "spec", "put", "--file", "spec/core.md"), "spec put drift")
	expectCode(t, runCLI(t, bin, root, "", "spec", "approve", "--review", digest), "TELOS_APPROVAL_STALE", "stale approve")
	review = expectOK(t, runCLI(t, bin, root, "", "spec", "review"), "spec re-review")
	digest, _ = review["digest"].(string)
	expectOK(t, runCLI(t, bin, root, "", "spec", "approve", "--review", digest), "spec approve")

	// Approved but unimplemented: the spec is ahead of the code.
	status := expectOK(t, runCLI(t, bin, root, "", "status"), "status")
	if status["phase"] != "implementing" {
		t.Fatalf("phase = %v, want implementing", status["phase"])
	}

	// Proof is test-first: no implementation before the witnessed failing test.
	appPatch := "diff --git a/app.txt b/app.txt\n--- a/app.txt\n+++ b/app.txt\n@@ -1,1 +1,2 @@\n+telos: RULE-001\n hello\n"
	expectCode(t, runCLI(t, bin, root, appPatch, "apply", "--rule", "RULE-001"), "TELOS_TEST_FIRST", "apply implementation first")

	// Broker-applied patches must leave every touched file annotated.
	expectCode(t, runCLI(t, bin, root, addPatch("tests/core_test.txt", []string{"unannotated", "asserts RULE-001"}), "apply", "--rule", "RULE-001"), "TELOS_ANNOTATION_MISMATCH", "apply unannotated")
	if _, err := os.Stat(filepath.Join(root, "tests", "core_test.txt")); !os.IsNotExist(err) {
		t.Fatal("rejected patch left the tree modified")
	}
	expectCode(t, runCLI(t, bin, root, addPatch("spec/evil.md", []string{"# nope"}), "apply", "--rule", "RULE-001"), "TELOS_INPUT_INVALID", "apply into spec/")
	expectCode(t, runCLI(t, bin, root, addPatch(".claude/hax.md", []string{"x"}), "apply", "--rule", "RULE-001"), "TELOS_INPUT_INVALID", "apply into .claude/")

	// A test the suite already passes proves nothing.
	expectCode(t, runCLI(t, bin, root, addPatch("tests/core_test.txt", []string{"telos: RULE-001", "asserts RULE-001", "expect app.txt"}), "apply", "--rule", "RULE-001"), "TELOS_RED_EXPECTED", "apply passing test")

	// The witnessed failing test is recorded and sealed as red evidence.
	red := expectOK(t, runCLI(t, bin, root, addPatch("tests/core_test.txt", []string{"telos: RULE-001", "asserts RULE-001", "expect out/greeting.txt"}), "apply", "--rule", "RULE-001"), "apply red test")
	if fmt.Sprint(red["suite"]) != "red" {
		t.Fatalf("red apply result = %v", red)
	}

	// Sealed: the failing test may not move to satisfy the implementation.
	sealedPatch := "diff --git a/tests/core_test.txt b/tests/core_test.txt\n--- a/tests/core_test.txt\n+++ b/tests/core_test.txt\n@@ -1,3 +1,3 @@\n telos: RULE-001\n asserts RULE-001\n-expect out/greeting.txt\n+expect app.txt\n" + appPatch
	expectCode(t, runCLI(t, bin, root, sealedPatch, "apply", "--rule", "RULE-001"), "TELOS_TEST_SEALED", "apply touching sealed test")

	// app.txt gains its annotation; the suite stays red until the greeting
	// exists, and verify says exactly that.
	expectOK(t, runCLI(t, bin, root, appPatch, "apply", "--rule", "RULE-001"), "apply app annotation")
	expectCode(t, runCLI(t, bin, root, "", "verify"), "TELOS_RED_PENDING", "verify (red pending)")

	// Only the implementation turns the witnessed red into a witnessed green.
	green := expectOK(t, runCLI(t, bin, root, addPatch("out/greeting.txt", []string{"telos: RULE-001", "greeting"}), "apply", "--rule", "RULE-001"), "apply implementation")
	if fmt.Sprint(green["suite"]) != "green" || fmt.Sprint(green["proven"]) != "[RULE-001]" {
		t.Fatalf("green apply result = %v", green)
	}
	verified := expectOK(t, runCLI(t, bin, root, "", "verify"), "verify (green)")
	if fmt.Sprint(verified["rules"]) != "1" {
		t.Fatalf("verify result = %v", verified)
	}
	git(t, root, "add", "-A")
	git(t, root, "commit", "--quiet", "-m", "green")

	// Out-of-band code edit corrupts; git is the recovery path.
	write(t, root, "app.txt", "tampered\n")
	expectCode(t, runCLI(t, bin, root, "", "verify"), "TELOS_CODE_CORRUPTED", "verify (tampered)")
	git(t, root, "checkout", "--", "app.txt")
	expectOK(t, runCLI(t, bin, root, "", "verify"), "verify (restored)")

	// A human spec edit is pending, not corrupted: it goes through adoption.
	write(t, root, "spec/core.md", coreSpec+"\nHuman refinement.\n")
	expectCode(t, runCLI(t, bin, root, "", "verify"), "TELOS_SPEC_UNAPPROVED", "verify (human spec edit)")
	review = expectOK(t, runCLI(t, bin, root, "", "spec", "review"), "adoption review")
	digest, _ = review["digest"].(string)
	expectOK(t, runCLI(t, bin, root, "", "spec", "approve", "--review", digest), "adoption approve")
	expectOK(t, runCLI(t, bin, root, "", "verify"), "verify (adopted)")

	// A rule documenting existing behavior can never be witnessed failing:
	// adoption is explicit, human-gated by the guard, and requires the suite
	// to pass with the documentation test in place.
	expectOK(t, runCLI(t, bin, root, legacySpec, "spec", "put", "--file", "spec/legacy.md"), "spec put legacy")
	review = expectOK(t, runCLI(t, bin, root, "", "spec", "review"), "legacy review")
	digest, _ = review["digest"].(string)
	expectOK(t, runCLI(t, bin, root, "", "spec", "approve", "--review", digest), "legacy approve")
	expectCode(t, runCLI(t, bin, root, "", "verify"), "TELOS_RULE_NOT_IMPLEMENTED", "verify (spec ahead)")
	adopted := expectOK(t, runCLI(t, bin, root, addPatch("tests/legacy_test.txt", []string{"telos: RULE-002", "asserts RULE-002", "expect app.txt"}), "apply", "--rule", "RULE-002", "--expect-pass"), "apply adoption test")
	if fmt.Sprint(adopted["proven"]) != "[RULE-002]" {
		t.Fatalf("adoption result = %v", adopted)
	}
	expectOK(t, runCLI(t, bin, root, "", "verify"), "verify (adopted rule)")

	// Traceability is derivable from the tree alone.
	trace := expectOK(t, runCLI(t, bin, root, "", "trace"), "trace")
	text := fmt.Sprint(trace)
	if !strings.Contains(text, "app.txt") || !strings.Contains(text, "tests/core_test.txt") {
		t.Fatalf("trace misses implementation or test files: %v", text)
	}
}
