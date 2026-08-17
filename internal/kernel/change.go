package kernel

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/evidence"
	"github.com/hugues31/telos-sdd/internal/gitx"
)

// Change lifecycle statuses.
const (
	ChangeDrafting         = "drafting"
	ChangeAwaitingApproval = "awaiting_approval"
	ChangeApproved         = "approved"
	ChangeReady            = "ready"
	ChangePromoted         = "promoted"
)

// changeBranchPrefix names candidate branches: telos/CHG-NNN.
const changeBranchPrefix = "telos/"

// PendingReview records the digest presented to the human, awaiting approval.
type PendingReview struct {
	Kind   string `json:"kind"` // contract|preserving_claim
	Digest string `json:"digest"`
}

// ChangeDoc is changes/CHG-NNN/change.json — the committed identity of a
// Change. It lives in the candidate worktree and is kernel-owned protected
// content there.
type ChangeDoc struct {
	Schema         int                            `json:"change"`
	ID             string                         `json:"id"`
	Category       string                         `json:"category"`
	Title          string                         `json:"title"`
	Base           string                         `json:"base"`
	TargetBranch   string                         `json:"target_branch"`
	Branch         string                         `json:"branch"`
	Status         string                         `json:"status"`
	Approvals      []Approval                     `json:"approvals"`
	Privileged     bool                           `json:"privileged"`
	CreatedAt      string                         `json:"created_at"`
	PromotedCommit string                         `json:"promoted_commit,omitempty"`
	Review         *PendingReview                 `json:"review,omitempty"`
	RedWitnesses   map[string]evidence.RedWitness `json:"red_witnesses,omitempty"`
}

var changeIDPattern = regexp.MustCompile(`^CHG-([0-9]{3,})$`)

func changeDir(id string) string { return "changes/" + id }

// ChangeContext reports the candidate change id when repo's worktree is a
// candidate (branch telos/CHG-NNN), or "" in the certified root.
func ChangeContext(repo *gitx.Repo) (string, error) {
	branch, err := repo.CurrentBranch()
	if err != nil {
		return "", err
	}
	if id, ok := strings.CutPrefix(branch, changeBranchPrefix); ok && changeIDPattern.MatchString(id) {
		return id, nil
	}
	return "", nil
}

// LoadChange reads the candidate's ChangeDoc from the worktree.
func LoadChange(repo *gitx.Repo) (*ChangeDoc, error) {
	id, err := ChangeContext(repo)
	if err != nil {
		return nil, err
	}
	if id == "" {
		return nil, coded.New("TELOS_CANDIDATE_REQUIRED", "this command runs inside a Change's candidate worktree")
	}
	data, err := os.ReadFile(filepath.Join(repo.WorkDir, filepath.FromSlash(changeDir(id)), "change.json"))
	if err != nil {
		return nil, coded.New("TELOS_CHANGE_UNKNOWN", id+" has no change.json in this worktree")
	}
	var doc ChangeDoc
	if err := json.Unmarshal(data, &doc); err != nil {
		return nil, coded.New("TELOS_CHANGE_UNKNOWN", id+": change.json does not parse: "+err.Error())
	}
	if doc.ID != id {
		return nil, coded.New("TELOS_CHANGE_UNKNOWN", "change.json names "+doc.ID+" but the branch is "+id)
	}
	return &doc, nil
}

func saveChange(repo *gitx.Repo, doc *ChangeDoc) error {
	data, err := json.MarshalIndent(doc, "", "  ")
	if err != nil {
		return err
	}
	path := filepath.Join(repo.WorkDir, filepath.FromSlash(changeDir(doc.ID)), "change.json")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	return os.WriteFile(path, append(data, '\n'), 0o644)
}

const intentTemplate = `# Intent

Describe why this change is desired: motivation, desired outcome, context.
New INT-* sections declared here enter the contract through the delta.
`

const deltaTemplate = `<!-- Describe the contract delta with telos:op markers:

<!- telos:op add file: spec/<domain>.md ->
### REQ-NNN — Title
Class: behavior
Motivated by: INT-NNN

...prose and a gherkin scenario block...

Other operations: "replace file:" swaps a section by id, "remove id:" deletes
one. A behavior-preserving change leaves this file empty. -->
`

const decisionsTemplate = `<!-- DEC-* sections recorded here are folded into spec/DECISIONS.md at
promotion. -->
`

