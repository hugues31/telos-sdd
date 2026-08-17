package gitx

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
)

// CurrentBranch returns the short branch name HEAD points at, or "" when
// detached.
func (r *Repo) CurrentBranch() (string, error) {
	out, err := r.run(nil, "symbolic-ref", "--short", "--quiet", "HEAD")
	if err != nil {
		return "", nil
	}
	return out, nil
}

// WorktreeInfo describes one linked worktree.
type WorktreeInfo struct {
	Path   string
	Branch string // short name, "" when detached
}

// WorktreeAdd creates a new worktree at path on a new branch starting at the
// given commit.
func (r *Repo) WorktreeAdd(path, branch string, at OID) error {
	_, err := r.run(nil, "worktree", "add", "--quiet", "-b", branch, path, string(at))
	return err
}

// WorktreeAddDetached creates a throwaway worktree at path checked out at the
// given commit, with no branch.
func (r *Repo) WorktreeAddDetached(path string, at OID) error {
	_, err := r.run(nil, "worktree", "add", "--quiet", "--detach", path, string(at))
	return err
}

// WorktreeRemove force-removes a worktree.
func (r *Repo) WorktreeRemove(path string) error {
	_, err := r.run(nil, "worktree", "remove", "--force", path)
	return err
}

// WorktreeList lists the repository's worktrees (including the main one).
func (r *Repo) WorktreeList() ([]WorktreeInfo, error) {
	out, err := r.run(nil, "worktree", "list", "--porcelain")
	if err != nil {
		return nil, err
	}
	var infos []WorktreeInfo
	var current WorktreeInfo
	flush := func() {
		if current.Path != "" {
			infos = append(infos, current)
		}
		current = WorktreeInfo{}
	}
	for _, line := range strings.Split(out, "\n") {
		line = strings.TrimSpace(line)
		switch {
		case line == "":
			flush()
		case strings.HasPrefix(line, "worktree "):
			flush()
			current.Path = strings.TrimPrefix(line, "worktree ")
		case strings.HasPrefix(line, "branch "):
			current.Branch = strings.TrimPrefix(strings.TrimPrefix(line, "branch "), "refs/heads/")
		}
	}
	flush()
	return infos, nil
}

// BranchDelete force-deletes a branch.
func (r *Repo) BranchDelete(name string) error {
	_, err := r.run(nil, "branch", "-D", "--quiet", name)
	return err
}

// Branches lists short branch names matching a refs/heads/ pattern
// (e.g. "telos/CHG-*"), sorted.
func (r *Repo) Branches(pattern string) ([]string, error) {
	out, err := r.run(nil, "for-each-ref", "--format=%(refname:short)", "refs/heads/"+pattern)
	if err != nil {
		return nil, err
	}
	if out == "" {
		return nil, nil
	}
	names := strings.Split(out, "\n")
	sort.Strings(names)
	return names, nil
}

// CommitAll stages everything in the worktree and commits it on the current
// branch. When nothing changed it returns HEAD unchanged.
func (r *Repo) CommitAll(message string) (OID, error) {
	if err := r.AddAll(); err != nil {
		return "", err
	}
	dirty, err := r.runRaw("status", "--porcelain")
	if err != nil {
		return "", err
	}
	if strings.TrimSpace(dirty) == "" {
		return r.Head()
	}
	if _, err := r.run(nil, "commit", "--quiet", "-m", message); err != nil {
		return "", err
	}
	return r.Head()
}

