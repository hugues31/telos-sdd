package telos

import (
	"bytes"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/hugues31/telos-sdd/internal/gitx"
	"github.com/hugues31/telos-sdd/internal/kernel"
)

func gitRepo(t *testing.T) string {
	t.Helper()
	root := t.TempDir()
	for _, args := range [][]string{
		{"init", "--quiet", "-b", "main"},
		{"config", "user.email", "telos@test"},
		{"config", "user.name", "telos test"},
	} {
		cmd := exec.Command("git", args...)
		cmd.Dir = root
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("git %v: %v\n%s", args, err, out)
		}
	}
	return root
}

func writeFile(t *testing.T, root, rel, content string) {
	t.Helper()
	path := filepath.Join(root, filepath.FromSlash(rel))
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

// guardDecision feeds one PreToolUse payload to the guard and returns the
// decision: "allow" (silence), "ask", or "deny".
func guardDecision(t *testing.T, cwd, toolName string, toolInput map[string]any) string {
	t.Helper()
	payload := map[string]any{"cwd": cwd, "tool_name": toolName, "tool_input": toolInput}
	raw, err := json.Marshal(payload)
	if err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := runGuard(bytes.NewReader(raw), &out); err != nil {
		t.Fatalf("runGuard: %v", err)
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
		t.Fatalf("guard output is not JSON: %v\n%s", err, out.String())
	}
	return response.HookSpecificOutput.PermissionDecision
}

func TestGuardDecisions(t *testing.T) {
	project := t.TempDir()
	writeFile(t, project, kernel.ConfigFile, "project_id = \"guard\"\nagents = [\"claude\"]\n")
	outside := t.TempDir()

	cases := []struct {
		name, cwd, tool string
		input           map[string]any
		want            string
	}{
		{"edit denied in project", project, "Edit", map[string]any{"file_path": "x.go"}, "deny"},
		{"write denied in project", project, "Write", map[string]any{"file_path": "x.go"}, "deny"},
		{"apply_patch denied in project", project, "apply_patch", map[string]any{}, "deny"},
		{"non-telos shell denied", project, "Bash", map[string]any{"command": "ls -la"}, "deny"},
		{"telos command silent", project, "Bash", map[string]any{"command": "telos status --json"}, "allow"},
		{"telos with chaining denied", project, "Bash", map[string]any{"command": "telos status && rm -rf ."}, "deny"},
		{"re-init asks", project, "Bash", map[string]any{"command": "telos init --agent claude"}, "ask"},
		{"outside a project everything passes", outside, "Edit", map[string]any{"file_path": "x.go"}, "allow"},
		{"unknown tool silent", project, "Read", map[string]any{}, "allow"},
	}
	for _, c := range cases {
		if got := guardDecision(t, c.cwd, c.tool, c.input); got != c.want {
			t.Errorf("%s: decision = %q, want %q", c.name, got, c.want)
		}
	}

	// Malformed input fails open.
	var out bytes.Buffer
	if err := runGuard(strings.NewReader("not json"), &out); err != nil || out.Len() != 0 {
		t.Errorf("malformed guard input must fail open: %v %q", err, out.String())
	}
}

const testProjectConfig = `project_id = "guard-candidate"
agents = ["claude"]
test_commands = []
test_files = ["tests/**"]
`

const testProduct = `# Product

### INT-001 — Greets reliably

The application greets.
`

const testDelta = "<!-- telos:op add file: spec/core.md -->\n" +
	"### REQ-001 — Emit the greeting\nClass: behavior\nMotivated by: INT-001\n\n```gherkin\nScenario: g\n  Given a\n  Then b\n```\n"

func TestGuardCandidateContext(t *testing.T) {
	root := gitRepo(t)
	writeFile(t, root, kernel.ConfigFile, testProjectConfig)
	writeFile(t, root, "spec/PRODUCT.md", testProduct)
	writeFile(t, root, "app.txt", "hello\n")
	repo, err := gitx.Open(root)
	if err != nil {
		t.Fatal(err)
	}
	cfg, err := kernel.ReadConfig(root)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := kernel.Genesis(repo, cfg, kernel.GenesisOptions{Version: "test"}); err != nil {
		t.Fatal(err)
	}
	doc, wt, err := kernel.StartChange(repo, kernel.CategoryBehaviorChange, "guard test")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = repo.WorktreeRemove(wt) })

	abs := func(rel string) string { return filepath.Join(wt, filepath.FromSlash(rel)) }
	cases := []struct {
		name, tool string
		input      map[string]any
		want       string
	}{
		{"code edit allowed", "Edit", map[string]any{"file_path": abs("src/main.go")}, "allow"},
		{"delta edit allowed", "Edit", map[string]any{"file_path": abs("changes/CHG-001/contract.delta.md")}, "allow"},
		{"intent edit allowed", "Write", map[string]any{"file_path": abs("changes/CHG-001/intent.md")}, "allow"},
		{"spec edit denied", "Edit", map[string]any{"file_path": abs("spec/core.md")}, "deny"},
		{"config edit denied", "Edit", map[string]any{"file_path": abs("telos.toml")}, "deny"},
		{"change record denied", "Write", map[string]any{"file_path": abs("changes/CHG-001/change.json")}, "deny"},
		{"evidence dir denied", "Write", map[string]any{"file_path": abs("changes/CHG-001/evidence/EVD-x.json")}, "deny"},
		{"provider assets denied", "Edit", map[string]any{"file_path": abs(".claude/settings.json")}, "deny"},
		{"builds run freely", "Bash", map[string]any{"command": "go test ./..."}, "allow"},
		{"apply_patch denied", "apply_patch", map[string]any{}, "deny"},
		{"abort asks", "Bash", map[string]any{"command": "telos change abort " + doc.ID}, "ask"},
		{"approve without review denied", "Bash", map[string]any{"command": "telos change approve --digest abc"}, "deny"},
	}
	for _, c := range cases {
		if got := guardDecision(t, wt, c.tool, c.input); got != c.want {
			t.Errorf("%s: decision = %q, want %q", c.name, got, c.want)
		}
	}

	// With a recorded review, approve asks on the right digest and denies a
	// stale one.
	wtRepo, err := gitx.Open(wt)
	if err != nil {
		t.Fatal(err)
	}
	writeFile(t, wt, "changes/"+doc.ID+"/contract.delta.md", testDelta)
	_, bundle, err := kernel.ReviewChange(wtRepo)
	if err != nil {
		t.Fatal(err)
	}
	if got := guardDecision(t, wt, "Bash", map[string]any{"command": "telos change approve --digest " + bundle.Digest}); got != "ask" {
		t.Errorf("approve with reviewed digest = %q, want ask", got)
	}
	if got := guardDecision(t, wt, "Bash", map[string]any{"command": "telos change approve --digest 0000"}); got != "deny" {
		t.Errorf("approve with stale digest = %q, want deny", got)
	}
}

