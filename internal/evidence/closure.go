package evidence

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"os/exec"
	"path"
	"path/filepath"
	"runtime"
	"sort"
	"strings"

	"github.com/hugues31/telos-sdd/internal/gitx"
)

// toolchain pins the running environment.
func toolchain() Toolchain {
	return Toolchain{Go: runtime.Version(), OS: runtime.GOOS, Arch: runtime.GOARCH}
}

// digestEntries hashes a sorted path→OID selection into a closure digest.
func digestEntries(entries map[string]gitx.OID) string {
	paths := make([]string, 0, len(entries))
	for p := range entries {
		paths = append(paths, p)
	}
	sort.Strings(paths)
	h := sha256.New()
	for _, p := range paths {
		io.WriteString(h, p)
		h.Write([]byte{0})
		io.WriteString(h, string(entries[p]))
		h.Write([]byte{'\n'})
	}
	return hex.EncodeToString(h.Sum(nil))
}

// codePath reports whether a tracked path belongs to the code closure: the
// contract, change records, and configuration are excluded (the contract
// enters contract-sensitive records via DependsOn.Contract, configuration
// via DependsOn.Policy).
func codePath(p string) bool {
	return p != "telos.toml" &&
		!strings.HasPrefix(p, "spec/") &&
		!strings.HasPrefix(p, "changes/") &&
		!strings.HasPrefix(p, ".claude/") && !strings.HasPrefix(p, ".codex/") && !strings.HasPrefix(p, ".agents/") &&
		p != "CLAUDE.md" && p != "AGENTS.md"
}

// TreeClosure computes the conservative closure: every tracked code path of
// the tree.
func TreeClosure(repo *gitx.Repo, tree gitx.OID) (DependsOn, error) {
	files, err := repo.LsTree(string(tree))
	if err != nil {
		return DependsOn{}, err
	}
	selected := map[string]gitx.OID{}
	for p, oid := range files {
		if codePath(p) {
			selected[p] = oid
		}
	}
	return DependsOn{Closure: "tracked_tree", ClosureDigest: digestEntries(selected), Toolchain: toolchain()}, nil
}

// GoClosure computes the import-graph closure of the given package dirs
// (slash-separated, repo-relative) by running `go list -deps -json` inside
// checkout (a worktree holding exactly the tree under proof). Any failure
// falls back to the conservative tree closure — unknown dependencies
// invalidate, they never silently narrow.
func GoClosure(repo *gitx.Repo, tree gitx.OID, checkout string, pkgDirs []string) (DependsOn, error) {
	if len(pkgDirs) == 0 {
		return TreeClosure(repo, tree)
	}
	sort.Strings(pkgDirs)
	args := []string{"list", "-deps", "-json=Dir,GoFiles,CgoFiles,TestGoFiles,XTestGoFiles,EmbedFiles,Standard"}
	for _, dir := range pkgDirs {
		args = append(args, "./"+dir)
	}
	cmd := exec.Command("go", args...)
	cmd.Dir = checkout
	out, err := cmd.Output()
	if err != nil {
		return TreeClosure(repo, tree)
	}

	files, err := repo.LsTree(string(tree))
	if err != nil {
		return DependsOn{}, err
	}
	selected := map[string]gitx.OID{}
	packageFiles := 0
	include := func(rel string) bool {
		if oid, ok := files[rel]; ok {
			selected[rel] = oid
			return true
		}
		return false
	}
	include("go.mod")
	include("go.sum")

	dec := json.NewDecoder(strings.NewReader(string(out)))
	for {
		var pkg struct {
			Dir          string
			GoFiles      []string
			CgoFiles     []string
			TestGoFiles  []string
			XTestGoFiles []string
			EmbedFiles   []string
			Standard     bool
		}
		if err := dec.Decode(&pkg); err != nil {
			break
		}
		if pkg.Standard {
			continue
		}
		// go list reports native paths while git reports slash-separated
		// ones (and forward slashes even on Windows): filepath.Rel resolves
		// the mixed separators lexically, then everything moves to slashes.
		relNative, err := filepath.Rel(checkout, pkg.Dir)
		if err != nil || strings.HasPrefix(relNative, "..") {
			continue // dependency outside the repo (module cache): pinned via go.sum
		}
		rel := filepath.ToSlash(relNative)
		for _, group := range [][]string{pkg.GoFiles, pkg.CgoFiles, pkg.TestGoFiles, pkg.XTestGoFiles, pkg.EmbedFiles} {
			for _, f := range group {
				if include(path.Join(rel, f)) {
					packageFiles++
				}
			}
		}
	}
	// A closure whose listed package files did not resolve against the tree
	// is too narrow to be a sound reuse key — fall back to the conservative
	// whole-tree closure.
	if packageFiles == 0 {
		return TreeClosure(repo, tree)
	}
	return DependsOn{Closure: "go_packages", ClosureDigest: digestEntries(selected), Packages: pkgDirs, Toolchain: toolchain()}, nil
}

// ClosureFor computes the closure for the configured mode over the packages
// containing the given repo-relative files (go mode) or the whole tree.
func ClosureFor(repo *gitx.Repo, tree gitx.OID, checkout, mode string, files []string) (DependsOn, error) {
	if mode != "go" {
		return TreeClosure(repo, tree)
	}
	dirs := map[string]bool{}
	for _, f := range files {
		dirs[path.Dir(f)] = true
	}
	var pkgDirs []string
	for d := range dirs {
		if d == "." {
			d = ""
		}
		pkgDirs = append(pkgDirs, d)
	}
	// "" (repo root) needs "./"; normalize.
	for i, d := range pkgDirs {
		if d == "" {
			pkgDirs[i] = "."
		}
	}
	return GoClosure(repo, tree, checkout, pkgDirs)
}

// Recompute recomputes the closure digest a record would have on the given
// tree, for staleness checks and reuse decisions.
func Recompute(repo *gitx.Repo, tree gitx.OID, checkout string, record *Record) (string, error) {
	switch record.DependsOn.Closure {
	case "go_packages":
		dep, err := GoClosure(repo, tree, checkout, record.DependsOn.Packages)
		if err != nil {
			return "", err
		}
		if dep.Closure != "go_packages" {
			return "", fmt.Errorf("go closure unavailable on this tree")
		}
		return dep.ClosureDigest, nil
	default:
		dep, err := TreeClosure(repo, tree)
		if err != nil {
			return "", err
		}
		return dep.ClosureDigest, nil
	}
}
