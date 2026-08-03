package telos

import (
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
)

func verifyProject(root string, stdout, stderr io.Writer, checkOnly bool) (map[string]any, error) {
	if err := requireRepositoryClean(root); err != nil {
		return nil, err
	}
	results, err := audit(root)
	if err != nil {
		return nil, err
	}
	for _, result := range results {
		if result.Status != "ok" {
			return nil, codedPaths("TELOS_INTEGRITY_ARTIFACT", fmt.Sprintf("%s: %s (%s)", result.Status, result.Path, result.Detail), []string{result.Path})
		}
	}
	flow, flowErr := activeFlow(root)
	if flowErr == nil {
		if err := auditFlowDrafts(root, flow); err != nil {
			return nil, err
		}
	} else if !errors.Is(flowErr, os.ErrNotExist) {
		return nil, flowErr
	}
	changePaths, err := filepath.Glob(filepath.Join(root, ".telos", "changes", "*.json"))
	if err != nil {
		return nil, err
	}
	for _, path := range changePaths {
		var change Change
		if err := readJSON(path, &change); err != nil {
			return nil, err
		}
		if err := auditChangeTransactions(root, change); err != nil {
			return nil, err
		}
	}
	cfg, err := readConfig(root)
	if err != nil {
		return nil, err
	}
	if err := runVerificationCommands(root, cfg.VerificationCommands, stdout, stderr); err != nil {
		return nil, err
	}
	if err := requireRepositoryClean(root); err != nil {
		return nil, err
	}
	lock, err := loadLock(root)
	if err != nil {
		return nil, err
	}
	repository, err := loadRepositoryLock(root)
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return nil, err
	}
	result := map[string]any{
		"sealed_artifacts": len(lock.Artifacts),
		"root_hash":        empty(lock.RootHash, "unsealed"),
		"repository_root":  repository.RootHash,
		"check_only":       checkOnly,
	}
	return result, nil
}

func auditChangeTransactions(root string, change Change) error {
	if change.Status == "active" && change.ContextHash != "" {
		contextPath := filepath.Join(root, ".telos", "context.md")
		actual, err := fileHash(contextPath)
		if err != nil || actual != change.ContextHash {
			return codedPaths("TELOS_INTEGRITY_UNDECLARED_CHANGE", "project corrupted: implementation context changed outside the Telos CLI", []string{".telos/context.md"})
		}
	}
	repository, err := loadRepositoryLock(root)
	if err != nil {
		return err
	}
	current := change.SourceBaseRoot
	evidence, err := mutationEvidence(root, change.ID)
	if err != nil {
		return err
	}
	for _, id := range change.Transactions {
		var mutation Mutation
		path := filepath.Join(root, ".telos", "mutations", strings.ToLower(id)+".json")
		if err := readJSON(path, &mutation); err != nil {
			return codedPaths("TELOS_INTEGRITY_JOURNAL", "mutation journal is missing or invalid", []string{relative(root, path)})
		}
		if mutation.Change != change.ID || mutation.BeforeRoot != current || mutation.ID != id {
			return codedPaths("TELOS_INTEGRITY_JOURNAL", "mutation chain was rewritten", []string{relative(root, path)})
		}
		patch, err := os.ReadFile(filepath.Join(root, filepath.FromSlash(mutation.PatchPath)))
		if err != nil || patchHash(patch) != mutation.PatchHash {
			return codedPaths("TELOS_INTEGRITY_JOURNAL", "recorded patch is missing or tampered", []string{mutation.PatchPath})
		}
		if evidence[id] != mutation.PatchHash {
			return codedPaths("TELOS_INTEGRITY_JOURNAL", "mutation differs from append-only ledger evidence", []string{relative(root, path)})
		}
		current = mutation.AfterRoot
	}
	if current != change.SourceCurrentRoot {
		return coded("TELOS_INTEGRITY_UNDECLARED_CHANGE", "project corrupted: mutation chain does not match repository content")
	}
	if change.Status == "active" && current != repository.RootHash {
		return coded("TELOS_INTEGRITY_UNDECLARED_CHANGE", "project corrupted: active change does not match repository content")
	}
	return nil
}

func mutationEvidence(root, changeID string) (map[string]string, error) {
	paths, err := filepath.Glob(filepath.Join(root, ".telos", "ledger", "events", "*.json"))
	if err != nil {
		return nil, err
	}
	evidence := map[string]string{}
	for _, path := range paths {
		var event Event
		if err := readJSON(path, &event); err != nil {
			return nil, err
		}
		if event.Type != "change.patch-applied" || event.Subject != changeID {
			continue
		}
		mutation, _ := event.Data["mutation"].(string)
		patch, _ := event.Data["patch_hash"].(string)
		if mutation != "" {
			evidence[mutation] = patch
		}
	}
	return evidence, nil
}
