package kernel

import (
	"bytes"
	"encoding/json"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/gitx"
)

const testProduct = `# Product

### INT-001 — Greets reliably

The application greets.
`

const testConfig = `project_id = "kernel-test"
agents = ["claude"]
test_commands = []
test_files = ["tests/**"]
`

func newProject(t *testing.T) *gitx.Repo {
	t.Helper()
	dir := t.TempDir()
	for _, args := range [][]string{
		{"init", "--quiet", "-b", "main"},
		{"config", "user.email", "telos@test"},
		{"config", "user.name", "telos test"},
		{"config", "core.autocrlf", "false"},
	} {
		cmd := exec.Command("git", args...)
		cmd.Dir = dir
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("git %v: %v\n%s", args, err, out)
		}
	}
	writeAt(t, dir, ConfigFile, testConfig)
	writeAt(t, dir, "spec/PRODUCT.md", testProduct)
	writeAt(t, dir, "app.txt", "hello\n")
	repo, err := gitx.Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	return repo
}

func writeAt(t *testing.T, root, rel, content string) {
	t.Helper()
	path := filepath.Join(root, filepath.FromSlash(rel))
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func errCode(t *testing.T, err error) string {
	t.Helper()
	if err == nil {
		t.Fatal("expected a coded error, got nil")
	}
	e, ok := coded.As(err)
	if !ok {
		t.Fatalf("expected a coded error, got %v", err)
	}
	return e.Code
}

func genesis(t *testing.T, repo *gitx.Repo) *Certificate {
	t.Helper()
	cfg, err := ReadConfig(repo.WorkDir)
	if err != nil {
		t.Fatal(err)
	}
	cert, err := Genesis(repo, cfg, GenesisOptions{Version: "test"})
	if err != nil {
		t.Fatal(err)
	}
	return cert
}

func TestReadConfig(t *testing.T) {
	dir := t.TempDir()
	writeAt(t, dir, ConfigFile, "project_id = \"p1\"\nagents = [\"claude\"]\ntest_commands = [\"go test ./...\"]\ntest_files = [\"tests/**\"]\nclosure = \"go\"\n")
	cfg, err := ReadConfig(dir)
	if err != nil {
		t.Fatal(err)
	}
	if cfg.ProjectID != "p1" || cfg.Closure != "go" || len(cfg.TestCommands) != 1 || cfg.TestFiles[0] != "tests/**" {
		t.Fatalf("cfg = %+v", cfg)
	}

	writeAt(t, dir, ConfigFile, "unknown_key = 1\n")
	if _, err := ReadConfig(dir); errCode(t, err) != "TELOS_CONFIG_INVALID" {
		t.Fatalf("unknown key error = %v", err)
	}
	writeAt(t, dir, ConfigFile, "closure = \"quantum\"\n")
	if _, err := ReadConfig(dir); errCode(t, err) != "TELOS_CONFIG_INVALID" {
		t.Fatalf("bad closure error = %v", err)
	}
	if _, err := ReadConfig(t.TempDir()); errCode(t, err) != "TELOS_NOT_INITIALIZED" {
		t.Fatalf("missing config error = %v", err)
	}
}

func TestEffectiveClosure(t *testing.T) {
	dir := t.TempDir()
	if got := (Config{}).EffectiveClosure(dir); got != "tree" {
		t.Fatalf("no go.mod → %q, want tree", got)
	}
	writeAt(t, dir, "go.mod", "module x\n")
	if got := (Config{}).EffectiveClosure(dir); got != "go" {
		t.Fatalf("go.mod present → %q, want go", got)
	}
	if got := (Config{Closure: "tree"}).EffectiveClosure(dir); got != "tree" {
		t.Fatalf("explicit closure → %q, want tree", got)
	}
}

func TestMarshalCanonicalDeterministic(t *testing.T) {
	payload := CertPayload{Version: 1, Commit: "abc", Tree: "def",
		Approvals:    []Approval{{Kind: "contract", Digest: "d", At: "2026-01-01T00:00:00Z"}},
		Verification: Verification{Evidence: []EvidenceEntry{}, RequirementsVerified: []string{"REQ-001"}, FindingsOpen: []string{}},
	}
	a, err := marshalCanonical(payload)
	if err != nil {
		t.Fatal(err)
	}
	b, _ := marshalCanonical(payload)
	if !bytes.Equal(a, b) {
		t.Fatal("canonical bytes are not deterministic")
	}
	if bytes.HasSuffix(a, []byte("\n")) {
		t.Fatal("canonical bytes must not end with a newline")
	}
	esc, _ := marshalCanonical(struct {
		S string `json:"s"`
	}{"<&>"})
	if string(esc) != `{"s":"<&>"}` {
		t.Fatalf("HTML escaping must be disabled: %s", esc)
	}
}

func TestGenesisCertifies(t *testing.T) {
	repo := newProject(t)
	cert := genesis(t, repo)

	head, err := repo.Head()
	if err != nil {
		t.Fatal(err)
	}
	if cert.Payload.Commit != string(head) || cert.Payload.Change.Category != CategoryGenesis {
		t.Fatalf("payload = %+v", cert.Payload)
	}
	loaded, err := LoadCertificate(repo, head)
	if err != nil {
		t.Fatal(err)
	}
	tree, _ := repo.TreeOf("HEAD")
	if err := loaded.Validate(head, tree); err != nil {
		t.Fatal(err)
	}
	st, err := Status(repo)
	if err != nil {
		t.Fatal(err)
	}
	if st.State != StateCertified || st.Certificate == nil || st.Certificate.Change != "CHG-000" {
		t.Fatalf("status = %+v", st)
	}
	if st.Contract == nil || st.Contract.Intents != 1 {
		t.Fatalf("contract counts = %+v", st.Contract)
	}
}

func TestGenesisRejectsInvalidContract(t *testing.T) {
	repo := newProject(t)
	writeAt(t, repo.WorkDir, "spec/core.md", "### REQ-001 — broken\nMotivated by: INT-001\n")
	cfg, _ := ReadConfig(repo.WorkDir)
	_, err := Genesis(repo, cfg, GenesisOptions{})
	if errCode(t, err) != "TELOS_CONTRACT_INVALID" {
		t.Fatalf("err = %v", err)
	}
}

func TestCertificateForgeryDetected(t *testing.T) {
	repo := newProject(t)
	genesis(t, repo)
	head, _ := repo.Head()
	tree, _ := repo.TreeOf("HEAD")

	// Forge the MAC: rewrite the note with a tampered seal.
	raw, err := repo.NoteShow(gitx.NotesRef, head)
	if err != nil {
		t.Fatal(err)
	}
	tampered := bytes.Replace(raw, []byte(`"mac":"`), []byte(`"mac":"00`), 1)
	if bytes.Equal(tampered, raw) {
		t.Fatal("tampering failed to change the note")
	}
	if err := repo.NoteAdd(gitx.NotesRef, head, tampered); err != nil {
		t.Fatal(err)
	}
	cert, err := LoadCertificate(repo, head)
	if err != nil {
		t.Fatal(err)
	}
	if errCode(t, cert.Validate(head, tree)) != "TELOS_CERTIFICATE_INVALID" {
		t.Fatal("forged MAC must not validate")
	}
}

func TestCertificateCannotMoveToAnotherCommit(t *testing.T) {
	repo := newProject(t)
	genesis(t, repo)
	head, _ := repo.Head()
	note, err := repo.NoteShow(gitx.NotesRef, head)
	if err != nil {
		t.Fatal(err)
	}

	// An out-of-band commit, with the genuine note copied onto it.
	writeAt(t, repo.WorkDir, "app.txt", "rogue\n")
	if err := repo.AddAll(); err != nil {
		t.Fatal(err)
	}
	tree, _ := repo.WriteTree()
	rogue, err := repo.CommitTree(tree, []gitx.OID{head}, "rogue")
	if err != nil {
		t.Fatal(err)
	}
	if err := repo.NoteAdd(gitx.NotesRef, rogue, note); err != nil {
		t.Fatal(err)
	}
	cert, err := LoadCertificate(repo, rogue)
	if err != nil {
		t.Fatal(err)
	}
	rogueTree, _ := repo.TreeOf(string(rogue))
	if errCode(t, cert.Validate(rogue, rogueTree)) != "TELOS_CERTIFICATE_INVALID" {
		t.Fatal("a copied note must not validate on another commit")
	}
}

func TestStatusStates(t *testing.T) {
	// Uninitialized: no telos.toml.
	bare := t.TempDir()
	cmd := exec.Command("git", "init", "--quiet", "-b", "main")
	cmd.Dir = bare
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("%v\n%s", err, out)
	}
	repo, err := gitx.Open(bare)
	if err != nil {
		t.Fatal(err)
	}
	st, err := Status(repo)
	if err != nil || st.State != StateUninitialized {
		t.Fatalf("status = %+v, %v", st, err)
	}

	// Certified, then dirty → corrupted with paths.
	repo = newProject(t)
	genesis(t, repo)
	writeAt(t, repo.WorkDir, "app.txt", "tampered\n")
	st, err = Status(repo)
	if err != nil {
		t.Fatal(err)
	}
	if st.State != StateCorrupted || st.Dirty == nil || len(st.Dirty.Paths) != 1 || st.Dirty.Paths[0] != "app.txt" {
		t.Fatalf("status = %+v", st)
	}

	// Out-of-band commit → corrupted (uncertified tip).
	if err := repo.AddAll(); err != nil {
		t.Fatal(err)
	}
	tree, _ := repo.WriteTree()
	head, _ := repo.Head()
	rogue, _ := repo.CommitTree(tree, []gitx.OID{head}, "rogue")
	if err := repo.UpdateRef("refs/heads/main", rogue); err != nil {
		t.Fatal(err)
	}
	if err := repo.ResetHardTo("HEAD"); err != nil {
		t.Fatal(err)
	}
	st, err = Status(repo)
	if err != nil {
		t.Fatal(err)
	}
	if st.State != StateCorrupted || !strings.Contains(st.Reason, "certificate") {
		t.Fatalf("status = %+v", st)
	}
}

