package kernel

import (
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/hugues31/telos-sdd/internal/evidence"
	"github.com/hugues31/telos-sdd/internal/gitx"
)

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

const evidenceConfig = `project_id = "evidence-test"
agents = ["claude"]
test_commands = ["go run tools/probe.go"]
test_files = ["tests/**"]
closure = "tree"
`

// evidenceProject is a certified project with a real (probe-based) suite.
func evidenceProject(t *testing.T) *gitx.Repo {
	t.Helper()
	repo := newProject(t)
	writeAt(t, repo.WorkDir, ConfigFile, evidenceConfig)
	writeAt(t, repo.WorkDir, "tools/probe.go", probeProgram)
	genesis(t, repo)
	return repo
}

// approvedGreeting starts a behavior change adding REQ-001 and approves it.
func approvedGreeting(t *testing.T, repo *gitx.Repo) (*ChangeDoc, *gitx.Repo) {
	t.Helper()
	doc, wt := startedChange(t, repo, CategoryBehaviorChange)
	writeAt(t, wt.WorkDir, "changes/"+doc.ID+"/contract.delta.md", greetingDelta)
	_, bundle, err := ReviewChange(wt)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := ApproveChange(wt, bundle.Digest); err != nil {
		t.Fatal(err)
	}
	return doc, wt
}

func TestWitnessedRedGreenAndPromotion(t *testing.T) {
	repo := evidenceProject(t)
	cfg, _ := ReadConfig(repo.WorkDir)
	doc, wt := approvedGreeting(t, repo)

	// A passing test proves nothing.
	writeAt(t, wt.WorkDir, "tests/core_test.txt", "asserts REQ-001\nexpect app.txt\n")
	if _, _, err := EvidenceRed(wt, cfg, "REQ-001", io.Discard); errCode(t, err) != "TELOS_RED_EXPECTED" {
		t.Fatal(err)
	}
	// An unknown requirement is refused.
	if _, _, err := EvidenceRed(wt, cfg, "REQ-404", io.Discard); errCode(t, err) != "TELOS_REQUIREMENT_UNKNOWN" {
		t.Fatal(err)
	}

	// Witnessed red: the citing test fails while the baseline is green.
	writeAt(t, wt.WorkDir, "tests/core_test.txt", "asserts REQ-001\nexpect out/greeting.txt\n")
	doc2, witness, err := EvidenceRed(wt, cfg, "REQ-001", io.Discard)
	if err != nil {
		t.Fatal(err)
	}
	if len(doc2.RedWitnesses) != 1 || len(witness.SealedTests) != 1 || witness.SealedTests[0].Path != "tests/core_test.txt" {
		t.Fatalf("witness = %+v", witness)
	}

	// Ready is gated on the open red.
	if _, err := ReadyChange(wt, cfg, io.Discard); errCode(t, err) != "TELOS_RED_PENDING" {
		t.Fatal(err)
	}

	// Sealed bytes may not move.
	writeAt(t, wt.WorkDir, "tests/core_test.txt", "asserts REQ-001\nexpect app.txt\n")
	if _, _, err := EvidenceGreen(wt, cfg, "REQ-001", io.Discard); errCode(t, err) != "TELOS_RED_STALE" {
		t.Fatal(err)
	}
	writeAt(t, wt.WorkDir, "tests/core_test.txt", "asserts REQ-001\nexpect out/greeting.txt\n")

	// Still red until the implementation exists.
	if _, _, err := EvidenceGreen(wt, cfg, "REQ-001", io.Discard); errCode(t, err) != "TELOS_RED_PENDING" {
		t.Fatal(err)
	}
	writeAt(t, wt.WorkDir, "out/greeting.txt", "greeting\n")
	doc3, record, err := EvidenceGreen(wt, cfg, "REQ-001", io.Discard)
	if err != nil {
		t.Fatal(err)
	}
	if len(doc3.RedWitnesses) != 0 || record.Kind != evidence.KindRedGreen || record.Witness == nil || !record.Witness.Green.SealedTestsIntact {
		t.Fatalf("green record = %+v", record)
	}
	if _, err := os.Stat(filepath.Join(wt.WorkDir, "changes", doc.ID, "evidence", evidence.FileName(record.Key()))); err != nil {
		t.Fatalf("record not committed: %v", err)
	}

	// Certification gates pass; promotion is atomic and folds the contract.
	report, err := ReadyChange(wt, cfg, io.Discard)
	if err != nil {
		t.Fatal(err)
	}
	if len(report.Evidence) < 2 || len(report.Requirements) != 1 || report.Requirements[0] != "REQ-001" {
		t.Fatalf("report = %+v", report)
	}
	result, err := PromoteChange(wt, cfg, "test", io.Discard)
	if err != nil {
		t.Fatal(err)
	}

	head, err := repo.Head()
	if err != nil || string(head) != result.Commit {
		t.Fatalf("main = %s, promote said %s (%v)", head, result.Commit, err)
	}
	cert, err := LoadCertificate(repo, head)
	if err != nil {
		t.Fatal(err)
	}
	tree, _ := repo.TreeOf("HEAD")
	if err := cert.Validate(head, tree); err != nil {
		t.Fatal(err)
	}
	if cert.Payload.Change.ID != doc.ID || cert.Payload.ParentCertified != doc.Base {
		t.Fatalf("cert = %+v", cert.Payload)
	}
	spec, err := repo.BlobAt("HEAD", "spec/core.md")
	if err != nil || !strings.Contains(string(spec), "REQ-001") {
		t.Fatalf("folded spec = %q, %v", spec, err)
	}
	if data, err := repo.BlobAt("HEAD", "changes/"+doc.ID+"/change.json"); err != nil || !strings.Contains(string(data), ChangePromoted) {
		t.Fatalf("retained change record = %q, %v", data, err)
	}
	// The root worktree was clean, so it fast-forwarded and stays certified.
	if st, _ := Status(repo); st.State != StateCertified {
		t.Fatalf("root after promotion = %+v", st)
	}
	if !result.Cleaned {
		t.Fatal("candidate cleanup did not run")
	}
	if branches, _ := repo.Branches("telos/CHG-*"); len(branches) != 0 {
		t.Fatalf("candidate branch survived: %v", branches)
	}

	report2, err := Verify(repo, cfg, io.Discard, io.Discard)
	if err != nil {
		t.Fatal(err)
	}
	if report2.Requirements != 1 {
		t.Fatalf("verify after promotion = %+v", report2)
	}
}

