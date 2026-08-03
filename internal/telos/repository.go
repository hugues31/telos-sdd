package telos

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

const repositoryLockPath = ".telos/repository-lock.json"

func loadRepositoryLock(root string) (RepositoryLock, error) {
	var lock RepositoryLock
	err := readJSON(filepath.Join(root, filepath.FromSlash(repositoryLockPath)), &lock)
	return lock, err
}

func saveRepositoryLock(root string, lock RepositoryLock) error {
	sort.Slice(lock.Files, func(i, j int) bool { return lock.Files[i].Path < lock.Files[j].Path })
	lock.RootHash = repositoryRootHash(lock.Files)
	return writeJSON(filepath.Join(root, filepath.FromSlash(repositoryLockPath)), lock)
}

func repositoryRootHash(files []RepositoryFile) string {
	ordered := append([]RepositoryFile(nil), files...)
	sort.Slice(ordered, func(i, j int) bool { return ordered[i].Path < ordered[j].Path })
	h := sha256.New()
	for _, file := range ordered {
		io.WriteString(h, filepath.ToSlash(file.Path))
		h.Write([]byte{0})
		io.WriteString(h, file.Hash)
		h.Write([]byte{'\n'})
	}
	return hex.EncodeToString(h.Sum(nil))
}

func repositoryInventory(root string) ([]RepositoryFile, error) {
	paths, err := gitInventory(root)
	if err != nil {
		paths, err = walkInventory(root)
		if err != nil {
			return nil, err
		}
	}
	files := make([]RepositoryFile, 0, len(paths))
	seen := map[string]bool{}
	for _, rel := range paths {
		rel = filepath.ToSlash(strings.TrimPrefix(filepath.Clean(rel), "."+string(filepath.Separator)))
		if rel == "" || rel == "." || strings.HasPrefix(rel, ".git/") || strings.HasPrefix(rel, ".telos/") || rel == ".git" || rel == ".telos" || seen[rel] {
			continue
		}
		path := filepath.Join(root, filepath.FromSlash(rel))
		info, err := os.Lstat(path)
		if errors.Is(err, os.ErrNotExist) || (err == nil && !info.Mode().IsRegular()) {
			continue
		}
		if err != nil {
			return nil, err
		}
		h, err := fileHash(path)
		if err != nil {
			return nil, err
		}
		seen[rel] = true
		files = append(files, RepositoryFile{Path: rel, Hash: h, Mode: uint32(info.Mode().Perm())})
	}
	sort.Slice(files, func(i, j int) bool { return files[i].Path < files[j].Path })
	return files, nil
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

func baselineRepository(root, typ, subject string) (RepositoryLock, error) {
	files, err := repositoryInventory(root)
	if err != nil {
		return RepositoryLock{}, err
	}
	lock := RepositoryLock{Files: files, RootHash: repositoryRootHash(files)}
	if err := storeRepositoryBlobs(root, files); err != nil {
		return RepositoryLock{}, err
	}
	if err := saveRepositoryLock(root, lock); err != nil {
		return RepositoryLock{}, err
	}
	if typ != "" {
		if err := appendEvent(root, typ, subject, map[string]any{"repository_root": lock.RootHash}, ""); err != nil {
			return RepositoryLock{}, err
		}
	}
	return lock, nil
}

func storeRepositoryBlobs(root string, files []RepositoryFile) error {
	for _, file := range files {
		if err := storeBlob(root, filepath.Join(root, filepath.FromSlash(file.Path)), file.Hash); err != nil {
			return err
		}
	}
	return nil
}

func storeBlob(root, path, hash string) error {
	blob := filepath.Join(root, ".telos", "blobs", hash)
	if _, err := os.Stat(blob); err == nil {
		return nil
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	return atomicWrite(blob, data, 0o444)
}

func auditRepository(root string) ([]string, string, string, error) {
	lock, err := loadRepositoryLock(root)
	if errors.Is(err, os.ErrNotExist) {
		return nil, "", "", nil
	}
	if err != nil {
		return nil, "", "", err
	}
	evidenceRoot, err := latestRepositoryEvidence(root)
	if err != nil {
		return nil, "", "", err
	}
	if evidenceRoot != "" && evidenceRoot != lock.RootHash {
		return []string{repositoryLockPath}, evidenceRoot, lock.RootHash, nil
	}
	actual, err := repositoryInventory(root)
	if err != nil {
		return nil, "", "", err
	}
	expectedByPath := map[string]RepositoryFile{}
	actualByPath := map[string]RepositoryFile{}
	for _, file := range lock.Files {
		expectedByPath[file.Path] = file
	}
	for _, file := range actual {
		actualByPath[file.Path] = file
	}
	var changed []string
	for path, expected := range expectedByPath {
		actualFile, ok := actualByPath[path]
		if !ok || actualFile.Hash != expected.Hash {
			changed = append(changed, path)
		}
	}
	for path := range actualByPath {
		if _, ok := expectedByPath[path]; !ok {
			changed = append(changed, path)
		}
	}
	sort.Strings(changed)
	return changed, lock.RootHash, repositoryRootHash(actual), nil
}

func latestRepositoryEvidence(root string) (string, error) {
	paths, err := filepath.Glob(filepath.Join(root, ".telos", "ledger", "events", "*.json"))
	if err != nil {
		return "", err
	}
	var events []Event
	for _, path := range paths {
		var event Event
		if err := readJSON(path, &event); err != nil {
			return "", err
		}
		events = append(events, event)
	}
	sort.Slice(events, func(i, j int) bool {
		if events[i].At.Equal(events[j].At) {
			return events[i].ID < events[j].ID
		}
		return events[i].At.Before(events[j].At)
	})
	latest := ""
	for _, event := range events {
		if rootHash, ok := event.Data["repository_root"].(string); ok && rootHash != "" {
			latest = rootHash
		}
	}
	return latest, nil
}

func requireRepositoryClean(root string) error {
	changed, expected, actual, err := auditRepository(root)
	if err != nil {
		return err
	}
	if len(changed) > 0 || (expected != "" && expected != actual) {
		return codedPaths("TELOS_INTEGRITY_UNDECLARED_CHANGE", "project corrupted: files changed outside a declared Telos transaction", changed)
	}
	return nil
}

func repairRepository(root string) ([]string, error) {
	lock, err := loadRepositoryLock(root)
	if err != nil {
		return nil, err
	}
	actual, err := repositoryInventory(root)
	if err != nil {
		return nil, err
	}
	expected := map[string]RepositoryFile{}
	actualPaths := map[string]bool{}
	for _, file := range lock.Files {
		expected[file.Path] = file
	}
	for _, file := range actual {
		actualPaths[file.Path] = true
	}
	var repaired []string
	for path, file := range expected {
		abs := filepath.Join(root, filepath.FromSlash(path))
		current, err := fileHash(abs)
		if err == nil && current == file.Hash {
			continue
		}
		data, err := os.ReadFile(filepath.Join(root, ".telos", "blobs", file.Hash))
		if err != nil {
			return repaired, fmt.Errorf("restore %s: expected blob is missing: %w", path, err)
		}
		if err := atomicWrite(abs, data, fs.FileMode(file.Mode)); err != nil {
			return repaired, err
		}
		repaired = append(repaired, path)
	}
	for path := range actualPaths {
		if _, ok := expected[path]; ok {
			continue
		}
		if err := os.Remove(filepath.Join(root, filepath.FromSlash(path))); err != nil && !errors.Is(err, os.ErrNotExist) {
			return repaired, err
		}
		repaired = append(repaired, path)
	}
	sort.Strings(repaired)
	if err := requireRepositoryClean(root); err != nil {
		return repaired, err
	}
	if err := appendEvent(root, "repository.repaired", "project", map[string]any{"paths": repaired, "repository_root": lock.RootHash}, ""); err != nil {
		return repaired, err
	}
	return repaired, nil
}

func repairManagedArtifacts(root string) ([]string, error) {
	type expectedFile struct {
		Hash string
		Mode fs.FileMode
	}
	expected := map[string]expectedFile{}
	lock, err := loadLock(root)
	if err != nil {
		return nil, err
	}
	for _, artifact := range lock.Artifacts {
		expected[artifact.Path] = expectedFile{Hash: artifact.Hash, Mode: 0o444}
	}
	flowPaths, err := filepath.Glob(filepath.Join(root, ".telos", "flows", "*.json"))
	if err != nil {
		return nil, err
	}
	for _, flowPath := range flowPaths {
		var flow Flow
		if err := readJSON(flowPath, &flow); err != nil {
			return nil, err
		}
		for id, hash := range flow.DraftHashes {
			path := ""
			if strings.HasSuffix(id, ":plan") {
				path = filepath.ToSlash(filepath.Join(".telos", "test-plans", strings.ToLower(strings.TrimSuffix(id, ":plan"))+".json"))
			} else if kind, err := artifactKind(id); err == nil {
				path = filepath.ToSlash(filepath.Join(".telos", kind+"s", strings.ToLower(id)+".md"))
			}
			if path == "" {
				continue
			}
			mode := fs.FileMode(0o644)
			if locked, ok := expected[path]; ok {
				mode = locked.Mode
			}
			expected[path] = expectedFile{Hash: hash, Mode: mode}
		}
		if flow.Change != "" {
			var change Change
			if err := readJSON(filepath.Join(root, ".telos", "changes", strings.ToLower(flow.Change)+".json"), &change); err == nil && change.Status == "active" && change.ContextHash != "" {
				expected[".telos/context.md"] = expectedFile{Hash: change.ContextHash, Mode: 0o644}
			}
		}
	}
	var repaired []string
	for rel, file := range expected {
		path := filepath.Join(root, filepath.FromSlash(rel))
		if actual, err := fileHash(path); err == nil && actual == file.Hash {
			continue
		}
		data, err := os.ReadFile(filepath.Join(root, ".telos", "blobs", file.Hash))
		if err != nil {
			return repaired, fmt.Errorf("restore %s: expected blob is missing: %w", rel, err)
		}
		if err := atomicWrite(path, data, file.Mode); err != nil {
			return repaired, err
		}
		repaired = append(repaired, rel)
	}
	sort.Strings(repaired)
	if len(repaired) > 0 {
		if err := appendEvent(root, "artifacts.repaired", "project", map[string]any{"paths": repaired}, ""); err != nil {
			return repaired, err
		}
	}
	return repaired, nil
}

func patchHash(patch []byte) string {
	sum := sha256.Sum256(normalize(patch))
	return hex.EncodeToString(sum[:])
}

func applyChangePatch(root string, change Change, patch []byte, rules, scenarios []string) (Change, Mutation, error) {
	originalChange := change
	if change.Status != "active" {
		return change, Mutation{}, coded("TELOS_PHASE_INVALID", "change is not active")
	}
	if len(bytes.TrimSpace(patch)) == 0 {
		return change, Mutation{}, coded("TELOS_INPUT_REQUIRED", "a Git patch is required")
	}
	if len(rules) == 0 || len(scenarios) == 0 {
		return change, Mutation{}, coded("TELOS_TRACEABILITY_GAP", "every patch requires at least one RULE and one SCN reference")
	}
	if err := requireRepositoryClean(root); err != nil {
		return change, Mutation{}, err
	}
	validRules, validScenarios, err := changeReferences(root, change)
	if err != nil {
		return change, Mutation{}, err
	}
	for _, rule := range rules {
		if !validRules[rule] {
			return change, Mutation{}, coded("TELOS_TRACEABILITY_GAP", fmt.Sprintf("unknown rule %s", rule))
		}
	}
	for _, scenario := range scenarios {
		if !validScenarios[scenario] {
			return change, Mutation{}, coded("TELOS_TRACEABILITY_GAP", fmt.Sprintf("unknown scenario %s", scenario))
		}
	}
	paths, err := patchPaths(patch)
	if err != nil {
		return change, Mutation{}, err
	}
	for _, path := range paths {
		if path == ".telos" || strings.HasPrefix(path, ".telos/") || path == "features" || strings.HasPrefix(path, "features/") {
			return change, Mutation{}, codedPaths("TELOS_DIRECT_WRITE_DENIED", "patch targets a Telos-managed artifact", []string{path})
		}
	}
	before, err := loadRepositoryLock(root)
	if err != nil {
		return change, Mutation{}, err
	}
	if change.SourceCurrentRoot != "" && before.RootHash != change.SourceCurrentRoot {
		return change, Mutation{}, coded("TELOS_INTEGRITY_UNDECLARED_CHANGE", "project corrupted: change root differs from repository root")
	}
	if err := gitApply(root, patch, false, true); err != nil {
		return change, Mutation{}, err
	}
	if err := gitApply(root, patch, false, false); err != nil {
		return change, Mutation{}, err
	}
	after, err := baselineRepository(root, "", "")
	if err != nil {
		_ = gitApply(root, patch, true, false)
		_ = saveRepositoryLock(root, before)
		return change, Mutation{}, err
	}
	id, err := newID("mut", time.Now())
	if err != nil {
		_ = gitApply(root, patch, true, false)
		_ = saveRepositoryLock(root, before)
		return change, Mutation{}, err
	}
	patchPath := filepath.Join(root, ".telos", "patches", strings.ToLower(id)+".patch")
	mutation := Mutation{ID: id, Change: change.ID, PatchHash: patchHash(patch), PatchPath: relative(root, patchPath), BeforeRoot: before.RootHash, AfterRoot: after.RootHash, Paths: paths, Rules: rules, Scenarios: scenarios, At: time.Now().UTC().Format(time.RFC3339)}
	mutationPath := filepath.Join(root, ".telos", "mutations", strings.ToLower(id)+".json")
	if err := atomicWrite(patchPath, normalize(patch), 0o444); err != nil {
		_ = gitApply(root, patch, true, false)
		_ = saveRepositoryLock(root, before)
		return change, Mutation{}, err
	}
	if err := writeJSON(mutationPath, mutation); err != nil {
		_ = gitApply(root, patch, true, false)
		_ = saveRepositoryLock(root, before)
		_ = os.Remove(patchPath)
		return change, Mutation{}, err
	}
	change.SourceCurrentRoot = after.RootHash
	change.Transactions = append(change.Transactions, id)
	if err := writeJSON(filepath.Join(root, ".telos", "changes", strings.ToLower(change.ID)+".json"), change); err != nil {
		_ = gitApply(root, patch, true, false)
		_ = saveRepositoryLock(root, before)
		_ = os.Remove(mutationPath)
		_ = os.Remove(patchPath)
		return change, Mutation{}, err
	}
	if err := appendEvent(root, "change.patch-applied", change.ID, map[string]any{"mutation": id, "patch_hash": mutation.PatchHash, "paths": paths, "rules": rules, "scenarios": scenarios, "repository_root": after.RootHash}, ""); err != nil {
		_ = gitApply(root, patch, true, false)
		_ = saveRepositoryLock(root, before)
		_ = writeJSON(filepath.Join(root, ".telos", "changes", strings.ToLower(change.ID)+".json"), originalChange)
		_ = os.Remove(mutationPath)
		_ = os.Remove(patchPath)
		return originalChange, Mutation{}, err
	}
	return change, mutation, nil
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

func abortChange(root string, change Change, reason string) (Change, error) {
	if change.Status != "active" {
		return change, coded("TELOS_PHASE_INVALID", "only an active change can be aborted")
	}
	if strings.TrimSpace(reason) == "" {
		return change, coded("TELOS_INPUT_REQUIRED", "an abort reason is required")
	}
	if err := requireRepositoryClean(root); err != nil {
		return change, err
	}
	for i := len(change.Transactions) - 1; i >= 0; i-- {
		id := change.Transactions[i]
		var mutation Mutation
		if err := readJSON(filepath.Join(root, ".telos", "mutations", strings.ToLower(id)+".json"), &mutation); err != nil {
			return change, coded("TELOS_INTEGRITY_JOURNAL", "cannot abort because a mutation record is missing")
		}
		patch, err := os.ReadFile(filepath.Join(root, filepath.FromSlash(mutation.PatchPath)))
		if err != nil || patchHash(patch) != mutation.PatchHash {
			return change, coded("TELOS_INTEGRITY_JOURNAL", "cannot abort because a recorded patch is missing or tampered")
		}
		if err := gitApply(root, patch, true, true); err != nil {
			return change, err
		}
		if err := gitApply(root, patch, true, false); err != nil {
			return change, err
		}
	}
	repository, err := baselineRepository(root, "", "")
	if err != nil {
		return change, err
	}
	if repository.RootHash != change.SourceBaseRoot {
		return change, coded("TELOS_INTEGRITY_UNDECLARED_CHANGE", "project corrupted: abort did not reconstruct the change baseline")
	}
	change.Status = "aborted"
	change.Completed = time.Now().UTC().Format(time.RFC3339)
	if err := writeJSON(filepath.Join(root, ".telos", "changes", strings.ToLower(change.ID)+".json"), change); err != nil {
		return change, err
	}
	if err := appendEvent(root, "change.aborted", change.ID, map[string]any{"reason": strings.TrimSpace(reason), "repository_root": repository.RootHash}, ""); err != nil {
		return change, err
	}
	return change, nil
}

func patchPaths(patch []byte) ([]string, error) {
	seen := map[string]bool{}
	var paths []string
	for _, line := range strings.Split(string(normalize(patch)), "\n") {
		if !strings.HasPrefix(line, "+++ ") {
			continue
		}
		path := strings.Fields(strings.TrimPrefix(line, "+++ "))[0]
		if path == "/dev/null" {
			continue
		}
		path = strings.TrimPrefix(path, "b/")
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
