package gitx

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

func newRepo(t *testing.T) *Repo {
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
	repo, err := Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	return repo
}

func writeFile(t *testing.T, repo *Repo, rel, content string) {
	t.Helper()
	path := filepath.Join(repo.WorkDir, filepath.FromSlash(rel))
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

// commitAll stages everything and creates a commit through the plumbing path
// the kernel uses (add -A, write-tree, commit-tree, update-ref).
func commitAll(t *testing.T, repo *Repo, msg string) OID {
	t.Helper()
	if err := repo.AddAll(); err != nil {
		t.Fatal(err)
	}
	tree, err := repo.WriteTree()
	if err != nil {
		t.Fatal(err)
	}
	var parents []OID
	if head, err := repo.Head(); err == nil {
		parents = append(parents, head)
	}
	commit, err := repo.CommitTree(tree, parents, msg)
	if err != nil {
		t.Fatal(err)
	}
	ref, err := repo.HeadRef()
	if err != nil || ref == "" {
		t.Fatalf("head ref: %q %v", ref, err)
	}
	if err := repo.UpdateRef(ref, commit); err != nil {
		t.Fatal(err)
	}
	if err := repo.ResetHardTo(string(commit)); err != nil {
		t.Fatal(err)
	}
	return commit
}

func TestUnbornHead(t *testing.T) {
	repo := newRepo(t)
	if _, err := repo.Head(); !errors.Is(err, ErrNoCommits) {
		t.Fatalf("Head on unborn = %v, want ErrNoCommits", err)
	}
	if ref, err := repo.HeadRef(); err != nil || ref != "refs/heads/main" {
		t.Fatalf("HeadRef = %q, %v", ref, err)
	}
}

func TestCommitRoundTrip(t *testing.T) {
	repo := newRepo(t)
	writeFile(t, repo, "a.txt", "one\n")
	writeFile(t, repo, "dir/b.txt", "two\n")
	commit := commitAll(t, repo, "first")

	head, err := repo.Head()
	if err != nil || head != commit {
		t.Fatalf("Head = %s, %v; want %s", head, err, commit)
	}
	files, err := repo.LsTree("HEAD")
	if err != nil {
		t.Fatal(err)
	}
	if len(files) != 2 || files["a.txt"] == "" || files["dir/b.txt"] == "" {
		t.Fatalf("LsTree = %v", files)
	}
	content, err := repo.BlobAt("HEAD", "dir/b.txt")
	if err != nil || string(content) != "two\n" {
		t.Fatalf("BlobAt = %q, %v", content, err)
	}
	tree, err := repo.TreeOf("HEAD")
	if err != nil || tree == "" {
		t.Fatalf("TreeOf = %q, %v", tree, err)
	}
	if sub, err := repo.SubtreeOf("HEAD", "dir"); err != nil || sub == "" {
		t.Fatalf("SubtreeOf(dir) = %q, %v", sub, err)
	}
	if sub, err := repo.SubtreeOf("HEAD", "missing"); err != nil || sub != "" {
		t.Fatalf("SubtreeOf(missing) = %q, %v — want empty, nil", sub, err)
	}
}

func TestDirtyPaths(t *testing.T) {
	repo := newRepo(t)
	writeFile(t, repo, "a.txt", "one\n")
	commitAll(t, repo, "first")

	dirty, err := repo.DirtyPaths()
	if err != nil || dirty != nil {
		t.Fatalf("clean repo DirtyPaths = %v, %v", dirty, err)
	}
	writeFile(t, repo, "a.txt", "changed\n")
	writeFile(t, repo, "new.txt", "untracked\n")
	dirty, err = repo.DirtyPaths()
	if err != nil {
		t.Fatal(err)
	}
	if len(dirty) != 2 || dirty[0] != "a.txt" || dirty[1] != "new.txt" {
		t.Fatalf("DirtyPaths = %v, want [a.txt new.txt]", dirty)
	}
	if err := repo.ResetHardTo("HEAD"); err != nil {
		t.Fatal(err)
	}
	os.Remove(filepath.Join(repo.WorkDir, "new.txt"))
	dirty, err = repo.DirtyPaths()
	if err != nil || dirty != nil {
		t.Fatalf("restored repo DirtyPaths = %v, %v", dirty, err)
	}
}

func TestNotesRoundTrip(t *testing.T) {
	repo := newRepo(t)
	writeFile(t, repo, "a.txt", "one\n")
	commit := commitAll(t, repo, "first")

	if _, err := repo.NoteShow(NotesRef, commit); !errors.Is(err, ErrNoNote) {
		t.Fatalf("NoteShow before add = %v, want ErrNoNote", err)
	}
	payload := []byte(`{"telos_certificate":1}`)
	if err := repo.NoteAdd(NotesRef, commit, payload); err != nil {
		t.Fatal(err)
	}
	got, err := repo.NoteShow(NotesRef, commit)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != string(payload)+"\n" && string(got) != string(payload) {
		t.Fatalf("NoteShow = %q", got)
	}
	// Replacing is explicit and allowed at the plumbing layer.
	if err := repo.NoteAdd(NotesRef, commit, []byte("v2")); err != nil {
		t.Fatal(err)
	}
	got, _ = repo.NoteShow(NotesRef, commit)
	if len(got) == 0 || got[0] != 'v' {
		t.Fatalf("NoteShow after replace = %q", got)
	}
}

func TestDiffNames(t *testing.T) {
	repo := newRepo(t)
	writeFile(t, repo, "a.txt", "one\n")
	first := commitAll(t, repo, "first")
	writeFile(t, repo, "a.txt", "changed\n")
	writeFile(t, repo, "b.txt", "new\n")
	second := commitAll(t, repo, "second")

	names, err := repo.DiffNames(string(first), string(second))
	if err != nil {
		t.Fatal(err)
	}
	if len(names) != 2 {
		t.Fatalf("DiffNames = %v", names)
	}
}

func TestHashObjectCat(t *testing.T) {
	repo := newRepo(t)
	oid, err := repo.HashObject([]byte("hello blob"))
	if err != nil {
		t.Fatal(err)
	}
	content, err := repo.CatBlob(oid)
	if err != nil || string(content) != "hello blob" {
		t.Fatalf("CatBlob = %q, %v", content, err)
	}
}
