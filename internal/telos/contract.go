package telos

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
)

var coverageCategories = []string{
	"positive", "negative", "boundary", "authorization", "state-transition",
	"retry-idempotency", "concurrency", "failure-recovery", "prohibited-side-effect",
}

var (
	criterionPattern = regexp.MustCompile(`(?m)^###\s+(CRIT-[0-9]{3})\b`)
	rulePattern      = regexp.MustCompile(`(?m)^###\s+(RULE-[0-9]{3})\b`)
	tracePattern     = regexp.MustCompile(`(?im)^Traces:\s*(.+)$`)
	idPattern        = regexp.MustCompile(`\b(?:CRIT|RULE)-[0-9]{3}\b`)
	scenarioPattern  = regexp.MustCompile(`^SCN-[0-9]{3}$`)
)

func criterionIDs(body string) []string {
	return uniqueMatches(criterionPattern, body)
}

func ruleIDs(body string) []string {
	return uniqueMatches(rulePattern, body)
}

func uniqueMatches(pattern *regexp.Regexp, body string) []string {
	seen := map[string]bool{}
	var out []string
	for _, match := range pattern.FindAllStringSubmatch(body, -1) {
		id := strings.ToUpper(match[1])
		if !seen[id] {
			seen[id] = true
			out = append(out, id)
		}
	}
	return out
}

func ruleTraces(body string) map[string][]string {
	matches := rulePattern.FindAllStringSubmatchIndex(body, -1)
	out := map[string][]string{}
	for i, match := range matches {
		rule := strings.ToUpper(body[match[2]:match[3]])
		end := len(body)
		if i+1 < len(matches) {
			end = matches[i+1][0]
		}
		section := body[match[1]:end]
		trace := tracePattern.FindStringSubmatch(section)
		if len(trace) < 2 {
			out[rule] = nil
			continue
		}
		for _, id := range idPattern.FindAllString(trace[1], -1) {
			if strings.HasPrefix(strings.ToUpper(id), "CRIT-") {
				out[rule] = append(out[rule], strings.ToUpper(id))
			}
		}
	}
	return out
}

func putTestPlan(root, specID string, data []byte) (string, error) {
	_, meta, _, err := findArtifact(root, "spec", specID)
	if err != nil {
		return "", err
	}
	if meta.Status == "sealed" {
		return "", coded("TELOS_ARTIFACT_SEALED", "sealed test contracts cannot be changed")
	}
	var plan TestPlan
	if err := json.Unmarshal(data, &plan); err != nil {
		return "", fmt.Errorf("test plan JSON: %w", err)
	}
	plan.Spec = specID
	if strings.TrimSpace(plan.Feature) == "" {
		plan.Feature = slug(specID)
	}
	path := filepath.Join(root, ".telos", "test-plans", strings.ToLower(specID)+".json")
	if err := writeJSON(path, plan); err != nil {
		return "", err
	}
	if meta.Flow != "" {
		flow, err := loadFlow(root, meta.Flow)
		if err != nil {
			return "", err
		}
		h, _ := fileHash(path)
		if err := storeBlob(root, path, h); err != nil {
			return "", err
		}
		flow.DraftHashes[specID+":plan"] = h
		flow.ContractReview = ""
		if err := saveFlow(root, flow); err != nil {
			return "", err
		}
	}
	if err := appendEvent(root, "test-plan.updated", specID, map[string]any{"path": relative(root, path)}, ""); err != nil {
		return "", err
	}
	return relative(root, path), nil
}

