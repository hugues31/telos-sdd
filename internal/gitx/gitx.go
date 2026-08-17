// Package gitx is the Git plumbing layer of the Telos kernel. It carries zero
// Telos semantics: it shells out to git, parses plumbing output, and exposes
// object identity (OIDs), tree access, refs, and notes. Everything above it
// (kernel, evidence, contract) speaks in these primitives; nothing above it
// runs git directly.
package gitx

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// OID is a full-hex git object id, opaque to callers (sha1 or sha256 repos).
type OID string

// NotesRef is the ref under which Telos certificates are stored.
const NotesRef = "refs/notes/telos"

// ErrNoCommits marks operations that need a commit in a repository whose HEAD
// is unborn.
var ErrNoCommits = errors.New("repository has no commits")

// ErrNoNote marks a commit that carries no note under the requested ref.
var ErrNoNote = errors.New("no note for commit")

// Repo is an open git repository rooted at WorkDir.
type Repo struct {
	WorkDir string
	GitDir  string
}

// Open resolves the repository containing dir.
func Open(dir string) (*Repo, error) {
	top, err := output(dir, "rev-parse", "--show-toplevel")
	if err != nil {
		return nil, fmt.Errorf("not inside a git worktree: %w", err)
	}
	gitDir, err := output(dir, "rev-parse", "--absolute-git-dir")
	if err != nil {
		return nil, err
	}
	return &Repo{WorkDir: top, GitDir: gitDir}, nil
}

func output(dir string, args ...string) (string, error) {
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	cmd.Env = append(os.Environ(), "GIT_TERMINAL_PROMPT=0")
	out, err := cmd.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("git %s: %w: %s", strings.Join(args, " "), err, strings.TrimSpace(string(out)))
	}
	return strings.TrimSpace(string(out)), nil
}

func (r *Repo) run(stdin []byte, args ...string) (string, error) {
	cmd := exec.Command("git", args...)
	cmd.Dir = r.WorkDir
	cmd.Env = append(os.Environ(), "GIT_TERMINAL_PROMPT=0")
	if stdin != nil {
		cmd.Stdin = strings.NewReader(string(stdin))
	}
	out, err := cmd.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("git %s: %w: %s", strings.Join(args, " "), err, strings.TrimSpace(string(out)))
	}
	return strings.TrimSpace(string(out)), nil
}

// RevParse resolves a revision to an OID.
func (r *Repo) RevParse(rev string) (OID, error) {
	out, err := r.run(nil, "rev-parse", "--verify", "--quiet", rev)
	if err != nil {
		return "", fmt.Errorf("cannot resolve %q: %w", rev, err)
	}
	return OID(out), nil
}

// Head returns the commit HEAD points at, or ErrNoCommits on an unborn HEAD.
func (r *Repo) Head() (OID, error) {
	oid, err := r.RevParse("HEAD")
	if err != nil {
		return "", ErrNoCommits
	}
	return oid, nil
}

// HeadRef returns the ref HEAD symbolically points at (e.g. refs/heads/main),
// or "" when HEAD is detached.
func (r *Repo) HeadRef() (string, error) {
	out, err := r.run(nil, "symbolic-ref", "--quiet", "HEAD")
	if err != nil {
		return "", nil
	}
	return out, nil
}

// TreeOf resolves a revision's tree OID.
func (r *Repo) TreeOf(rev string) (OID, error) {
	out, err := r.run(nil, "rev-parse", "--verify", "--quiet", rev+"^{tree}")
	if err != nil {
		return "", fmt.Errorf("cannot resolve tree of %q: %w", rev, err)
	}
	return OID(out), nil
}

// SubtreeOf resolves the tree OID of a directory inside a revision. It
// returns "" (no error) when the directory does not exist there.
func (r *Repo) SubtreeOf(rev, dir string) (OID, error) {
	out, err := r.run(nil, "rev-parse", "--verify", "--quiet", rev+":"+dir)
	if err != nil {
		return "", nil
	}
	return OID(out), nil
}

// runRaw executes git without trimming stdout — porcelain -z records are
// position-sensitive (a leading space is a status column, not noise).
func (r *Repo) runRaw(args ...string) (string, error) {
	cmd := exec.Command("git", args...)
	cmd.Dir = r.WorkDir
	cmd.Env = append(os.Environ(), "GIT_TERMINAL_PROMPT=0")
	out, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("git %s: %w", strings.Join(args, " "), err)
	}
	return string(out), nil
}

// DirtyPaths lists worktree paths differing from HEAD (staged, unstaged, and
// untracked-not-ignored), sorted by git. Empty means clean.
func (r *Repo) DirtyPaths() ([]string, error) {
	out, err := r.runRaw("status", "--porcelain", "-z", "--untracked-files=all")
	if err != nil {
		return nil, err
	}
	if out == "" {
		return nil, nil
	}
	var paths []string
	records := strings.Split(out, "\x00")
	for i := 0; i < len(records); i++ {
		rec := records[i]
		if len(rec) < 4 {
			continue
		}
		status, p := rec[:2], rec[3:]
		paths = append(paths, p)
		// Rename/copy records carry the original path as the next NUL field.
		if status[0] == 'R' || status[0] == 'C' {
			i++
			if i < len(records) && records[i] != "" {
				paths = append(paths, records[i])
			}
		}
	}
	return paths, nil
}

