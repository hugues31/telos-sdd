package telos

import (
	"fmt"
	"io"
	"os/exec"
	"path/filepath"
	"runtime"
	"sort"
)

// runVerify recomputes every invariant from the working tree: declared code
// root, spec structure, approved spec root, file annotations, per-rule test
// coverage, and the configured test commands. It never writes.
func runVerify(root string, stdout, stderr io.Writer) (map[string]any, error) {
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
		return nil, codedPaths("TELOS_CODE_CORRUPTED", "project corrupted: code changed outside the Telos broker; recover via git (restore or checkout a green commit) or deliberately re-baseline with `telos init`", changedPaths(st.Code.Files, code))
	}
	model, problems := loadSpec(root, specFiles)
	if len(problems) > 0 {
		return nil, codedPaths("TELOS_SPEC_INVALID", "the spec has structural problems", problems)
	}
	if rootHashMap(specFiles) != st.Spec.Root {
		return nil, codedPaths("TELOS_SPEC_UNAPPROVED", "the spec has unapproved changes; challenge and normalize the diff, then run `telos spec review` and `telos spec approve` — do not restore it", changedPaths(st.Spec.Files, specFiles))
	}
	var missing, orphans []string
	for _, rel := range sortedKeys(code) {
		if matchAny(cfg.Untraced, rel) {
			continue
		}
		ids, found, annErr := fileAnnotations(filepath.Join(root, filepath.FromSlash(rel)))
		if annErr != nil {
			return nil, annErr
		}
		if !found {
			missing = append(missing, rel)
			continue
		}
		for _, id := range ids {
			if _, ok := model.Rules[id]; !ok {
				orphans = append(orphans, rel+": "+id)
			}
		}
	}
	if len(missing) > 0 {
		return nil, codedPaths("TELOS_ANNOTATION_MISSING", "every file must carry a `telos: RULE-NNN` annotation or match an untraced pattern in telos.toml", missing)
	}
	if len(orphans) > 0 {
		return nil, codedPaths("TELOS_ANNOTATION_ORPHAN", "annotations reference rules that do not exist in the spec", orphans)
	}
	if len(model.Rules) > 0 && (len(cfg.TestCommands) == 0 || len(cfg.TestFiles) == 0) {
		return nil, coded("TELOS_CONFIG_INVALID", "test_commands and test_files must be configured in telos.toml once the spec has rules")
	}
	tested, err := testedRules(root, cfg, code)
	if err != nil {
		return nil, err
	}
	untested := untestedRules(model, tested)
	if len(untested) > 0 {
		return nil, codedPaths("TELOS_RULE_NOT_IMPLEMENTED", "these rules have no tagged test yet; the spec is ahead of the implementation", untested)
	}
	if len(st.Red) > 0 {
		var stale, pending []string
		for _, id := range sortedRedIDs(st.Red) {
			intact := true
			for rel, hash := range st.Red[id].Tests {
				if code[rel] != hash {
					intact = false
					break
				}
			}
			if intact {
				pending = append(pending, id)
			} else {
				stale = append(stale, id)
			}
		}
		if len(stale) > 0 {
			return nil, codedPaths("TELOS_RED_STALE", "sealed failing tests no longer match their recorded red evidence; the state or tree was tampered with", stale)
		}
		return nil, codedPaths("TELOS_RED_PENDING", "these rules hold a witnessed failing test and await their green witness; complete the implementation through `telos apply`", pending)
	}
	if err := runTestCommands(root, cfg.TestCommands, stdout, stderr); err != nil {
		return nil, coded("TELOS_TESTS_FAILED", err.Error())
	}
	codeAfter, _, err := inventories(root)
	if err != nil {
		return nil, err
	}
	if rootHashMap(codeAfter) != st.Code.Root {
		return nil, codedPaths("TELOS_CODE_CORRUPTED", "project corrupted: the test commands mutated tracked files", changedPaths(st.Code.Files, codeAfter))
	}
	return map[string]any{
		"objectives": len(model.Objectives),
		"rules":      len(model.Rules),
		"tested":     len(model.Rules),
		"spec_root":  st.Spec.Root,
		"code_root":  st.Code.Root,
	}, nil
}

// runStatus derives the phase from the working tree without running tests and
// without failing on business states.
func runStatus(root string) (map[string]any, []string, error) {
	cfg, err := readConfig(root)
	if err != nil {
		return nil, nil, err
	}
	st, err := loadState(root)
	if err != nil {
		return nil, nil, err
	}
	code, specFiles, err := inventories(root)
	if err != nil {
		return nil, nil, err
	}
	model, problems := loadSpec(root, specFiles)
	tested, err := testedRules(root, cfg, code)
	if err != nil {
		return nil, nil, err
	}
	untested := untestedRules(model, tested)
	specRoot := rootHashMap(specFiles)
	codeRoot := rootHashMap(code)
	phase, next := derivePhase(st, specRoot, codeRoot, len(untested))
	result := map[string]any{
		"phase": phase,
		"spec": map[string]any{
			"approved_root": st.Spec.Root,
			"current_root":  specRoot,
			"pending_files": changedPaths(st.Spec.Files, specFiles),
			"review":        st.Review,
		},
		"code": map[string]any{
			"declared_root": st.Code.Root,
			"current_root":  codeRoot,
			"changed_files": changedPaths(st.Code.Files, code),
		},
		"rules": map[string]any{
			"total":    len(model.Rules),
			"tested":   len(model.Rules) - len(untested) - len(st.Red),
			"untested": untested,
			"red":      sortedRedIDs(st.Red),
		},
		"spec_problems": problems,
	}
	return result, next, nil
}

// derivePhase orders the derived states: an out-of-band code edit outranks a
// pending spec, which outranks unimplemented rules. A rule holding red
// evidence is still implementing — its failing test awaits its green witness.
// Phases are never stored.
func derivePhase(st State, specRoot, codeRoot string, untested int) (string, []string) {
	switch {
	case codeRoot != st.Code.Root:
		return "corrupted", nil
	case specRoot != st.Spec.Root && st.Review == specRoot:
		return "awaiting_approval", []string{"spec approve"}
	case specRoot != st.Spec.Root:
		return "spec_pending", []string{"spec review"}
	case untested > 0 || len(st.Red) > 0:
		return "implementing", []string{"apply", "verify"}
	}
	return "clean", []string{"spec put"}
}

func untestedRules(model specModel, tested map[string]bool) []string {
	var out []string
	for id := range model.Rules {
		if !tested[id] {
			out = append(out, id)
		}
	}
	sort.Strings(out)
	return out
}

func runTestCommands(root string, commands []string, stdout, stderr io.Writer) error {
	for _, command := range commands {
		var cmd *exec.Cmd
		if runtime.GOOS == "windows" {
			cmd = exec.Command("cmd", "/C", command)
		} else {
			cmd = exec.Command("sh", "-c", command)
		}
		cmd.Dir = root
		cmd.Stdout, cmd.Stderr = stdout, stderr
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("test command failed (%s): %w", command, err)
		}
	}
	return nil
}