func validateContract(root, flowID string) (Flow, error) {
	flow, err := loadFlow(root, flowID)
	if err != nil {
		return flow, err
	}
	if flow.Intent == "" || len(flow.Specs) == 0 {
		return flow, coded("TELOS_CONTRACT_INVALID", "contract requires one intent and at least one spec")
	}
	_, intentMeta, intentBody, err := findArtifact(root, "intent", flow.Intent)
	if err != nil {
		return flow, err
	}
	if intentMeta.Status != "sealed" {
		return flow, coded("TELOS_PHASE_INVALID", "intent must be sealed before contract validation")
	}
	criteria := map[string]bool{}
	criterionMatches := criterionPattern.FindAllStringSubmatch(intentBody, -1)
	for _, id := range criterionIDs(intentBody) {
		criteria[id] = true
	}
	if len(criterionMatches) != len(criteria) {
		return flow, coded("TELOS_CONTRACT_INVALID", "intent contains duplicate CRIT-NNN identifiers")
	}
	if len(criteria) == 0 {
		return flow, coded("TELOS_CONTRACT_INVALID", "intent has no CRIT-NNN criterion")
	}
	tracedCriteria := map[string]bool{}
	allRules := map[string]bool{}
	allScenarios := map[string]bool{}
	for _, specID := range flow.Specs {
		_, meta, body, err := findArtifact(root, "spec", specID)
		if err != nil {
			return flow, err
		}
		if meta.Flow != flow.ID || meta.Intent != flow.Intent || meta.Status != "draft" {
			return flow, coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("spec %s is not a draft in flow %s", specID, flow.ID))
		}
		if err := validateBody("spec", body); err != nil {
			return flow, fmt.Errorf("spec %s: %w", specID, err)
		}
		traces := ruleTraces(body)
		if len(rulePattern.FindAllStringSubmatch(body, -1)) != len(traces) {
			return flow, coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("spec %s contains duplicate RULE-NNN identifiers", specID))
		}
		if len(traces) == 0 {
			return flow, coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("spec %s has no RULE-NNN heading", specID))
		}
		for rule, refs := range traces {
			if allRules[rule] {
				return flow, coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("duplicate rule %s", rule))
			}
			allRules[rule] = true
			if len(refs) == 0 {
				return flow, coded("TELOS_TRACEABILITY_GAP", fmt.Sprintf("rule %s has no Traces: CRIT-NNN declaration", rule))
			}
			seenRefs := map[string]bool{}
			for _, criterion := range refs {
				if seenRefs[criterion] {
					return flow, coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("rule %s contains duplicate trace reference %s", rule, criterion))
				}
				seenRefs[criterion] = true
				if !criteria[criterion] {
					return flow, coded("TELOS_TRACEABILITY_GAP", fmt.Sprintf("rule %s references unknown criterion %s", rule, criterion))
				}
				tracedCriteria[criterion] = true
			}
		}
		plan, err := loadPlan(root, specID)
		if err != nil {
			return flow, err
		}
		if err := validatePlan(plan, traces, allScenarios); err != nil {
			return flow, fmt.Errorf("test plan %s: %w", specID, err)
		}
	}
	for criterion := range criteria {
		if !tracedCriteria[criterion] {
			return flow, coded("TELOS_TRACEABILITY_GAP", fmt.Sprintf("criterion %s is not traced by any rule", criterion))
		}
	}
	return flow, nil
}

func loadPlan(root, specID string) (TestPlan, error) {
	var plan TestPlan
	path := filepath.Join(root, ".telos", "test-plans", strings.ToLower(specID)+".json")
	if err := readJSON(path, &plan); err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return plan, coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("spec %s has no test plan", specID))
		}
		return plan, err
	}
	if plan.Spec != specID {
		return plan, coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("test plan targets %s instead of %s", plan.Spec, specID))
	}
	return plan, nil
}

