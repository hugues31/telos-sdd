package telos

import (
	"bytes"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
)

func loadState(root string) (State, error) {
	var st State
	err := readJSON(filepath.Join(root, filepath.FromSlash(stateFile)), &st)
	if errors.Is(err, os.ErrNotExist) {
		return st, coded("TELOS_STATE_MISSING", ".telos/state.json is missing; run `telos init`")
	}
	if err != nil {
		return st, coded("TELOS_STATE_MISSING", ".telos/state.json is unreadable: "+err.Error())
	}
	return st, nil
}

func saveState(root string, st State) error {
	st.Version = 1
	return writeJSON(filepath.Join(root, filepath.FromSlash(stateFile)), st)
}

func snapshotOf(files map[string]string) Snapshot {
	return Snapshot{Root: rootHashMap(files), Files: files}
}

// inventories hashes every Git-tracked or non-ignored regular file and
// partitions it into the code tree and the spec tree. telos.toml (human-owned
// configuration), .telos/** (state), and .git/** are outside both trees.
func inventories(root string) (code, spec map[string]string, err error) {
	paths, err := gitInventory(root)
	if err != nil {
		paths, err = walkInventory(root)
		if err != nil {
			return nil, nil, err
		}
	}
	code, spec = map[string]string{}, map[string]string{}
	for _, rel := range paths {
		rel = filepath.ToSlash(strings.TrimPrefix(filepath.Clean(rel), "."+string(filepath.Separator)))
		if rel == "" || rel == "." || rel == ".git" || rel == ".telos" || rel == configFile ||
			strings.HasPrefix(rel, ".git/") || strings.HasPrefix(rel, ".telos/") {
			continue
		}
		path := filepath.Join(root, filepath.FromSlash(rel))
		info, statErr := os.Lstat(path)
		if errors.Is(statErr, os.ErrNotExist) || (statErr == nil && !info.Mode().IsRegular()) {
			continue
		}
		if statErr != nil {
			return nil, nil, statErr
		}
		h, hashErr := fileHash(path)
		if hashErr != nil {
			return nil, nil, hashErr
		}
		if rel == specDir || strings.HasPrefix(rel, specDir+"/") {
			spec[rel] = h
		} else {
			code[rel] = h
		}
	}
	return code, spec, nil
}

func gitInventory(root string) ([]string, error) {
	cmd := exec.Command("git", "-C", root, "ls-files", "--cached", "--others", "--exclude-standard", "-z")
	out, err := cmd.Output()
	if err != nil {
		return nil, err
	}
	parts := bytes.Split(out, []byte{0})
	paths := make([]string, 0, len(parts))
	for _, part := range parts {
		if len(part) > 0 {
			paths = append(paths, string(part))
		}
	}
	return paths, nil
}

func walkInventory(root string) ([]string, error) {
	var paths []string
	err := filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, err := filepath.Rel(root, path)
		if err != nil {
			return err
		}
		rel = filepath.ToSlash(rel)
		if entry.IsDir() && (rel == ".git" || rel == ".telos") {
			return filepath.SkipDir
		}
		if !entry.IsDir() && entry.Type().IsRegular() {
			paths = append(paths, rel)
		}
		return nil
	})
	return paths, err
}

// changedPaths lists every path added, removed, or whose hash differs between
// two file maps, sorted for stable output.
func changedPaths(before, after map[string]string) []string {
	set := map[string]bool{}
	for p, h := range after {
		if before[p] != h {
			set[p] = true
		}
	}
	for p := range before {
		if _, ok := after[p]; !ok {
			set[p] = true
		}
	}
	out := make([]string, 0, len(set))
	for p := range set {
		out = append(out, p)
	}
	sort.Strings(out)
	return out
}

func gitApply(root string, patch []byte, reverse, check bool) error {
	args := []string{"-C", root, "apply", "--whitespace=nowarn"}
	if reverse {
		args = append(args, "--reverse")
	}
	if check {
		args = append(args, "--check")
	}
	cmd := exec.Command("git", args...)
	cmd.Stdin = bytes.NewReader(patch)
	if out, err := cmd.CombinedOutput(); err != nil {
		return fmt.Errorf("git apply: %w: %s", err, strings.TrimSpace(string(out)))
	}
	return nil
}

func patchPaths(patch []byte) ([]string, error) {
	seen := map[string]bool{}
	var paths []string
	for _, line := range strings.Split(string(normalize(patch)), "\n") {
		prefix := ""
		switch {
		case strings.HasPrefix(line, "+++ "):
			prefix = "+++ "
		case strings.HasPrefix(line, "--- "):
			prefix = "--- "
		default:
			continue
		}
		path := strings.Fields(strings.TrimPrefix(line, prefix))[0]
		if path == "/dev/null" {
			continue
		}
		path = strings.TrimPrefix(strings.TrimPrefix(path, "b/"), "a/")
		path = filepath.ToSlash(filepath.Clean(path))
		if path == ".." || strings.HasPrefix(path, "../") || filepath.IsAbs(path) {
			return nil, coded("TELOS_INPUT_INVALID", "patch contains a path outside the repository")
		}
		if !seen[path] {
			seen[path] = true
			paths = append(paths, path)
		}
	}
	if len(paths) == 0 {
		return nil, coded("TELOS_INPUT_INVALID", "patch contains no file changes")
	}
	sort.Strings(paths)
	return paths, nil
}
