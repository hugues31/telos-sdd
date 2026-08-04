package telos

import (
	"bytes"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
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
	model, problems := loadSpec(root, specFiles)
	if len(problems) > 0 {
		return nil, codedPaths("TELOS_SPEC_INVALID", "the spec has structural problems; fix them before approval", problems)
	}
	st.Spec = snapshotOf(specFiles)
	st.Review = ""
	for id := range st.Red {
		if _, ok := model.Rules[id]; !ok {
			delete(st.Red, id)
		}
	}
	if err := saveState(root, st); err != nil {
		return nil, err
	}
	return map[string]any{"spec_root": st.Spec.Root}, nil
}

// runApply is the only write path for code and the arbiter of the test-first
// cycle. It requires clean declared roots, applies the patch as one
// transaction, and validates the post-image: every touched file must either
// match an untraced pattern or carry a valid `telos:` annotation intersecting
// the cited rules. Proof is witnessed, not declared: the first patch for an
// unproven rule must be a test-only patch the broker sees fail on a green
// baseline, the files carrying that red evidence are sealed until the rule is
// proven — they may change only through another test-only patch that fails
// again — and while red evidence is pending every apply runs the suite so the
// green that completes the cycle is witnessed by the broker itself.
func runApply(root string, rules []string, patch []byte, expectPass bool, suiteOut io.Writer) (map[string]any, error) {
	if suiteOut == nil {
		suiteOut = io.Discard
	}
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

	testedBefore, err := testedRules(root, cfg, code)
	if err != nil {
		return nil, err
	}
	var needTest []string
	for _, id := range rules {
		if _, red := st.Red[id]; !testedBefore[id] && !red {
			needTest = append(needTest, id)
		}
	}
	sealedBy := map[string]string{}
	for _, id := range sortedRedIDs(st.Red) {
		for rel := range st.Red[id].Tests {
			sealedBy[rel] = id
		}
	}
	var sealedTouched []string
	testOnly := true
	for _, rel := range paths {
		if id, ok := sealedBy[rel]; ok {
			sealedTouched = append(sealedTouched, rel+" ("+id+")")
		}
		if !matchAny(cfg.TestFiles, rel) {
			testOnly = false
		}
	}
	redMode := !expectPass && testOnly && (len(needTest) > 0 || len(sealedTouched) > 0)
	needsSuite := expectPass || redMode || len(st.Red) > 0
	if needsSuite && (len(cfg.TestCommands) == 0 || len(cfg.TestFiles) == 0) {
		return nil, coded("TELOS_CONFIG_INVALID", "test_commands and test_files must be configured in telos.toml before rules can be proven")
	}
	switch {
	case expectPass && len(needTest) != len(rules):
		return nil, coded("TELOS_INPUT_INVALID", "--expect-pass adopts existing behavior: every cited rule must be one no test references yet")
	case expectPass && !testOnly:
		return nil, coded("TELOS_INPUT_INVALID", "--expect-pass submits documentation tests only; the patch may touch only test_files matches")
	case !redMode && !expectPass && len(sealedTouched) > 0:
		return nil, codedPaths("TELOS_TEST_SEALED", "these tests are sealed red evidence: until their rules are proven they may change only through a test-only patch the suite fails again — fix the implementation instead, or rewrite the test back through red", sealedTouched)
	case !redMode && !expectPass && len(needTest) > 0:
		return nil, codedPaths("TELOS_TEST_FIRST", "unproven rules are implemented test-first: submit a test-only patch referencing them and witness it fail before any implementation", needTest)
	}
	// A new failing test is attributable only when the suite was green without
	// it; witness the baseline once and cache the root it held at.
	if (redMode || expectPass) && len(needTest) > 0 && st.Green != st.Code.Root {
		if err := runTestCommands(root, cfg.TestCommands, suiteOut, suiteOut); err != nil {
			return nil, coded("TELOS_BASELINE_RED", "a new test is only evidence on a green baseline; make the suite pass before submitting the test for "+strings.Join(needTest, ", ")+" ("+err.Error()+")")
		}
		after, _, invErr := inventories(root)
		if invErr != nil {
			return nil, invErr
		}
		if rootHashMap(after) != st.Code.Root {
			return nil, codedPaths("TELOS_CODE_CORRUPTED", "project corrupted: the test commands mutated tracked files", changedPaths(st.Code.Files, after))
		}
		st.Green = st.Code.Root
		if err := saveState(root, st); err != nil {
			return nil, err
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
		if matchAny(cfg.Untraced, rel) {
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
		return nil, codedPaths("TELOS_ANNOTATION_MISMATCH", "every touched file must match an untraced pattern or carry a `telos:` annotation of existing rules intersecting the cited --rule references; patch reversed", bad)
	}
	codeAfter, _, err := inventories(root)
	if err != nil {
		rollback()
		return nil, err
	}
	refs, err := testFileRefs(root, cfg, codeAfter)
	if err != nil {
		rollback()
		return nil, err
	}
	// Test references are free text, so their entry points are policed on the
	// post-image: a rule may only become referenced through its own witnessed
	// cycle, never as a side effect of another patch.
	allowedNew := map[string]bool{}
	for _, id := range needTest {
		allowedNew[id] = true
	}
	var uncited []string
	seenUncited := map[string]bool{}
	for _, ids := range refs {
		for _, id := range ids {
			if !testedBefore[id] && !allowedNew[id] && !seenUncited[id] {
				seenUncited[id] = true
				uncited = append(uncited, id)
			}
		}
	}
	sort.Strings(uncited)
	if len(uncited) > 0 {
		rollback()
		return nil, codedPaths("TELOS_TEST_FIRST", "the patch introduces test references outside the witnessed cycle; each unproven rule enters through its own failing test-only patch", uncited)
	}
	var missingRefs []string
	for _, id := range needTest {
		if len(citingTests(codeAfter, refs, id)) == 0 {
			missingRefs = append(missingRefs, id)
		}
	}
	if len(missingRefs) > 0 {
		rollback()
		return nil, codedPaths("TELOS_TEST_FIRST", "the patch must add a test referencing every cited unproven rule", missingRefs)
	}

	result := map[string]any{"paths": paths, "rules": rules}
	if needsSuite {
		var suiteLog bytes.Buffer
		sink := io.MultiWriter(&suiteLog, suiteOut)
		suiteErr := runTestCommands(root, cfg.TestCommands, sink, sink)
		afterSuite, _, invErr := inventories(root)
		if invErr != nil {
			rollback()
			return nil, invErr
		}
		if rootHashMap(afterSuite) != rootHashMap(codeAfter) {
			return nil, codedPaths("TELOS_CODE_CORRUPTED", "project corrupted: the test commands mutated tracked files", changedPaths(codeAfter, afterSuite))
		}
		switch {
		case redMode && suiteErr == nil:
			rollback()
			return nil, coded("TELOS_RED_EXPECTED", "the suite passes with this test in place, so it proves nothing: strengthen the test until it fails, or — if the rule documents behavior the code already has — resubmit with --expect-pass")
		case redMode:
			if st.Red == nil {
				st.Red = map[string]RedEvidence{}
			}
			for _, id := range needTest {
				st.Red[id] = RedEvidence{Tests: citingTests(codeAfter, refs, id)}
			}
			for _, id := range sortedRedIDs(st.Red) {
				touched := false
				for _, rel := range paths {
					if _, ok := st.Red[id].Tests[rel]; ok {
						touched = true
					}
				}
				if !touched {
					continue
				}
				if files := citingTests(codeAfter, refs, id); len(files) == 0 {
					delete(st.Red, id)
				} else {
					st.Red[id] = RedEvidence{Tests: files}
				}
			}
			result["suite"] = "red"
			result["red"] = sortedRedIDs(st.Red)
			result["test_output"] = outputTail(suiteLog.Bytes())
		case expectPass && suiteErr != nil:
			rollback()
			return nil, coded("TELOS_TESTS_FAILED", "--expect-pass claims the cited rules are already satisfied, but the suite fails with the documentation test in place: "+suiteErr.Error())
		case expectPass:
			st.Green = rootHashMap(codeAfter)
			result["suite"] = "green"
			result["proven"] = needTest
		case suiteErr == nil:
			var stale []string
			for _, id := range sortedRedIDs(st.Red) {
				for rel, hash := range st.Red[id].Tests {
					if codeAfter[rel] != hash {
						stale = append(stale, id)
						break
					}
				}
			}
			if len(stale) > 0 {
				rollback()
				return nil, codedPaths("TELOS_RED_STALE", "sealed tests no longer match their recorded red evidence; the state was tampered with", stale)
			}
			result["proven"] = sortedRedIDs(st.Red)
			st.Red = nil
			st.Green = rootHashMap(codeAfter)
			result["suite"] = "green"
		default:
			result["suite"] = "red"
			result["red"] = sortedRedIDs(st.Red)
			result["test_output"] = outputTail(suiteLog.Bytes())
		}
	}
	st.Code = snapshotOf(codeAfter)
	if err := saveState(root, st); err != nil {
		return nil, err
	}
	result["code_root"] = st.Code.Root
	return result, nil
}

// citingTests returns the test files whose content references the rule, with
// their inventory hashes — the exact bytes a red witness seals.
func citingTests(code map[string]string, refs map[string][]string, id string) map[string]string {
	out := map[string]string{}
	for rel, ids := range refs {
		for _, ref := range ids {
			if ref == id {
				out[rel] = code[rel]
				break
			}
		}
	}
	return out
}

func sortedRedIDs(red map[string]RedEvidence) []string {
	out := make([]string, 0, len(red))
	for id := range red {
		out = append(out, id)
	}
	sort.Strings(out)
	return out
}

// outputTail bounds captured suite output for the JSON envelope so the agent
// sees why the run was red without flooding the transcript.
func outputTail(b []byte) string {
	const limit = 2000
	s := strings.TrimSpace(string(b))
	if len(s) > limit {
		s = "…" + s[len(s)-limit:]
	}
	return s
}

// traceMaps derives, from the tree alone, which files implement each rule
// (annotations) and which tests prove it (test_files references).
func traceMaps(root string, cfg Config, code map[string]string) (implementedBy, testedBy map[string][]string) {
	implementedBy, testedBy = map[string][]string{}, map[string][]string{}
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
	return implementedBy, testedBy
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
	implementedBy, testedBy := traceMaps(root, cfg, code)
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
