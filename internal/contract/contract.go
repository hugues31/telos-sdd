// Package contract parses and validates the canonical contract under spec/:
// product intents (INT-*) in spec/PRODUCT.md, requirements (REQ-*) in domain
// files, decisions (DEC-*) in spec/DECISIONS.md. It operates on file contents
// (not paths) so the same parser serves worktrees and git trees. spec/ always
// describes the current certified state; future semantics live in a Change's
// contract delta until promotion.
package contract

import (
	"regexp"
	"sort"
	"strings"
)

const (
	// Dir is the contract directory at the repository root.
	Dir = "spec"
	// ProductFile holds the product intents.
	ProductFile = "spec/PRODUCT.md"
	// DecisionsFile holds the decisions.
	DecisionsFile = "spec/DECISIONS.md"
)

// Class categorizes a requirement and drives its evidence policy.
type Class string

const (
	ClassBehavior     Class = "behavior"
	ClassSecurity     Class = "security"
	ClassInvariant    Class = "invariant"
	ClassConcurrency  Class = "concurrency"
	ClassPerformance  Class = "performance"
	ClassArchitecture Class = "architecture"
)

// Classes lists every valid requirement class.
var Classes = []Class{ClassBehavior, ClassSecurity, ClassInvariant, ClassConcurrency, ClassPerformance, ClassArchitecture}

// gherkinRequired reports whether a class must carry an executable scenario
// block; the two structural classes only warn without one.
func gherkinRequired(c Class) bool {
	return c == ClassBehavior || c == ClassSecurity || c == ClassInvariant || c == ClassConcurrency
}

var (
	intHeading  = regexp.MustCompile(`(?m)^###\s+(INT-[0-9]{3,})\b(?:\s*[—-]\s*(.*))?$`)
	reqHeading  = regexp.MustCompile(`(?m)^###\s+(REQ-[0-9]{3,})\b(?:\s*[—-]\s*(.*))?$`)
	decHeading  = regexp.MustCompile(`(?m)^###\s+(DEC-[0-9]{3,})\b(?:\s*[—-]\s*(.*))?$`)
	anyHeading  = regexp.MustCompile(`(?m)^###\s+`)
	classLine   = regexp.MustCompile(`(?im)^Class:\s*(\S+)\s*$`)
	motivatedBy = regexp.MustCompile(`(?im)^Motivated by:\s*(.+)$`)
	statusLine  = regexp.MustCompile(`(?im)^Status:\s*(.+)$`)
	intRef      = regexp.MustCompile(`\bINT-[0-9]{3,}\b`)
	reqRef      = regexp.MustCompile(`\bREQ-[0-9]{3,}\b`)
	decRef      = regexp.MustCompile(`\bDEC-[0-9]{3,}\b`)
	constraint  = regexp.MustCompile("(?s)```telos-constraint\n(.*?)```")
)

// Intent is one INT-* section.
type Intent struct {
	ID, Title, Section, File string
}

// Requirement is one REQ-* section.
type Requirement struct {
	ID, Title   string
	Class       Class
	MotivatedBy []string
	Gherkin     bool
	Constraint  string // raw ```telos-constraint body, "" when absent
	Section     string
	File        string
}

// Decision is one DEC-* section.
type Decision struct {
	ID, Title    string
	Status       string // "accepted" or "superseded"
	SupersededBy string
	Section      string
	File         string
}

// Contract is the parsed canonical contract. Warnings are non-blocking
// (currently: missing scenario block on the two structural classes).
type Contract struct {
	Intents      map[string]Intent
	Requirements map[string]*Requirement
	Decisions    map[string]Decision
	Warnings     []string
}

// ReqRefs returns the unique REQ ids referenced anywhere in content, in
// appearance order. Test files use this free-text mechanism to cite the
// requirements they verify.
func ReqRefs(content []byte) []string {
	return unique(reqRef.FindAllString(string(normalize(content)), -1))
}

