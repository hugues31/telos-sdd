package kernel

import (
	"strings"
	"testing"
)

func TestSalvageEdges(t *testing.T) {
	repo := newProject(t)
	genesis(t, repo)

	// Nothing to salvage on a clean root.
	if _, err := Salvage(repo, "", ""); errCode(t, err) != "TELOS_NOTHING_PENDING" {
		t.Fatal(err)
	}
	if _, err := Restore(repo); errCode(t, err) != "TELOS_NOTHING_PENDING" {
		t.Fatal(err)
	}

	// Routing into an unknown change restores the diff to the root.
	writeAt(t, repo.WorkDir, "app.txt", "tampered\n")
	if _, err := Salvage(repo, "CHG-404", ""); errCode(t, err) != "TELOS_CHANGE_UNKNOWN" {
		t.Fatal(err)
	}
	if dirty, _ := repo.DirtyPaths(); len(dirty) != 1 {
		t.Fatalf("failed salvage lost the diff: %v", dirty)
	}

	// Status proposes the capture with the next id.
	st, err := Status(repo)
	if err != nil || st.Salvage == nil {
		t.Fatalf("status = %+v, %v", st, err)
	}
	if st.Salvage.Proposal != "new_change" || !strings.Contains(st.Salvage.Prompt, "CHG-001") {
		t.Fatalf("proposal = %+v", st.Salvage)
	}

	// A spec edit is captured too, with the warning that it must become a delta.
	writeAt(t, repo.WorkDir, "spec/PRODUCT.md", testProduct+"\nedited\n")
	result, err := Salvage(repo, "", "captured")
	if err != nil {
		t.Fatal(err)
	}
	if len(result.SpecTouched) != 1 || result.Change != "CHG-001" {
		t.Fatalf("result = %+v", result)
	}
	if dirty, _ := repo.DirtyPaths(); dirty != nil {
		t.Fatalf("root not restored: %v", dirty)
	}
	if st, _ := Status(repo); st.State != StateCertified {
		t.Fatalf("root after salvage = %+v", st)
	}

	// Restore discards.
	writeAt(t, repo.WorkDir, "junk.txt", "x\n")
	paths, err := Restore(repo)
	if err != nil || len(paths) != 1 {
		t.Fatalf("restore = %v, %v", paths, err)
	}
	if dirty, _ := repo.DirtyPaths(); dirty != nil {
		t.Fatalf("restore left dirt: %v", dirty)
	}
}