func TestInitCreatesGenesis(t *testing.T) {
	root := gitRepo(t)
	writeFile(t, root, "app.txt", "hello\n")
	t.Chdir(root)

	var stdout, stderr bytes.Buffer
	if err := Run([]string{"init", "--agent", "all", "--json"}, "test", strings.NewReader(""), &stdout, &stderr); err != nil {
		t.Fatalf("init: %v\n%s", err, stderr.String())
	}
	var envelope struct {
		OK     bool           `json:"ok"`
		Result map[string]any `json:"result"`
	}
	if err := json.Unmarshal(stdout.Bytes(), &envelope); err != nil || !envelope.OK {
		t.Fatalf("init envelope: %v\n%s", err, stdout.String())
	}
	if envelope.Result["change"] != "CHG-000" {
		t.Fatalf("init result = %v", envelope.Result)
	}
	for _, rel := range []string{
		"telos.toml", "spec/PRODUCT.md", ".gitignore",
		".claude/skills/telos/SKILL.md", ".agents/skills/telos/SKILL.md",
		".claude/settings.json", ".codex/hooks.json",
	} {
		if _, err := os.Stat(filepath.Join(root, filepath.FromSlash(rel))); err != nil {
			t.Errorf("init did not create %s: %v", rel, err)
		}
	}
	gitignore, _ := os.ReadFile(filepath.Join(root, ".gitignore"))
	if !strings.Contains(string(gitignore), ".telos/") {
		t.Errorf(".gitignore does not ignore .telos/: %q", gitignore)
	}

	// The project is certified after genesis.
	stdout.Reset()
	if err := Run([]string{"status", "--json"}, "test", strings.NewReader(""), &stdout, &stderr); err != nil {
		t.Fatalf("status: %v", err)
	}
	var status struct {
		Result map[string]any `json:"result"`
	}
	if err := json.Unmarshal(stdout.Bytes(), &status); err != nil {
		t.Fatal(err)
	}
	if status.Result["state"] != "certified" {
		t.Fatalf("state after init = %v", status.Result)
	}

	// Re-init is a fresh genesis (destructive reset, guard-gated in agent
	// contexts) and must succeed again.
	stdout.Reset()
	if err := Run([]string{"init", "--agent", "claude", "--json"}, "test", strings.NewReader(""), &stdout, &stderr); err != nil {
		t.Fatalf("re-init: %v", err)
	}
}

