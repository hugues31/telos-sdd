package telos

import (
	"bytes"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

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