// TreeFromFiles writes the given path→content mapping as a tree object using
// a temporary index, without touching the worktree or the real index. Paths
// are slash-separated and repo-relative. The resulting OID is deterministic
// for identical contents — it is the digest primitive for folded contracts.
func (r *Repo) TreeFromFiles(files map[string][]byte) (OID, error) {
	paths := make([]string, 0, len(files))
	for p := range files {
		paths = append(paths, p)
	}
	sort.Strings(paths)

	var lines strings.Builder
	for _, p := range paths {
		blob, err := r.HashObject(files[p])
		if err != nil {
			return "", err
		}
		fmt.Fprintf(&lines, "100644 %s\t%s\n", blob, p)
	}

	tmp, err := os.CreateTemp("", "telos-index-*")
	if err != nil {
		return "", err
	}
	tmpPath := tmp.Name()
	tmp.Close()
	os.Remove(tmpPath) // git wants to create it itself
	defer os.Remove(tmpPath)

	env := append(os.Environ(), "GIT_TERMINAL_PROMPT=0", "GIT_INDEX_FILE="+tmpPath)
	index := exec.Command("git", "update-index", "--add", "--index-info")
	index.Dir = r.WorkDir
	index.Env = env
	index.Stdin = strings.NewReader(lines.String())
	if out, err := index.CombinedOutput(); err != nil {
		return "", fmt.Errorf("git update-index: %w: %s", err, strings.TrimSpace(string(out)))
	}
	write := exec.Command("git", "write-tree")
	write.Dir = r.WorkDir
	write.Env = env
	out, err := write.Output()
	if err != nil {
		return "", fmt.Errorf("git write-tree: %w", err)
	}
	return OID(strings.TrimSpace(string(out))), nil
}

// StashPush stashes every worktree change including untracked files.
func (r *Repo) StashPush(message string) error {
	_, err := r.run(nil, "stash", "push", "--include-untracked", "--quiet", "-m", message)
	return err
}

// StashApply applies the most recent stash without dropping it.
func (r *Repo) StashApply() error {
	_, err := r.run(nil, "stash", "apply", "--quiet")
	return err
}

// StashDrop drops the most recent stash.
func (r *Repo) StashDrop() error {
	_, err := r.run(nil, "stash", "drop", "--quiet")
	return err
}

// CleanUntracked removes untracked files and directories.
func (r *Repo) CleanUntracked() error {
	_, err := r.run(nil, "clean", "-fdq")
	return err
}

// RebaseOnto replays branch's commits after oldBase onto newTip. On conflict
// the rebase is aborted (the worktree returns to its pre-rebase state) and
// the error carries git's conflict report.
func (r *Repo) RebaseOnto(newTip, oldBase OID, branch string) error {
	_, err := r.run(nil, "rebase", "--onto", string(newTip), string(oldBase), branch)
	if err != nil {
		_, _ = r.run(nil, "rebase", "--abort")
		return err
	}
	return nil
}

// TreeFromEntries writes a tree from existing blob OIDs (no re-hashing)
// using a temporary index. Paths are slash-separated and repo-relative.
func (r *Repo) TreeFromEntries(entries map[string]OID) (OID, error) {
	paths := make([]string, 0, len(entries))
	for p := range entries {
		paths = append(paths, p)
	}
	sort.Strings(paths)

	var lines strings.Builder
	for _, p := range paths {
		fmt.Fprintf(&lines, "100644 %s\t%s\n", entries[p], p)
	}
	tmp, err := os.CreateTemp("", "telos-index-*")
	if err != nil {
		return "", err
	}
	tmpPath := tmp.Name()
	tmp.Close()
	os.Remove(tmpPath)
	defer os.Remove(tmpPath)

	env := append(os.Environ(), "GIT_TERMINAL_PROMPT=0", "GIT_INDEX_FILE="+tmpPath)
	index := exec.Command("git", "update-index", "--add", "--index-info")
	index.Dir = r.WorkDir
	index.Env = env
	index.Stdin = strings.NewReader(lines.String())
	if out, err := index.CombinedOutput(); err != nil {
		return "", fmt.Errorf("git update-index: %w: %s", err, strings.TrimSpace(string(out)))
	}
	write := exec.Command("git", "write-tree")
	write.Dir = r.WorkDir
	write.Env = env
	out, err := write.Output()
	if err != nil {
		return "", fmt.Errorf("git write-tree: %w", err)
	}
	return OID(strings.TrimSpace(string(out))), nil
}

// SiblingWorktreePath computes the conventional candidate location for a
// change: a sibling directory of the repository named after it.
func (r *Repo) SiblingWorktreePath(changeID string) string {
	return filepath.Join(filepath.Dir(r.WorkDir), filepath.Base(r.WorkDir)+"-"+changeID)
}