// StartChange opens a new Change from the certified state: allocates the id,
// creates the candidate worktree on branch telos/CHG-NNN, scaffolds and
// commits the Change record.
func StartChange(repo *gitx.Repo, category, title string) (*ChangeDoc, string, error) {
	if ctx, err := ChangeContext(repo); err != nil {
		return nil, "", err
	} else if ctx != "" {
		return nil, "", coded.New("TELOS_ROOT_REQUIRED", "change start runs in the certified root, not a candidate")
	}
	switch category {
	case CategoryBehaviorChange, CategoryBehaviorPreserving:
	default:
		return nil, "", coded.New("TELOS_INPUT_INVALID", fmt.Sprintf("category must be %s or %s", CategoryBehaviorChange, CategoryBehaviorPreserving))
	}
	st, err := Status(repo)
	if err != nil {
		return nil, "", err
	}
	switch st.State {
	case StateCertified:
	case StateUninitialized:
		return nil, "", coded.New("TELOS_NOT_INITIALIZED", "run `telos init` first")
	default:
		return nil, "", coded.New("TELOS_STATE_CORRUPTED", "the certified worktree diverged; salvage or restore before starting a Change")
	}

	head, err := repo.Head()
	if err != nil {
		return nil, "", err
	}
	branch, err := repo.CurrentBranch()
	if err != nil {
		return nil, "", err
	}
	if branch == "" {
		return nil, "", coded.New("TELOS_STATE_CORRUPTED", "the certified root is on a detached HEAD")
	}
	id, err := nextChangeID(repo)
	if err != nil {
		return nil, "", err
	}
	path := repo.SiblingWorktreePath(id)
	if err := repo.WorktreeAdd(path, changeBranchPrefix+id, head); err != nil {
		return nil, "", err
	}
	wt, err := gitx.Open(path)
	if err != nil {
		return nil, "", err
	}
	doc := &ChangeDoc{
		Schema:       1,
		ID:           id,
		Category:     category,
		Title:        title,
		Base:         string(head),
		TargetBranch: branch,
		Branch:       changeBranchPrefix + id,
		Status:       ChangeDrafting,
		Approvals:    []Approval{},
		CreatedAt:    time.Now().UTC().Format(time.RFC3339),
	}
	if err := saveChange(wt, doc); err != nil {
		return nil, "", err
	}
	dir := filepath.Join(path, filepath.FromSlash(changeDir(id)))
	for name, content := range map[string]string{
		"intent.md":         intentTemplate,
		"contract.delta.md": deltaTemplate,
		"decisions.md":      decisionsTemplate,
		"findings.json":     "[]\n",
	} {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(content), 0o644); err != nil {
			return nil, "", err
		}
	}
	if _, err := wt.CommitAll("telos: start " + id); err != nil {
		return nil, "", err
	}
	return doc, path, nil
}

// nextChangeID allocates the next CHG-NNN across retained change records,
// candidate branches, and worktrees.
func nextChangeID(repo *gitx.Repo) (string, error) {
	max := 0
	consider := func(id string) {
		if m := changeIDPattern.FindStringSubmatch(id); m != nil {
			if n, err := strconv.Atoi(m[1]); err == nil && n > max {
				max = n
			}
		}
	}
	if files, err := repo.LsTree("HEAD"); err == nil {
		for path := range files {
			if rest, ok := strings.CutPrefix(path, "changes/"); ok {
				if i := strings.IndexByte(rest, '/'); i > 0 {
					consider(rest[:i])
				}
			}
		}
	}
	branches, err := repo.Branches(changeBranchPrefix + "CHG-*")
	if err != nil {
		return "", err
	}
	for _, b := range branches {
		consider(strings.TrimPrefix(b, changeBranchPrefix))
	}
	return fmt.Sprintf("CHG-%03d", max+1), nil
}

// ChangeSummary is one open change as listed by the root status.
type ChangeSummary struct {
	ID        string `json:"id"`
	Status    string `json:"status"`
	Category  string `json:"category"`
	Title     string `json:"title"`
	BaseStale bool   `json:"base_stale"`
	Worktree  string `json:"worktree"`
}

// OpenChanges lists the open candidates (branches telos/CHG-*), reading each
// change.json from its branch tip.
func OpenChanges(repo *gitx.Repo) ([]ChangeSummary, error) {
	branches, err := repo.Branches(changeBranchPrefix + "CHG-*")
	if err != nil {
		return nil, err
	}
	worktrees, err := repo.WorktreeList()
	if err != nil {
		return nil, err
	}
	byBranch := map[string]string{}
	for _, info := range worktrees {
		byBranch[info.Branch] = info.Path
	}
	var out []ChangeSummary
	for _, branch := range branches {
		id := strings.TrimPrefix(branch, changeBranchPrefix)
		summary := ChangeSummary{ID: id, Worktree: byBranch[branch]}
		if data, err := repo.BlobAt(branch, changeDir(id)+"/change.json"); err == nil {
			var doc ChangeDoc
			if json.Unmarshal(data, &doc) == nil {
				summary.Status = doc.Status
				summary.Category = doc.Category
				summary.Title = doc.Title
				if tip, err := repo.RevParse(doc.TargetBranch); err == nil {
					summary.BaseStale = string(tip) != doc.Base
				}
			}
		}
		out = append(out, summary)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out, nil
}

// AbortChange removes a candidate worktree and its branch. Destructive and
// guard-gated at the CLI.
func AbortChange(repo *gitx.Repo, id string) error {
	if !changeIDPattern.MatchString(id) {
		return coded.New("TELOS_INPUT_INVALID", "abort takes a CHG-NNN id")
	}
	branch := changeBranchPrefix + id
	branches, err := repo.Branches(branch)
	if err != nil {
		return err
	}
	if len(branches) == 0 {
		return coded.New("TELOS_CHANGE_UNKNOWN", "no open change "+id)
	}
	worktrees, err := repo.WorktreeList()
	if err != nil {
		return err
	}
	for _, info := range worktrees {
		if info.Branch == branch {
			if err := repo.WorktreeRemove(info.Path); err != nil {
				return err
			}
		}
	}
	return repo.BranchDelete(branch)
}