// AddAll stages every change in the worktree, including untracked files.
func (r *Repo) AddAll() error {
	_, err := r.run(nil, "add", "-A")
	return err
}

// WriteTree writes the index as a tree object.
func (r *Repo) WriteTree() (OID, error) {
	out, err := r.run(nil, "write-tree")
	return OID(out), err
}

// CommitTree creates a commit object for tree with the given parents.
func (r *Repo) CommitTree(tree OID, parents []OID, message string) (OID, error) {
	args := []string{"commit-tree", string(tree), "-m", message}
	for _, p := range parents {
		args = append(args, "-p", string(p))
	}
	out, err := r.run(nil, args...)
	return OID(out), err
}

// UpdateRef points ref at oid, creating it if needed.
func (r *Repo) UpdateRef(ref string, oid OID) error {
	_, err := r.run(nil, "update-ref", ref, string(oid))
	return err
}

// ResetHardTo makes the worktree and index match rev exactly.
func (r *Repo) ResetHardTo(rev string) error {
	_, err := r.run(nil, "reset", "--hard", "--quiet", rev)
	return err
}

// HashObject writes data as a blob and returns its OID.
func (r *Repo) HashObject(data []byte) (OID, error) {
	out, err := r.run(data, "hash-object", "-w", "--stdin")
	return OID(out), err
}

// CatBlob returns a blob's content. Blob content is returned verbatim, so it
// bypasses run's trimming.
func (r *Repo) CatBlob(oid OID) ([]byte, error) {
	cmd := exec.Command("git", "cat-file", "blob", string(oid))
	cmd.Dir = r.WorkDir
	out, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("git cat-file blob %s: %w", oid, err)
	}
	return out, nil
}

// BlobAt returns the content of path inside rev, or ErrNoNote-style not-found
// via the wrapped git error.
func (r *Repo) BlobAt(rev, path string) ([]byte, error) {
	oid, err := r.RevParse(rev + ":" + path)
	if err != nil {
		return nil, err
	}
	return r.CatBlob(oid)
}

// LsTree maps every file path inside rev's tree to its blob OID.
func (r *Repo) LsTree(rev string) (map[string]OID, error) {
	out, err := r.run(nil, "ls-tree", "-r", "-z", "--full-tree", rev)
	if err != nil {
		return nil, err
	}
	files := map[string]OID{}
	for _, rec := range strings.Split(out, "\x00") {
		// "<mode> <type> <oid>\t<path>"
		meta, path, ok := strings.Cut(rec, "\t")
		if !ok {
			continue
		}
		fields := strings.Fields(meta)
		if len(fields) != 3 || fields[1] != "blob" {
			continue
		}
		files[path] = OID(fields[2])
	}
	return files, nil
}

// DiffNames lists paths differing between two revisions.
func (r *Repo) DiffNames(a, b string) ([]string, error) {
	out, err := r.run(nil, "diff", "--name-only", "-z", a, b)
	if err != nil {
		return nil, err
	}
	if out == "" {
		return nil, nil
	}
	var paths []string
	for _, p := range strings.Split(out, "\x00") {
		if p != "" {
			paths = append(paths, p)
		}
	}
	return paths, nil
}

// DiffPatch returns the unified diff between two revisions, verbatim.
func (r *Repo) DiffPatch(a, b string) (string, error) {
	return r.runRaw("diff", "--binary", a, b)
}

// NoteShow returns the note blob attached to commit under ref, or ErrNoNote.
func (r *Repo) NoteShow(ref string, commit OID) ([]byte, error) {
	oid, err := r.run(nil, "notes", "--ref="+ref, "list", string(commit))
	if err != nil {
		return nil, ErrNoNote
	}
	return r.CatBlob(OID(oid))
}

// NoteAdd attaches (or replaces) the note blob on commit under ref.
func (r *Repo) NoteAdd(ref string, commit OID, data []byte) error {
	tmp, err := os.CreateTemp("", "telos-note-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	_, err = r.run(nil, "notes", "--ref="+ref, "add", "-f", "-F", tmp.Name(), string(commit))
	return err
}

// Available reports whether the git binary can be found.
func Available() bool {
	_, err := exec.LookPath("git")
	return err == nil
}

// IsRepo reports whether dir is inside a git worktree.
func IsRepo(dir string) bool {
	out, err := output(dir, "rev-parse", "--is-inside-work-tree")
	return err == nil && out == "true"
}

// Rel converts an absolute path under the worktree to a slash-separated
// repo-relative path.
func (r *Repo) Rel(abs string) (string, error) {
	rel, err := filepath.Rel(r.WorkDir, abs)
	if err != nil {
		return "", err
	}
	return filepath.ToSlash(rel), nil
}
