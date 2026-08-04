package telos

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// brokerProtectedPrefixes are paths a code patch may never touch: the spec has
// its own brokered path, telos.toml is human-owned, and the provider adapters
// and instruction files guard the guard itself.
var brokerProtectedPrefixes = []string{"spec/", ".telos/", ".claude/", ".codex/", ".agents/"}
var brokerProtectedFiles = []string{configFile, "CLAUDE.md", "AGENTS.md"}

func protectedPath(rel string) bool {
	for _, p := range brokerProtectedFiles {
		if rel == p {
			return true
		}
	}
	for _, prefix := range brokerProtectedPrefixes {
		if strings.HasPrefix(rel, prefix) {
			return true
		}
	}
	return false
}

func specPut(root, file string, content []byte, del bool) (map[string]any, error) {
	if file == "" {
		return nil, coded("TELOS_INPUT_REQUIRED", "--file is required")
	}
	rel := filepath.ToSlash(filepath.Clean(file))
	if filepath.IsAbs(rel) || rel == ".." || strings.HasPrefix(rel, "../") ||
		!strings.HasPrefix(rel, specDir+"/") || !strings.HasSuffix(rel, ".md") {
		return nil, coded("TELOS_INPUT_INVALID", "spec files live under spec/ and use the .md extension")
	}
	path := filepath.Join(root, filepath.FromSlash(rel))
	if del {
		if err := os.Remove(path); err != nil {
			return nil, coded("TELOS_INPUT_INVALID", "cannot delete "+rel+": "+err.Error())
		}
		return map[string]any{"path": rel, "deleted": true}, nil
	}
	if len(bytes.TrimSpace(content)) == 0 {
		return nil, coded("TELOS_INPUT_REQUIRED", "spec content on stdin is required")
	}
	if err := atomicWrite(path, normalize(content), 0o644); err != nil {
		return nil, err
	}
	return map[string]any{"path": rel}, nil
}

// specReview validates the pending spec, records its digest, and returns the
// exact content the orchestrator must present to the user. Any later spec
// change makes the digest stale by recomputation.
func specReview(root string) (map[string]any, error) {
	st, err := loadState(root)
	if err != nil {
		return nil, err
	}
	_, specFiles, err := inventories(root)
	if err != nil {
		return nil, err
	}
	model, problems := loadSpec(root, specFiles)
	if len(problems) > 0 {
		return nil, codedPaths("TELOS_SPEC_INVALID", "the spec has structural problems; fix them before review", problems)
	}
	digest := rootHashMap(specFiles)
	if digest == st.Spec.Root {
		return nil, coded("TELOS_NOTHING_PENDING", "the spec matches the approved state; nothing to review")
	}
	changed := changedPaths(st.Spec.Files, specFiles)
	files := make([]map[string]any, 0, len(changed))
	for _, rel := range changed {
		if _, exists := specFiles[rel]; !exists {
			files = append(files, map[string]any{"path": rel, "deleted": true})
			continue
		}
		data, err := os.ReadFile(filepath.Join(root, filepath.FromSlash(rel)))
		if err != nil {
			return nil, err
		}
		files = append(files, map[string]any{"path": rel, "content": string(normalize(data))})
	}
	st.Review = digest
	if err := saveState(root, st); err != nil {
		return nil, err
	}
	return map[string]any{
		"digest":     digest,
		"files":      files,
		"objectives": sortedObjectiveIDs(model),
		"rules":      sortedRuleIDs(model),
	}, nil
}

// specApprove is the single human gate. The provider permission prompt raised
// by `telos guard` is the approval record; the command re-checks the digest so
// approval always binds to the exact bytes that were reviewed.
func specApprove(root, digest string) (map[string]any, error) {
	if digest == "" {
		return nil, coded("TELOS_INPUT_REQUIRED", "--review <digest> is required")
	}
	st, err := loadState(root)
	if err != nil {
		return nil, err
	}
	_, specFiles, err := inventories(root)
	if err != nil {
		return nil, err
	}
	if st.Review == "" || digest != st.Review || rootHashMap(specFiles) != st.Review {
		return nil, coded("TELOS_APPROVAL_STALE", "the review digest is missing or stale; run `telos spec review` and present the returned content again")
	}
	if _, problems := loadSpec(root, specFiles); len(problems) > 0 {
		return nil, codedPaths("TELOS_SPEC_INVALID", "the spec has structural problems; fix them before approval", problems)
	}
	st.Spec = snapshotOf(specFiles)
	st.Review = ""
	if err := saveState(root, st); err != nil {
		return nil, err
	}
	return map[string]any{"spec_root": st.Spec.Root}, nil
}