func TestReadyGates(t *testing.T) {
	repo := evidenceProject(t)
	cfg, _ := ReadConfig(repo.WorkDir)
	doc, wt := startedChange(t, repo, CategoryBehaviorChange)
	writeAt(t, wt.WorkDir, "changes/"+doc.ID+"/contract.delta.md", greetingDelta)

	// No approval yet.
	if _, err := ReadyChange(wt, cfg, io.Discard); errCode(t, err) != "TELOS_APPROVAL_REQUIRED" {
		t.Fatal(err)
	}
	_, bundle, err := ReviewChange(wt)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := ApproveChange(wt, bundle.Digest); err != nil {
		t.Fatal(err)
	}

	// Approved, but REQ-001 has no citing test.
	if _, err := ReadyChange(wt, cfg, io.Discard); errCode(t, err) != "TELOS_OBLIGATION_UNMET" {
		t.Fatal(err)
	}

	// A cited but unwitnessed requirement still lacks its proof.
	writeAt(t, wt.WorkDir, "tests/core_test.txt", "asserts REQ-001\nexpect app.txt\n")
	if _, err := ReadyChange(wt, cfg, io.Discard); errCode(t, err) != "TELOS_OBLIGATION_UNMET" {
		t.Fatal(err)
	}

	// Base staleness gates before obligations are even considered.
	writeAt(t, repo.WorkDir, "app.txt", "moved\n")
	genesis(t, repo)
	if _, err := ReadyChange(wt, cfg, io.Discard); errCode(t, err) != "TELOS_BASE_STALE" {
		t.Fatal(err)
	}
}

func TestFindingsGate(t *testing.T) {
	repo := evidenceProject(t)
	cfg, _ := ReadConfig(repo.WorkDir)
	doc, wt := approvedGreeting(t, repo)

	// Prove REQ-001 so only findings can block.
	writeAt(t, wt.WorkDir, "tests/core_test.txt", "asserts REQ-001\nexpect out/greeting.txt\n")
	if _, _, err := EvidenceRed(wt, cfg, "REQ-001", io.Discard); err != nil {
		t.Fatal(err)
	}
	writeAt(t, wt.WorkDir, "out/greeting.txt", "greeting\n")
	if _, _, err := EvidenceGreen(wt, cfg, "REQ-001", io.Discard); err != nil {
		t.Fatal(err)
	}

	// A critic's proposed blocking does not block by itself.
	critic, err := AddFinding(wt, Finding{
		Source:           FindingSource{Kind: "critic", Name: "verifier"},
		Target:           FindingTarget{Requirements: []string{"REQ-001"}},
		ProposedSeverity: SeverityBlocking,
		Confidence:       0.9,
		Rationale:        "the scenario does not cover the empty greeting",
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := ReadyChange(wt, cfg, io.Discard); err != nil {
		t.Fatalf("proposed-only blocking must not gate: %v", err)
	}

	// Human confirmation makes it effective.
	if _, err := ConfirmFinding(wt, critic.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := ReadyChange(wt, cfg, io.Discard); errCode(t, err) != "TELOS_FINDING_BLOCKING" {
		t.Fatal(err)
	}

	// Resolution with taxonomy unblocks.
	if _, err := ResolveFinding(wt, critic.ID, "not_an_issue", "", "scenario covers it via the Given"); err != nil {
		t.Fatal(err)
	}
	if _, err := ReadyChange(wt, cfg, io.Discard); err != nil {
		t.Fatalf("resolved finding still gates: %v", err)
	}
	_ = doc
}