func validatePlan(plan TestPlan, rules map[string][]string, contractScenarioIDs map[string]bool) error {
	if len(plan.Scenarios) == 0 {
		return coded("TELOS_CONTRACT_INVALID", "test plan has no scenarios")
	}
	scenariosByRuleCategory := map[string]map[string]bool{}
	for _, scenario := range plan.Scenarios {
		if scenario.ID == "" || scenario.Rule == "" || scenario.Name == "" || len(scenario.Given) == 0 || len(scenario.When) == 0 || len(scenario.Then) == 0 {
			return coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("incomplete scenario %q", scenario.ID))
		}
		if contractScenarioIDs[scenario.ID] {
			return coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("duplicate scenario %s", scenario.ID))
		}
		if !scenarioPattern.MatchString(scenario.ID) {
			return coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("scenario id %q must match SCN-NNN", scenario.ID))
		}
		contractScenarioIDs[scenario.ID] = true
		if _, ok := rules[scenario.Rule]; !ok {
			return coded("TELOS_TRACEABILITY_GAP", fmt.Sprintf("scenario %s references unknown rule %s", scenario.ID, scenario.Rule))
		}
		probe := strings.ToLower(strings.Join(append(append(append([]string{scenario.Name}, scenario.Tags...), scenario.Given...), append(scenario.When, scenario.Then...)...), " "))
		for _, forbidden := range []string{"todo", "skip", "pending", "ignore", "always pass"} {
			if strings.Contains(probe, forbidden) {
				return coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("scenario %s contains forbidden marker %q", scenario.ID, forbidden))
			}
		}
		if scenariosByRuleCategory[scenario.Rule] == nil {
			scenariosByRuleCategory[scenario.Rule] = map[string]bool{}
		}
		for _, tag := range scenario.Tags {
			scenariosByRuleCategory[scenario.Rule][tag] = true
		}
	}
	coverage := map[string]map[string]Coverage{}
	allowedCategories := map[string]bool{}
	for _, category := range coverageCategories {
		allowedCategories[category] = true
	}
	for _, entry := range plan.Coverage {
		if _, ok := rules[entry.Rule]; !ok {
			return coded("TELOS_TRACEABILITY_GAP", fmt.Sprintf("coverage references unknown rule %s", entry.Rule))
		}
		if coverage[entry.Rule] == nil {
			coverage[entry.Rule] = map[string]Coverage{}
		}
		if !allowedCategories[entry.Category] {
			return coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("unknown coverage category %q", entry.Category))
		}
		if _, exists := coverage[entry.Rule][entry.Category]; exists {
			return coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("duplicate coverage entry for %s/%s", entry.Rule, entry.Category))
		}
		coverage[entry.Rule][entry.Category] = entry
	}
	for rule := range rules {
		for _, category := range coverageCategories {
			entry, ok := coverage[rule][category]
			if !ok {
				return coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("missing coverage decision for %s/%s", rule, category))
			}
			switch entry.Status {
			case "covered":
				if !scenariosByRuleCategory[rule][category] {
					return coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("%s/%s is covered but no scenario has that tag", rule, category))
				}
			case "not_applicable":
				if strings.TrimSpace(entry.Rationale) == "" {
					return coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("%s/%s needs a not_applicable rationale", rule, category))
				}
			default:
				return coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("invalid coverage status %q", entry.Status))
			}
		}
	}
	return nil
}