// runApply is the only write path for code. It requires clean declared roots,
// applies the patch as one transaction, and validates the post-image: every
// touched non-infra file must carry a valid `telos:` annotation intersecting
// the cited rules.
func runApply(root string, rules []string, patch []byte) (map[string]any, error) {
	cfg, err := readConfig(root)
	if err != nil {
		return nil, err
	}
	st, err := loadState(root)
	if err != nil {
		return nil, err
	}
	code, specFiles, err := inventories(root)
	if err != nil {
		return nil, err
	}
	if rootHashMap(code) != st.Code.Root {
		return nil, codedPaths("TELOS_CODE_CORRUPTED", "project corrupted: code changed outside the Telos broker; recover via git or deliberately re-baseline with `telos init`", changedPaths(st.Code.Files, code))
	}
	if rootHashMap(specFiles) != st.Spec.Root {
		return nil, codedPaths("TELOS_SPEC_UNAPPROVED", "the spec has unapproved changes; run `telos spec review` and obtain approval before implementing", changedPaths(st.Spec.Files, specFiles))
	}
	if len(rules) == 0 {
		return nil, coded("TELOS_INPUT_REQUIRED", "at least one --rule reference is required")
	}
	model, problems := loadSpec(root, specFiles)
	if len(problems) > 0 {
		return nil, codedPaths("TELOS_SPEC_INVALID", "the approved spec has structural problems", problems)
	}
	rules = uniqueStrings(rules)
	for _, id := range rules {
		if _, ok := model.Rules[id]; !ok {
			return nil, coded("TELOS_TRACEABILITY_GAP", "unknown rule "+id+"; patches trace only to rules in the approved spec")
		}
	}
	if len(bytes.TrimSpace(patch)) == 0 {
		return nil, coded("TELOS_INPUT_REQUIRED", "a Git patch on stdin is required")
	}
	paths, err := patchPaths(patch)
	if err != nil {
		return nil, err
	}
	for _, rel := range paths {
		if protectedPath(rel) {
			return nil, coded("TELOS_INPUT_INVALID", "patch may not touch "+rel+"; spec changes go through `telos spec put` and telos.toml belongs to the human")
		}
	}
	if err := gitApply(root, patch, false, true); err != nil {
		return nil, coded("TELOS_INPUT_INVALID", err.Error())
	}
	if err := gitApply(root, patch, false, false); err != nil {
		return nil, coded("TELOS_INPUT_INVALID", err.Error())
	}
	rollback := func() { _ = gitApply(root, patch, true, false) }
	var bad []string
	for _, rel := range paths {
		path := filepath.Join(root, filepath.FromSlash(rel))
		if _, statErr := os.Lstat(path); os.IsNotExist(statErr) {
			continue
		}
		if matchAny(cfg.Infra, rel) {
			continue
		}
		ids, found, annErr := fileAnnotations(path)
		if annErr != nil {
			rollback()
			return nil, annErr
		}
		valid := found
		intersects := false
		for _, id := range ids {
			if _, ok := model.Rules[id]; !ok {
				valid = false
			}
			for _, cited := range rules {
				if id == cited {
					intersects = true
				}
			}
		}
		if !valid || !intersects {
			bad = append(bad, rel)
		}
	}
	if len(bad) > 0 {
		rollback()
		return nil, codedPaths("TELOS_ANNOTATION_MISMATCH", "every touched non-infra file must carry a `telos:` annotation of existing rules intersecting the cited --rule references; patch reversed", bad)
	}
	codeAfter, _, err := inventories(root)
	if err != nil {
		rollback()
		return nil, err
	}
	st.Code = snapshotOf(codeAfter)
	if err := saveState(root, st); err != nil {
		return nil, err
	}
	return map[string]any{"paths": paths, "rules": rules, "code_root": st.Code.Root}, nil
}

// runTrace maps rules to the files that implement them (annotations) and the
// tests that prove them (test_files references).
func runTrace(root, id string) (any, error) {
	cfg, err := readConfig(root)
	if err != nil {
		return nil, err
	}
	code, specFiles, err := inventories(root)
	if err != nil {
		return nil, err
	}
	model, problems := loadSpec(root, specFiles)
	if len(problems) > 0 {
		return nil, codedPaths("TELOS_SPEC_INVALID", "the spec has structural problems", problems)
	}
	implementedBy := map[string][]string{}
	testedBy := map[string][]string{}
	for _, rel := range sortedKeys(code) {
		path := filepath.Join(root, filepath.FromSlash(rel))
		if ids, found, annErr := fileAnnotations(path); annErr == nil && found {
			for _, rule := range ids {
				implementedBy[rule] = append(implementedBy[rule], rel)
			}
		}
		if matchAny(cfg.TestFiles, rel) {
			data, readErr := os.ReadFile(path)
			if readErr != nil {
				continue
			}
			for _, rule := range uniqueStrings(ruleRef.FindAllString(string(normalize(data)), -1)) {
				testedBy[rule] = append(testedBy[rule], rel)
			}
		}
	}
	entry := func(rule string) map[string]any {
		info := model.Rules[rule]
		return map[string]any{
			"rule":       rule,
			"title":      info.Title,
			"spec_file":  info.File,
			"objectives": info.Traces,
			"files":      implementedBy[rule],
			"tests":      testedBy[rule],
		}
	}
	if id != "" {
		id = strings.ToUpper(strings.TrimSpace(id))
		if _, ok := model.Rules[id]; !ok {
			return nil, coded("TELOS_INPUT_INVALID", fmt.Sprintf("unknown rule %s", id))
		}
		return entry(id), nil
	}
	out := make([]map[string]any, 0, len(model.Rules))
	for _, rule := range sortedRuleIDs(model) {
		out = append(out, entry(rule))
	}
	return map[string]any{"rules": out}, nil
}
