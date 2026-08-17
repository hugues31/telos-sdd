package kernel

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/hugues31/telos-sdd/internal/gitx"
)

const greetingDelta = "<!-- telos:op add file: spec/core.md -->\n" +
	"### REQ-001 — Emit the greeting\nClass: behavior\nMotivated by: INT-001\n\n" +
	"```gherkin\nScenario: greeting\n  Given the app runs\n  Then the greeting is produced\n```\n"

func startedChange(t *testing.T, repo *gitx.Repo, category string) (*ChangeDoc, *gitx.Repo) {
	t.Helper()
	doc, path, err := StartChange(repo, category, "test change")
	if err != nil {
		t.Fatal(err)
	}
	wt, err := gitx.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = repo.WorktreeRemove(path) })
	return doc, wt
}

func TestStartChange(t *testing.T) {
	repo := newProject(t)
	genesis(t, repo)

	doc, wt := startedChange(t, repo, CategoryBehaviorChange)
	if doc.ID != "CHG-001" || doc.Status != ChangeDrafting || doc.TargetBranch != "main" {
		t.Fatalf("doc = %+v", doc)
	}
	if ctx, _ := ChangeContext(wt); ctx != "CHG-001" {
		t.Fatalf("candidate context = %q", ctx)
	}
	for _, rel := range []string{"change.json", "intent.md", "contract.delta.md", "decisions.md", "findings.json"} {
		if _, err := os.Stat(filepath.Join(wt.WorkDir, "changes", "CHG-001", rel)); err != nil {
			t.Errorf("scaffold misses %s: %v", rel, err)
		}
	}
	// The scaffold is committed on the candidate branch; the root is untouched.
	if dirty, _ := wt.DirtyPaths(); dirty != nil {
		t.Fatalf("candidate dirty after start: %v", dirty)
	}
	if st, _ := Status(repo); st.State != StateCertified || len(st.Changes) != 1 || st.Changes[0].ID != "CHG-001" {
		t.Fatalf("root status = %+v", st)
	}

	// A second change allocates the next id.
	doc2, _ := startedChange(t, repo, CategoryBehaviorPreserving)
	if doc2.ID != "CHG-002" {
		t.Fatalf("second id = %s", doc2.ID)
	}

	// Guards: category, context, corrupted root.
	if _, _, err := StartChange(repo, "quantum", "x"); errCode(t, err) != "TELOS_INPUT_INVALID" {
		t.Fatal(err)
	}
	if _, _, err := StartChange(wt, CategoryBehaviorChange, "x"); errCode(t, err) != "TELOS_ROOT_REQUIRED" {
		t.Fatal(err)
	}
	writeAt(t, repo.WorkDir, "app.txt", "tampered\n")
	if _, _, err := StartChange(repo, CategoryBehaviorChange, "x"); errCode(t, err) != "TELOS_STATE_CORRUPTED" {
		t.Fatal(err)
	}
}

func TestReviewAndApproveBehaviorChange(t *testing.T) {
	repo := newProject(t)
	genesis(t, repo)
	doc, wt := startedChange(t, repo, CategoryBehaviorChange)

	// The scaffold template is an empty delta: nothing to review yet.
	if _, _, err := ReviewChange(wt); errCode(t, err) != "TELOS_NOTHING_PENDING" {
		t.Fatal(err)
	}

	writeAt(t, wt.WorkDir, "changes/CHG-001/contract.delta.md", greetingDelta)
	doc, bundle, err := ReviewChange(wt)
	if err != nil {
		t.Fatal(err)
	}
	if bundle.Digest == "" || bundle.Kind != "contract" || doc.Status != ChangeAwaitingApproval {
		t.Fatalf("review = %+v, doc = %+v", bundle, doc)
	}
	if len(bundle.SpecChanged) != 1 || bundle.SpecChanged[0] != "spec/core.md" {
		t.Fatalf("spec_changed = %v", bundle.SpecChanged)
	}

	// Drift after review invalidates the presented digest.
	writeAt(t, wt.WorkDir, "changes/CHG-001/contract.delta.md", greetingDelta+"\n<!-- telos:op remove id: INT-001 -->\n")
	if _, err := ApproveChange(wt, bundle.Digest); errCode(t, err) != "TELOS_APPROVAL_STALE" {
		t.Fatal(err)
	}

	writeAt(t, wt.WorkDir, "changes/CHG-001/contract.delta.md", greetingDelta)
	_, bundle, err = ReviewChange(wt)
	if err != nil {
		t.Fatal(err)
	}
	doc, err = ApproveChange(wt, bundle.Digest)
	if err != nil {
		t.Fatal(err)
	}
	if doc.Status != ChangeApproved || len(doc.Approvals) != 1 || doc.Approvals[0].Kind != "contract" || doc.Approvals[0].Digest != bundle.Digest {
		t.Fatalf("approved doc = %+v", doc)
	}

	// The digest is deterministic: re-review yields the same value.
	_, again, err := ReviewChange(wt)
	if err != nil || again.Digest != bundle.Digest {
		t.Fatalf("digest changed: %s vs %s (%v)", again.Digest, bundle.Digest, err)
	}
}