func reviewContract(root, flowID string) (Flow, string, string, error) {
	flow, err := validateContract(root, flowID)
	if err != nil {
		return flow, "", "", err
	}
	var inputs []string
	var summary strings.Builder
	for _, specID := range flow.Specs {
		path, _, body, _ := findArtifact(root, "spec", specID)
		h, _ := fileHash(path)
		inputs = append(inputs, relative(root, path)+"\x00"+h)
		planPath := filepath.Join(root, ".telos", "test-plans", strings.ToLower(specID)+".json")
		planHash, _ := fileHash(planPath)
		inputs = append(inputs, relative(root, planPath)+"\x00"+planHash)
		plan, _ := loadPlan(root, specID)
		summary.WriteString("## Specification\n\n")
		summary.WriteString(body)
		summary.WriteByte('\n')
		summary.WriteString("### Executable scenarios\n\n")
		for _, scenario := range plan.Scenarios {
			fmt.Fprintf(&summary, "#### %s / %s — %s\n\n", scenario.Rule, scenario.ID, scenario.Name)
			fmt.Fprintf(&summary, "- Given: %s\n- When: %s\n- Then: %s\n\n", strings.Join(scenario.Given, "; "), strings.Join(scenario.When, "; "), strings.Join(scenario.Then, "; "))
		}
		summary.WriteString("### Coverage decisions\n\n")
		for _, coverage := range plan.Coverage {
			detail := coverage.Status
			if coverage.Rationale != "" {
				detail += " — " + coverage.Rationale
			}
			fmt.Fprintf(&summary, "- %s / %s: %s\n", coverage.Rule, coverage.Category, detail)
		}
		summary.WriteByte('\n')
	}
	sort.Strings(inputs)
	sum := sha256.Sum256([]byte(strings.Join(inputs, "\n")))
	digest := hex.EncodeToString(sum[:])
	flow.ContractReview = digest
	flow.Phase = "contract_review"
	if err := saveFlow(root, flow); err != nil {
		return flow, "", "", err
	}
	return flow, digest, summary.String(), nil
}

type contractWrite struct {
	path         string
	data         []byte
	mode         os.FileMode
	existed      bool
	original     []byte
	originalMode os.FileMode
}

func sealReviewedContract(root, flowID, digest string) (Flow, error) {
	flow, currentDigest, _, err := reviewContract(root, flowID)
	if err != nil {
		return flow, err
	}
	if digest == "" || digest != currentDigest || digest != flow.ContractReview {
		return flow, coded("TELOS_APPROVAL_STALE", "contract seal requires the current review digest")
	}
	if err := requireRepositoryClean(root); err != nil {
		return flow, err
	}
	originalFlow := flow
	oldLock, err := loadLock(root)
	if err != nil {
		return flow, err
	}
	oldRepo, err := loadRepositoryLock(root)
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return flow, err
	}
	newLock := oldLock
	var writes []contractWrite
	for _, specID := range flow.Specs {
		path, meta, body, err := findArtifact(root, "spec", specID)
		if err != nil {
			return flow, err
		}
		meta.Status = "sealed"
		writes = append(writes, contractWrite{path: path, data: renderArtifact(meta, body), mode: 0o444})
		planPath := filepath.Join(root, ".telos", "test-plans", strings.ToLower(specID)+".json")
		planData, err := os.ReadFile(planPath)
		if err != nil {
			return flow, err
		}
		writes = append(writes, contractWrite{path: planPath, data: planData, mode: 0o444})
		plan, _ := loadPlan(root, specID)
		featurePath := filepath.Join(root, "features", slug(plan.Feature)+".feature")
		for _, locked := range oldLock.Artifacts {
			if locked.Path == relative(root, featurePath) && locked.ID != specID+":feature" {
				return flow, coded("TELOS_CONTRACT_INVALID", fmt.Sprintf("feature path %s is already owned by %s", relative(root, featurePath), locked.ID))
			}
		}
		writes = append(writes, contractWrite{path: featurePath, data: []byte(renderFeature(plan)), mode: 0o444})
	}
	for i := range writes {
		if info, err := os.Stat(writes[i].path); err == nil {
			writes[i].existed = true
			writes[i].originalMode = info.Mode().Perm()
			writes[i].original, _ = os.ReadFile(writes[i].path)
		}
		if err := atomicWrite(writes[i].path, writes[i].data, writes[i].mode); err != nil {
			rollbackContractWrites(writes[:i])
			return flow, err
		}
	}
	for _, specID := range flow.Specs {
		specPath, specMeta, _, _ := findArtifact(root, "spec", specID)
		planPath := filepath.Join(root, ".telos", "test-plans", strings.ToLower(specID)+".json")
		plan, _ := loadPlan(root, specID)
		featurePath := filepath.Join(root, "features", slug(plan.Feature)+".feature")
		entries := []LockedFile{
			{ID: specID, Kind: "spec", Path: relative(root, specPath), Parents: specMeta.Parents},
			{ID: specID + ":plan", Kind: "test-plan", Path: relative(root, planPath), Parents: []string{specID}},
			{ID: specID + ":feature", Kind: "feature", Path: relative(root, featurePath), Parents: []string{specID, specID + ":plan"}},
		}
		for i := range entries {
			entryPath := filepath.Join(root, filepath.FromSlash(entries[i].Path))
			entries[i].Hash, _ = fileHash(entryPath)
			if err := storeBlob(root, entryPath, entries[i].Hash); err != nil {
				rollbackContractWrites(writes)
				return flow, err
			}
			newLock.Artifacts = upsertLocked(newLock.Artifacts, entries[i])
		}
	}
	if err := saveLock(root, newLock); err != nil {
		rollbackContractWrites(writes)
		return flow, err
	}
	repo, err := baselineRepository(root, "", "")
	if err != nil {
		rollbackContractWrites(writes)
		_ = saveLock(root, oldLock)
		_ = saveRepositoryLock(root, oldRepo)
		return flow, err
	}
	flow.Phase = "ready_to_implement"
	for _, specID := range flow.Specs {
		path, _, _, _ := findArtifact(root, "spec", specID)
		h, _ := fileHash(path)
		flow.DraftHashes[specID] = h
		planPath := filepath.Join(root, ".telos", "test-plans", strings.ToLower(specID)+".json")
		ph, _ := fileHash(planPath)
		flow.DraftHashes[specID+":plan"] = ph
	}
	if err := saveFlow(root, flow); err != nil {
		rollbackContractWrites(writes)
		_ = saveLock(root, oldLock)
		_ = saveRepositoryLock(root, oldRepo)
		return flow, err
	}
	newLock, _ = loadLock(root)
	if err := appendEvent(root, "contract.sealed", flow.ID, map[string]any{"intent": flow.Intent, "specs": flow.Specs, "review": digest, "repository_root": repo.RootHash}, newLock.RootHash); err != nil {
		rollbackContractWrites(writes)
		_ = saveLock(root, oldLock)
		_ = saveRepositoryLock(root, oldRepo)
		_ = saveFlow(root, originalFlow)
		return flow, err
	}
	return flow, nil
}

