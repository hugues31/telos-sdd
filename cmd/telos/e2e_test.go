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

// TestCLIGenesis is the M1 oracle: init seals a genesis certificate on the
// substrate, status/verify derive certified/corrupted from it, and the only
// recoveries from corruption are explicit.
func TestCLIGenesis(t *testing.T) {
	bin := buildCLI(t)
	root := setupCertified(t, bin)

	status := expectOK(t, runCLI(t, bin, root, "", "status"), "status")
	if status["state"] != "certified" || status["context"] != "root" {
		t.Fatalf("status = %v", status)
	}
	certBlock, _ := status["certificate"].(map[string]any)
	if certBlock == nil || certBlock["change"] != "CHG-000" {
		t.Fatalf("certificate block = %v", certBlock)
	}
	counts, _ := status["contract"].(map[string]any)
	if counts == nil || fmt.Sprint(counts["intents"]) != "1" {
		t.Fatalf("contract counts = %v", counts)
	}

	// The sealed note is a valid certificate envelope bound to HEAD.
	cert := certificateJSON(t, root)
	payload, _ := cert["payload"].(map[string]any)
	change, _ := payload["change"].(map[string]any)
	if change["id"] != "CHG-000" || change["category"] != "genesis" {
		t.Fatalf("certificate change = %v", change)
	}
	seal, _ := cert["seal"].(map[string]any)
	if seal["mode"] != "SEALED" || seal["mac"] == "" {
		t.Fatalf("seal = %v", seal)
	}

	expectOK(t, runCLI(t, bin, root, "", "doctor"), "doctor")

	// Out-of-band edit: corruption is a status, verify names the paths.
	write(t, root, "app.txt", "tampered\n")
	status = expectOK(t, runCLI(t, bin, root, "", "status"), "status (dirty)")
	if status["state"] != "corrupted" {
		t.Fatalf("dirty state = %v", status["state"])
	}
	if dirty, _ := status["dirty"].(map[string]any); dirty == nil {
		t.Fatalf("status misses dirty block: %v", status)
	}
	expectCode(t, runCLI(t, bin, root, "", "verify"), "TELOS_STATE_CORRUPTED", "verify (dirty)")
	git(t, root, "checkout", "--", "app.txt")
	expectOK(t, runCLI(t, bin, root, "", "verify"), "verify (restored)")

	// Out-of-band commit: the tip carries no valid certificate.
	write(t, root, "app.txt", "rogue\n")
	git(t, root, "add", "-A")
	git(t, root, "commit", "--quiet", "-m", "rogue")
	expectCode(t, runCLI(t, bin, root, "", "verify"), "TELOS_CERTIFICATE_INVALID", "verify (uncertified tip)")
	status = expectOK(t, runCLI(t, bin, root, "", "status"), "status (uncertified tip)")
	if status["state"] != "corrupted" {
		t.Fatalf("uncertified-tip state = %v", status["state"])
	}

	// Deliberate destructive reset: re-init seals a fresh genesis.
	expectOK(t, runCLI(t, bin, root, "", "init", "--agent", "claude"), "re-init")
	expectOK(t, runCLI(t, bin, root, "", "verify"), "verify (re-genesis)")

	// The V1 surface is gone.
	expectCode(t, runCLI(t, bin, root, "", "spec", "review"), "TELOS_INPUT_INVALID", "dead verb")
}