func TestDefaultConfigParses(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, dir, kernel.ConfigFile, defaultConfig([]string{"claude", "codex"}))
	cfg, err := kernel.ReadConfig(dir)
	if err != nil {
		t.Fatal(err)
	}
	if cfg.ProjectID == "" || len(cfg.Agents) != 2 {
		t.Fatalf("cfg = %+v", cfg)
	}
}

func TestMergeSettingsPreservesAndDedupes(t *testing.T) {
	existing := []byte(`{
  "custom": {"keep": true},
  "hooks": {
    "PreToolUse": [
      {"matcher": "Bash", "hooks": [{"command": "telos guard"}]},
      {"matcher": "Other", "hooks": [{"command": "something-else"}]}
    ]
  }
}`)
	generated := []byte(`{"hooks":{"PreToolUse":[{"matcher":"Bash|Edit|Write","hooks":[{"command":"telos guard"}]}]}}`)
	merged, err := mergeSettings(existing, generated)
	if err != nil {
		t.Fatal(err)
	}
	text := string(merged)
	if !strings.Contains(text, `"keep": true`) || !strings.Contains(text, "something-else") {
		t.Fatalf("merge lost existing content:\n%s", text)
	}
	if strings.Count(text, "telos guard") != 1 {
		t.Fatalf("merge did not dedupe the guard hook:\n%s", text)
	}
	// Idempotence: merging again changes nothing.
	again, err := mergeSettings(merged, generated)
	if err != nil {
		t.Fatal(err)
	}
	if string(again) != text {
		t.Fatalf("merge is not idempotent:\n%s\nvs\n%s", text, again)
	}
}

func TestManagedBlock(t *testing.T) {
	out := managed("", "instructions v1")
	if !strings.Contains(out, managedStart) || !strings.Contains(out, "instructions v1") {
		t.Fatalf("managed on empty = %q", out)
	}
	out2 := managed("# Mine\n\n"+out, "instructions v2")
	if strings.Contains(out2, "instructions v1") || !strings.Contains(out2, "instructions v2") || !strings.Contains(out2, "# Mine") {
		t.Fatalf("managed replace = %q", out2)
	}
}

func TestEnsureGitignore(t *testing.T) {
	root := t.TempDir()
	writeFile(t, root, ".gitignore", "dist/\n")
	if err := ensureGitignore(root); err != nil {
		t.Fatal(err)
	}
	data, _ := os.ReadFile(filepath.Join(root, ".gitignore"))
	if !strings.Contains(string(data), "dist/") || !strings.Contains(string(data), ".telos/") {
		t.Fatalf(".gitignore = %q", data)
	}
	before := string(data)
	if err := ensureGitignore(root); err != nil {
		t.Fatal(err)
	}
	data, _ = os.ReadFile(filepath.Join(root, ".gitignore"))
	if string(data) != before {
		t.Fatal("ensureGitignore is not idempotent")
	}
}
