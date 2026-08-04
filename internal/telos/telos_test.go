package telos

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func gitRepo(t *testing.T) string {
	t.Helper()
	root := t.TempDir()
	for _, args := range [][]string{
		{"init", "--quiet"},
		{"config", "user.email", "telos@test"},
		{"config", "user.name", "telos"},
	} {
		cmd := exec.Command("git", append([]string{"-C", root}, args...)...)
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("git %v: %v\n%s", args, err, out)
		}
	}
	return root
}

func initTelos(t *testing.T, root string) {
	t.Helper()
	if err := initProject(root, "claude", false); err != nil {
		t.Fatalf("init: %v", err)
	}
}

func writeTestConfig(t *testing.T, root string, testCommands []string) {
	t.Helper()
	cfg := "agents = [\"claude\"]\n" +
		"test_commands = " + quoteList(testCommands) + "\n" +
		"test_files = [\"tests/**\"]\n" +
		"untraced = [\"README.md\", \".claude/**\", \".agents/**\", \".codex/**\", \".github/**\", \"CLAUDE.md\", \"AGENTS.md\", \"tools/**\"]\n"
	if err := os.WriteFile(filepath.Join(root, configFile), []byte(cfg), 0o644); err != nil {
		t.Fatal(err)
	}
}

const probeCommand = "go run tools/probe.go"

