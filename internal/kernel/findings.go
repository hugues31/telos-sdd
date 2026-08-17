package kernel

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/gitx"
)

// Finding severities and statuses.
const (
	SeverityInfo     = "info"
	SeverityMinor    = "minor"
	SeverityMajor    = "major"
	SeverityBlocking = "blocking"

	FindingOpen     = "open"
	FindingResolved = "resolved"
)

// FindingSource names who raised a finding.
type FindingSource struct {
	Kind string `json:"kind"` // critic|human|kernel
	Name string `json:"name"`
}

// FindingTarget names what a finding is about.
type FindingTarget struct {
	Requirements []string `json:"requirements,omitempty"`
	Paths        []string `json:"paths,omitempty"`
	Evidence     []string `json:"evidence,omitempty"`
}

// Resolution closes a finding with the taxonomy that makes the critic
// false-positive rate computable.
type Resolution struct {
	Kind        string `json:"kind"` // real|not_an_issue|duplicate
	By          string `json:"by"`
	DuplicateOf string `json:"duplicate_of,omitempty"`
	Note        string `json:"note,omitempty"`
}

// Finding is one entry of changes/CHG-NNN/findings.json. A critic only ever
// proposes a severity; the effective Severity is set by a human (or, from
// M6 on, by deterministic policy escalation). Only an open finding with
// effective severity "blocking" forbids certification (KERNEL-006).
type Finding struct {
	Schema           int            `json:"finding"`
	ID               string         `json:"id"`
	Change           string         `json:"change"`
	Source           FindingSource  `json:"source"`
	Target           FindingTarget  `json:"target"`
	ProposedSeverity string         `json:"proposed_severity"`
	Confidence       float64        `json:"confidence,omitempty"`
	Rationale        string         `json:"rationale"`
	Severity         string         `json:"severity,omitempty"`
	EscalatedBy      string         `json:"escalated_by,omitempty"`
	Status           string         `json:"status"`
	Resolution       *Resolution    `json:"resolution,omitempty"`
	CreatedAt        string         `json:"created_at"`
}

func findingsPath(wt *gitx.Repo, id string) string {
	return filepath.Join(wt.WorkDir, filepath.FromSlash(changeDir(id)), "findings.json")
}

// LoadFindings reads the candidate's findings.
func LoadFindings(wt *gitx.Repo, changeID string) ([]Finding, error) {
	data, err := os.ReadFile(findingsPath(wt, changeID))
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var findings []Finding
	if err := json.Unmarshal(data, &findings); err != nil {
		return nil, coded.New("TELOS_CHANGE_UNKNOWN", "findings.json does not parse: "+err.Error())
	}
	return findings, nil
}

func saveFindings(wt *gitx.Repo, changeID string, findings []Finding) error {
	data, err := json.MarshalIndent(findings, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(findingsPath(wt, changeID), append(data, '\n'), 0o644)
}

func validSeverity(s string) bool {
	switch s {
	case SeverityInfo, SeverityMinor, SeverityMajor, SeverityBlocking:
		return true
	}
	return false
}

// AddFinding appends a finding. A human source's proposed severity becomes
// effective immediately (the human IS the authority); a critic's stays
// proposed-only until confirmed.
func AddFinding(wt *gitx.Repo, f Finding) (*Finding, error) {
	doc, err := LoadChange(wt)
	if err != nil {
		return nil, err
	}
	if !validSeverity(f.ProposedSeverity) {
		return nil, coded.New("TELOS_INPUT_INVALID", "proposed severity must be info, minor, major, or blocking")
	}
	if f.Rationale == "" {
		return nil, coded.New("TELOS_INPUT_REQUIRED", "a finding needs a rationale")
	}
	findings, err := LoadFindings(wt, doc.ID)
	if err != nil {
		return nil, err
	}
	f.Schema = 1
	f.ID = fmt.Sprintf("FND-%03d", len(findings)+1)
	f.Change = doc.ID
	f.Status = FindingOpen
	f.CreatedAt = time.Now().UTC().Format(time.RFC3339)
	if f.Source.Kind == "human" {
		f.Severity = f.ProposedSeverity
		f.EscalatedBy = "human"
	}
	findings = append(findings, f)
	if err := saveFindings(wt, doc.ID, findings); err != nil {
		return nil, err
	}
	if _, err := wt.CommitAll("telos: finding " + f.ID); err != nil {
		return nil, err
	}
	return &f, nil
}

// ConfirmFinding makes a critic's proposed severity effective (human
// decision, guard-gated when blocking).
func ConfirmFinding(wt *gitx.Repo, findingID string) (*Finding, error) {
	return mutateFinding(wt, findingID, func(f *Finding) error {
		if f.Status != FindingOpen {
			return coded.New("TELOS_CHANGE_STATE_INVALID", findingID+" is not open")
		}
		f.Severity = f.ProposedSeverity
		f.EscalatedBy = "human"
		return nil
	})
}

// ResolveFinding closes a finding with its resolution taxonomy.
func ResolveFinding(wt *gitx.Repo, findingID, as, duplicateOf, note string) (*Finding, error) {
	switch as {
	case "real", "not_an_issue", "duplicate":
	default:
		return nil, coded.New("TELOS_INPUT_INVALID", "resolve --as takes real, not_an_issue, or duplicate")
	}
	if as == "duplicate" && duplicateOf == "" {
		return nil, coded.New("TELOS_INPUT_REQUIRED", "resolving as duplicate needs --of FND-NNN")
	}
	return mutateFinding(wt, findingID, func(f *Finding) error {
		if f.Status != FindingOpen {
			return coded.New("TELOS_CHANGE_STATE_INVALID", findingID+" is not open")
		}
		f.Status = FindingResolved
		f.Resolution = &Resolution{Kind: as, By: "human", DuplicateOf: duplicateOf, Note: note}
		return nil
	})
}

func mutateFinding(wt *gitx.Repo, findingID string, mutate func(*Finding) error) (*Finding, error) {
	doc, err := LoadChange(wt)
	if err != nil {
		return nil, err
	}
	findings, err := LoadFindings(wt, doc.ID)
	if err != nil {
		return nil, err
	}
	for i := range findings {
		if findings[i].ID == findingID {
			if err := mutate(&findings[i]); err != nil {
				return nil, err
			}
			if err := saveFindings(wt, doc.ID, findings); err != nil {
				return nil, err
			}
			if _, err := wt.CommitAll("telos: finding " + findingID + " updated"); err != nil {
				return nil, err
			}
			return &findings[i], nil
		}
	}
	return nil, coded.New("TELOS_NODE_NOT_FOUND", "no finding "+findingID)
}

// openBlocking lists the open findings whose effective severity blocks
// certification (KERNEL-006).
func openBlocking(findings []Finding) []string {
	var out []string
	for _, f := range findings {
		if f.Status == FindingOpen && f.Severity == SeverityBlocking {
			out = append(out, f.ID)
		}
	}
	return out
}

// openFindingIDs lists every open finding id (recorded in the certificate).
func openFindingIDs(findings []Finding) []string {
	out := []string{}
	for _, f := range findings {
		if f.Status == FindingOpen {
			out = append(out, f.ID)
		}
	}
	return out
}
