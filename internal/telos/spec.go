package telos

import (
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
)

var (
	objHeading     = regexp.MustCompile(`(?m)^###\s+(OBJ-[0-9]{3,})\b(?:\s*[—-]\s*(.*))?$`)
	ruleHeading    = regexp.MustCompile(`(?m)^###\s+(RULE-[0-9]{3,})\b(?:\s*[—-]\s*(.*))?$`)
	anyHeading     = regexp.MustCompile(`(?m)^###\s+`)
	traceLine      = regexp.MustCompile(`(?im)^Traces:\s*(.+)$`)
	objRef         = regexp.MustCompile(`\bOBJ-[0-9]{3,}\b`)
	ruleRef        = regexp.MustCompile(`\bRULE-[0-9]{3,}\b`)
	annotationLine = regexp.MustCompile(`telos:\s*(RULE-[0-9]{3,}(?:[\s,]+RULE-[0-9]{3,})*)`)
)

// annotationScanLines bounds how deep in a file the `telos:` header line may
// appear.
const annotationScanLines = 10

type specModel struct {
	Objectives map[string]string
	Rules      map[string]*specRule
}

type specRule struct {
	File    string
	Title   string
	Traces  []string
	Gherkin bool
	Section string
}

// loadSpec parses every file of the spec tree and returns the model plus the
// list of structural problems. An empty spec is valid (bootstrap); OBJ ids
// live in spec/PRODUCT.md only, RULE ids in domain files only, and both are
// unique across the repository.
func loadSpec(root string, specFiles map[string]string) (specModel, []string) {
	model := specModel{Objectives: map[string]string{}, Rules: map[string]*specRule{}}
	var problems []string
	for _, rel := range sortedKeys(specFiles) {
		if !strings.HasSuffix(rel, ".md") {
			problems = append(problems, rel+": only Markdown files are allowed under spec/")
			continue
		}
		data, err := os.ReadFile(filepath.Join(root, filepath.FromSlash(rel)))
		if err != nil {
			problems = append(problems, rel+": "+err.Error())
			continue
		}
		body := string(normalize(data))
		if rel == productFile {
			if ruleHeading.MatchString(body) {
				problems = append(problems, rel+": RULE sections belong in spec domain files, not PRODUCT.md")
			}
			for _, m := range objHeading.FindAllStringSubmatch(body, -1) {
				id := m[1]
				if _, dup := model.Objectives[id]; dup {
					problems = append(problems, rel+": duplicate objective "+id)
					continue
				}
				model.Objectives[id] = rel
			}
			continue
		}
		if objHeading.MatchString(body) {
			problems = append(problems, rel+": OBJ sections belong in spec/PRODUCT.md")
		}
		matches := ruleHeading.FindAllStringSubmatchIndex(body, -1)
		for _, m := range matches {
			id := body[m[2]:m[3]]
			title := ""
			if m[4] >= 0 {
				title = strings.TrimSpace(body[m[4]:m[5]])
			}
			end := len(body)
			if next := anyHeading.FindStringIndex(body[m[1]:]); next != nil {
				end = m[1] + next[0]
			}
			section := body[m[0]:end]
			if existing, dup := model.Rules[id]; dup {
				problems = append(problems, rel+": duplicate rule "+id+" (also in "+existing.File+")")
				continue
			}
			rule := &specRule{File: rel, Title: title, Gherkin: strings.Contains(section, "```gherkin"), Section: section}
			if trace := traceLine.FindStringSubmatch(section); trace != nil {
				rule.Traces = objRef.FindAllString(trace[1], -1)
			}
			if len(rule.Traces) == 0 {
				problems = append(problems, rel+": "+id+" is missing a `Traces: OBJ-NNN` line")
			}
			if !rule.Gherkin {
				problems = append(problems, rel+": "+id+" is missing a ```gherkin scenario block")
			}
			model.Rules[id] = rule
		}
	}
	for _, id := range sortedRuleIDs(model) {
		for _, obj := range model.Rules[id].Traces {
			if _, ok := model.Objectives[obj]; !ok {
				problems = append(problems, model.Rules[id].File+": "+id+" traces unknown objective "+obj)
			}
		}
	}
	if len(model.Rules) > 0 {
		if _, ok := specFiles[productFile]; !ok {
			problems = append(problems, productFile+": missing; rules require product objectives to trace to")
		}
	}
	sort.Strings(problems)
	return model, problems
}

// fileAnnotations returns the RULE ids declared on a `telos:` line within the
// first annotationScanLines lines of the file, and whether such a line exists.
func fileAnnotations(path string) ([]string, bool, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, false, err
	}
	lines := strings.Split(string(normalize(data)), "\n")
	if len(lines) > annotationScanLines {
		lines = lines[:annotationScanLines]
	}
	for _, line := range lines {
		if m := annotationLine.FindStringSubmatch(line); m != nil {
			return uniqueStrings(ruleRef.FindAllString(m[1], -1)), true, nil
		}
	}
	return nil, false, nil
}

// testedRules collects every RULE id referenced by a file matching the
// configured test_files patterns. A rule counts as implemented when it appears
// here and the configured test commands pass.
func testedRules(root string, cfg Config, code map[string]string) (map[string]bool, error) {
	out := map[string]bool{}
	for rel := range code {
		if !matchAny(cfg.TestFiles, rel) {
			continue
		}
		data, err := os.ReadFile(filepath.Join(root, filepath.FromSlash(rel)))
		if err != nil {
			return nil, err
		}
		for _, id := range ruleRef.FindAllString(string(normalize(data)), -1) {
			out[id] = true
		}
	}
	return out, nil
}

func sortedKeys(m map[string]string) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

func sortedRuleIDs(model specModel) []string {
	out := make([]string, 0, len(model.Rules))
	for id := range model.Rules {
		out = append(out, id)
	}
	sort.Strings(out)
	return out
}

func sortedObjectiveIDs(model specModel) []string {
	out := make([]string, 0, len(model.Objectives))
	for id := range model.Objectives {
		out = append(out, id)
	}
	sort.Strings(out)
	return out
}

func uniqueStrings(in []string) []string {
	seen := map[string]bool{}
	var out []string
	for _, s := range in {
		if !seen[s] {
			seen[s] = true
			out = append(out, s)
		}
	}
	return out
}