// writeProbe installs, before init so it lands in the declared baseline, a
// minimal cross-platform suite: every `expect <path>` line found under tests/
// must name an existing file, so a test stays red exactly until its
// implementation exists.
func writeProbe(t *testing.T, root string) {
	t.Helper()
	probe := `package main

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
	path := filepath.Join(root, "tools", "probe.go")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(probe), 0o644); err != nil {
		t.Fatal(err)
	}
}

func apply(t *testing.T, root string, rules []string, patch []byte) map[string]any {
	t.Helper()
	result, err := runApply(root, rules, patch, false, io.Discard)
	if err != nil {
		t.Fatalf("apply %v: %v", rules, err)
	}
	return result
}

// redTestPatch is the canonical witnessed-failing test for RULE-001: it
// expects src/auth.txt, which no implementation provides yet.
func redTestPatch() []byte {
	return addPatch("tests/auth_test.txt", []string{"telos: RULE-001", "asserts RULE-001", "expect src/auth.txt"})
}

func implPatch() []byte {
	return addPatch("src/auth.txt", []string{"telos: RULE-001", "content"})
}

// proveRule001 walks RULE-001 through the full witnessed cycle: red test-only
// patch, then the implementation the test expects.
func proveRule001(t *testing.T, root string) {
	t.Helper()
	apply(t, root, []string{"RULE-001"}, redTestPatch())
	apply(t, root, []string{"RULE-001"}, implPatch())
}

func writeSpecFile(t *testing.T, root, rel, content string) {
	t.Helper()
	if _, err := specPut(root, rel, []byte(content), false); err != nil {
		t.Fatalf("spec put %s: %v", rel, err)
	}
}

const productBody = `# Product

## Objectives

### OBJ-001 — Locked accounts stay out

A locked account can never authenticate.
`

const authBody = "# Authentication\n\n### RULE-001 — Deny locked account sign-in\n\nTraces: OBJ-001\n\nA locked account is denied authentication.\n\n```gherkin\nScenario: locked account is denied\n  Given a locked account\n  When it signs in\n  Then access is denied\n```\n"

// approveSpec drafts, reviews, and approves the standard two-file spec.
func approveSpec(t *testing.T, root string) {
	t.Helper()
	writeSpecFile(t, root, productFile, productBody)
	writeSpecFile(t, root, "spec/auth.md", authBody)
	review, err := specReview(root)
	if err != nil {
		t.Fatalf("spec review: %v", err)
	}
	if _, err := specApprove(root, review["digest"].(string)); err != nil {
		t.Fatalf("spec approve: %v", err)
	}
}

func addPatch(path string, lines []string) []byte {
	var b strings.Builder
	fmt.Fprintf(&b, "diff --git a/%s b/%s\n", path, path)
	b.WriteString("new file mode 100644\n--- /dev/null\n")
	fmt.Fprintf(&b, "+++ b/%s\n@@ -0,0 +1,%d @@\n", path, len(lines))
	for _, line := range lines {
		b.WriteString("+" + line + "\n")
	}
	return []byte(b.String())
}

func errCode(t *testing.T, err error) string {
	t.Helper()
	var commandErr *commandError
	if !errors.As(err, &commandErr) {
		t.Fatalf("error = %v, want a coded command error", err)
	}
	return commandErr.Code
}

func TestNormalizeAndRootHashAreDeterministic(t *testing.T) {
	if got := string(normalize([]byte("a\r\nb\rc\n"))); got != "a\nb\nc\n" {
		t.Fatalf("normalize = %q", got)
	}
	a := map[string]string{"z/file": "2", "a/file": "1"}
	b := map[string]string{"a/file": "1", "z/file": "2"}
	if rootHashMap(a) != rootHashMap(b) {
		t.Fatal("root hash depends on map ordering")
	}
	if rootHashMap(a) == rootHashMap(map[string]string{"a/file": "1"}) {
		t.Fatal("root hash ignores files")
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
		t.Fatalf("atomic write content = %q", data)
	}
}

func TestGlobMatch(t *testing.T) {
	cases := []struct {
		pattern, rel string
		want         bool
	}{
		{"**/*_test.go", "a/b/x_test.go", true},
		{"**/*_test.go", "x_test.go", true},
		{"*_test.go", "x_test.go", true},
		{"*_test.go", "a/x_test.go", false},
		{".github/**", ".github/workflows/ci.yml", true},
		{".github/**", ".githubx/ci.yml", false},
		{"README.md", "README.md", true},
		{"README.md", "docs/README.md", false},
		{"spec/*.md", "spec/auth.md", true},
		{"spec/*.md", "spec/sub/auth.md", false},
		{"tests/**", "tests/deep/nested/file.txt", true},
	}
	for _, c := range cases {
		if got := globMatch(c.pattern, c.rel); got != c.want {
			t.Errorf("globMatch(%q, %q) = %v, want %v", c.pattern, c.rel, got, c.want)
		}
	}
}

func TestReadConfigMultilineAndComments(t *testing.T) {
	root := t.TempDir()
	cfg := `# header comment
agents = ["claude", "codex"]
test_commands = [
  "go version",
  "go vet ./...",
] # trailing comment
test_files = ["**/*_test.go"]
untraced = ["README.md"]
`
	if err := os.WriteFile(filepath.Join(root, configFile), []byte(cfg), 0o644); err != nil {
		t.Fatal(err)
	}
	got, err := readConfig(root)
	if err != nil {
		t.Fatal(err)
	}
	if len(got.Agents) != 2 || got.Agents[0] != "claude" {
		t.Fatalf("agents = %v", got.Agents)
	}
	if len(got.TestCommands) != 2 || got.TestCommands[1] != "go vet ./..." {
		t.Fatalf("test_commands = %v", got.TestCommands)
	}
	if len(got.TestFiles) != 1 || len(got.Untraced) != 1 {
		t.Fatalf("test_files = %v, untraced = %v", got.TestFiles, got.Untraced)
	}
}

func TestReadConfigRejectsUnknownKeys(t *testing.T) {
	root := t.TempDir()
	cfg := "agents = [\"claude\"]\ninfra = [\"README.md\"]\n"
	if err := os.WriteFile(filepath.Join(root, configFile), []byte(cfg), 0o644); err != nil {
		t.Fatal(err)
	}
	_, err := readConfig(root)
	if errCode(t, err) != "TELOS_CONFIG_INVALID" {
		t.Fatalf("unknown key must fail config parsing, got %v", err)
	}
	if !strings.Contains(err.Error(), `"infra"`) || !strings.Contains(err.Error(), "untraced") {
		t.Fatalf("error must name the bad key and the valid ones, got %v", err)
	}
}

func TestFileAnnotations(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "a.go")
	if err := os.WriteFile(path, []byte("// header\r\n// telos: RULE-001, RULE-002\r\npackage a\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	ids, found, err := fileAnnotations(path)
	if err != nil || !found {
		t.Fatalf("annotations not found: %v", err)
	}
	if len(ids) != 2 || ids[0] != "RULE-001" || ids[1] != "RULE-002" {
		t.Fatalf("ids = %v", ids)
	}
	deep := strings.Repeat("filler\n", annotationScanLines) + "telos: RULE-001\n"
	if err := os.WriteFile(path, []byte(deep), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, found, _ := fileAnnotations(path); found {
		t.Fatal("annotation beyond the scan window should not count")
	}
}

func TestLoadSpecValidation(t *testing.T) {
	root := gitRepo(t)
	write := func(rel, content string) {
		t.Helper()
		path := filepath.Join(root, filepath.FromSlash(rel))
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	specFiles := func() map[string]string {
		_, spec, err := inventories(root)
		if err != nil {
			t.Fatal(err)
		}
		return spec
	}

	if _, problems := loadSpec(root, specFiles()); len(problems) != 0 {
		t.Fatalf("empty spec should be valid, got %v", problems)
	}

	write(productFile, productBody)
	write("spec/auth.md", authBody)
	if _, problems := loadSpec(root, specFiles()); len(problems) != 0 {
		t.Fatalf("valid spec reported problems: %v", problems)
	}

	write("spec/dup.md", authBody)
	if _, problems := loadSpec(root, specFiles()); len(problems) == 0 || !strings.Contains(strings.Join(problems, "|"), "duplicate rule RULE-001") {
		t.Fatalf("duplicate rule not detected: %v", problems)
	}
	if err := os.Remove(filepath.Join(root, "spec", "dup.md")); err != nil {
		t.Fatal(err)
	}

	write("spec/bad.md", "# Bad\n\n### RULE-002 — No trace no gherkin\n\nBody only.\n")
	_, problems := loadSpec(root, specFiles())
	joined := strings.Join(problems, "|")
	if !strings.Contains(joined, "RULE-002 is missing a `Traces: OBJ-NNN` line") || !strings.Contains(joined, "RULE-002 is missing a ```gherkin scenario block") {
		t.Fatalf("missing trace/gherkin not detected: %v", problems)
	}
	write("spec/bad.md", "# Bad\n\n### RULE-002 — Ghost trace\n\nTraces: OBJ-999\n\n```gherkin\nScenario: x\n```\n")
	if _, problems := loadSpec(root, specFiles()); !strings.Contains(strings.Join(problems, "|"), "traces unknown objective OBJ-999") {
		t.Fatalf("unknown objective not detected: %v", problems)
	}
	if err := os.Remove(filepath.Join(root, "spec", "bad.md")); err != nil {
		t.Fatal(err)
	}

	write("spec/rules-in-product.md", "")
	write(productFile, productBody+"\n### RULE-009 — Misplaced\n")
	if _, problems := loadSpec(root, specFiles()); !strings.Contains(strings.Join(problems, "|"), "RULE sections belong in spec domain files") {
		t.Fatalf("rule in PRODUCT.md not detected: %v", problems)
	}
	write(productFile, productBody)
	write("spec/rules-in-product.md", "### OBJ-002 — Misplaced objective\n")
	if _, problems := loadSpec(root, specFiles()); !strings.Contains(strings.Join(problems, "|"), "OBJ sections belong in spec/PRODUCT.md") {
		t.Fatalf("objective outside PRODUCT.md not detected: %v", problems)
	}
	if err := os.Remove(filepath.Join(root, "spec", "rules-in-product.md")); err != nil {
		t.Fatal(err)
	}

	write("spec/binary.txt", "not markdown")
	if _, problems := loadSpec(root, specFiles()); !strings.Contains(strings.Join(problems, "|"), "only Markdown files are allowed under spec/") {
		t.Fatalf("non-markdown spec file not detected: %v", problems)
	}
}