func TestVerify(t *testing.T) {
	repo := newProject(t)
	genesis(t, repo)
	cfg, _ := ReadConfig(repo.WorkDir)

	report, err := Verify(repo, cfg, io.Discard, io.Discard)
	if err != nil {
		t.Fatal(err)
	}
	if report.Intents != 1 || report.Change != "CHG-000" {
		t.Fatalf("report = %+v", report)
	}

	writeAt(t, repo.WorkDir, "app.txt", "tampered\n")
	if _, err := Verify(repo, cfg, io.Discard, io.Discard); errCode(t, err) != "TELOS_STATE_CORRUPTED" {
		t.Fatalf("dirty verify = %v", err)
	}
	if err := repo.ResetHardTo("HEAD"); err != nil {
		t.Fatal(err)
	}

	cfg.TestCommands = []string{"definitely-not-a-command-xyz"}
	if _, err := Verify(repo, cfg, io.Discard, io.Discard); errCode(t, err) != "TELOS_TESTS_FAILED" {
		t.Fatalf("failing suite verify = %v", err)
	}
}

func TestVerifyDetectsSuiteMutatingTree(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("sh-based mutation command")
	}
	repo := newProject(t)
	genesis(t, repo)
	cfg, _ := ReadConfig(repo.WorkDir)
	cfg.TestCommands = []string{"echo mutated >> app.txt"}
	if _, err := Verify(repo, cfg, io.Discard, io.Discard); errCode(t, err) != "TELOS_STATE_CORRUPTED" {
		t.Fatalf("mutating suite verify = %v", err)
	}
}

func TestStatusJSONShape(t *testing.T) {
	repo := newProject(t)
	genesis(t, repo)
	st, err := Status(repo)
	if err != nil {
		t.Fatal(err)
	}
	raw, err := json.Marshal(st)
	if err != nil {
		t.Fatal(err)
	}
	var m map[string]any
	if err := json.Unmarshal(raw, &m); err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{"context", "state", "certificate", "contract"} {
		if _, ok := m[key]; !ok {
			t.Errorf("status JSON misses %q: %s", key, raw)
		}
	}
}
