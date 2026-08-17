package kernel

import (
	"errors"
	"os"
	"path/filepath"

	"github.com/hugues31/telos-sdd/internal/contract"
	"github.com/hugues31/telos-sdd/internal/gitx"
)

// Project states.
const (
	StateCertified     = "certified"
	StateCorrupted     = "corrupted"
	StateUninitialized = "uninitialized"
)

// CertStatus summarizes the certificate of the current HEAD.
type CertStatus struct {
	Commit   string `json:"commit"`
	Change   string `json:"change"`
	SealedAt string `json:"sealed_at"`
}

// DirtyInfo lists the paths diverging from the certified state.
type DirtyInfo struct {
	Paths []string `json:"paths"`
}

// ContractCounts summarizes the certified contract.
type ContractCounts struct {
	Intents      int `json:"intents"`
	Requirements int `json:"requirements"`
	Decisions    int `json:"decisions"`
}

// CandidateStatus describes the Change of a candidate-context status.
type CandidateStatus struct {
	ID        string `json:"id"`
	Status    string `json:"status"`
	Category  string `json:"category"`
	Title     string `json:"title"`
	BaseStale bool   `json:"base_stale"`
	Review    string `json:"review,omitempty"`
	Worktree  string `json:"worktree"`
}

// SalvageProposal is the one-gesture conversion status offers on a corrupted
// root: capture the diff as a Change (new, or routed into an open one whose
// paths overlap).
type SalvageProposal struct {
	Proposal string `json:"proposal"` // new_change|into
	Into     string `json:"into,omitempty"`
	Prompt   string `json:"prompt"`
}

// ProjectStatus is the frozen result schema of `telos status`.
type ProjectStatus struct {
	Context     string           `json:"context"`
	State       string           `json:"state"`
	Certificate *CertStatus      `json:"certificate,omitempty"`
	Dirty       *DirtyInfo       `json:"dirty,omitempty"`
	Salvage     *SalvageProposal `json:"salvage,omitempty"`
	Reason      string           `json:"reason,omitempty"`
	Contract    *ContractCounts  `json:"contract,omitempty"`
	Changes     []ChangeSummary  `json:"changes,omitempty"`
	Change      *CandidateStatus `json:"change,omitempty"`
}

// Status derives the project state from the substrate. It never fails on
// business states — corruption is a status, not an error.
func Status(repo *gitx.Repo) (ProjectStatus, error) {
	if id, err := ChangeContext(repo); err != nil {
		return ProjectStatus{}, err
	} else if id != "" {
		return candidateStatus(repo)
	}
	st := ProjectStatus{Context: "root"}
	if _, err := os.Stat(filepath.Join(repo.WorkDir, ConfigFile)); err != nil {
		st.State = StateUninitialized
		return st, nil
	}
	head, err := repo.Head()
	if errors.Is(err, gitx.ErrNoCommits) {
		st.State = StateUninitialized
		st.Reason = "no commits; run `telos init`"
		return st, nil
	} else if err != nil {
		return st, err
	}

	dirty, err := repo.DirtyPaths()
	if err != nil {
		return st, err
	}

	cert, err := LoadCertificate(repo, head)
	certValid := err == nil
	if certValid {
		tree, terr := repo.TreeOf("HEAD")
		if terr != nil {
			return st, terr
		}
		certValid = cert.Validate(head, tree) == nil
	}

	switch {
	case !certValid:
		st.State = StateCorrupted
		st.Reason = "HEAD carries no valid certificate (out-of-band commit?)"
		if len(dirty) > 0 {
			st.Dirty = &DirtyInfo{Paths: dirty}
		}
	case len(dirty) > 0:
		st.State = StateCorrupted
		st.Reason = "worktree diverged from the certified state"
		st.Dirty = &DirtyInfo{Paths: dirty}
		st.Salvage = salvageProposal(repo, dirty)
	default:
		st.State = StateCertified
		st.Certificate = &CertStatus{Commit: string(head), Change: cert.Payload.Change.ID, SealedAt: cert.Payload.SealedAt}
		if files, ferr := contractFilesAt(repo, "HEAD"); ferr == nil {
			if parsed, problems := contract.Parse(files); len(problems) == 0 {
				st.Contract = &ContractCounts{Intents: len(parsed.Intents), Requirements: len(parsed.Requirements), Decisions: len(parsed.Decisions)}
			}
		}
	}
	if changes, cerr := OpenChanges(repo); cerr == nil {
		st.Changes = changes
	}
	return st, nil
}

// salvageProposal proposes the one-gesture capture: into an open change
// whose diff overlaps the dirty paths, or as a new change otherwise.
func salvageProposal(repo *gitx.Repo, dirty []string) *SalvageProposal {
	dirtySet := map[string]bool{}
	for _, p := range dirty {
		dirtySet[p] = true
	}
	if changes, err := OpenChanges(repo); err == nil {
		for _, c := range changes {
			if c.Base == "" {
				continue
			}
			if paths, err := repo.DiffNames(c.Base, "refs/heads/"+changeBranchPrefix+c.ID); err == nil {
				for _, p := range paths {
					if dirtySet[p] {
						return &SalvageProposal{Proposal: "into", Into: c.ID,
							Prompt: "Capture this edit into " + c.ID + "? (telos salvage --into " + c.ID + ")"}
					}
				}
			}
		}
	}
	next := "a new Change"
	if id, err := nextChangeID(repo); err == nil {
		next = id
	}
	return &SalvageProposal{Proposal: "new_change", Prompt: "Capture this edit as " + next + "? (telos salvage)"}
}

// candidateStatus reports the status seen from inside a Change's candidate
// worktree: the state of the certified target branch plus the Change itself.
func candidateStatus(repo *gitx.Repo) (ProjectStatus, error) {
	st := ProjectStatus{Context: "candidate"}
	doc, err := LoadChange(repo)
	if err != nil {
		return st, err
	}
	candidate := &CandidateStatus{
		ID:       doc.ID,
		Status:   doc.Status,
		Category: doc.Category,
		Title:    doc.Title,
		Worktree: repo.WorkDir,
	}
	if doc.Review != nil {
		candidate.Review = doc.Review.Digest
	}
	tip, err := repo.RevParse(doc.TargetBranch)
	if err != nil {
		st.State = StateCorrupted
		st.Reason = "the target branch " + doc.TargetBranch + " no longer resolves"
		st.Change = candidate
		return st, nil
	}
	candidate.BaseStale = string(tip) != doc.Base

	cert, err := LoadCertificate(repo, tip)
	certValid := err == nil
	if certValid {
		tree, terr := repo.TreeOf(string(tip))
		if terr != nil {
			return st, terr
		}
		certValid = cert.Validate(tip, tree) == nil
	}
	if certValid {
		st.State = StateCertified
		st.Certificate = &CertStatus{Commit: string(tip), Change: cert.Payload.Change.ID, SealedAt: cert.Payload.SealedAt}
	} else {
		st.State = StateCorrupted
		st.Reason = "the target branch tip carries no valid certificate"
	}
	st.Change = candidate
	return st, nil
}