func TestReviewRejectsDirectSpecEdit(t *testing.T) {
	repo := newProject(t)
	genesis(t, repo)
	_, wt := startedChange(t, repo, CategoryBehaviorChange)

	writeAt(t, wt.WorkDir, "spec/core.md", "### REQ-009 — smuggled\n")
	_, _, err := ReviewChange(wt)
	if errCode(t, err) != "TELOS_CONTRACT_TAMPERED" {
		t.Fatal(err)
	}
}

func TestReviewRejectsInvalidFoldedContract(t *testing.T) {
	repo := newProject(t)
	genesis(t, repo)
	_, wt := startedChange(t, repo, CategoryBehaviorChange)

	// INT-999 does not exist: the folded contract is invalid.
	writeAt(t, wt.WorkDir, "changes/CHG-001/contract.delta.md",
		"<!-- telos:op add file: spec/core.md -->\n### REQ-001 — bad\nClass: behavior\nMotivated by: INT-999\n\n```gherkin\nScenario: x\n```\n")
	if _, _, err := ReviewChange(wt); errCode(t, err) != "TELOS_CONTRACT_INVALID" {
		t.Fatal(err)
	}
}

func TestPreservingChangeClaim(t *testing.T) {
	repo := newProject(t)
	genesis(t, repo)
	doc, wt := startedChange(t, repo, CategoryBehaviorPreserving)

	// A preserving change cannot carry a delta.
	writeAt(t, wt.WorkDir, "changes/"+doc.ID+"/contract.delta.md", greetingDelta)
	if _, _, err := ReviewChange(wt); errCode(t, err) != "TELOS_INPUT_INVALID" {
		t.Fatal(err)
	}
	writeAt(t, wt.WorkDir, "changes/"+doc.ID+"/contract.delta.md", deltaTemplate)

	// The claim digest is the candidate tree.
	writeAt(t, wt.WorkDir, "app.txt", "refactored\n")
	_, bundle, err := ReviewChange(wt)
	if err != nil {
		t.Fatal(err)
	}
	if bundle.Kind != "preserving_claim" || len(bundle.CodePaths) == 0 {
		t.Fatalf("bundle = %+v", bundle)
	}
	if _, err := ApproveChange(wt, bundle.Digest); err != nil {
		t.Fatal(err)
	}

	// Touching telos.toml flags the change privileged (KERNEL-009).
	writeAt(t, wt.WorkDir, ConfigFile, testConfig+"# privileged edit\n")
	_, bundle, err = ReviewChange(wt)
	if err != nil {
		t.Fatal(err)
	}
	if !bundle.Privileged {
		t.Fatal("telos.toml edit did not flag the change privileged")
	}
}

func TestApproveWithoutReview(t *testing.T) {
	repo := newProject(t)
	genesis(t, repo)
	_, wt := startedChange(t, repo, CategoryBehaviorChange)
	if _, err := ApproveChange(wt, "whatever"); errCode(t, err) != "TELOS_NOTHING_PENDING" {
		t.Fatal(err)
	}
}

func TestCandidateStatusAndAbort(t *testing.T) {
	repo := newProject(t)
	genesis(t, repo)
	doc, wt := startedChange(t, repo, CategoryBehaviorChange)

	st, err := Status(wt)
	if err != nil {
		t.Fatal(err)
	}
	if st.Context != "candidate" || st.State != StateCertified || st.Change == nil {
		t.Fatalf("candidate status = %+v", st)
	}
	if st.Change.ID != doc.ID || st.Change.BaseStale || st.Change.Status != ChangeDrafting {
		t.Fatalf("candidate change = %+v", st.Change)
	}

	if err := AbortChange(repo, doc.ID); err != nil {
		t.Fatal(err)
	}
	if branches, _ := repo.Branches("telos/CHG-*"); len(branches) != 0 {
		t.Fatalf("abort left branches: %v", branches)
	}
	if err := AbortChange(repo, "CHG-404"); errCode(t, err) != "TELOS_CHANGE_UNKNOWN" {
		t.Fatal(err)
	}
	if _, err := os.Stat(wt.WorkDir); !os.IsNotExist(err) {
		t.Fatal("abort left the worktree directory")
	}
}

func TestChangeSummaryBaseStale(t *testing.T) {
	repo := newProject(t)
	genesis(t, repo)
	_, _ = startedChange(t, repo, CategoryBehaviorChange)

	// Move main out from under the change (fresh genesis on a modified tree).
	writeAt(t, repo.WorkDir, "app.txt", "moved\n")
	genesis(t, repo)

	changes, err := OpenChanges(repo)
	if err != nil || len(changes) != 1 {
		t.Fatalf("changes = %v, %v", changes, err)
	}
	if !changes[0].BaseStale {
		t.Fatal("base staleness not detected")
	}
	if wtSt, _ := gitx.Open(changes[0].Worktree); wtSt != nil {
		st, err := Status(wtSt)
		if err != nil {
			t.Fatal(err)
		}
		if !st.Change.BaseStale {
			t.Fatal("candidate status misses base staleness")
		}
	}
}

func TestDeltaTemplateIsInert(t *testing.T) {
	if !strings.Contains(deltaTemplate, "telos:op") {
		t.Fatal("template lost its guidance")
	}
}