func TestInventoriesPartitionAndExclusions(t *testing.T) {
	root := gitRepo(t)
	initTelos(t, root)
	if err := os.WriteFile(filepath.Join(root, "app.txt"), []byte("code\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	code, spec, err := inventories(root)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := code["app.txt"]; !ok {
		t.Fatal("code inventory misses app.txt")
	}
	if _, ok := spec["spec/PRODUCT.md"]; !ok {
		t.Fatal("spec inventory misses spec/PRODUCT.md")
	}
	for rel := range code {
		if rel == configFile || strings.HasPrefix(rel, ".telos/") || strings.HasPrefix(rel, "spec/") {
			t.Fatalf("code inventory contains excluded path %s", rel)
		}
	}
	for rel := range spec {
		if !strings.HasPrefix(rel, "spec/") {
			t.Fatalf("spec inventory contains foreign path %s", rel)
		}
	}
}

func TestSpecPutValidatesPaths(t *testing.T) {
	root := gitRepo(t)
	initTelos(t, root)
	for _, bad := range []string{"", "notes.md", "spec/../escape.md", "spec/plain.txt", "/abs/spec/x.md"} {
		if _, err := specPut(root, bad, []byte("x"), false); err == nil {
			t.Fatalf("specPut accepted %q", bad)
		}
	}
	if _, err := specPut(root, "spec/domains/auth.md", []byte("# ok\n"), false); err != nil {
		t.Fatalf("nested spec file rejected: %v", err)
	}
}

func TestReviewDigestBecomesStaleAfterSpecChange(t *testing.T) {
	root := gitRepo(t)
	initTelos(t, root)
	writeTestConfig(t, root, []string{"go version"})
	writeSpecFile(t, root, productFile, productBody)
	writeSpecFile(t, root, "spec/auth.md", authBody)
	review, err := specReview(root)
	if err != nil {
		t.Fatal(err)
	}
	digest := review["digest"].(string)
	writeSpecFile(t, root, "spec/auth.md", authBody+"\nMore prose.\n")
	if _, err := specApprove(root, digest); errCode(t, err) != "TELOS_APPROVAL_STALE" {
		t.Fatalf("approve after mutation = %v, want TELOS_APPROVAL_STALE", err)
	}
	review, err = specReview(root)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := specApprove(root, review["digest"].(string)); err != nil {
		t.Fatalf("fresh approve failed: %v", err)
	}
	if _, err := specReview(root); errCode(t, err) != "TELOS_NOTHING_PENDING" {
		t.Fatalf("review with clean spec = %v, want TELOS_NOTHING_PENDING", err)
	}
}

func TestSpecReviewRejectsInvalidSpec(t *testing.T) {
	root := gitRepo(t)
	initTelos(t, root)
	writeSpecFile(t, root, "spec/auth.md", "# Auth\n\n### RULE-001 — Broken\n\nNo trace.\n")
	if _, err := specReview(root); errCode(t, err) != "TELOS_SPEC_INVALID" {
		t.Fatalf("review of invalid spec = %v, want TELOS_SPEC_INVALID", err)
	}
}

func TestApplyEnforcesAnnotationIntersection(t *testing.T) {
	root := gitRepo(t)
	writeProbe(t, root)
	initTelos(t, root)
	writeTestConfig(t, root, []string{probeCommand})
	approveSpec(t, root)

	if _, err := runApply(root, []string{"RULE-001"}, addPatch("tests/auth_test.txt", []string{"no annotation", "asserts RULE-001", "expect src/auth.txt"}), false, io.Discard); errCode(t, err) != "TELOS_ANNOTATION_MISMATCH" {
		t.Fatal("un-annotated created file must be rejected")
	}
	if _, err := os.Stat(filepath.Join(root, "tests", "auth_test.txt")); !os.IsNotExist(err) {
		t.Fatal("rejected patch was not reversed")
	}

	if _, err := runApply(root, []string{"RULE-001"}, addPatch("tests/auth_test.txt", []string{"telos: RULE-999", "asserts RULE-001", "expect src/auth.txt"}), false, io.Discard); errCode(t, err) != "TELOS_ANNOTATION_MISMATCH" {
		t.Fatal("annotation referencing an unknown rule must be rejected")
	}

	apply(t, root, []string{"RULE-001"}, redTestPatch())
	if _, err := runApply(root, []string{"RULE-001"}, addPatch("tools/tool.cfg", []string{"anything"}), false, io.Discard); err != nil {
		t.Fatalf("untraced file should not need an annotation: %v", err)
	}
	if _, err := runApply(root, []string{"RULE-001"}, implPatch(), false, io.Discard); err != nil {
		t.Fatalf("valid annotated patch rejected: %v", err)
	}

	if _, err := runApply(root, []string{"RULE-999"}, addPatch("src/other.txt", []string{"telos: RULE-999"}), false, io.Discard); errCode(t, err) != "TELOS_TRACEABILITY_GAP" {
		t.Fatal("citing an unknown rule must fail")
	}
	for _, target := range []string{"spec/evil.md", ".telos/state.json", ".claude/settings.json", "telos.toml", "CLAUDE.md"} {
		if _, err := runApply(root, []string{"RULE-001"}, addPatch(target, []string{"x"}), false, io.Discard); errCode(t, err) != "TELOS_INPUT_INVALID" {
			t.Fatalf("patch touching %s must be rejected", target)
		}
	}
}

func TestApplyRequiresCleanAndApprovedTrees(t *testing.T) {
	root := gitRepo(t)
	initTelos(t, root)
	writeTestConfig(t, root, []string{"go version"})
	approveSpec(t, root)
	patch := addPatch("src/auth.txt", []string{"telos: RULE-001"})

	writeSpecFile(t, root, "spec/auth.md", authBody+"\nPending edit.\n")
	if _, err := runApply(root, []string{"RULE-001"}, patch, false, io.Discard); errCode(t, err) != "TELOS_SPEC_UNAPPROVED" {
		t.Fatal("apply with pending spec must fail")
	}
	review, err := specReview(root)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := specApprove(root, review["digest"].(string)); err != nil {
		t.Fatal(err)
	}

	if err := os.WriteFile(filepath.Join(root, "rogue.txt"), []byte("out of band\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := runApply(root, []string{"RULE-001"}, patch, false, io.Discard); errCode(t, err) != "TELOS_CODE_CORRUPTED" {
		t.Fatal("apply with out-of-band code must fail")
	}
}

func TestVerifyPipelineErrorCodes(t *testing.T) {
	root := gitRepo(t)
	writeProbe(t, root)
	initTelos(t, root)
	writeTestConfig(t, root, []string{probeCommand})
	approveSpec(t, root)
	var out bytes.Buffer

	if _, err := runVerify(root, &out, &out); errCode(t, err) != "TELOS_RULE_NOT_IMPLEMENTED" {
		t.Fatal("rule without tagged test must fail verify")
	}

	apply(t, root, []string{"RULE-001"}, redTestPatch())
	if _, err := runVerify(root, &out, &out); errCode(t, err) != "TELOS_RED_PENDING" {
		t.Fatal("witnessed red without its green witness must fail verify")
	}
	apply(t, root, []string{"RULE-001"}, implPatch())
	if result, err := runVerify(root, &out, &out); err != nil {
		t.Fatalf("verify should pass: %v", err)
	} else if result["rules"].(int) != 1 {
		t.Fatalf("verify result = %v", result)
	}

	writeTestConfig(t, root, nil)
	if _, err := runVerify(root, &out, &out); errCode(t, err) != "TELOS_CONFIG_INVALID" {
		t.Fatal("rules without test_commands must fail verify")
	}
	noTestFiles := "agents = [\"claude\"]\n" +
		"test_commands = [\"go version\"]\n" +
		"test_files = []\n" +
		"untraced = [\"README.md\", \".claude/**\", \".agents/**\", \".codex/**\", \".github/**\", \"CLAUDE.md\", \"AGENTS.md\", \"tools/**\"]\n"
	if err := os.WriteFile(filepath.Join(root, configFile), []byte(noTestFiles), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := runVerify(root, &out, &out); errCode(t, err) != "TELOS_CONFIG_INVALID" {
		t.Fatal("rules without test_files must fail verify")
	}
	writeTestConfig(t, root, []string{"telos-definitely-missing-command"})
	if _, err := runVerify(root, &out, &out); errCode(t, err) != "TELOS_TESTS_FAILED" {
		t.Fatal("failing test command must fail verify")
	}
	writeTestConfig(t, root, []string{probeCommand})

	if err := os.WriteFile(filepath.Join(root, "tests", "auth_test.txt"), []byte("tampered\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := runVerify(root, &out, &out); errCode(t, err) != "TELOS_CODE_CORRUPTED" {
		t.Fatal("out-of-band code edit must corrupt the project")
	}
}

func TestVerifyDetectsAnnotationGaps(t *testing.T) {
	root := gitRepo(t)
	initTelos(t, root)
	writeTestConfig(t, root, []string{"go version"})
	approveSpec(t, root)
	var out bytes.Buffer

	if err := os.WriteFile(filepath.Join(root, "loose.txt"), []byte("no annotation\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	code, spec, err := inventories(root)
	if err != nil {
		t.Fatal(err)
	}
	st, err := loadState(root)
	if err != nil {
		t.Fatal(err)
	}
	st.Code, st.Spec = snapshotOf(code), snapshotOf(spec)
	if err := saveState(root, st); err != nil {
		t.Fatal(err)
	}
	if _, err := runVerify(root, &out, &out); errCode(t, err) != "TELOS_ANNOTATION_MISSING" {
		t.Fatal("un-annotated file must fail verify")
	}

	if err := os.WriteFile(filepath.Join(root, "loose.txt"), []byte("telos: RULE-999\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	code, _, err = inventories(root)
	if err != nil {
		t.Fatal(err)
	}
	st.Code = snapshotOf(code)
	if err := saveState(root, st); err != nil {
		t.Fatal(err)
	}
	if _, err := runVerify(root, &out, &out); errCode(t, err) != "TELOS_ANNOTATION_ORPHAN" {
		t.Fatal("orphan annotation must fail verify")
	}
}

func TestVerifyReportsUnapprovedSpecForAdoption(t *testing.T) {
	root := gitRepo(t)
	initTelos(t, root)
	writeTestConfig(t, root, []string{"go version"})
	var out bytes.Buffer
	if _, err := runVerify(root, &out, &out); err != nil {
		t.Fatalf("fresh project should verify: %v", err)
	}
	if err := os.WriteFile(filepath.Join(root, "spec", "manual.md"), []byte("# Manual edit\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := runVerify(root, &out, &out); errCode(t, err) != "TELOS_SPEC_UNAPPROVED" {
		t.Fatal("human spec edit must surface as unapproved, not corrupted")
	}
}

func TestStatusPhases(t *testing.T) {
	root := gitRepo(t)
	writeProbe(t, root)
	initTelos(t, root)
	writeTestConfig(t, root, []string{probeCommand})
	phase := func() string {
		result, _, err := runStatus(root)
		if err != nil {
			t.Fatal(err)
		}
		return result["phase"].(string)
	}
	if got := phase(); got != "clean" {
		t.Fatalf("fresh phase = %s", got)
	}
	writeSpecFile(t, root, productFile, productBody)
	writeSpecFile(t, root, "spec/auth.md", authBody)
	if got := phase(); got != "spec_pending" {
		t.Fatalf("after edit phase = %s", got)
	}
	review, err := specReview(root)
	if err != nil {
		t.Fatal(err)
	}
	if got := phase(); got != "awaiting_approval" {
		t.Fatalf("after review phase = %s", got)
	}
	if _, err := specApprove(root, review["digest"].(string)); err != nil {
		t.Fatal(err)
	}
	if got := phase(); got != "implementing" {
		t.Fatalf("after approve phase = %s", got)
	}
	apply(t, root, []string{"RULE-001"}, redTestPatch())
	if got := phase(); got != "implementing" {
		t.Fatalf("with red evidence pending phase = %s, want implementing", got)
	}
	apply(t, root, []string{"RULE-001"}, implPatch())
	if got := phase(); got != "clean" {
		t.Fatalf("after implementation phase = %s", got)
	}
	if err := os.WriteFile(filepath.Join(root, "rogue.txt"), []byte("x\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if got := phase(); got != "corrupted" {
		t.Fatalf("after out-of-band edit phase = %s", got)
	}
}

func TestTraceMapsRulesToFilesAndTests(t *testing.T) {
	root := gitRepo(t)
	writeProbe(t, root)
	initTelos(t, root)
	writeTestConfig(t, root, []string{probeCommand})
	approveSpec(t, root)
	proveRule001(t, root)
	result, err := runTrace(root, "RULE-001")
	if err != nil {
		t.Fatal(err)
	}
	entry := result.(map[string]any)
	if files := fmt.Sprint(entry["files"]); !strings.Contains(files, "src/auth.txt") {
		t.Fatalf("trace files = %v", files)
	}
	if tests := fmt.Sprint(entry["tests"]); !strings.Contains(tests, "tests/auth_test.txt") {
		t.Fatalf("trace tests = %v", tests)
	}
	if _, err := runTrace(root, "RULE-404"); errCode(t, err) != "TELOS_INPUT_INVALID" {
		t.Fatal("unknown rule must be rejected")
	}
}

func guardDecision(t *testing.T, root, toolName string, toolInput map[string]any) string {
	t.Helper()
	input, err := json.Marshal(map[string]any{"cwd": root, "tool_name": toolName, "tool_input": toolInput})
	if err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := runGuard(bytes.NewReader(input), &out); err != nil {
		t.Fatalf("guard: %v", err)
	}
	if out.Len() == 0 {
		return "allow"
	}
	var response struct {
		HookSpecificOutput struct {
			PermissionDecision string `json:"permissionDecision"`
		} `json:"hookSpecificOutput"`
	}
	if err := json.Unmarshal(out.Bytes(), &response); err != nil {
		t.Fatal(err)
	}
	return response.HookSpecificOutput.PermissionDecision
}

func TestGuardDecisions(t *testing.T) {
	root := gitRepo(t)
	initTelos(t, root)
	writeTestConfig(t, root, []string{"go version"})

	if got := guardDecision(t, root, "Edit", map[string]any{"file_path": "x"}); got != "deny" {
		t.Fatalf("Edit = %s, want deny", got)
	}
	if got := guardDecision(t, root, "Write", nil); got != "deny" {
		t.Fatalf("Write = %s, want deny", got)
	}
	if got := guardDecision(t, root, "Bash", map[string]any{"command": "rm -rf ."}); got != "deny" {
		t.Fatalf("non-broker bash = %s, want deny", got)
	}
	if got := guardDecision(t, root, "Bash", map[string]any{"command": "telos status --json"}); got != "allow" {
		t.Fatalf("broker status = %s, want allow", got)
	}
	if got := guardDecision(t, root, "Bash", map[string]any{"command": "telos verify --json && rm -rf ."}); got != "deny" {
		t.Fatalf("chained broker command = %s, want deny", got)
	}
	if got := guardDecision(t, root, "Bash", map[string]any{"command": "telos init --agent all"}); got != "ask" {
		t.Fatalf("re-init = %s, want ask", got)
	}
	outside := t.TempDir()
	if got := guardDecision(t, outside, "Bash", map[string]any{"command": "telos init"}); got != "allow" {
		t.Fatalf("first init outside a project = %s, want allow", got)
	}

	if got := guardDecision(t, root, "Bash", map[string]any{"command": "telos spec approve --review deadbeef"}); got != "deny" {
		t.Fatalf("approve without review = %s, want deny", got)
	}
	writeSpecFile(t, root, productFile, productBody)
	writeSpecFile(t, root, "spec/auth.md", authBody)
	review, err := specReview(root)
	if err != nil {
		t.Fatal(err)
	}
	digest := review["digest"].(string)
	if got := guardDecision(t, root, "Bash", map[string]any{"command": "telos spec approve --review " + digest}); got != "ask" {
		t.Fatalf("fresh approve = %s, want ask", got)
	}
	writeSpecFile(t, root, "spec/auth.md", authBody+"\nDrift.\n")
	if got := guardDecision(t, root, "Bash", map[string]any{"command": "telos spec approve --review " + digest}); got != "deny" {
		t.Fatalf("stale approve = %s, want deny", got)
	}
}

func TestGuardGatesApplyOnCleanProject(t *testing.T) {
	root := gitRepo(t)
	writeProbe(t, root)
	initTelos(t, root)
	writeTestConfig(t, root, []string{probeCommand})
	approveSpec(t, root)

	applyCommand := "telos apply --rule RULE-001 --json <<'EOF'\ndiff --git a/src/auth.txt b/src/auth.txt\nEOF"
	adoptCommand := "telos apply --rule RULE-001 --expect-pass --json <<'EOF'\ndiff --git a/tests/auth_test.txt b/tests/auth_test.txt\nEOF"
	if got := guardDecision(t, root, "Bash", map[string]any{"command": applyCommand}); got != "allow" {
		t.Fatalf("apply while implementing = %s, want allow", got)
	}
	if got := guardDecision(t, root, "Bash", map[string]any{"command": adoptCommand}); got != "ask" {
		t.Fatalf("apply --expect-pass while implementing = %s, want ask", got)
	}
	proveRule001(t, root)
	if got := guardDecision(t, root, "Bash", map[string]any{"command": applyCommand}); got != "ask" {
		t.Fatalf("apply on a clean project = %s, want ask", got)
	}
	if got := guardDecision(t, root, "Bash", map[string]any{"command": adoptCommand}); got != "allow" {
		t.Fatalf("apply --expect-pass on a clean project = %s, want silence (the command fails precisely)", got)
	}
}

func TestApplyWitnessesRedThenGreen(t *testing.T) {
	root := gitRepo(t)
	writeProbe(t, root)
	initTelos(t, root)
	writeTestConfig(t, root, []string{probeCommand})
	approveSpec(t, root)

	if _, err := runApply(root, []string{"RULE-001"}, implPatch(), false, io.Discard); errCode(t, err) != "TELOS_TEST_FIRST" {
		t.Fatal("implementation before the witnessed failing test must be rejected")
	}
	if _, err := os.Stat(filepath.Join(root, "src", "auth.txt")); !os.IsNotExist(err) {
		t.Fatal("test-first rejection must leave the tree untouched")
	}

	passing := addPatch("tests/auth_test.txt", []string{"telos: RULE-001", "asserts RULE-001", "expect tools/probe.go"})
	if _, err := runApply(root, []string{"RULE-001"}, passing, false, io.Discard); errCode(t, err) != "TELOS_RED_EXPECTED" {
		t.Fatal("a test the suite already passes is no evidence and must be rejected")
	}
	if _, err := os.Stat(filepath.Join(root, "tests", "auth_test.txt")); !os.IsNotExist(err) {
		t.Fatal("rejected passing test was not reversed")
	}

	result, err := runApply(root, []string{"RULE-001"}, redTestPatch(), false, io.Discard)
	if err != nil {
		t.Fatalf("witnessed red rejected: %v", err)
	}
	if result["suite"] != "red" {
		t.Fatalf("red apply result = %v", result)
	}
	st, err := loadState(root)
	if err != nil {
		t.Fatal(err)
	}
	ev, ok := st.Red["RULE-001"]
	if !ok {
		t.Fatal("red evidence was not recorded")
	}
	if _, sealed := ev.Tests["tests/auth_test.txt"]; !sealed {
		t.Fatalf("red evidence files = %v", ev.Tests)
	}

	result, err = runApply(root, []string{"RULE-001"}, implPatch(), false, io.Discard)
	if err != nil {
		t.Fatalf("implementation apply failed: %v", err)
	}
	if result["suite"] != "green" || fmt.Sprint(result["proven"]) != "[RULE-001]" {
		t.Fatalf("green apply result = %v", result)
	}
	st, err = loadState(root)
	if err != nil {
		t.Fatal(err)
	}
	if len(st.Red) != 0 {
		t.Fatalf("red evidence survived the green witness: %v", st.Red)
	}
	if st.Green != st.Code.Root {
		t.Fatal("the witnessed green root was not recorded")
	}
}

func TestApplySealsRedTests(t *testing.T) {
	root := gitRepo(t)
	writeProbe(t, root)
	initTelos(t, root)
	writeTestConfig(t, root, []string{probeCommand})
	approveSpec(t, root)
	apply(t, root, []string{"RULE-001"}, redTestPatch())

	weaken := "diff --git a/tests/auth_test.txt b/tests/auth_test.txt\n--- a/tests/auth_test.txt\n+++ b/tests/auth_test.txt\n@@ -1,3 +1,3 @@\n telos: RULE-001\n asserts RULE-001\n-expect src/auth.txt\n+expect tools/probe.go\n"
	mixed := weaken + "diff --git a/src/other.txt b/src/other.txt\nnew file mode 100644\n--- /dev/null\n+++ b/src/other.txt\n@@ -0,0 +1,1 @@\n+telos: RULE-001\n"
	if _, err := runApply(root, []string{"RULE-001"}, []byte(mixed), false, io.Discard); errCode(t, err) != "TELOS_TEST_SEALED" {
		t.Fatal("a mixed patch touching a sealed test must be rejected")
	}
	if _, err := runApply(root, []string{"RULE-001"}, []byte(weaken), false, io.Discard); errCode(t, err) != "TELOS_RED_EXPECTED" {
		t.Fatal("rewriting a sealed test into a passing one must be rejected")
	}

	rewrite := "diff --git a/tests/auth_test.txt b/tests/auth_test.txt\n--- a/tests/auth_test.txt\n+++ b/tests/auth_test.txt\n@@ -1,3 +1,3 @@\n telos: RULE-001\n asserts RULE-001\n-expect src/auth.txt\n+expect src/auth_v2.txt\n"
	if _, err := runApply(root, []string{"RULE-001"}, []byte(rewrite), false, io.Discard); err != nil {
		t.Fatalf("rewriting a sealed test back through red must be allowed: %v", err)
	}
	st, err := loadState(root)
	if err != nil {
		t.Fatal(err)
	}
	code, _, err := inventories(root)
	if err != nil {
		t.Fatal(err)
	}
	if st.Red["RULE-001"].Tests["tests/auth_test.txt"] != code["tests/auth_test.txt"] {
		t.Fatal("re-red did not re-seal the rewritten test bytes")
	}

	apply(t, root, []string{"RULE-001"}, addPatch("src/auth_v2.txt", []string{"telos: RULE-001", "content"}))
	if st, err = loadState(root); err != nil || len(st.Red) != 0 {
		t.Fatalf("cycle did not complete: %v %v", st.Red, err)
	}
}

func TestApplyExpectPassAdoptsExistingBehavior(t *testing.T) {
	root := gitRepo(t)
	writeProbe(t, root)
	initTelos(t, root)
	writeTestConfig(t, root, []string{probeCommand})
	approveSpec(t, root)

	adoption := addPatch("tests/auth_test.txt", []string{"telos: RULE-001", "asserts RULE-001", "expect tools/probe.go"})
	if _, err := runApply(root, []string{"RULE-001"}, addPatch("src/auth.txt", []string{"telos: RULE-001"}), true, io.Discard); errCode(t, err) != "TELOS_INPUT_INVALID" {
		t.Fatal("--expect-pass must submit documentation tests only")
	}
	failing := redTestPatch()
	if _, err := runApply(root, []string{"RULE-001"}, failing, true, io.Discard); errCode(t, err) != "TELOS_TESTS_FAILED" {
		t.Fatal("--expect-pass with a failing test contradicts the adoption claim")
	}
	result, err := runApply(root, []string{"RULE-001"}, adoption, true, io.Discard)
	if err != nil {
		t.Fatalf("adoption apply failed: %v", err)
	}
	if result["suite"] != "green" || fmt.Sprint(result["proven"]) != "[RULE-001]" {
		t.Fatalf("adoption result = %v", result)
	}
	st, err := loadState(root)
	if err != nil {
		t.Fatal(err)
	}
	if len(st.Red) != 0 {
		t.Fatal("adoption must not record red evidence")
	}
	if _, err := runApply(root, []string{"RULE-001"}, addPatch("tests/extra_test.txt", []string{"telos: RULE-001", "expect tools/probe.go"}), true, io.Discard); errCode(t, err) != "TELOS_INPUT_INVALID" {
		t.Fatal("--expect-pass citing an already-referenced rule must be rejected")
	}
}

func TestApplyRequiresGreenBaselineForNewRedTests(t *testing.T) {
	root := gitRepo(t)
	writeProbe(t, root)
	initTelos(t, root)
	writeTestConfig(t, root, []string{probeCommand})
	auditBody := "# Audit\n\n### RULE-002 — Log denied attempts\n\nTraces: OBJ-001\n\nDenied attempts are logged.\n\n```gherkin\nScenario: denied attempt is logged\n  Given a locked account\n  When it signs in\n  Then the attempt is logged\n```\n"
	writeSpecFile(t, root, productFile, productBody)
	writeSpecFile(t, root, "spec/auth.md", authBody)
	writeSpecFile(t, root, "spec/audit.md", auditBody)
	review, err := specReview(root)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := specApprove(root, review["digest"].(string)); err != nil {
		t.Fatal(err)
	}

	apply(t, root, []string{"RULE-001"}, redTestPatch())
	second := addPatch("tests/audit_test.txt", []string{"telos: RULE-002", "asserts RULE-002", "expect src/audit.txt"})
	if _, err := runApply(root, []string{"RULE-002"}, second, false, io.Discard); errCode(t, err) != "TELOS_BASELINE_RED" {
		t.Fatal("a second red test on a red suite is unattributable and must be rejected")
	}
	apply(t, root, []string{"RULE-001"}, implPatch())
	if _, err := runApply(root, []string{"RULE-002"}, second, false, io.Discard); err != nil {
		t.Fatalf("red test on a green baseline rejected: %v", err)
	}
}

func TestApplyRejectsUncitedTestReferences(t *testing.T) {
	root := gitRepo(t)
	writeProbe(t, root)
	initTelos(t, root)
	writeTestConfig(t, root, []string{probeCommand})
	approveSpec(t, root)
	apply(t, root, []string{"RULE-001"}, redTestPatch())

	smuggled := addPatch("tests/extra_test.txt", []string{"telos: RULE-001", "mentions RULE-002 and RULE-003", "expect src/never.txt"})
	if _, err := runApply(root, []string{"RULE-001"}, smuggled, false, io.Discard); errCode(t, err) != "TELOS_TEST_FIRST" {
		t.Fatal("test references outside the witnessed cycle must be rejected")
	}
	if _, err := os.Stat(filepath.Join(root, "tests", "extra_test.txt")); !os.IsNotExist(err) {
		t.Fatal("rejected smuggling patch was not reversed")
	}
}

func TestSpecApproveSweepsOrphanRedEvidence(t *testing.T) {
	root := gitRepo(t)
	writeProbe(t, root)
	initTelos(t, root)
	writeTestConfig(t, root, []string{probeCommand})
	approveSpec(t, root)
	apply(t, root, []string{"RULE-001"}, redTestPatch())

	writeSpecFile(t, root, "spec/auth.md", "# Authentication\n\nNo rules remain here.\n")
	review, err := specReview(root)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := specApprove(root, review["digest"].(string)); err != nil {
		t.Fatal(err)
	}
	st, err := loadState(root)
	if err != nil {
		t.Fatal(err)
	}
	if len(st.Red) != 0 {
		t.Fatalf("red evidence for a deleted rule must be swept: %v", st.Red)
	}
}

func TestVerifyDetectsTestCommandsMutatingSources(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("shell redirection differs on Windows")
	}
	root := gitRepo(t)
	writeProbe(t, root)
	initTelos(t, root)
	writeTestConfig(t, root, []string{probeCommand})
	approveSpec(t, root)
	proveRule001(t, root)
	writeTestConfig(t, root, []string{"echo drift >> tests/auth_test.txt"})
	var out bytes.Buffer
	if _, err := runVerify(root, &out, &out); errCode(t, err) != "TELOS_CODE_CORRUPTED" {
		t.Fatal("a test command mutating tracked files must corrupt the project")
	}
}

func TestInitIsIdempotentAndPreservesConfig(t *testing.T) {
	root := gitRepo(t)
	initTelos(t, root)
	custom := "# my custom notes\nagents = [\"claude\"]\ntest_commands = [\"go version\"]\ntest_files = []\nuntraced = [\".claude/**\", \"CLAUDE.md\"]\n"
	if err := os.WriteFile(filepath.Join(root, configFile), []byte(custom), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := initProject(root, "codex", false); err != nil {
		t.Fatalf("re-init: %v", err)
	}
	data, err := os.ReadFile(filepath.Join(root, configFile))
	if err != nil {
		t.Fatal(err)
	}
	text := string(data)
	if !strings.Contains(text, "# my custom notes") || !strings.Contains(text, "test_commands = [\"go version\"]") {
		t.Fatalf("re-init clobbered human config:\n%s", text)
	}
	cfg, err := readConfig(root)
	if err != nil {
		t.Fatal(err)
	}
	if len(cfg.Agents) != 2 {
		t.Fatalf("agents after merge = %v", cfg.Agents)
	}
	st, err := loadState(root)
	if err != nil {
		t.Fatal(err)
	}
	code, spec, err := inventories(root)
	if err != nil {
		t.Fatal(err)
	}
	if st.Code.Root != rootHashMap(code) || st.Spec.Root != rootHashMap(spec) {
		t.Fatal("re-init did not re-baseline the declared roots")
	}
}

func TestViewGeneratesSelfContainedEscapedHTML(t *testing.T) {
	root := gitRepo(t)
	writeProbe(t, root)
	initTelos(t, root)
	writeTestConfig(t, root, []string{probeCommand})
	writeSpecFile(t, root, productFile, productBody)
	writeSpecFile(t, root, "spec/auth.md", authBody)
	hostile := "# Web\n\n### RULE-002 — Escape <script>alert(1)</script> everywhere\n\nTraces: OBJ-001\n\nOutput is escaped.\n\n```gherkin\nScenario: escaped\n  Then no <script>alert(1)</script> runs\n```\n"
	writeSpecFile(t, root, "spec/web.md", hostile)
	review, err := specReview(root)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := specApprove(root, review["digest"].(string)); err != nil {
		t.Fatal(err)
	}
	proveRule001(t, root)

	out := filepath.Join(t.TempDir(), "view.html")
	result, err := runView(root, "test", out, false)
	if err != nil {
		t.Fatalf("view: %v", err)
	}
	if result["path"] != out {
		t.Fatalf("view path = %v", result["path"])
	}
	data, err := os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	page := string(data)
	for _, want := range []string{"RULE-001", "RULE-002", "OBJ-001", "Deny locked account sign-in", "proven by tests", "not implemented", "tests/auth_test.txt", "class=\"code gherkin\"", "<span class=\"g-sec\">Scenario:</span>", "<span class=\"g-kw\">Given</span>", "by telos test", "Verification setup", "<code>go run tools/probe.go</code>", "<time id=\"gen\" datetime=\"", "id=\"panel-intent\"", "id=\"panel-contract\""} {
		if !strings.Contains(page, want) {
			t.Fatalf("view page misses %q", want)
		}
	}
	if strings.Contains(page, "<script>alert(1)</script>") {
		t.Fatal("view page did not escape hostile spec content")
	}

	if _, err := runView(root, "test", filepath.Join(root, "spec-view.html"), false); errCode(t, err) != "TELOS_INPUT_INVALID" {
		t.Fatal("non-ignored --out inside the repository must be rejected")
	}
	if err := os.WriteFile(filepath.Join(root, ".gitignore"), []byte("/spec-view.html\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := runView(root, "test", filepath.Join(root, "spec-view.html"), false); err != nil {
		t.Fatalf("git-ignored --out inside the repository should be accepted: %v", err)
	}
}

func TestPatchPathsRejectsTraversalAndSeesDeletions(t *testing.T) {
	if _, err := patchPaths([]byte("--- a/../escape\n+++ b/../escape\n")); err == nil {
		t.Fatal("path traversal accepted")
	}
	paths, err := patchPaths([]byte("diff --git a/gone.txt b/gone.txt\n--- a/gone.txt\n+++ /dev/null\n"))
	if err != nil {
		t.Fatal(err)
	}
	if len(paths) != 1 || paths[0] != "gone.txt" {
		t.Fatalf("deletion paths = %v", paths)
	}
}