func rollbackContractWrites(writes []contractWrite) {
	for i := len(writes) - 1; i >= 0; i-- {
		write := writes[i]
		if write.existed {
			_ = atomicWrite(write.path, write.original, write.originalMode)
		} else {
			_ = os.Remove(write.path)
		}
	}
}

func upsertLocked(files []LockedFile, entry LockedFile) []LockedFile {
	for i := range files {
		if files[i].Path == entry.Path {
			files[i] = entry
			return files
		}
	}
	return append(files, entry)
}

func changeReferences(root string, change Change) (map[string]bool, map[string]bool, error) {
	rules := map[string]bool{}
	scenarios := map[string]bool{}
	for _, specID := range change.Specs {
		_, _, body, err := findArtifact(root, "spec", specID)
		if err != nil {
			return nil, nil, err
		}
		for _, rule := range ruleIDs(body) {
			rules[rule] = true
		}
		plan, err := loadPlan(root, specID)
		if err != nil {
			return nil, nil, err
		}
		for _, scenario := range plan.Scenarios {
			scenarios[scenario.ID] = true
		}
	}
	return rules, scenarios, nil
}

func contractDigest(paths []string, root string) (string, error) {
	sort.Strings(paths)
	h := sha256.New()
	for _, path := range paths {
		digest, err := fileHash(filepath.Join(root, filepath.FromSlash(path)))
		if err != nil {
			return "", err
		}
		io.WriteString(h, path+"\x00"+digest+"\n")
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}
