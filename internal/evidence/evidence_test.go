package evidence

import (
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/hugues31/telos-sdd/internal/gitx"
)

func newRepo(t *testing.T) *gitx.Repo {
	t.Helper()
	dir := t.TempDir()
	for _, args := range [][]string{
		{"init", "--quiet", "-b", "main"},
		{"config", "user.email", "telos@test"},
		{"config", "user.name", "telos test"},
		{"config", "core.autocrlf", "false"},
		{"config", "gc.auto", "0"},
	} {
		cmd := exec.Command("git", args...)
		cmd.Dir = dir
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("git %v: %v\n%s", args, err, out)
		}
	}
	repo, err := gitx.Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	return repo
}

func writeFile(t *testing.T, repo *gitx.Repo, rel, content string) {
	t.Helper()
	path := filepath.Join(repo.WorkDir, filepath.FromSlash(rel))
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func snapshot(t *testing.T, repo *gitx.Repo) gitx.OID {
	t.Helper()
	if _, err := repo.CommitAll("snapshot"); err != nil {
		t.Fatal(err)
	}
	tree, err := repo.TreeOf("HEAD")
	if err != nil {
		t.Fatal(err)
	}
	return tree
}

func TestRecordKeyDeterministic(t *testing.T) {
	r := Record{Kind: KindSuite, Command: "go test ./...", Cwd: ".",
		DependsOn: DependsOn{Closure: "tracked_tree", ClosureDigest: "abc", Toolchain: Toolchain{Go: "go1.24", OS: "linux", Arch: "amd64"}}}
	a, b := r.Key(), r.Key()
	if a != b || a == "" {
		t.Fatalf("key not deterministic: %s vs %s", a, b)
	}
	r2 := r
	r2.DependsOn.ClosureDigest = "different"
	if r2.Key() == a {
		t.Fatal("closure change did not change the key")
	}
	// The result is NOT part of the key: same inputs, same key.
	r3 := r
	r3.Result = Result{Status: "fail"}
	if r3.Key() != a {
		t.Fatal("result leaked into the key")
	}
	if FileName(a) != "EVD-"+a[:12]+".json" {
		t.Fatalf("FileName = %s", FileName(a))
	}
}

func TestTreeClosureExcludesContractAndChanges(t *testing.T) {
	repo := newRepo(t)
	writeFile(t, repo, "src/app.go", "package app\n")
	writeFile(t, repo, "spec/PRODUCT.md", "# P\n")
	writeFile(t, repo, "changes/CHG-001/intent.md", "why\n")
	writeFile(t, repo, "telos.toml", "project_id = \"x\"\n")
	tree := snapshot(t, repo)

	dep, err := TreeClosure(repo, tree)
	if err != nil {
		t.Fatal(err)
	}
	if dep.Closure != "tracked_tree" || dep.ClosureDigest == "" {
		t.Fatalf("dep = %+v", dep)
	}

	// Contract and change-record edits do not move the code closure.
	writeFile(t, repo, "spec/PRODUCT.md", "# P changed\n")
	writeFile(t, repo, "changes/CHG-001/intent.md", "why changed\n")
	tree2 := snapshot(t, repo)
	dep2, _ := TreeClosure(repo, tree2)
	if dep2.ClosureDigest != dep.ClosureDigest {
		t.Fatal("spec/changes edits moved the tree closure")
	}

	// Code edits do.
	writeFile(t, repo, "src/app.go", "package app // changed\n")
	tree3 := snapshot(t, repo)
	dep3, _ := TreeClosure(repo, tree3)
	if dep3.ClosureDigest == dep.ClosureDigest {
		t.Fatal("code edit did not move the tree closure")
	}
}

func TestGoClosureSeparatesPackages(t *testing.T) {
	repo := newRepo(t)
	writeFile(t, repo, "go.mod", "module example.com/toy\n\ngo 1.24\n")
	writeFile(t, repo, "pkga/pkga.go", "package pkga\n\nfunc A() int { return 0 }\n")
	writeFile(t, repo, "pkgb/pkgb.go", "package pkgb\n\nfunc B() int { return 0 }\n")
	tree := snapshot(t, repo)

	depA, err := GoClosure(repo, tree, repo.WorkDir, []string{"pkga"})
	if err != nil {
		t.Fatal(err)
	}
	if depA.Closure != "go_packages" {
		t.Fatalf("depA = %+v", depA)
	}

	// A change in pkgb does not move pkga's closure.
	writeFile(t, repo, "pkgb/pkgb.go", "package pkgb\n\nfunc B() int { return 2 }\n")
	tree2 := snapshot(t, repo)
	depA2, _ := GoClosure(repo, tree2, repo.WorkDir, []string{"pkga"})
	if depA2.ClosureDigest != depA.ClosureDigest {
		t.Fatal("pkgb edit moved pkga's closure")
	}
	// A change in pkga does.
	writeFile(t, repo, "pkga/pkga.go", "package pkga\n\nfunc A() int { return 1 }\n")
	tree3 := snapshot(t, repo)
	depA3, _ := GoClosure(repo, tree3, repo.WorkDir, []string{"pkga"})
	if depA3.ClosureDigest == depA.ClosureDigest {
		t.Fatal("pkga edit did not move its closure")
	}

	// Recompute agrees with GoClosure.
	record := &Record{Kind: KindRedGreen, DependsOn: depA}
	digest, err := Recompute(repo, tree2, repo.WorkDir, record)
	if err != nil || digest != depA.ClosureDigest {
		t.Fatalf("Recompute = %s, %v; want %s", digest, err, depA.ClosureDigest)
	}

	// Unknown packages fall back to the conservative tree closure.
	depBad, err := GoClosure(repo, tree, repo.WorkDir, []string{"missing"})
	if err != nil || depBad.Closure != "tracked_tree" {
		t.Fatalf("fallback = %+v, %v", depBad, err)
	}
}

func TestRunSuiteOnTree(t *testing.T) {
	repo := newRepo(t)
	writeFile(t, repo, "marker.txt", "present\n")
	tree := snapshot(t, repo)

	run, err := RunSuiteOnTree(repo, tree, []string{"go version"}, io.Discard)
	if err != nil || !run.Pass {
		t.Fatalf("passing run = %+v, %v", run, err)
	}
	run, err = RunSuiteOnTree(repo, tree, []string{"go definitely-not-a-subcommand"}, io.Discard)
	if err != nil || run.Pass || run.OutputTail == "" {
		t.Fatalf("failing run = %+v, %v", run, err)
	}
	// The candidate worktree was never touched.
	if dirty, _ := repo.DirtyPaths(); dirty != nil {
		t.Fatalf("suite run dirtied the repo: %v", dirty)
	}
}
