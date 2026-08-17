package gitx

import (
	"os"
	"path/filepath"
	"testing"
)

func TestWorktreeLifecycle(t *testing.T) {
	repo := newRepo(t)
	writeFile(t, repo, "a.txt", "one\n")
	base := commitAll(t, repo, "first")

	wtPath := repo.SiblingWorktreePath("CHG-001")
	if err := repo.WorktreeAdd(wtPath, "telos/CHG-001", base); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = repo.WorktreeRemove(wtPath) })

	wt, err := Open(wtPath)
	if err != nil {
		t.Fatal(err)
	}
	if branch, _ := wt.CurrentBranch(); branch != "telos/CHG-001" {
		t.Fatalf("candidate branch = %q", branch)
	}
	if content, err := os.ReadFile(filepath.Join(wtPath, "a.txt")); err != nil || string(content) != "one\n" {
		t.Fatalf("worktree content = %q, %v", content, err)
	}

	// Commit inside the candidate; the base repo's branch is unaffected.
	writeFile(t, wt, "b.txt", "two\n")
	c1, err := wt.CommitAll("candidate work")
	if err != nil {
		t.Fatal(err)
	}
	if c1 == base {
		t.Fatal("CommitAll did not advance the candidate branch")
	}
	if again, err := wt.CommitAll("noop"); err != nil || again != c1 {
		t.Fatalf("clean CommitAll = %s, %v; want unchanged %s", again, err, c1)
	}
	if head, _ := repo.Head(); head != base {
		t.Fatal("candidate commit leaked into the main worktree's branch")
	}

	list, err := repo.WorktreeList()
	if err != nil || len(list) != 2 {
		t.Fatalf("WorktreeList = %v, %v", list, err)
	}
	found := false
	for _, info := range list {
		if info.Branch == "telos/CHG-001" {
			found = true
		}
	}
	if !found {
		t.Fatalf("worktree list misses the candidate: %v", list)
	}

	branches, err := repo.Branches("telos/CHG-*")
	if err != nil || len(branches) != 1 || branches[0] != "telos/CHG-001" {
		t.Fatalf("Branches = %v, %v", branches, err)
	}

	if err := repo.WorktreeRemove(wtPath); err != nil {
		t.Fatal(err)
	}
	if err := repo.BranchDelete("telos/CHG-001"); err != nil {
		t.Fatal(err)
	}
	if branches, _ := repo.Branches("telos/CHG-*"); len(branches) != 0 {
		t.Fatalf("branch not deleted: %v", branches)
	}
}

func TestTreeFromFilesDeterministic(t *testing.T) {
	repo := newRepo(t)
	files := map[string][]byte{
		"spec/PRODUCT.md": []byte("# P\n"),
		"spec/core.md":    []byte("# C\n"),
	}
	a, err := repo.TreeFromFiles(files)
	if err != nil {
		t.Fatal(err)
	}
	b, err := repo.TreeFromFiles(files)
	if err != nil || a != b {
		t.Fatalf("TreeFromFiles not deterministic: %s vs %s (%v)", a, b, err)
	}
	files["spec/core.md"] = []byte("# C changed\n")
	c, err := repo.TreeFromFiles(files)
	if err != nil || c == a {
		t.Fatalf("content change did not change the tree: %s (%v)", c, err)
	}
	// The worktree and real index are untouched.
	if dirty, _ := repo.DirtyPaths(); dirty != nil {
		t.Fatalf("TreeFromFiles dirtied the worktree: %v", dirty)
	}
	// The subtree digest is addressable.
	sub, err := repo.SubtreeOf(string(a), "spec")
	if err != nil || sub == "" {
		t.Fatalf("SubtreeOf(folded, spec) = %q, %v", sub, err)
	}
}