// Parse validates the contract files (path → content, paths slash-separated
// and spec/-prefixed) and returns the model plus sorted structural problems.
// An empty contract is valid (bootstrap).
func Parse(files map[string][]byte) (Contract, []string) {
	c := Contract{
		Intents:      map[string]Intent{},
		Requirements: map[string]*Requirement{},
		Decisions:    map[string]Decision{},
	}
	var problems []string
	for _, rel := range sortedKeys(files) {
		if !strings.HasSuffix(rel, ".md") {
			problems = append(problems, rel+": only Markdown files are allowed under spec/")
			continue
		}
		body := string(normalize(files[rel]))
		switch rel {
		case ProductFile:
			if reqHeading.MatchString(body) {
				problems = append(problems, rel+": REQ sections belong in spec domain files, not PRODUCT.md")
			}
			if decHeading.MatchString(body) {
				problems = append(problems, rel+": DEC sections belong in spec/DECISIONS.md")
			}
			for _, s := range sections(body, intHeading) {
				if _, dup := c.Intents[s.id]; dup {
					problems = append(problems, rel+": duplicate intent "+s.id)
					continue
				}
				c.Intents[s.id] = Intent{ID: s.id, Title: s.title, Section: s.section, File: rel}
			}
		case DecisionsFile:
			if reqHeading.MatchString(body) || intHeading.MatchString(body) {
				problems = append(problems, rel+": only DEC sections belong in spec/DECISIONS.md")
			}
			for _, s := range sections(body, decHeading) {
				if _, dup := c.Decisions[s.id]; dup {
					problems = append(problems, rel+": duplicate decision "+s.id)
					continue
				}
				dec := Decision{ID: s.id, Title: s.title, Section: s.section, File: rel}
				m := statusLine.FindStringSubmatch(s.section)
				if m == nil {
					problems = append(problems, rel+": "+s.id+" is missing a `Status:` line (accepted, or superseded by DEC-NNN)")
				} else {
					status := strings.TrimSpace(m[1])
					switch {
					case strings.EqualFold(status, "accepted"):
						dec.Status = "accepted"
					case len(decRef.FindAllString(status, 1)) == 1 && strings.HasPrefix(strings.ToLower(status), "superseded by"):
						dec.Status = "superseded"
						dec.SupersededBy = decRef.FindString(status)
					default:
						problems = append(problems, rel+": "+s.id+" has invalid status "+status)
					}
				}
				c.Decisions[s.id] = dec
			}
		default:
			if intHeading.MatchString(body) {
				problems = append(problems, rel+": INT sections belong in spec/PRODUCT.md")
			}
			if decHeading.MatchString(body) {
				problems = append(problems, rel+": DEC sections belong in spec/DECISIONS.md")
			}
			for _, s := range sections(body, reqHeading) {
				if existing, dup := c.Requirements[s.id]; dup {
					problems = append(problems, rel+": duplicate requirement "+s.id+" (also in "+existing.File+")")
					continue
				}
				req := &Requirement{ID: s.id, Title: s.title, Section: s.section, File: rel,
					Gherkin: strings.Contains(s.section, "```gherkin")}
				if m := constraint.FindStringSubmatch(s.section); m != nil {
					req.Constraint = m[1]
				}
				if m := classLine.FindStringSubmatch(s.section); m == nil {
					problems = append(problems, rel+": "+s.id+" is missing a `Class:` line")
				} else if cl := Class(strings.ToLower(strings.TrimSpace(m[1]))); !validClass(cl) {
					problems = append(problems, rel+": "+s.id+" has unknown class "+m[1])
				} else {
					req.Class = cl
					if !req.Gherkin {
						msg := rel + ": " + s.id + " is missing a ```gherkin scenario block"
						if gherkinRequired(cl) {
							problems = append(problems, msg)
						} else {
							c.Warnings = append(c.Warnings, msg)
						}
					}
				}
				if m := motivatedBy.FindStringSubmatch(s.section); m != nil {
					req.MotivatedBy = unique(intRef.FindAllString(m[1], -1))
				}
				if len(req.MotivatedBy) == 0 {
					problems = append(problems, rel+": "+s.id+" is missing a `Motivated by: INT-NNN` line")
				}
				c.Requirements[s.id] = req
			}
		}
	}
	for _, id := range sortedReqIDs(c) {
		for _, ref := range c.Requirements[id].MotivatedBy {
			if _, ok := c.Intents[ref]; !ok {
				problems = append(problems, c.Requirements[id].File+": "+id+" is motivated by unknown intent "+ref)
			}
		}
	}
	for _, id := range sortedKeysDec(c.Decisions) {
		dec := c.Decisions[id]
		if dec.SupersededBy != "" {
			if _, ok := c.Decisions[dec.SupersededBy]; !ok {
				problems = append(problems, dec.File+": "+id+" is superseded by unknown decision "+dec.SupersededBy)
			}
		}
	}
	if len(c.Requirements) > 0 {
		if _, ok := files[ProductFile]; !ok {
			problems = append(problems, ProductFile+": missing; requirements need product intents to be motivated by")
		}
	}
	sort.Strings(problems)
	sort.Strings(c.Warnings)
	return c, problems
}

type section struct {
	id, title, section string
}

func sections(body string, heading *regexp.Regexp) []section {
	var out []section
	for _, m := range heading.FindAllStringSubmatchIndex(body, -1) {
		id := body[m[2]:m[3]]
		title := ""
		if m[4] >= 0 {
			title = strings.TrimSpace(body[m[4]:m[5]])
		}
		end := len(body)
		if next := anyHeading.FindStringIndex(body[m[1]:]); next != nil {
			end = m[1] + next[0]
		}
		out = append(out, section{id: id, title: title, section: body[m[0]:end]})
	}
	return out
}

func validClass(c Class) bool {
	for _, v := range Classes {
		if c == v {
			return true
		}
	}
	return false
}

func normalize(data []byte) []byte {
	s := strings.ReplaceAll(string(data), "\r\n", "\n")
	return []byte(strings.ReplaceAll(s, "\r", "\n"))
}

func unique(in []string) []string {
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

func sortedKeys(m map[string][]byte) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

func sortedReqIDs(c Contract) []string {
	out := make([]string, 0, len(c.Requirements))
	for id := range c.Requirements {
		out = append(out, id)
	}
	sort.Strings(out)
	return out
}

func sortedKeysDec(m map[string]Decision) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}
